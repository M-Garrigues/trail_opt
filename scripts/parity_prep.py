"""Parité axe données de la préparation : Python (`pipeline/load.py` + `trailopt`) contre le
moteur Rust (`engine plan`), mêmes dalles tiles/1 (D18, étape 3).

    python scripts/parity_prep.py --tiles scripts/experiments/tiles_pilote [--solve --seeds 5 --time 10]

Compare, pour chaque cas : le graphe élagué avant réduction (arêtes, Σlongueur, Σw, borne du sac
à dos), le Problem final (après réduction aux arêtes pentues), puis, avec --solve, le D+ médian
des deux chaînes complètes.
"""
from __future__ import annotations

import argparse
import json
import os
import statistics
import subprocess
import tempfile
import time
from pathlib import Path

import numpy as np

CASES = {"massy10": (48.7309, 2.2713, 10.0), "alpes12": (45.0920, 6.0700, 12.0)}
ENGINE = Path(__file__).resolve().parents[1] / "engine/target/release/engine"


def stats(P) -> dict:
    return {"edges": len(P.len), "sum_len_m": float(P.len.sum()), "sum_w_m": float(P.w.sum()),
            "dplus_upper_bound_m": P.dplus_upper_bound(),
            "parallel_pairs": sum(len(v) for v in P.parallel.values()) // 2}


def python_stats(lat, lon, km):
    from scripts.export_problem import capture
    from trailopt import pipeline
    from trailopt.pipeline import Params
    full = stats(capture(Params(lat, lon, km)))
    real = pipeline.REDUCE_MIN_EDGES
    pipeline.REDUCE_MIN_EDGES = 10**9          # graphe élagué, sans réduction
    try:
        pruned = stats(capture(Params(lat, lon, km)))
    finally:
        pipeline.REDUCE_MIN_EDGES = real
    return pruned, full


def rust(tiles, req: dict, prep_only: bool) -> dict:
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
        json.dump(req, f)
    try:
        cmd = [str(ENGINE), "plan", "--tiles", str(tiles), "--request", f.name]
        out = subprocess.run(cmd + (["--prep-only"] if prep_only else []),
                             capture_output=True, text=True, check=False)
    finally:
        os.unlink(f.name)
    r = json.loads(out.stdout)
    if "error" in r:
        raise RuntimeError(r["error"])
    return r


def gap(a, b):
    return f"{100 * (b - a) / a:+.2f} %" if a else "—"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--tiles", required=True)
    ap.add_argument("--solve", action="store_true")
    ap.add_argument("--seeds", type=int, default=5)
    ap.add_argument("--time", type=float, default=10.0)
    ap.add_argument("--only")
    a = ap.parse_args()
    os.environ["TRAILOPT_TILES_DIR"] = a.tiles
    os.environ.setdefault("TRAILOPT_OFFLINE", "1")
    report = {}
    for name, (lat, lon, km) in CASES.items():
        if a.only and name != a.only:
            continue
        py_pruned, py_full = python_stats(lat, lon, km)
        r = rust(a.tiles, {"lat": lat, "lon": lon, "distance_km": km}, True)
        rs_pruned, rs_full = r["debug"]["attempt"]["pruned"], r["problem_stats"]
        print(f"\n{name} (Python / Rust / écart)")
        for label, p, q in (("élagué", py_pruned, rs_pruned), ("final", py_full, rs_full)):
            for k in ("edges", "sum_len_m", "sum_w_m", "dplus_upper_bound_m", "parallel_pairs"):
                if k in p and k in q:
                    print(f"  {label:7s} {k:20s} {p[k]:12.1f} {q[k]:12.1f} {gap(p[k], q[k]):>9s}")
        rep = {"python": {"pruned": py_pruned, "final": py_full},
               "rust": {"pruned": rs_pruned, "final": rs_full}}
        if a.solve:
            from trailopt.pipeline import Params, plan_loop
            py, rs, tpy, trs = [], [], [], []
            for seed in range(a.seeds):
                t = time.time()
                res = plan_loop(Params(lat, lon, km, time_s=a.time, seed=seed))
                tpy.append(time.time() - t)
                py.append(res.dplus)
                t = time.time()
                o = rust(a.tiles, {"lat": lat, "lon": lon, "distance_km": km, "time_s": a.time,
                                   "seed": seed}, False)
                trs.append(time.time() - t)
                rs.append(o["candidates"][0]["dplus_m"])
            print(f"  D+ médian  Python {statistics.median(py):.0f} m ({np.median(tpy):.1f} s) / "
                  f"Rust {statistics.median(rs):.0f} m ({np.median(trs):.1f} s) "
                  f"{gap(statistics.median(py), statistics.median(rs))}")
            print(f"    Python {[round(x) for x in py]}  Rust {[round(x) for x in rs]}")
            rep["dplus"] = {"python": py, "rust": rs, "python_s": tpy, "rust_s": trs}
        report[name] = rep
    print(json.dumps(report, indent=1)[:0])


if __name__ == "__main__":
    main()
