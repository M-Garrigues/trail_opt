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
INNER_MAX_DM = int(MARGIN_M * 10)  # chaîne de ponts/tunnels interpolée seulement si plus courte que la marge (dm)
DATA_VERSION = "bdtopo-wfs-2026-10e"  # e : France entière, même calcul que d (pilote IdF + Isère + Lyon, 87 dalles) ; d : altitude de nœud unique (z sur tous les tronçons voisins) ; c : nœuds intérieurs des ponts/tunnels interpolés (T35, D37) ; b : falaises, portails (D27)
SOURCE = "BD TOPO® IGN, RGE ALTI®, LiDAR HD — Etalab 2.0"
STEEP = 0.60               # validation : pente max > 60 % (artefact probable)
ROAD_MAX_GRADE = 0.30     # pente physique max d'une voie carrossable, par pas de 5 m (D27)
ROAD_NATURES = [ign.NATURES.index(k) for k in sorted(ign.ROADS | {"Route empierrée", "Type autoroutier"})]
JUMP_DM = 100              # validation : saut > 10 m entre deux points de 5 m (200 %, pic probable)
_TO_WGS = Transformer.from_crs("EPSG:2154", "EPSG:4326", always_xy=True)
_TO_L93 = Transformer.from_crs("EPSG:4326", "EPSG:2154", always_xy=True)

# Zones (tiles.md § Zones) : un repère métrique par territoire, même grille de 20 km dans ce repère.
# Métropole `fxx` : clés `<ix>_<iy>` (inchangées). DOM : clés `<zone>/<ix>_<iy>`, fichiers dans un
# sous-dossier, et entrée `zones` au manifeste (crs + bbox WGS84 ouest, sud, est, nord pour choisir la zone).
# MNT : RGE ALTI (couche HIGHRES, servie dans chaque repère), précédé du LiDAR HD là où il est publié.
_RGE = elevation.LAYERS[1]
ZONES = {
    "fxx": dict(crs="EPSG:2154", dem=None),
    "re": dict(crs="EPSG:2975", bbox=[55.15, -21.45, 55.90, -20.80],     # RGR92 / UTM 40S
               dem=["IGNF_LIDAR-HD_MNT_ELEVATION.ELEVATIONGRIDCOVERAGE.RGR92UTM40S", _RGE]),
    "gp": dict(crs="EPSG:5490", bbox=[-61.90, 15.75, -60.95, 16.60], dem=[_RGE]),   # RGAF09 / UTM 20N
    "mq": dict(crs="EPSG:5490", bbox=[-61.30, 14.30, -60.75, 14.95], dem=[_RGE]),   # RGAF09 / UTM 20N
    "gf": dict(crs="EPSG:2972", bbox=[-54.70, 2.00, -51.50, 5.90], dem=[_RGE]),     # RGFG95 / UTM 22N
    "yt": dict(crs="EPSG:4471", bbox=[44.95, -13.10, 45.35, -12.55], dem=[_RGE]),   # RGM04 / UTM 38S
}
_TR = {}


def transformers(zone: str = "fxx"):
    """(repère de la zone -> WGS84, WGS84 -> repère de la zone)."""
    if zone not in _TR:
        crs = ZONES[zone]["crs"]
        _TR[zone] = (Transformer.from_crs(crs, "EPSG:4326", always_xy=True),
                     Transformer.from_crs("EPSG:4326", crs, always_xy=True))
    return _TR[zone]


def split_key(key: str) -> tuple[str, int, int]:
    """'32_342' -> ('fxx', 32, 342) ; 're/17_382' -> ('re', 17, 382)."""
    zone, _, name = key.rpartition("/")
    ix, iy = map(int, name.split("_"))
    return zone or "fxx", ix, iy


def sampler(zone: str = "fxx", stats: dict | None = None):
    """sample(X, Y) -> z (m) dans le repère de la zone."""
    z = ZONES[zone]
    return lambda X, Y: elevation.sample_l93(X, Y, stats, layers=z["dem"], crs=z["crs"])


def _deltas(a, n, base=0):
    """Valeurs absolues -> 1er élément de chaque tronçon : a - base, suivants : delta au précédent."""
    a = np.asarray(a, np.int64)
    d = np.diff(a, prepend=0)
    first = np.cumsum(n) - n
    d[first] = a[first] - base
    return d


def _inner_nodes(keyed, len_dm, node_z, box=None) -> dict:
    """Nœuds touchés seulement par des ponts/tunnels (intérieur d'un ouvrage) : altitude interpolée
    le long des ouvrages entre les nœuds au sol (solution harmonique pondérée 1/longueur, linéaire
    sur une chaîne), au lieu du MNT au nœud (rivière sous un pont, colline au-dessus d'un tunnel).
    Un ouvrage sans aucun nœud au sol est laissé au MNT (appelant)."""
    from scipy.sparse import coo_matrix
    from scipy.sparse.csgraph import connected_components
    from scipy.sparse.linalg import spsolve
    flat = [(e.u, e.v, max(int(L), 1)) for e, L in zip(keyed, len_dm) if e.flat and e.u != e.v]
    free = sorted({n for u, v, _ in flat for n in (u, v)} - node_z.keys())
    if not free:
        return {}
    ix = {n: i for i, n in enumerate(free)}
    r, c, w, b = [], [], [], np.zeros(len(free))
    for u, v, L in flat:
        for a, o in ((u, v), (v, u)):
            if a in ix:
                r.append(ix[a]); c.append(ix[a]); w.append(1.0 / L)
                if o in ix:
                    r.append(ix[a]); c.append(ix[o]); w.append(-1.0 / L)
                else:
                    b[ix[a]] += node_z[o] / L
    A = coo_matrix((w, (r, c)), shape=(len(free),) * 2).tocsr()
    _, comp = connected_components(A, directed=False)
    ok = np.isin(comp, np.unique(comp[b != 0]))       # composantes reliées au sol
    # Chaîne interpolée seulement si elle est vue en entier par toute dalle qui en contient un nœud :
    # plus courte que la marge de chargement (INNER_MAX_DM) et tous ses nœuds dans la zone chargée
    # `box` (xmin, ymin, xmax, ymax en dm). Sinon altitude MNT (déterministe d'une dalle à l'autre).
    tot = np.zeros(comp.max() + 1)
    cut = np.zeros(comp.max() + 1, bool)
    for u, v, L in flat:
        if u not in ix and v not in ix:
            continue                                  # ouvrage entre deux nœuds au sol
        c = comp[ix[u] if u in ix else ix[v]]
        tot[c] += L
        if box is not None:
            for k in (u, v):
                x, y = k >> 32, k & 0xFFFFFFFF
                cut[c] |= not (box[0] < x < box[2] and box[1] < y < box[3])
    ok &= (tot[comp] < INNER_MAX_DM) & ~cut[comp]
    if not ok.any():
        return {}
    sel = np.flatnonzero(ok)
    z = np.atleast_1d(spsolve(A[sel][:, sel].tocsc(), b[sel]))
    return {free[i]: float(v) for i, v in zip(sel, z)}


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
    key_u = (Xd[first] << 32) | Yd[first]
    key_v = (Xd[off[1:] - 1] << 32) | Yd[off[1:] - 1]
    # Altitude de nœud unique d'une dalle à l'autre : elle est calculée sur TOUS les tronçons qui
    # touchent un nœud de la dalle (ceux des dalles voisines compris), rangés par id : deux dalles
    # qui partagent un nœud voient le même ensemble et prennent la même valeur (10c bis).
    nodes = np.unique(np.concatenate([key_u[mem], key_v[mem]]))
    # ... et sur les chaînes de ponts/tunnels qui touchent ces nœuds, avec les tronçons au sol de
    # leurs extrémités : l'interpolation d'une chaîne (_inner_nodes) voit la même chose partout.
    fl = np.flatnonzero(valid & A["flat"].astype(bool))
    reach = nodes
    if len(fl):
        from scipy.sparse import coo_matrix
        from scipy.sparse.csgraph import connected_components
        allk, inv = np.unique(np.concatenate([key_u[fl], key_v[fl]]), return_inverse=True)
        _, lab = connected_components(coo_matrix((np.ones(len(fl)), (inv[:len(fl)], inv[len(fl):])),
                                                 shape=(len(allk),) * 2), directed=False)
        reach = np.union1d(nodes, allk[np.isin(lab, np.unique(lab[np.isin(allk, nodes)]))])
    ext = np.flatnonzero(valid & (np.isin(key_u, reach) | np.isin(key_v, reach)))
    ext = ext[np.argsort(ident[ext], kind="stable")]
    P = np.vstack([edges[i].xy for i in ext]) if len(ext) else np.zeros((0, 2))
    z = np.asarray(sample(P[:, 0] + x0 / 10.0, P[:, 1] + y0 / 10.0), float)
    k, zs, in_mem = 0, [], []
    for i in ext.tolist():
        m = len(edges[i].xy)
        zs.append(graph.fill_nan(edges[i], graph.despike(edges[i], z[k:k + m])))
        in_mem.append(np.full(m, bool(inside[i])))
        k += m
    in_mem = np.concatenate(in_mem) if in_mem else np.zeros(0, bool)
    zmem = z[in_mem]
    nodata = float(np.isnan(zmem).mean()) if len(zmem) else 0.0
    zs_of = dict(zip(ext.tolist(), zs))
    keyed = [graph.Edge(int(key_u[i]), int(key_v[i]), None, None, edges[i].flat) for i in ext.tolist()]
    node_z = graph.node_elevations([e for e in keyed if not e.flat], [zk for e, zk in zip(keyed, zs) if not e.flat], {})
    m = int(MARGIN_M * 10)
    box = (x0 - m, y0 - m, x0 + TILE_M * 10 + m, y0 + TILE_M * 10 + m)
    node_z.update(_inner_nodes(keyed, len_dm[ext], node_z, box))   # intérieur des ponts et tunnels
    node_z = graph.node_elevations(keyed, zs, node_z)          # ponts/tunnels sans aucun appui au sol
    fb = float(np.nanmean(zmem)) if np.isfinite(zmem).any() else 0.0
    no_z = sorted(set(nodes.tolist()) - node_z.keys())   # aucun MNT sur le tronçon
    if no_z:   # MNT à l'extrémité la plus proche (≤ 1 km) de tous les tronçons chargés : ne dépend pas
        from scipy.spatial import cKDTree   # de la dalle (pas la moyenne de dalle : pics)
        ends = np.unique(np.concatenate([key_u[valid], key_v[valid]]))
        ez = np.asarray(sample((ends >> 32) / 10.0, (ends & 0xFFFFFFFF) / 10.0), float)
        ends, ez = ends[np.isfinite(ez)], ez[np.isfinite(ez)]
        q = np.array(no_z, np.int64)
        if len(ends):
            dist, j = cKDTree(np.column_stack([ends >> 32, ends & 0xFFFFFFFF])).query(
                np.column_stack([q >> 32, q & 0xFFFFFFFF]), distance_upper_bound=10_000)
            node_z.update({int(n): float(ez[k]) for n, k, r in zip(q, j, dist) if np.isfinite(r)})
    rest = sorted(set(no_z) - node_z.keys())
    known = np.array(sorted(set(node_z) - set(no_z)), np.int64)
    if rest and len(known):   # sinon altitude du nœud connu le plus proche
        _, j = cKDTree(np.column_stack([known >> 32, known & 0xFFFFFFFF])).query(
            np.column_stack([np.array(rest, np.int64) >> 32, np.array(rest, np.int64) & 0xFFFFFFFF]))
        node_z.update({n: node_z[int(known[k])] for n, k in zip(rest, j)})
    cols = {c: [] for c in ("jump", "dplus_dm", "dminus_dm", "max_grade_pm", "z0", "prof_d", "par_n", "par_id")}
    for i in mem.tolist():
        zi, e = zs_of[i], edges[i]
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
                par_links=len(cols["par_id"]), z_min_m=round(float(np.nanmin(zmem)), 1) if len(zmem) else None,
                z_max_m=round(float(np.nanmax(zmem)), 1) if len(zmem) else None)
    return T, info


def build_tile(ix: int, iy: int, out: Path, zone: str = "fxx") -> dict:
    """Télécharge (ou lit en cache) les sources d'une dalle, l'écrit dans out/[<zone>/]<ix>_<iy>.npz."""
    t0 = time.time()
    to_wgs, to_crs = transformers(zone)
    x0, y0, x1, y1 = (ix * TILE_M - MARGIN_M, iy * TILE_M - MARGIN_M,
                      (ix + 1) * TILE_M + MARGIN_M, (iy + 1) * TILE_M + MARGIN_M)
    lon, lat = to_wgs.transform([x0, x1, x0, x1], [y0, y0, y1, y1])
    stats = {"ign": ign.cache.new_stats(), "dem": ign.cache.new_stats()}
    A = ign.fetch((min(lat), min(lon), max(lat), max(lon)), stats["ign"])
    t_src = time.time() - t0
    X, Y = to_crs.transform(A["lon"], A["lat"])
    Xd = np.round(np.asarray(X) * 10).astype(np.int64)
    Yd = np.round(np.asarray(Y) * 10).astype(np.int64)
    T, info = tile_columns(ix, iy, A, Xd, Yd, sampler(zone, stats["dem"]))
    buf = io.BytesIO()
    np.savez_compressed(buf, **T)
    data = buf.getvalue()
    p = out / ("" if zone == "fxx" else zone) / f"{ix}_{iy}.npz"
    p.parent.mkdir(parents=True, exist_ok=True)
    cache.write_atomic(p, data)
    info.update(sha256=hashlib.sha256(data).hexdigest(), bytes=len(data),
                o_par_troncon=round(len(data) / max(1, info["n"]), 1),
                t_sources_s=round(t_src, 1), t_total_s=round(time.time() - t0, 1),
                requetes={k: v["requests"] for k, v in stats.items()})
    return info


def tiles_for(spec: str, zone: str = "fxx") -> list[str]:
    """'32_342,re/17_382' (liste, zone en préfixe sinon `zone`) ou 'sud,ouest,nord,est' (bbox WGS) -> clés."""
    pre = "" if zone == "fxx" else f"{zone}/"
    if "_" in spec:
        return [t if "/" in t else pre + t for t in spec.split(",")]
    s, w, n, e = map(float, spec.split(","))
    X, Y = transformers(zone)[1].transform([w, e, w, e], [s, s, n, n])
    return [f"{pre}{ix}_{iy}" for ix in range(int(min(X) // TILE_M), int(max(X) // TILE_M) + 1)
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


def totals(tiles: dict) -> dict:
    """Somme des validations par dalle (clé `totals` du manifeste)."""
    t = tiles.values()
    n = sum(x["n"] for x in t)
    return dict(tiles=len(tiles), n=n, bytes=sum(x["bytes"] for x in t),
                steep_gt60=sum(x["steep_gt60"] for x in t),
                jump_gt10=sum(x.get("jump_gt10", 0) for x in t),
                node_fallback=sum(x.get("node_fallback", 0) for x in t),
                nodata_frac=round(sum(x["nodata_frac"] * x["n"] for x in t) / max(1, n), 5))


def build(spec: str, out, log=print, force: bool = False, zone: str = "fxx") -> dict:
    """Construit les dalles demandées et met à jour out/manifest.json (fusion avec l'existant).
    Reprise : une dalle présente et intacte est sautée (sauf force). Plusieurs processus peuvent
    écrire dans le même dossier : le manifeste est relu et réécrit sous verrou."""
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    mpath = out / "manifest.json"
    m = _read_manifest(mpath)
    for name in tiles_for(spec, zone):
        z, ix, iy = split_key(name)
        if not force and _done(out, m.get("tiles", {}).get(name), name):
            log(f"{name} : déjà faite")
            continue
        info = build_tile(ix, iy, out, z)
        log(f"{name} : {json.dumps(info, ensure_ascii=False)}")
        with open(out / "manifest.lock", "w") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            m = _read_manifest(mpath)
            m.update(format=FORMAT, data_version=DATA_VERSION, source=SOURCE, natures=ign.NATURES)
            m.setdefault("tiles", {})[name] = info
            if z != "fxx":
                m.setdefault("zones", {})[z] = {k: ZONES[z][k] for k in ("crs", "bbox")}
            m["totals"] = totals(m["tiles"])
            cache.write_atomic(mpath, json.dumps(m, ensure_ascii=False, indent=1).encode())
    return m
