"""Exp. 2 : solveurs × réductions sur le graphe de production (w LiDAR nettoyé).

    PYTHONPATH=.:scripts/experiments .venv/bin/python scripts/experiments/exp2_solvers.py NAME BUDGET GRAPHS SOLVERS [SEEDS]
    GRAPHS  : full,r6.9,r9.9,steep4,...   SOLVERS : base,plus,face,faceplus,face0
Sortie : lignes JSON sur stdout et dans /tmp/tp_exp/exp2_NAME.jsonl
"""
import json
import sys
import time

import numpy as np

import common as C
import solvers as S

name, budget = sys.argv[1], float(sys.argv[2])
graphs, sols = sys.argv[3].split(","), sys.argv[4].split(",")
seeds = [int(x) for x in (sys.argv[5] if len(sys.argv) > 5 else "0").split(",")]
g0 = C.load_prod(name)
L = g0.L
Lmax = 1.05 * L
out = open(C.OUT / f"exp2_{name}.jsonl", "a")

RUN = dict(
    base=S.run_baseline, plus=S.run_plus,
    face=lambda g, L, b, s: S.run_faces(g, L, b, s, "construct"),
    faceplus=lambda g, L, b, s: S.run_faces(g, L, b, s, "plus", 0.3),
    face0=lambda g, L, b, s: S.run_faces(g, L, b, s, "face"),
)
for gn in graphs:
    t = time.time()
    if gn == "full":
        g = g0
    elif gn[0] == "r":
        g = S.by_radius(g0, float(gn[1:]) * 1000, Lmax)
    elif gn.startswith("steep"):
        g = S.by_steep(g0, Lmax, float(gn[5:]))
    t_red = time.time() - t
    g.adj
    for sn in sols:
        for seed in seeds:
            route, st = RUN[sn](g, L, budget, seed)
            ln, dp = C.check_route(g, route)
            ids = [e for e, _, _ in route]
            far = float(np.hypot(*g.nxy[[a for _, a, _ in route]].T).max())
            row = dict(case=name, graph=gn, edges=g.M, t_reduce=round(t_red, 1), solver=sn, seed=seed,
                       budget=budget, km=round(ln / 1000, 2), dplus=round(dp), feasible=bool(0.95 * L <= ln <= Lmax),
                       max_dist_km=round(far / 1000, 1), **st)
            print(json.dumps(row), flush=True)
            out.write(json.dumps(row) + "\n")
            out.flush()
