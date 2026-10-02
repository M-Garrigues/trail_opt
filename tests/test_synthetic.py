"""Solveurs sur graphe synthétique + relief analytique (sans réseau)."""
import numpy as np
import pytest

from trailopt import graph
from trailopt.solvers import exact, optimize
from trailopt.solvers.anneal import Annealer

from conftest import build, check_loop


@pytest.mark.parametrize("solver", ["exact", "anneal"])
@pytest.mark.parametrize("mode", ["max", "target"])
def test_loops_valid(solver, mode):
    L = 2000.0
    P, _ = build(L, mode=mode, D=60.0 if mode == "target" else None)
    res = optimize(P, budget=4.0, solver=solver, seed=1)
    length, dplus = check_loop(P, res.circuit)
    assert abs(length - res.length) < 1e-6 and abs(dplus - res.dplus) < 1e-6
    if mode == "max":
        assert P.Lmin <= length <= P.Lmax, "distance hors tolérance"
    else:
        assert 0.7 * L <= length <= 1.3 * L


def test_cpsat_optimal_beats_anneal():
    P, _ = build(1500, n=5)
    ann = Annealer(P, seed=3)
    route = ann.run(2.0)
    sc_a, _, dp_a, feas_a = ann.evaluate(route)
    r = exact.solve(P, 30.0, workers=2)
    assert r["status"] == "OPTIMAL"
    sc_e, _, dp_e, feas_e = P.score(r["chosen"])
    assert feas_e
    # Arrondis CP-SAT (dm, cm) : tolérance de quelques cm par arête.
    if feas_a:
        assert dp_e >= dp_a - 0.01 * len(P.len)
    check_loop(P, graph.euler_circuit(P.g, r["chosen"], P.s))


def test_cpsat_target_optimal_beats_anneal():
    P, _ = build(1500, mode="target", D=40.0, n=5)
    ann = Annealer(P, seed=3)
    route = ann.run(2.0)
    sc_a = ann.evaluate(route)[0]
    r = exact.solve(P, 30.0, workers=2)
    assert r["status"] == "OPTIMAL"
    assert P.score(r["chosen"])[0] >= sc_a - 1e-3
    assert r["bound"] <= -P.score(r["chosen"])[0] + 1e-3


def test_grade_filter_removes_steep_edges():
    P0, _ = build(2000)
    grades = np.array([e.max_grade for e in P0.g.edges.values()])
    thr = float(np.quantile(grades, 0.85))
    P1, dbg = build(2000, max_grade=thr)
    assert dbg["edges_after_grade"] < len(P0.len)
    assert all(e.max_grade <= thr for e in P1.g.edges.values())
    res = optimize(P1, budget=2.0, solver="anneal")
    check_loop(P1, res.circuit)
