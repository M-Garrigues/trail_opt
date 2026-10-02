"""Briques des grandes zones : densification vectorisée, départ par projection, criblage,
réduction du graphe, grilles de dalles."""
import numpy as np
from shapely.geometry import box

from trailopt import elevation, graph
from trailopt.pipeline import base_edges, build_candidate

from conftest import grid_raw, relief


def test_densify_many_matches_densify():
    rng = np.random.default_rng(1)
    polys = [np.cumsum(rng.uniform(-40, 40, (rng.integers(2, 9), 2)), axis=0) for _ in range(100)]
    pts, off = graph.densify_many(polys, 5.0)
    for i, p in enumerate(polys):
        ref, _ = graph.densify(p, 5.0)
        assert np.allclose(pts[off[i]:off[i + 1]], ref)


def test_insert_start_projects_on_segment():
    """Arête non densifiée (2 sommets à 400 m) : le départ tombe au milieu, pas sur un sommet."""
    xy = np.array([[-200.0, 30.0], [200.0, 30.0]])
    edges = {0: graph.Edge(1, 2, xy, np.array([0.0, 400.0]))}
    node, snap, pos = graph.insert_start(edges, 1)
    assert node == graph.START and abs(snap - 30.0) < 1e-6 and np.allclose(pos, [0.0, 30.0])
    assert len(edges) == 2 and abs(sum(e.length for e in edges.values()) - 400.0) < 1e-6
    # point très proche d'un sommet : on réutilise le nœud existant
    edges = {0: graph.Edge(1, 2, xy, np.array([0.0, 400.0]))}
    node, snap, _ = graph.insert_start(edges, 1, point=(-199.9, 31.0))
    assert node == 1 and len(edges) == 1


def test_edges_stay_raw_until_elevation():
    raw = grid_raw(7)
    edges = base_edges(raw, box(-1e4, -1e4, 1e4, 1e4))
    assert not any(e.dense for e in edges.values())
    g, s = build_candidate(edges, 0.0, 2100.0, relief)
    assert all(e.dense and e.z is not None for e in g.edges.values())
    assert max(np.hypot(*np.diff(e.xy, axis=0).T).max() for e in g.edges.values()) <= 5.0 + 1e-9


def test_screening_w_tracks_fine_w():
    """Le D+ de criblage (points tous les 50 m) classe les arêtes comme le D+ fin."""
    raw = grid_raw(9, h=300.0)
    g, s = build_candidate(base_edges(raw, box(-1e5, -1e5, 1e5, 1e5)), 0.0, 9000.0, relief)
    w = graph.screening_w(g, relief, step=50.0)
    fine = np.array([g.edges[k].w for k in g.edges])
    coarse = np.array([w[k] for k in g.edges])
    assert np.corrcoef(fine, coarse)[0, 1] > 0.9


def test_steep_reduction_keeps_a_loopable_graph():
    raw = grid_raw(11, h=200.0)
    edges = base_edges(raw, box(-1e5, -1e5, 1e5, 1e5))
    info = {}
    g, s = build_candidate(edges, 0.0, 4200.0, relief, info=info)
    w = {k: e.w for k, e in g.edges.items()}
    keep = graph.steep_reduction(g, s, 4200.0, w, k=1.5)
    assert 0 < len(keep) < len(g.edges)
    red = graph.prune(g.sub(keep), s, 4200.0)
    assert s in red.adj and len(red.edges) > 10          # le départ reste sur un réseau bouclable
    kept_ratio = np.mean([e.w / e.length for e in red.edges.values()])
    all_ratio = np.mean([e.w / e.length for e in g.edges.values()])
    assert kept_ratio >= all_ratio                        # on a gardé les arêtes les plus pentues


def test_elevation_tiles_follow_a_fixed_grid():
    """Deux zones voisines partagent les mêmes dalles : le cache est réutilisable."""
    span = elevation.TILE_PX * elevation.RES
    a = set(elevation._tiles(np.array([100.0, 2600.0]), np.array([50.0, 60.0])))
    b = set(elevation._tiles(np.array([1200.0, 2700.0]), np.array([900.0, 10.0])))
    assert a == b == {(0.0, 0.0, span, span), (span, 0.0, 2 * span, span)}


def test_suggested_time_grows_with_distance():
    from trailopt.pipeline import TIME_S, Params, suggested_time
    assert suggested_time(2) == suggested_time(10) == 20.0
    assert suggested_time(100) == 60.0
    ts = [suggested_time(d) for d in range(2, 101)]
    assert all(b >= a for a, b in zip(ts, ts[1:])) and max(ts) <= TIME_S[1]
    assert Params(48.7, 2.27, 40).time_s is None          # défaut : budget selon la distance
