"""Moteur Rust (`engine/`) appelé en sous-processus sur le Problem exporté en JSON
(contrat `.team/contracts/problem.md`). Activé par `solver="rust"` ou `TRAILOPT_SOLVER=rust` ;
`optimize` retombe sur Python si le binaire manque ou échoue."""
from __future__ import annotations

import json
import os
import subprocess
import tempfile
import time
from pathlib import Path

import numpy as np

from ..graph import Problem, euler_circuit

VERSION = 1
ENGINE = Path(os.environ.get("TRAILOPT_ENGINE",
                             Path(__file__).resolve().parents[2] / "engine/target/release/engine"))


def to_dict(P: Problem) -> dict:
    """Nœuds réindexés 0..N-1 dans l'ordre de P.nodes ; arêtes 0..M-1 (clés de P.g.edges)."""
    idx = {n: i for i, n in enumerate(P.nodes)}
    E = P.g.edges
    m = len(P.len)
    d0 = np.array([E[e].xy[1] - E[e].xy[0] for e in range(m)]).reshape(-1, 2)
    d1 = np.array([E[e].xy[-2] - E[e].xy[-1] for e in range(m)]).reshape(-1, 2)
    return {
        "version": VERSION, "mode": P.mode, "L": P.L, "Lmin": P.Lmin, "Lmax": P.Lmax,
        "D": P.D, "s": idx[P.s], "node_simple": bool(P.node_simple),
        "xy": [[float(x), float(y)] for x, y in (P.nxy[n] for n in P.nodes)],
        "far": sorted(idx[n] for n in P.far),
        "u": [idx[E[e].u] for e in range(m)], "v": [idx[E[e].v] for e in range(m)],
        "len": P.len.tolist(), "w": P.w.tolist(),
        "ang_u": np.arctan2(d0[:, 1], d0[:, 0]).tolist(),
        "ang_v": np.arctan2(d1[:, 1], d1[:, 0]).tolist(),
        "parallel": sorted([a, b] for a, bs in P.parallel.items() for b in bs if a < b),
    }


def run(P: Problem, budget: float, seed: int = 0, n_candidates: int = 1, cancel=None,
        extra=()) -> dict:
    """Résout P avec le moteur ; renvoie sa sortie JSON. Lève RuntimeError en cas d'échec."""
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
        json.dump(to_dict(P), f)
    try:
        cmd = [str(ENGINE), "solve", "--problem", f.name, "--time", f"{budget:g}",
               "--seed", str(seed), "--candidates", str(n_candidates), *extra]
        proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        t0 = time.time()
        while proc.poll() is None:        # attente annulable ; le moteur a son propre plafond
            if (cancel is not None and cancel.is_set()) or time.time() - t0 > budget + 10:
                proc.kill()
                proc.wait()
                raise RuntimeError("moteur Rust interrompu")
            time.sleep(0.02)
        out, err = proc.communicate()
    finally:
        os.unlink(f.name)
    try:
        r = json.loads(out)
    except ValueError:
        raise RuntimeError(f"sortie illisible du moteur Rust (code {proc.returncode}) : {err[-300:]}") from None
    if "error" in r:
        raise RuntimeError(f"moteur Rust : {r['error']}")
    return r


def optimize(P: Problem, budget: float, seed: int = 0, n_candidates: int = 1, cancel=None):
    """Même interface et mêmes vérifications que `solvers.optimize`."""
    from . import SolveResult
    t0 = time.time()
    r = run(P, budget, seed, n_candidates, cancel)
    circuits = [euler_circuit(P.g, ids, P.s) for ids in
                [r["edges"]] + [a["edges"] for a in r["alternatives"]]]
    for c in circuits:      # le moteur vérifie déjà ; on revérifie à la frontière
        ids = [e for e, _, _ in c]
        assert len(ids) == len(set(ids)) and c[0][1] == P.s and c[-1][2] == P.s
        assert P.parallel_ok(ids) and (not P.node_simple or P.node_simple_ok(c))
    _, length, dplus, feas = P.score(r["edges"])
    dbg = {"edges": len(P.len), "nodes": len(P.nodes), "solver": f"rust ({r['method']})",
           "solver_reason": "moteur Rust", "rust": {k: r[k] for k in (
               "method", "depart", "iterations", "anneal_iterations", "it_per_s", "solve_s")},
           "parallel_pairs": sum(len(v) for v in P.parallel.values()) // 2,
           "node_simple": P.node_simple, "solve_time_s": round(time.time() - t0, 2)}
    alts = [(c, *P.score([e for e, _, _ in c])[1:3]) for c in circuits[1:]]
    method = f"moteur Rust ({r['method']}, {r['iterations']} itérations)"
    return SolveResult(circuits[0], length, dplus, feas, method, dbg, alts)
