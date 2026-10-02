"""Construit le graphe de PRODUCTION (chaîne actuelle, LiDAR 5 m) et le sauve en tableaux compacts.

    PYTHONPATH=. .venv/bin/python scripts/experiments/build_prod.py NAME LAT LON DIST_KM [RADIUS_KM] [ROADS]

Sortie : /tmp/tp_exp/NAME.pkl  (u, v, len, w, flat, xy/z concaténés, parallèles, s, timings).
"""
import math
import pickle
import sys
import time
from pathlib import Path

import numpy as np

from trailopt import cache, elevation, graph, ign
from trailopt.geo import LocalFrame
from trailopt.pipeline import OSM_MARGIN_M, base_edges, build_candidate, peak_rss_mb

OUT = Path("/tmp/tp_exp")
OUT.mkdir(exist_ok=True)


def bbox_of(reg, frame):
    minx, miny, maxx, maxy = reg.bounds
    m = OSM_MARGIN_M
    corners = np.array([[minx - m, miny - m], [maxx + m, miny - m],
                        [minx - m, maxy + m], [maxx + m, maxy + m]])
    lon, lat = frame.to_wgs(corners)
    return (min(lat), min(lon), max(lat), max(lon))


def region_for(dist_km, radius_km=None, tol=0.05):
    L = dist_km * 1000.0
    Lmax = L * (1 + tol)
    R = min(L / 2, math.sqrt(0.99 * 1965.0 * 1e6 / math.pi))   # pipeline.default_radius
    reg = graph.clip_to_reach(graph.disk(R), Lmax)
    if radius_km:
        reg = reg.intersection(graph.disk(radius_km * 1000.0))
    return reg, L, Lmax


def main():
    name, lat, lon, dist = sys.argv[1], float(sys.argv[2]), float(sys.argv[3]), float(sys.argv[4])
    radius = float(sys.argv[5]) if len(sys.argv) > 5 and sys.argv[5] != "-" else None
    roads = sys.argv[6] if len(sys.argv) > 6 else "minor"
    frame = LocalFrame(lat, lon)
    reg, L, Lmax = region_for(dist, radius)
    T = {}
    t = time.time()
    st = cache.new_stats()
    data = ign.fetch(bbox_of(reg, frame), st)
    T["fetch_s"] = time.time() - t
    t = time.time()
    raw, nways = ign.to_edges(data, frame, roads)
    T["to_edges_s"] = time.time() - t
    del data
    t = time.time()
    dbg = {}
    edges = base_edges(raw, reg, dbg)
    T["base_edges_s"] = time.time() - t
    del raw
    t = time.time()
    info = {}
    steps = {}

    def step(n):
        steps[n] = time.time()
    t0 = time.time()
    g, s = build_candidate(edges, L * 0.95, Lmax, elevation.sampler_for(frame, st), info=info, step=step)
    T["prune_s"] = steps["altitude"] - t0
    T["elevation_s"] = steps["élagage"] - steps["altitude"]
    t = time.time()
    P = graph.Problem(g, s, L, "max", 0.05)
    T["problem_parallel_s"] = time.time() - t
    T["rss_mb"] = peak_rss_mb()
    m = len(g.edges)
    E = [g.edges[i] for i in range(m)]
    npts = np.array([len(e.xy) for e in E])
    out = dict(
        lat=lat, lon=lon, L=L, s=s, ways=nways, dbg=dbg, info=info, T=T, net=st,
        u=np.array([e.u for e in E]), v=np.array([e.v for e in E]),
        len=P.len, w=P.w, flat=np.array([e.flat for e in E]),
        twin=np.array([-1 if e.twin is None else next(i for i, f in enumerate(E) if f is e.twin) for e in E]),
        off=np.concatenate([[0], np.cumsum(npts)]),
        xy=np.vstack([e.xy for e in E]).astype(np.float32),
        z=np.concatenate([e.z for e in E]).astype(np.float32),
        parallel=P.parallel,
    )
    with open(OUT / f"{name}.pkl", "wb") as f:
        pickle.dump(out, f, protocol=4)
    print(name, "ways", nways, dbg, info, {k: round(v, 1) for k, v in T.items()}, st)


if __name__ == "__main__":
    main()
