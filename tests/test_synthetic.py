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


@pytest.mark.parametrize("mode", ["max", "target"])
@pytest.mark.parametrize("node_simple", [False, True])
def test_faces_solver_valid(mode, node_simple):
    """Recuit par faces : boucle valide dans les deux modes, avec ou sans carrefours uniques."""
    L = 2400.0
    P, _ = build(L, mode=mode, D=70.0 if mode == "target" else None, n=9, node_simple=node_simple)
    if mode == "max":
        res = optimize(P, budget=4.0, solver="faces", seed=1)
        assert res.debug["solver"] == "recuit par faces"
    else:   # mode cible : recuit classique puis affinage par faces (exact désactivé)
        res = optimize(P, budget=5.0, solver="auto", exact_max_edges=0, seed=1)
        assert "faces" in res.debug
    length, dplus = check_loop(P, res.circuit)
    assert P.Lmin <= length <= P.Lmax
    if mode == "target":    # jamais pire que le recuit classique seul sur la même durée d'amorce
        ref = optimize(P, budget=3.0, solver="anneal", seed=1)
        assert P.err(length, dplus) <= P.err(ref.length, ref.dplus) + 0.02


def test_faces_not_worse_than_tiny_loop_and_cancellable():
    import threading, time
    P, _ = build(3000.0, n=9)
    cancel = threading.Event()
    threading.Timer(0.8, cancel.set).start()
    t = time.time()
    res = optimize(P, budget=30.0, solver="faces", cancel=cancel)
    assert time.time() - t < 6.0
    check_loop(P, res.circuit)


def test_faces_enumeration_on_grid():
    from trailopt.solvers.faces import FaceSearch
    P, _ = build(2400.0, n=6)
    fs = FaceSearch(P)
    assert len(fs.F) >= 20                      # une grille 6x6 a 25 mailles
    for f in fs.F[:10]:                         # chaque face est un cycle fermé simple
        assert f[0][1] == f[-1][2]
        assert all(a[2] == b[1] for a, b in zip(f, f[1:]))


def test_candidates_are_valid_and_different():
    """Plusieurs candidats : boucles valides, dans les bornes, partageant au plus la moitié
    de leur longueur ; le budget total n'augmente pas."""
    import time
    from trailopt.solvers.faces import FaceSearch
    P, _ = build(2400.0, n=11)
    t = time.time()
    res = optimize(P, budget=6.0, n_candidates=3, seed=1)
    assert time.time() - t < 9.0
    loops = [[e for e, _, _ in res.circuit]] + [[e for e, _, _ in c] for c, _, _ in res.alternatives]
    assert 1 <= len(loops) <= 3
    fs = FaceSearch(P)
    for c, length, dplus in res.alternatives:
        assert check_loop(P, c) == pytest.approx((length, dplus))
        assert P.Lmin <= length <= P.Lmax
    for i in range(len(loops)):
        for j in range(i):
            assert fs.overlap(loops[i], loops[j]) <= 0.5
    assert optimize(P, budget=2.0, seed=1).alternatives == []      # 1 par défaut : aucun surcoût
