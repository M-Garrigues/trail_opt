"""Parité axe solveur (D16) : moteur Rust contre les références Python figées par
scripts/parity_refs.py, même Problem, même budget, 10 graines, 2 fils.
Critères : médiane Rust >= médiane Python, min Rust >= p10 Python, échecs Rust <= Python."""
import gzip
import json
import os
import subprocess
import tempfile
import time
from pathlib import Path

import numpy as np
import pytest

from trailopt.solvers import rust

DIR = Path(__file__).parent / "fixtures/parity"
NAMES = sorted(p.name.removesuffix(".ref.json") for p in DIR.glob("*.ref.json"))


def rust_stats(name):
    """Valeurs (D+ ou -erreur), échecs et temps moyen du moteur sur l'instance `name`."""
    ref = json.loads((DIR / f"{name}.ref.json").read_text())
    pb = json.loads(gzip.open(DIR / f"{name}.problem.json.gz", "rt").read())
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
        json.dump(pb, f)
    vals, fails, times = [], 0, []
    try:
        for seed in range(ref["seeds"]):
            t = time.time()
            out = subprocess.run([str(rust.ENGINE), "solve", "--problem", f.name, "--time",
                                  str(ref["budget_s"]), "--seed", str(seed)], check=True,
                                 capture_output=True, text=True,
                                 env=os.environ | {"RAYON_NUM_THREADS": "2"}).stdout
            times.append(time.time() - t)
            r = json.loads(out)
            if not r["feasible"]:
                fails += 1
            elif pb["mode"] == "max":
                vals.append(r["dplus"])
            else:
                vals.append(-(abs(r["length"] - pb["L"]) / pb["L"] + abs(r["dplus"] - pb["D"]) / pb["D"]))
    finally:
        os.unlink(f.name)
    return ref, vals, fails, float(np.mean(times))


@pytest.mark.skipif(not rust.ENGINE.exists(), reason="moteur Rust non compilé (cargo build --release)")
@pytest.mark.parametrize("name", NAMES)
def test_parity(name):
    ref, vals, fails, _ = rust_stats(name)
    assert fails <= ref["failures"], f"{name} : {fails} échecs (Python {ref['failures']})"
    if ref["median"] is None:
        return
    assert vals and np.median(vals) >= ref["median"] - 1e-6, f"{name} : médiane {np.median(vals)} < {ref['median']}"
    assert min(vals) >= ref["p10"] - 1e-6, f"{name} : min {min(vals)} < p10 {ref['p10']}"
