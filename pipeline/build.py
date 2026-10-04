"""Construction des dalles `tiles/1` depuis la BD TOPO (WFS) et le MNT (WMS-R), via trailopt.

Sources et cache : ceux de `trailopt.ign` (tronçons) et `trailopt.elevation` (LiDAR HD puis
RGE ALTI, `mask_unreliable` compris). Rien n'est recalculé si le cache est chaud.
"""
from __future__ import annotations

import fcntl
import hashlib
import io
import json
import time
from pathlib import Path

import numpy as np
from pyproj import Transformer

from trailopt import cache, elevation, graph, ign

from .load import FORMAT, TILE_M, lengths, profile_counts, profile_points

MARGIN_M = 2000.0          # voisins chargés pour les parallèles en bord de dalle
DATA_VERSION = "bdtopo-wfs-2026-10b"  # b : altitudes corrigées (falaises, portails de tunnel, D27)
SOURCE = "BD TOPO® IGN, RGE ALTI®, LiDAR HD — Etalab 2.0"
STEEP = 0.60               # validation : pente max > 60 % (artefact probable)
ROAD_MAX_GRADE = 0.30     # pente physique max d'une voie carrossable, par pas de 5 m (D27)
ROAD_NATURES = [ign.NATURES.index(k) for k in sorted(ign.ROADS | {"Route empierrée", "Type autoroutier"})]
JUMP_DM = 100              # validation : saut > 10 m entre deux points de 5 m (200 %, pic probable)
_TO_WGS = Transformer.from_crs("EPSG:2154", "EPSG:4326", always_xy=True)
_TO_L93 = Transformer.from_crs("EPSG:4326", "EPSG:2154", always_xy=True)


def _deltas(a, n, base=0):
    """Valeurs absolues -> 1er élément de chaque tronçon : a - base, suivants : delta au précédent."""
    a = np.asarray(a, np.int64)
    d = np.diff(a, prepend=0)
    first = np.cumsum(n) - n
    d[first] = a[first] - base
    return d


def tile_columns(ix: int, iy: int, A: dict, Xd, Yd, sample):
    """A : tronçons au format `trailopt.ign.fetch` (dalle + marge), Xd, Yd : leurs sommets L93 en
    dm (int64), sample(X, Y) -> z (m, NaN sans donnée). Renvoie (colonnes du .npz, validations)."""
    n = A["n"].astype(np.int64)
    off = np.concatenate([[0], np.cumsum(n)])
    first = off[:-1]
    ident = np.array([int(s[8:]) if s[:8].isalpha() and s[8:].isdigit() else -1
                      for s in np.char.decode(A["ident"].astype("S24"))], np.int64)
    len_dm = np.round(lengths(Xd / 10.0, Yd / 10.0, n) * 10).astype(np.int64)
    valid = (n >= 2) & (ident >= 0) & (len_dm >= 1)
    x0, y0 = ix * TILE_M * 10, iy * TILE_M * 10
    inside = valid & (Xd[first] >= x0) & (Xd[first] < x0 + TILE_M * 10) \
        & (Yd[first] >= y0) & (Yd[first] < y0 + TILE_M * 10)

    # Profils (repère : origine de dalle, en m) de tous les tronçons utiles, pour les parallèles.
    work = np.flatnonzero(valid)
    sel = np.concatenate([np.arange(off[i], off[i + 1]) for i in work] + [np.zeros(0, np.int64)])
    px, py = profile_points((Xd[sel] - x0) / 10.0, (Yd[sel] - y0) / 10.0, n[work], len_dm[work])
    pn = profile_counts(len_dm[work])
    poff = np.concatenate([[0], np.cumsum(pn)])
    edges = {}
    for j, i in enumerate(work.tolist()):
        xy = np.column_stack([px[poff[j]:poff[j + 1]], py[poff[j]:poff[j + 1]]])
        s = np.minimum(np.arange(len(xy)) * 5.0, len_dm[i] / 10.0)
        edges[i] = graph.Edge(0, 0, xy, s, bool(A["flat"][i]))
    par = graph.parallel_pairs(graph.Graph(edges), center=(-1e9, -1e9))

    # Tronçons de la dalle, rangés par id ; pics retirés et trous comblés le long du tronçon, puis
    # altitude unique par nœud (tronçons au sol d'abord), comme graph.assign_elevation.
    mem = np.flatnonzero(inside)
    mem = mem[np.argsort(ident[mem], kind="stable")]
    P = np.vstack([edges[i].xy for i in mem]) if len(mem) else np.zeros((0, 2))
    z = np.asarray(sample(P[:, 0] + x0 / 10.0, P[:, 1] + y0 / 10.0), float)
    nodata = float(np.isnan(z).mean()) if len(z) else 0.0
    key_u = (Xd[first] << 32) | Yd[first]
    key_v = (Xd[off[1:] - 1] << 32) | Yd[off[1:] - 1]
    k, zs = 0, []
    for i in mem.tolist():
        m = len(edges[i].xy)
        zs.append(graph.fill_nan(edges[i], graph.despike(edges[i], z[k:k + m])))
        k += m
    keyed = [graph.Edge(int(key_u[i]), int(key_v[i]), None, None, edges[i].flat) for i in mem.tolist()]
    node_z = graph.node_elevations(keyed, zs, {})
    fb = float(np.nanmean(z)) if np.isfinite(z).any() else 0.0
    no_z = sorted({n for e in keyed for n in (e.u, e.v)} - node_z.keys())   # aucun MNT sur le tronçon
    if no_z and node_z:   # altitude du nœud connu le plus proche (pas la moyenne de dalle : pics)
        from scipy.spatial import cKDTree
        known = np.array(list(node_z), np.int64)
        _, j = cKDTree(np.column_stack([known >> 32, known & 0xFFFFFFFF])).query(
            np.column_stack([np.array(no_z, np.int64) >> 32, np.array(no_z, np.int64) & 0xFFFFFFFF]))
        node_z.update({n: node_z[int(known[k])] for n, k in zip(no_z, j)})
    cols = {c: [] for c in ("jump", "dplus_dm", "dminus_dm", "max_grade_pm", "z0", "prof_d", "par_n", "par_id")}
    for i, zi in zip(mem.tolist(), zs):
        e = edges[i]
        zi = zi.copy()
        zi[0], zi[-1] = node_z.get(int(key_u[i]), fb), node_z.get(int(key_v[i]), fb)
        zi = graph.fill_nan(e, zi)
        if not e.flat and A["nature"][i] in ROAD_NATURES:
            zi = graph.cap_grade(e.s, zi, ROAD_MAX_GRADE)
        graph.compute_profile(e, zi)
        q = np.round(e.z * 10).astype(np.int64)
        dz = np.diff(q)
        if np.abs(dz).max(initial=0) > 32767:
            raise ValueError(f"saut de profil > 3 276 m sur 5 m (tronçon {ident[i]})")
        cols["jump"].append(np.abs(dz).max(initial=0) > JUMP_DM)
        cols["dplus_dm"].append(int(dz[dz > 0].sum()))
        cols["dminus_dm"].append(int(-dz[dz < 0].sum()))
        cols["max_grade_pm"].append(min(65535, round(e.max_grade * 1000)))
        cols["z0"].append(int(q[0]))
        cols["prof_d"].append(np.concatenate([[0], dz]))
        nb = sorted(int(ident[o]) for o in par.get(i, ()))
        cols["par_n"].append(len(nb))
        cols["par_id"] += nb
    gsel = np.concatenate([np.arange(off[i], off[i + 1]) for i in mem] + [np.zeros(0, np.int64)])
    T = dict(
        id_d=np.diff(ident[mem], prepend=0).astype(np.int64),
        len_dm=len_dm[mem].astype(np.int32),
        dplus_dm=np.array(cols["dplus_dm"], np.int32),
        dminus_dm=np.array(cols["dminus_dm"], np.int32),
        max_grade_pm=np.array(cols["max_grade_pm"], np.uint16),
        nature=A["nature"][mem].astype(np.uint8),
        importance=A["importance"][mem].astype(np.uint8),
        flags=(A["flat"][mem].astype(np.uint8) | (A["ok"][mem].astype(np.uint8) << 1)),
        geom_n=n[mem].astype(np.int32),
        geom_x=_deltas(Xd[gsel], n[mem], x0).astype(np.int32),
        geom_y=_deltas(Yd[gsel], n[mem], y0).astype(np.int32),
        prof_d=(np.concatenate(cols["prof_d"]) if len(mem) else np.zeros(0)).astype(np.int16),
        prof_z0_dm=np.array(cols["z0"], np.int32),
        par_n=np.array(cols["par_n"], np.uint16),
        par_id=np.array(cols["par_id"], np.int64),
    )
    steep = int((T["max_grade_pm"] > STEEP * 1000).sum())
    info = dict(n=len(mem), km=round(int(T["len_dm"].sum()) / 1e4, 1), nodata_frac=round(nodata, 5),
                steep_gt60=steep, steep_frac=round(steep / max(1, len(mem)), 5),
                jump_gt10=int(sum(cols["jump"])), node_fallback=len(no_z),
                par_links=len(cols["par_id"]), z_min_m=round(float(np.nanmin(z)), 1) if len(z) else None,
                z_max_m=round(float(np.nanmax(z)), 1) if len(z) else None)
    return T, info


def build_tile(ix: int, iy: int, out: Path) -> dict:
    """Télécharge (ou lit en cache) les sources d'une dalle, l'écrit dans out/<ix>_<iy>.npz."""
    t0 = time.time()
    x0, y0, x1, y1 = (ix * TILE_M - MARGIN_M, iy * TILE_M - MARGIN_M,
                      (ix + 1) * TILE_M + MARGIN_M, (iy + 1) * TILE_M + MARGIN_M)
    lon, lat = _TO_WGS.transform([x0, x1, x0, x1], [y0, y0, y1, y1])
    stats = {"ign": ign.cache.new_stats(), "dem": ign.cache.new_stats()}
    A = ign.fetch((min(lat), min(lon), max(lat), max(lon)), stats["ign"])
    t_src = time.time() - t0
    X, Y = _TO_L93.transform(A["lon"], A["lat"])
    Xd = np.round(np.asarray(X) * 10).astype(np.int64)
    Yd = np.round(np.asarray(Y) * 10).astype(np.int64)
    T, info = tile_columns(ix, iy, A, Xd, Yd, lambda X, Y: elevation.sample_l93(X, Y, stats["dem"]))
    buf = io.BytesIO()
    np.savez_compressed(buf, **T)
    data = buf.getvalue()
    cache.write_atomic(out / f"{ix}_{iy}.npz", data)
    info.update(sha256=hashlib.sha256(data).hexdigest(), bytes=len(data),
                o_par_troncon=round(len(data) / max(1, info["n"]), 1),
                t_sources_s=round(t_src, 1), t_total_s=round(time.time() - t0, 1),
                requetes={k: v["requests"] for k, v in stats.items()})
    return info


def tiles_for(spec: str) -> list[tuple[int, int]]:
    """'32_342,33_342' (liste) ou 'sud,ouest,nord,est' (bbox WGS) -> dalles."""
    if "_" in spec:
        return [tuple(map(int, t.split("_"))) for t in spec.split(",")]
    s, w, n, e = map(float, spec.split(","))
    X, Y = _TO_L93.transform([w, e, w, e], [s, s, n, n])
    return [(ix, iy) for ix in range(int(min(X) // TILE_M), int(max(X) // TILE_M) + 1)
            for iy in range(int(min(Y) // TILE_M), int(max(Y) // TILE_M) + 1)]


def _read_manifest(mpath: Path) -> dict:
    m = json.loads(mpath.read_text()) if mpath.exists() else {}
    if m.get("format", FORMAT) != FORMAT:
        raise RuntimeError(f"{mpath} : format {m['format']}, attendu {FORMAT}")
    return m


def _done(out: Path, info: dict | None, name: str) -> bool:
    """Dalle déjà écrite et intacte (sha256 du manifeste) : on ne la refait pas."""
    p = out / f"{name}.npz"
    return bool(info) and p.exists() and hashlib.sha256(p.read_bytes()).hexdigest() == info["sha256"]


def build(spec: str, out, log=print, force: bool = False) -> dict:
    """Construit les dalles demandées et met à jour out/manifest.json (fusion avec l'existant).
    Reprise : une dalle présente et intacte est sautée (sauf force). Plusieurs processus peuvent
    écrire dans le même dossier : le manifeste est relu et réécrit sous verrou."""
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    mpath = out / "manifest.json"
    m = _read_manifest(mpath)
    for ix, iy in tiles_for(spec):
        name = f"{ix}_{iy}"
        if not force and _done(out, m.get("tiles", {}).get(name), name):
            log(f"{name} : déjà faite")
            continue
        info = build_tile(ix, iy, out)
        log(f"{name} : {json.dumps(info, ensure_ascii=False)}")
        with open(out / "manifest.lock", "w") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            m = _read_manifest(mpath)
            m.update(format=FORMAT, data_version=DATA_VERSION, source=SOURCE, natures=ign.NATURES)
            m.setdefault("tiles", {})[name] = info
            tiles = m["tiles"].values()
            n = sum(t["n"] for t in tiles)
            m["totals"] = dict(tiles=len(m["tiles"]), n=n, bytes=sum(t["bytes"] for t in tiles),
                               steep_gt60=sum(t["steep_gt60"] for t in tiles),
                               jump_gt10=sum(t.get("jump_gt10", 0) for t in tiles),
                               node_fallback=sum(t.get("node_fallback", 0) for t in tiles),
                               nodata_frac=round(sum(t["nodata_frac"] * t["n"] for t in tiles) / max(1, n), 5))
            cache.write_atomic(mpath, json.dumps(m, ensure_ascii=False, indent=1).encode())
    return m
