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
    assert dbg["edges_doubled_near_start"] == 1      # seul le tronçon d'accès, pas le cul-de-sac
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


def test_no_out_and_back_farming_near_start():
    """Sur une grille (aucun accès obligé), rien n'est doublé près du départ : la boucle
    ne peut pas reprendre deux fois la même arête pour gagner du D+."""
    P, dbg = build(2000)
    assert dbg["edges_doubled_near_start"] == 0
    assert all(e.twin is None for e in P.g.edges.values())


def test_access_bridges_ignore_dead_ends():
    import numpy as np
    def E(u, v):
        xy = np.array([[u * 10.0, 0.0], [v * 10.0, 5.0]])
        return graph.Edge(u, v, xy, np.array([0.0, np.hypot(*(xy[1] - xy[0]))]))
    # s=0 -1- 1 -2- 2, triangle 2-3-4, impasse 1-9, impasse 4-8
    edges = {0: E(0, 1), 1: E(1, 2), 2: E(2, 3), 3: E(3, 4), 4: E(4, 2), 5: E(1, 9), 6: E(4, 8)}
    assert graph.access_bridges(graph.Graph(edges), 0) == {0, 1}
