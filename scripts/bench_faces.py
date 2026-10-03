"""Recuit par faces : Python vs moteur Rust (`engine/`) sur le même Problem.

    cargo build --release --manifest-path engine/Cargo.toml
    python scripts/bench_faces.py --distance 10 --times 2,5,10,20 --iters 1e5,1e6,1e7 --seeds 0,1,2

Une ligne JSON par calcul. Chaque boucle (Python ou Rust) est vérifiée par `check_loop`.
Rust est lancé deux fois : à itérations égales à celles du run Python (même graine), puis sur
la grille --iters. RAYON_NUM_THREADS=1 pour un débit mono-cœur.
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT), str(ROOT / "scripts")]

from export_problem import capture, to_dict  # noqa: E402
from tests.conftest import check_loop  # noqa: E402
from trailopt.graph import euler_circuit  # noqa: E402
from trailopt.pipeline import Params  # noqa: E402
from trailopt.solvers.faces import FaceSearch  # noqa: E402

ENGINE = ROOT / "engine/target/release/engine"

ap = argparse.ArgumentParser()
ap.add_argument("--start", default="48.7309,2.2713")
ap.add_argument("--distance", type=float, default=10.0)
ap.add_argument("--no-revisit", action="store_true")
ap.add_argument("--times", default="2,5,10,20", help="budgets Python (s)")
ap.add_argument("--iters", default="1e5,1e6,1e7", help="budgets Rust (itérations)")
ap.add_argument("--seeds", default="0,1,2")
ap.add_argument("--threads", default="1", help="RAYON_NUM_THREADS pour Rust")
a = ap.parse_args()

lat, lon = map(float, a.start.split(","))
P = capture(Params(lat, lon, a.distance, node_simple=a.no_revisit))
path = Path(tempfile.gettempdir()) / f"problem_{a.distance:g}km.json"
path.write_text(json.dumps(to_dict(P)))
base = {"km": a.distance, "edges": len(P.len), "node_simple": a.no_revisit}


def emit(row):
    print(json.dumps(base | row, ensure_ascii=False), flush=True)


def rust(iters, seed):
    env = os.environ | {"RAYON_NUM_THREADS": a.threads}
    r = json.loads(subprocess.run([ENGINE, "solve", "--problem", path, "--iters", str(int(iters)), "--anneal-iters", "0",
                                   "--seed", str(seed)], env=env, capture_output=True, text=True,
                                  check=True).stdout)
    length, dplus = check_loop(P, euler_circuit(P.g, r["edges"], P.s))
    assert abs(dplus - r["dplus"]) < 1e-6 * max(1, dplus) and r["check"] == "ok"
    return {"solver": "rust", "threads": a.threads, "iterations": r["iterations"],
            "time_s": round(r["solve_s"], 3), "it_per_s": round(r["it_per_s"]),
            "dplus": round(dplus, 1), "km_out": round(length / 1000, 2), "feasible": r["feasible"]}


for seed in map(int, a.seeds.split(",")):
    for T in [float(x) for x in a.times.split(",") if x]:
        t = time.time()
        fs = FaceSearch(P, seed)
        faces_s = time.time() - t
        t = time.time()
        circuit, st = fs.solve(T)
        dt = time.time() - t
        length, dplus = check_loop(P, circuit)
        emit({"solver": "python", "seed": seed, "budget_s": T, "iterations": st["iterations"],
              "time_s": round(dt, 3), "it_per_s": round(st["iterations"] / dt),
              "dplus": round(dplus, 1), "km_out": round(length / 1000, 2),
              "feasible": bool(P.Lmin <= length <= P.Lmax), "faces_s": round(faces_s, 3)})
        emit({"seed": seed, "budget": "iters_python", **rust(st["iterations"], seed)})
    for n in map(float, a.iters.split(",")):
        emit({"seed": seed, "budget": int(n), **rust(n, seed)})
