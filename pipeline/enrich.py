"""Étiquettes d'agrément ajoutées aux dalles `tiles/1` DÉJÀ construites (post-traitement, aucune requête IGN).

Chaque `.npz` du dossier est réécrit avec des colonnes en plus (les colonnes existantes sont recopiées
telles quelles), le manifeste reçoit sha256/tailles, `columns` (source et licence par colonne),
`derived_from` et une `data_version` dérivée `<base>.<n>`. Contrat : .team/contracts/tiles.md § Étiquettes.

- `calm` (uint8, 0–15) : éloignement moyen de la route importante la plus proche × nature propre du
  tronçon. Calculé depuis les seules dalles (la dalle et ses 8 voisines), donc sans licence nouvelle.
- `osm_hike` (uint8, 0 aucun / 1 balisé / 2 balisé régional ou plus), chemins seulement, et
  `osm_water` (uint8, 0–15 = part de la longueur à moins de 50 m de l'eau) : seulement avec `--osm`.
  © les contributeurs d'OpenStreetMap, ODbL. Étiquettes seulement : aucune géométrie OSM n'entre dans
  la dalle, et l'absence d'étiquette est neutre (0 = « rien de connu », jamais une pénalité).

Chaque valeur ne dépend que du tronçon et de ce qui l'entoure à 300 m au plus : le résultat est le même
quel que soit l'ordre ou le lot de traitement, pourvu que les dalles voisines soient dans le dossier.
"""
from __future__ import annotations

import datetime
import functools
import hashlib
import io
import json
import re
import resource
import subprocess
import sys
import time
from array import array
from pathlib import Path

import numpy as np
from scipy.spatial import cKDTree

from trailopt import cache

from .build import ZONES, split_key, totals, transformers
from .load import FORMAT, TILE_M, _undelta

CALM_MAX_M = 300.0         # au-delà, la route ne s'entend plus : note pleine
HIKE_BUF_M = 15.0          # écart toléré entre le tracé IGN et le tracé OSM du même chemin
HIKE_COS = 0.8             # ... et directions compatibles (|cos|)
HIKE_FRAC = 0.6            # part de la longueur appariée pour poser l'étiquette
WATER_M = 50.0             # « au bord de l'eau »
WATER_MIN_AREA_M2 = 5000.0  # plan d'eau d'un seul tenant plus petit (mare, bassin) : ignoré
ROADS = ("Route à 1 chaussée", "Route à 2 chaussées", "Rond-point", "Bretelle")
PATHS = ("Sentier", "Chemin", "Route empierrée", "Escalier", "Piste cyclable")
MOTORWAY = "Type autoroutier"
ADDED = ("calm", "osm_hike", "osm_water")
IGN = dict(source="dérivé des dalles (BD TOPO® IGN)", license="Licence Ouverte Etalab 2.0")
OSM = dict(source="© les contributeurs d'OpenStreetMap", license="ODbL 1.0")
COLUMNS = {
    "calm": dict(dtype="uint8", range=[0, 15], **IGN),
    "osm_hike": dict(dtype="uint8", range=[0, 2], **OSM),
    "osm_water": dict(dtype="uint8", range=[0, 15], **OSM),
}
# Extrait OSM réduit à ce qui sert (relations de randonnée et leurs ways, eau) : `osmium tags-filter`.
OSM_FILTER = ("r/route=hiking,foot", "w/waterway=river,canal", "wr/natural=water", "w/natural=coastline")
METRO = (-6.0, 41.0, 10.5, 51.6)   # ouest, sud, est, nord : ways OSM projetés en Lambert-93


def _points(x, y, n, step, sel=None):
    """Milieux de pas d'au plus `step` m le long de chaque ligne (x, y concaténés, n sommets par ligne).
    Renvoie px, py, ux, uy (direction unitaire), rang de la ligne, longueur du pas."""
    n = np.asarray(n, np.int64)
    sx, sy = np.diff(x), np.diff(y)
    L = np.hypot(sx, sy)
    owner = np.repeat(np.arange(len(n)), n)[:-1]
    k = np.maximum(1, np.ceil(L / step)).astype(np.int64)
    k[np.cumsum(n)[:-1] - 1] = 0                   # pas de segment entre deux lignes
    if sel is not None:
        k[~sel[owner]] = 0
    seg = np.repeat(np.arange(len(L)), k)
    r = (np.arange(len(seg)) - np.repeat(np.cumsum(k) - k, k) + 0.5) / k[seg]
    Ls = np.maximum(L[seg], 1e-9)
    return x[seg] + r * sx[seg], y[seg] + r * sy[seg], sx[seg] / Ls, sy[seg] / Ls, owner[seg], L[seg] / k[seg]


def _share(owner, w, mask, n):
    """Part de la longueur de chaque ligne dont les pas vérifient `mask`."""
    return np.bincount(owner, weights=w * mask, minlength=n) / np.maximum(np.bincount(owner, weights=w, minlength=n), 1e-9)


def _geom(path: Path, ix: int, iy: int) -> dict:
    """Ce qu'il faut d'une dalle pour l'étiqueter : sommets absolus (m), nature, importance."""
    with np.load(path) as f:
        n = f["geom_n"].astype(np.int64)
        return dict(n=n, x=_undelta(f["geom_x"], n, ix * TILE_M * 10) / 10.0,
                    y=_undelta(f["geom_y"], n, iy * TILE_M * 10) / 10.0,
                    nature=f["nature"], importance=f["importance"])


def _codes(natures, names):
    return [natures.index(k) for k in names if k in natures]


@functools.lru_cache(maxsize=512)
def _major(d: Path, key: str, natures: tuple) -> np.ndarray:
    """Points (pas de 20 m) des routes importantes d'une dalle : carrossables d'importance 1 à 3, ou
    type autoroutier. Dalle absente du dossier = aucune voie (mer, étranger)."""
    p = d / f"{key}.npz"
    if not p.exists():
        return np.zeros((0, 2))
    _, ix, iy = split_key(key)
    g = _geom(p, ix, iy)
    sel = (np.isin(g["nature"], _codes(natures, ROADS)) & np.isin(g["importance"], (1, 2, 3))) \
        | np.isin(g["nature"], _codes(natures, [MOTORWAY]))
    px, py, *_ = _points(g["x"], g["y"], g["n"], 20.0, sel)
    return np.column_stack([px, py])


def calm(d: Path, key: str, natures) -> np.ndarray:
    """round(15 × min(distance moyenne à la route importante la plus proche / 300 m, 1) × propre), où
    propre = 0,6 sur une route d'importance 4, 1 sinon. Une route importante est à distance nulle
    d'elle-même : importance 1 à 3 et autoroutes valent donc 0."""
    d, natures = Path(d), tuple(natures)
    zone, ix, iy = split_key(key)
    pre = "" if zone == "fxx" else f"{zone}/"
    g = _geom(d / f"{key}.npz", ix, iy)
    ns = len(g["n"])
    Q = np.vstack([_major(d, f"{pre}{i}_{j}", natures) for i in (ix - 1, ix, ix + 1) for j in (iy - 1, iy, iy + 1)])
    px, py, _, _, ow, w = _points(g["x"], g["y"], g["n"], 10.0)
    dist = np.full(len(px), CALM_MAX_M)
    if len(Q) and len(px):
        dist = np.minimum(cKDTree(Q).query(np.column_stack([px, py]), distance_upper_bound=CALM_MAX_M)[0], CALM_MAX_M)
    own = np.where(np.isin(g["nature"], _codes(natures, ROADS)) & (g["importance"] == 4), 0.6, 1.0)
    return np.round(15 * _share(ow, w, dist / CALM_MAX_M, ns) * own).astype(np.uint8)


# ---------------------------------------------------------------------------------------------- OSM

def filter_pbf(pbf: Path) -> Path:
    """Extrait .osm.pbf (Geofabrik) -> texte OPL réduit à OSM_FILTER, ways porteurs de leurs coordonnées.
    Demande `osmium` (paquet osmium-tool) ; le résultat est gardé à côté de l'extrait."""
    out, tmp = pbf.with_suffix(".opl"), pbf.with_suffix(".filtre.pbf")
    if not out.exists() or out.stat().st_mtime < pbf.stat().st_mtime:
        subprocess.run(["osmium", "tags-filter", "-O", "-o", str(tmp), str(pbf), *OSM_FILTER], check=True)
        subprocess.run(["osmium", "add-locations-to-ways", "-O", "-f", "opl,add_metadata=false",
                        "-o", str(out), str(tmp)], check=True)
        tmp.unlink()
    return out


_XY = re.compile(r"x(-?[0-9.]+)y(-?[0-9.]+)")
_LEVEL = {"rwn": 2, "nwn": 2, "iwn": 2}


def read_osm(paths) -> dict:
    """Ways utiles des fichiers OPL (ou .osm.pbf, filtrés au passage) : lon, lat concaténés, n sommets
    par way, `hike` (0, 1 balisé, 2 régional ou plus) et `water` (0, 1 rivière, canal, côte ou
    contour de multipolygone d'eau, 3 plan d'eau d'un seul tenant, à trier par surface)."""
    lon, lat, n, wid, water = array("d"), array("d"), array("i"), array("q"), array("b")
    hike, lake, rel_ways, supers = {}, set(), {}, []   # way -> niveau ; ways de contour d'eau ; relations
    for path in map(Path, paths):
        if path.suffix == ".pbf":
            path = filter_pbf(path)
        with open(path, encoding="utf-8") as f:
            for line in f:
                kind = line[0]
                if kind not in "wr":
                    continue
                ident = int(line[1:line.index(" ")])
                fld = {p[0]: p[1:] for p in line.rstrip("\n").split(" ")[1:] if p}
                tags = dict(kv.split("=", 1) for kv in fld.get("T", "").split(",") if "=" in kv)
                if kind == "w":
                    xy = _XY.findall(fld.get("N", ""))
                    if len(xy) < 2:
                        continue
                    c = 0
                    if tags.get("waterway") in ("river", "canal"):      # pas `stream` : en montagne tout sentier en croise
                        c = int(tags.get("intermittent") != "yes" and tags.get("tunnel", "no") == "no")
                    elif tags.get("natural") == "coastline":
                        c = 1
                    elif tags.get("natural") == "water" and tags.get("intermittent") != "yes" \
                            and tags.get("water") not in ("basin", "wastewater"):
                        c = 3
                    wid.append(ident)
                    water.append(c)
                    n.append(len(xy))
                    lon.extend(float(a) for a, _ in xy)
                    lat.extend(float(b) for _, b in xy)
                    continue
                mem = [m.split("@")[0] for m in fld.get("M", "").split(",")]
                rel_ways[ident] = ways = [int(m[1:]) for m in mem if m[:1] == "w"]
                if tags.get("route") in ("hiking", "foot"):
                    lv = _LEVEL.get(tags.get("network"), 1)
                    for i in ways:
                        hike[i] = max(hike.get(i, 0), lv)
                    supers += [(int(m[1:]), lv) for m in mem if m[:1] == "r"]
                elif tags.get("natural") == "water" and tags.get("intermittent") != "yes":
                    lake.update(ways)
    for rid, lv in supers:      # un GR est souvent une relation de relations : le niveau descend sur les étapes
        for i in rel_ways.get(rid, ()):
            hike[i] = max(hike.get(i, 0), lv)
    wid_a, wat = np.array(wid, np.int64), np.array(water, np.uint8)
    hk = np.array([hike.get(i, 0) for i in wid], np.uint8)
    wat[(wat == 0) & np.isin(wid_a, np.fromiter(lake, np.int64, len(lake)))] = 1
    return dict(lon=np.array(lon), lat=np.array(lat), n=np.array(n, np.int64), hike=hk, water=wat)


def _project(osm: dict, zone: str) -> dict:
    """Ways de la zone dans son repère métrique, avec leur emprise ; petits plans d'eau écartés."""
    n = osm["n"]
    first = np.cumsum(n) - n
    w, s, e, nn = ZONES[zone].get("bbox", METRO)
    lo, la = osm["lon"][first], osm["lat"][first]
    keep = (lo >= w) & (lo <= e) & (la >= s) & (la <= nn)
    pts = np.repeat(keep, n)
    n = n[keep]
    x, y = map(np.asarray, transformers(zone)[1].transform(osm["lon"][pts], osm["lat"][pts]))
    hike, water = osm["hike"][keep], osm["water"][keep].copy()
    if len(n):
        owner = np.repeat(np.arange(len(n)), n)
        # surface du contour fermé (lacet), coordonnées ramenées au 1er sommet pour la précision
        fx, fy = x - np.repeat(x[np.cumsum(n) - n], n), y - np.repeat(y[np.cumsum(n) - n], n)
        cross = fx[:-1] * fy[1:] - fx[1:] * fy[:-1]
        cross[np.cumsum(n)[:-1] - 1] = 0.0
        area = np.abs(np.bincount(owner[:-1], weights=cross, minlength=len(n))) / 2
        water[(water == 3) & (area < WATER_MIN_AREA_M2)] = 0
        water[water == 3] = 1
        ends = np.cumsum(n)
        box = [f.reduceat(v, ends - n) for v in (x, y) for f in (np.minimum, np.maximum)]
    else:
        box = [np.zeros(0)] * 4
    return dict(x=x, y=y, n=n, hike=hike, water=water, box=box)


def _pick(W: dict, sel) -> tuple:
    pts = np.repeat(sel, W["n"])
    return W["x"][pts], W["y"][pts], W["n"][sel]


def osm_labels(g: dict, W: dict, natures) -> tuple[np.ndarray, np.ndarray]:
    """(osm_hike, osm_water) des tronçons `g` d'une dalle, d'après les ways OSM projetés `W`."""
    ns = len(g["n"])
    hike, water = np.zeros(ns, np.uint8), np.zeros(ns, np.uint8)
    if not ns or not len(W["n"]):
        return hike, water
    m = WATER_M + 10
    near = (W["box"][1] >= g["x"].min() - m) & (W["box"][0] <= g["x"].max() + m) \
        & (W["box"][3] >= g["y"].min() - m) & (W["box"][2] <= g["y"].max() + m)
    # Balisage : points tous les 5 m des deux côtés, plus proche point OSM de même direction à 15 m.
    hw = near & (W["hike"] > 0)
    path = np.isin(g["nature"], _codes(list(natures), PATHS))
    if hw.any() and path.any():
        ox, oy, oux, ouy, oo, _ = _points(*_pick(W, hw), 5.0)
        px, py, ux, uy, ow, w = _points(g["x"], g["y"], g["n"], 5.0, path)
        D, J = cKDTree(np.column_stack([ox, oy])).query(np.column_stack([px, py]), k=min(6, len(ox)),
                                                        distance_upper_bound=HIKE_BUF_M)
        D, J = D.reshape(len(px), -1), J.reshape(len(px), -1)
        Jc = np.where(np.isfinite(D), J, 0)
        ok = np.isfinite(D) & (np.abs(oux[Jc] * ux[:, None] + ouy[Jc] * uy[:, None]) >= HIKE_COS)
        lv = np.where(ok, W["hike"][hw][oo[Jc]], 0).max(axis=1)
        hike[_share(ow, w, lv >= 1, ns) >= HIKE_FRAC] = 1
        hike[_share(ow, w, lv >= 2, ns) >= HIKE_FRAC] = 2
        hike[~path] = 0
    # Eau : part de la longueur à moins de 50 m d'une rivière, d'un canal, d'un plan d'eau ou de la côte.
    ww = near & (W["water"] == 1)
    if ww.any():
        ox, oy, *_ = _points(*_pick(W, ww), 10.0)
        px, py, _, _, ow, w = _points(g["x"], g["y"], g["n"], 10.0)
        D, _ = cKDTree(np.column_stack([ox, oy])).query(np.column_stack([px, py]), distance_upper_bound=WATER_M)
        water = np.round(15 * _share(ow, w, np.isfinite(D), ns)).astype(np.uint8)
    return hike, water


# ----------------------------------------------------------------------------------------- dossier

def derived_version(v: str) -> str:
    """'bdtopo-wfs-2026-10e' -> '…-10e.1' ; '…-10e.1' -> '…-10e.2'."""
    base, _, k = v.partition(".")
    return f"{base}.{int(k or 0) + 1}"


def enrich(tiles_dir, osm_paths=(), version: str | None = None, log=print) -> dict:
    d = Path(tiles_dir)
    m = json.loads((d / "manifest.json").read_text())
    if m.get("format") != FORMAT:
        raise RuntimeError(f"format {m.get('format')}, attendu {FORMAT}")
    natures = m["natures"]
    _major.cache_clear()
    t0 = time.time()
    osm = read_osm(osm_paths) if osm_paths else None
    if osm:
        log(f"OSM : {len(osm['n'])} ways, {len(osm['lon'])} sommets, {int((osm['hike'] > 0).sum())} balisés, "
            f"{int((osm['water'] > 0).sum())} d'eau, lus en {time.time() - t0:.0f} s")
    W = {}
    cols = ["calm"] + (["osm_hike", "osm_water"] if osm else [])
    for key, info in sorted(m["tiles"].items()):
        t = time.time()
        zone, ix, iy = split_key(key)
        p = d / f"{key}.npz"
        with np.load(p) as f:
            T = {k: f[k] for k in f.files if k not in ADDED}
        T["calm"] = calm(d, key, natures)
        if osm:
            if zone not in W:
                W[zone] = _project(osm, zone)
            g = _geom(p, ix, iy)
            T["osm_hike"], T["osm_water"] = osm_labels(g, W[zone], natures)
        buf = io.BytesIO()
        np.savez_compressed(buf, **T)
        data = buf.getvalue()
        cache.write_atomic(p, data)
        km = T["len_dm"] / 1e4
        info.update(sha256=hashlib.sha256(data).hexdigest(), bytes=len(data),
                    o_par_troncon=round(len(data) / max(1, info["n"]), 1),
                    calm_km=round(float((km * T["calm"]).sum() / 15), 1))
        for k in ("path_km", "osm_hike_km", "osm_water_km"):
            info.pop(k, None)
        if osm:
            path = np.isin(T["nature"], _codes(natures, PATHS))
            info.update(path_km=round(float(km[path].sum()), 1), osm_hike_km=round(float(km[T["osm_hike"] > 0].sum()), 1),
                        osm_water_km=round(float((km * T["osm_water"]).sum() / 15), 1))
        log(f"{key} : {info['n']} tronçons, {len(data)} octets, {time.time() - t:.1f} s")
    m["derived_from"] = m.get("derived_from") or m["data_version"]
    m["data_version"] = version or derived_version(m["data_version"])
    m["columns"] = {k: COLUMNS[k] for k in cols}
    m["enriched"] = dict(date=datetime.date.today().isoformat(), osm=sorted(Path(p).name for p in osm_paths))
    m["totals"] = totals(m["tiles"])
    for k in ("calm_km", "path_km", "osm_hike_km", "osm_water_km"):
        if any(k in t for t in m["tiles"].values()):
            m["totals"][k] = round(sum(t.get(k, 0) for t in m["tiles"].values()), 1)
    cache.write_atomic(d / "manifest.json", json.dumps(m, ensure_ascii=False, indent=1).encode())
    rss = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / (1e6 if sys.platform == "darwin" else 1e3)
    log(f"{len(m['tiles'])} dalles, version {m['data_version']} (depuis {m['derived_from']}), {time.time() - t0:.0f} s, "
        f"mémoire max {rss:.0f} Mo (hors osmium)")
    return m


def verify(tiles_dir, m: dict, skip=()) -> list[str]:
    """Contrôle des colonnes annoncées par `columns` : présentes, de la taille de la dalle, dans leurs
    bornes ; `calm` recalculé depuis le dossier = `calm` stocké (une dalle étiquetée sans ses voisines,
    ou des voisines changées depuis, donnent un écart près des bords)."""
    d, bad = Path(tiles_dir), []
    _major.cache_clear()
    cols = m.get("columns") or {}
    if not cols:
        return bad                      # dossier non enrichi : rien à contrôler
    for key in sorted(k for k in m.get("tiles", {}) if k not in skip):
        with np.load(d / f"{key}.npz") as f:
            n = len(f["len_dm"])
            got = {k: f[k] for k in cols if k in f.files}
            path = np.isin(f["nature"], _codes(m["natures"], PATHS))
        for k, c in cols.items():
            a = got.get(k)
            if a is None or len(a) != n or str(a.dtype) != c["dtype"]:
                bad.append(f"{key} : colonne {k} absente, de mauvaise taille ou de mauvais type")
            elif n and int(a.max()) > c["range"][1]:
                bad.append(f"{key} : {k} = {int(a.max())} hors bornes {c['range']}")
        if "osm_hike" in got and len(got["osm_hike"]) == n and got["osm_hike"][~path].any():
            bad.append(f"{key} : osm_hike posé hors chemin")
        if "calm" in got and len(got["calm"]) == n:
            dif = int((calm(d, key, m["natures"]) != got["calm"]).sum())
            if dif:
                bad.append(f"{key} : calm différent du recalcul sur {dif} tronçons (dalles voisines absentes ou changées ?)")
    return bad
