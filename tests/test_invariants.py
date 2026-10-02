import numpy as np
import pytest

from trailopt import graph
from trailopt.solvers import exact, optimize
from trailopt.solvers.anneal import Annealer

from conftest import build, check_loop


def _edges(spec):
    out = {}
    for k, (u, v) in enumerate(spec):
        xy = np.array([[u, 0.0], [u + 0.5, 1.0 + k], [v, 0.0]]) * 10
        if u == v:
            xy = np.array([[u, 0], [u + 1, 1 + k], [u - 1, 1 + k], [u, 0]], float) * 10
        s = np.concatenate([[0], np.cumsum(np.hypot(*np.diff(xy, axis=0).T))])
        out[k] = graph.Edge(u, v, xy, s)
    return out


def test_hierholzer_loops_and_parallel_edges():
    # 0=1 parallèles, boucle en 1, 1=2 parallèles, boucle en 0 (le départ)
    spec = [(0, 1), (1, 0), (1, 1), (1, 2), (2, 1), (0, 0)]
    g = graph.Graph(_edges(spec))
    circ = graph.euler_circuit(g, list(g.edges), 0)
    assert sorted(e for e, _, _ in circ) == list(range(len(spec)))
    assert circ[0][1] == 0 and circ[-1][2] == 0
    for (_, _, b), (_, a, _) in zip(circ, circ[1:]):
        assert a == b


def test_hierholzer_rejects_odd():
    g = graph.Graph(_edges([(0, 1), (1, 2), (2, 0), (0, 3)]))
    with pytest.raises(RuntimeError):
        graph.euler_circuit(g, list(g.edges), 0)


def test_bridges_multi_edges():
    g = graph.Graph(_edges([(0, 1), (1, 0), (1, 2), (2, 2)]))
    # (1,2) est un pont, les parallèles 0=1 non, la boucle (2,2) non plus
    assert graph.find_bridges(g) == {2}


def test_contract_degree2_keeps_bridges_separate():
    a = np.array([[0, 0], [1, 0]], float)
    b = np.array([[1, 0], [2, 0]], float)
    c = np.array([[2, 0], [3, 0]], float)
    out = graph.contract_degree2([(0, 1, a, False), (1, 2, b, False), (2, 3, c, True)])
    assert len(out) == 2
    merged = [e for e in out if not e[3]][0]
    assert {merged[0], merged[1]} == {0, 2} and len(merged[2]) == 3


def test_bilinear_exact_on_plane():
    from trailopt import elevation as el
    bb = (0.0, 0.0, 100.0, 100.0)
    h = w = int(100 / el.RES)
    cx = bb[0] + (np.arange(w) + 0.5) * el.RES
    cy = bb[3] - (np.arange(h) + 0.5) * el.RES
    grid = 2 * cx[None, :] + 3 * cy[:, None]
    X = np.array([10.0, 33.3, 71.2])
    Y = np.array([12.0, 50.0, 88.8])
    assert np.allclose(el.bilinear(grid, bb, X, Y), 2 * X + 3 * Y)


def test_pruning_and_simplification_counts():
    P, dbg = build(2000)
    assert dbg["edges_simplified"] < dbg["edges_in_zone"]   # chaîne contractée
    assert dbg["edges_pruned"] <= dbg["edges_simplified"] + dbg["edges_doubled_near_start"]
    assert 10_000 not in P.nodes


def test_clip_to_region_cuts_edges_at_boundary():
    from shapely.geometry import box
    zone = box(0, -50, 100, 50)
    raw = [
        (1, 2, np.array([[10.0, 0], [90, 0]]), False),            # entièrement dedans
        (2, 3, np.array([[90.0, 0], [300, 0]]), False),           # sort par l'est : coupée
        (4, 5, np.array([[-200.0, 20], [200, 20]]), False),       # traverse : milieu gardé
        (6, 7, np.array([[150.0, 0], [300, 0]]), False),          # entièrement dehors
        (8, 9, np.array([[-50.0, -20], [50, -20], [50, 200]]), True),  # entre, puis ressort
    ]
    out = graph.clip_to_region(raw, zone)
    assert len(out) == 4
    assert out[0][:2] == (1, 2) and len(out[0][2]) == 2
    u, v, xy, flat = out[1]
    assert u == 2 and v < 0 and xy[:, 0].max() <= 100 and xy[:, 0].max() > 90
    u, v, xy, flat = out[2]
    assert u < 0 and v < 0 and u != v and xy[:, 0].min() >= 0 and xy[:, 0].max() <= 100
    u, v, xy, flat = out[3]
    assert flat and u < 0 and v < 0 and xy[:, 1].max() <= 50
    assert len({n for e in out for n in e[:2] if n < 0}) == 5    # nœuds de coupe tous distincts


def test_elevation_masks_blended_values_near_holes():
    """Bord d'un trou de données : la rampe -9999 -> altitude réelle ne doit pas passer
    pour du relief (cas réel au nord de la base de Villacoublay)."""
    from trailopt import elevation as el
    a = np.full((120, 120), 177.0)
    a[:, :40] = np.nan                                   # trou (nodata déjà converti)
    ramp = np.array([-900.0, -600.0, -300.0, -80.0, 30.0, 110.0, 150.0, 170.0])
    a[:, 40:48] = ramp                                   # mélanges d'apparence plausible
    m = el.mask_unreliable(a)
    assert np.isnan(m[:, 40:48]).all()                   # toute la rampe est invalidée
    assert np.isnan(m[:, :40 + el.HOLE_MARGIN_PX]).all()
    assert np.allclose(m[:, 40 + el.HOLE_MARGIN_PX + 2:], 177.0)   # loin du trou : inchangé

    b = np.full((60, 60), 300.0) + np.arange(60)[None, :] * 2.0     # pente de 40 % : relief réel
    assert np.array_equal(el.mask_unreliable(b), b)

    c = np.full((60, 60), 177.0)
    c[:, :3] = [-700.0, -200.0, 60.0]                    # rampe dont le trou est hors dalle
    assert np.isnan(el.mask_unreliable(c)[:, :4]).all()
