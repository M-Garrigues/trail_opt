"""Exporte l'instance `Problem` d'un calcul en JSON pour le moteur Rust (contrat :
`.team/contracts/problem.md`). Le pipeline tourne jusqu'au solveur, qui n'est pas lancé.

    python scripts/export_problem.py --start 48.7309,2.2713 --distance 10 --out massy10.json
"""
from __future__ import annotations

import argparse
import json

from trailopt import pipeline
from trailopt.pipeline import Params
from trailopt.solvers.rust import to_dict  # noqa: F401 (réexporté pour bench_faces)


class _Captured(Exception):
    pass


def capture(p: Params):
    """Problem que plan_loop passerait au solveur (premier sous-réseau essayé)."""
    real = pipeline.optimize

    def stop(P, *a, **k):
        raise _Captured(P)
    pipeline.optimize = stop
    try:
        pipeline.plan_loop(p)
    except _Captured as c:
        return c.args[0]
    finally:
        pipeline.optimize = real
    raise RuntimeError("le solveur n'a pas été appelé")


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--start", default="48.7309,2.2713")
    ap.add_argument("--distance", type=float, default=10.0)
    ap.add_argument("--roads", default="minor")
    ap.add_argument("--no-revisit", action="store_true")
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    lat, lon = map(float, a.start.split(","))
    P = capture(Params(lat, lon, a.distance, roads=a.roads, node_simple=a.no_revisit))
    with open(a.out, "w") as f:
        json.dump(to_dict(P), f)
    print(f"{a.out} : {len(P.len)} arêtes, {len(P.nodes)} nœuds")
