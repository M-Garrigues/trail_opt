"""Calibre EXACT_MAX_EDGES : CP-SAT (warm-start 20 % + CP-SAT) vs recuit seul, même budget.

    python scripts/bench_exact.py [--time 20] [--start 48.7303,2.2725]
"""
from __future__ import annotations

import argparse
import json
import sys

from trailopt.pipeline import Params, plan_loop

ap = argparse.ArgumentParser()
ap.add_argument("--start", default="48.7303,2.2725")
ap.add_argument("--time", type=float, default=20.0)
ap.add_argument("--dists", default="5,10,20")
ap.add_argument("--roads", default="pedestrian,minor")
ap.add_argument("--seeds", default="0,1")
a = ap.parse_args()
lat, lon = map(float, a.start.split(","))
rows = []
for d in map(float, a.dists.split(",")):
    for roads in a.roads.split(","):
        for seed in map(int, a.seeds.split(",")):
            row = {"km": d, "roads": roads, "seed": seed}
            for solver in ("anneal", "exact"):
                r = plan_loop(Params(lat, lon, d, roads=roads, time_s=a.time, seed=seed, source="osm",
                                     solver=solver))
                row["edges"] = r.debug["edges"]
                row[solver] = round(r.dplus, 1) if r.feasible else None
                if solver == "exact":
                    row.update(status=r.debug.get("cpsat_status"),
                               bound=r.debug.get("cpsat_bound"), method=r.method,
                               rss=r.debug["peak_rss_mb"])
            rows.append(row)
            print(json.dumps(row, ensure_ascii=False), flush=True)
print("\n| km | voies | arêtes | graine | recuit D+ | CP-SAT D+ | statut | borne |", file=sys.stderr)
print("|---|---|---|---|---|---|---|---|", file=sys.stderr)
for r in rows:
    b = f"{r['bound']:.0f}" if r.get("bound") else "-"
    print(f"| {r['km']:g} | {r['roads']} | {r['edges']} | {r['seed']} | {r['anneal']} | "
          f"{r['exact']} | {r['status']} | {b} |", file=sys.stderr)
