"""Rayon libre de 200 m autour du départ et option « carrefours uniques »."""
import pytest

from trailopt import graph
from trailopt.solvers import optimize

from conftest import build, check_loop, dead_end_raw


@pytest.mark.parametrize("solver", ["exact", "anneal"])
@pytest.mark.parametrize("node_simple", [False, True])
def test_dead_end_start_allows_out_and_back(solver, node_simple):
    # impasse 150 m (aller-retour 300 m) + carré 1600 m
    P, dbg = build(1900, raw=dead_end_raw(), node_simple=node_simple)
    assert dbg["edges_doubled_near_start"] == 2
    res = optimize(P, budget=3.0, solver=solver)
    length, _ = check_loop(P, res.circuit)
    assert P.Lmin <= length <= P.Lmax
    # l'accès est bien parcouru deux fois (original + copie)
    near = [e for e, _, _ in res.circuit
            if max(abs(P.g.edges[e].xy[:, 1]).max(), abs(P.g.edges[e].xy[:, 0]).max()) <= 200]
    assert len(near) >= 2


def test_dead_end_without_free_radius_is_impossible(monkeypatch):
    monkeypatch.setattr(graph, "FREE_RADIUS", 0.0)
    from trailopt.pipeline import UserError
    with pytest.raises(UserError):
        build(1900, raw=dead_end_raw())


@pytest.mark.parametrize("solver", ["exact", "anneal"])
@pytest.mark.parametrize("mode", ["max", "target"])
def test_node_simple_loops(solver, mode):
    P, _ = build(2000, mode=mode, D=60.0 if mode == "target" else None, node_simple=True)
    res = optimize(P, budget=4.0, solver=solver, seed=2)
    length, _ = check_loop(P, res.circuit)
    assert res.debug["node_simple"]
    if mode == "max":
        assert P.Lmin <= length <= P.Lmax


def test_node_simple_cpsat_never_beats_unconstrained():
    P0, _ = build(1500, n=5)
    P1, _ = build(1500, n=5, node_simple=True)
    from trailopt.solvers import exact
    r0, r1 = exact.solve(P0, 20.0), exact.solve(P1, 20.0)
    assert r0["status"] == "OPTIMAL" and r1["chosen"]
    assert P1.score(r1["chosen"])[2] <= P0.score(r0["chosen"])[2] + 1e-6
    check_loop(P1, graph.euler_circuit(P1.g, r1["chosen"], P1.s))
