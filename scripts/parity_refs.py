"""Corpus de parité, axe solveur (D16) : fige le Problem de chaque instance et la distribution
des résultats du solveur Python (10 graines, mode auto, 2 fils), plus l'optimum CP-SAT sur les
petites instances. `tests/test_parity.py` compare le moteur Rust à ces références.

    TRAILOPT_OFFLINE=1 python scripts/parity_refs.py [--only nom]   # ~10 min

Instances réelles 3 km : cache /tmp/trailopt_cache (IGN) ; l'OSM piétons n'existe que dans
tests/fixtures/cache (TRAILOPT_CACHE_DIR=tests/fixtures/cache --only reel3_osm_pietons_carrefours).
Massy 10 km demande le cache /tmp/trailopt_cache (ou le réseau). Massy 100 km (8,9 Mo) n'est
pas dans le corpus : voir scripts/bench_faces.py.
"""
from __future__ import annotations

import argparse
import gzip
import json
import sys
import time
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT), str(ROOT / "tests"), str(ROOT / "scripts")]

from conftest import build  # noqa: E402
from export_problem import capture  # noqa: E402
from trailopt.pipeline import Params  # noqa: E402
from trailopt.solvers import exact, optimize  # noqa: E402
from trailopt.solvers.rust import to_dict  # noqa: E402

OUT = ROOT / "tests/fixtures/parity"
SEEDS = range(10)
START3 = (48.7303, 2.2725)          # cas réel de tests/fixtures/cache
MASSY = (48.7309, 2.2713)

# nom -> (fabrique du Problem, budget Python en s)
INSTANCES = {
    "grille11_carrefours": (lambda: build(4000.0, n=11, node_simple=True)[0], 5.0),
    "grille11_cible": (lambda: build(3000.0, mode="target", D=150.0, n=11)[0], 5.0),
    "reel3_ign": (lambda: capture(Params(*START3, 3, time_s=5)), 5.0),
    "reel3_ign_cible": (lambda: capture(Params(*START3, 3, mode="target", target_dplus=80,
                                               time_s=5)), 5.0),
    "reel3_osm_pietons_carrefours": (lambda: capture(Params(*START3, 3, time_s=5, roads="pedestrian",
                                                    node_simple=True)), 5.0),
    "massy10_ign": (lambda: capture(Params(*MASSY, 10)), 20.0),
}


def value(P, length, dplus):
    """À maximiser : D+ (mode max) ou -erreur (mode cible)."""
    return dplus if P.mode == "max" else -P.err(length, dplus)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--only")
    a = ap.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    for name, (make, budget) in INSTANCES.items():
        if a.only and name != a.only:
            continue
        P = make()
        with gzip.open(OUT / f"{name}.problem.json.gz", "wt") as f:
            json.dump(to_dict(P), f)
        vals, fails, times = [], 0, []
        for seed in SEEDS:
            t = time.time()
            r = optimize(P, budget, seed=seed, workers=2)
            times.append(time.time() - t)
            if r.feasible:
                vals.append(value(P, r.length, r.dplus))
            else:
                fails += 1
        ref = {"mode": P.mode, "edges": len(P.len), "budget_s": budget, "seeds": len(SEEDS),
               "values": [round(v, 4) for v in vals], "failures": fails,
               "median": float(np.median(vals)) if vals else None,
               "p10": float(np.percentile(vals, 10)) if vals else None,
               "time_s": round(float(np.mean(times)), 2)}
        if len(P.len) <= 1000 and P.mode == "max":     # optimum de référence (D6)
            r = exact.solve(P, 120.0, workers=8)
            ref["cpsat"] = {"status": r["status"], "bound": r["bound"],
                            "value": P.stats(r["chosen"])[1] if r["chosen"] else None}
        (OUT / f"{name}.ref.json").write_text(json.dumps(ref, indent=1) + "\n")
        print(name, json.dumps({k: v for k, v in ref.items() if k != "values"}), flush=True)


if __name__ == "__main__":
    main()
