"""Types de voies (non revêtu) et départ de repli vers le sous-réseau bouclable le plus proche."""
import numpy as np
import pytest
from shapely.geometry import box

from trailopt import osm
from trailopt.pipeline import Params, UserError, search_loop

from conftest import check_loop, relief


@pytest.mark.parametrize("tags,expected", [
    ({"highway": "path"}, True),
    ({"highway": "path", "surface": "asphalt"}, False),
    ({"highway": "footway"}, False),
    ({"highway": "footway", "surface": "gravel"}, True),
    ({"highway": "track"}, True),
    ({"highway": "track", "tracktype": "grade1"}, False),
    ({"highway": "residential", "surface": "dirt;grass"}, True),
    ({"highway": "bridleway", "surface": "paving_stones"}, False),
])
def test_is_unpaved(tags, expected):
    assert osm.is_unpaved(tags) is expected


def _square(x0, y0, side, first_id):
    c = [np.array(p, float) for p in
         [(x0, y0), (x0 + side, y0), (x0 + side, y0 + side), (x0, y0 + side)]]
    ids = list(range(first_id, first_id + 4))
    return [(ids[k], ids[(k + 1) % 4], np.array([c[k], (c[k] + c[(k + 1) % 4]) / 2, c[(k + 1) % 4]]),
             False) for k in range(4)]


def _run(raw, L, **kw):
    p = Params(48.7, 2.27, L / 1000, time_s=6, **kw)
    dbg, timings = {}, {}
    out = search_loop(raw, box(-1e4, -1e4, 1e4, 1e4), p, L, L * 1.05, relief, dbg, timings)
    return out, dbg


@pytest.mark.parametrize("solver", ["auto", "anneal"])
def test_fallback_to_nearest_loopable_network(solver):
    # petit carré (400 m de tour) autour du départ : trop court pour 1,5 km ;
    # carré de 375 m de côté (1,5 km) à 600 m ; grand carré (4 km) plus loin.
    raw = (_square(-50, -50, 100, 1) + _square(600, 0, 375, 10)
           + _square(-3000, 0, 1000, 20))
    (P, g, res, info, _acc), dbg = _run(raw, 1500, solver=solver)
    assert info["kind"] == "repli"
    assert res.feasible
    check_loop(P, res.circuit)
    assert 590 <= info["start_snap_m"] <= 620          # point le plus proche du carré de 1,5 km
    assert [a["status"] for a in dbg["attempts"]][:2] == ["réseau trop court", "réseau trop court"]


def test_start_network_used_when_possible():
    raw = _square(-200, -200, 400, 1) + _square(600, 0, 375, 10)
    (P, g, res, info, _acc), dbg = _run(raw, 1600)
    assert info["kind"] == "départ" and res.feasible and len(dbg["attempts"]) == 1


def test_no_loop_anywhere():
    with pytest.raises(UserError, match="Aucune boucle"):
        _run(_square(-50, -50, 100, 1), 1500)


def test_cancel_stops_solver_quickly():
    """Un Event positionné pendant le calcul arrête le solveur bien avant la fin du budget."""
    import threading
    import time
    from trailopt.pipeline import Cancelled
    from trailopt.solvers import optimize
    from conftest import build

    for solver in ("anneal", "exact"):
        P, _ = build(2000)
        cancel = threading.Event()
        threading.Timer(1.0, cancel.set).start()
        t = time.time()
        optimize(P, budget=30.0, solver=solver, cancel=cancel)
        assert time.time() - t < 8.0, solver

    p = Params(48.7, 2.27, 1.6, time_s=30)
    cancel = threading.Event()
    threading.Timer(0.5, cancel.set).start()
    t = time.time()
    with pytest.raises(Cancelled):
        search_loop(_square(-200, -200, 400, 1), box(-1e4, -1e4, 1e4, 1e4), p, 1600, 1680, relief,
                    {}, {}, cancel=cancel)
    assert time.time() - t < 8.0


def test_access_out_and_back_keeps_clicked_start():
    """Réseau bouclable à 600 m, relié au point cliqué par une route hors type de voies :
    la boucle part du point cliqué, avec un aller-retour sur cette route."""
    from trailopt.pipeline import assemble, make_access
    from trailopt import graph as G
    trails = _square(600, 0, 375, 10)                       # 1,5 km de sentiers, à 600 m
    road = [(100, 10, np.array([[0.0, 0.0], [300.0, 0.0], [600.0, 0.0]]), False)]
    region = box(-1e4, -1e4, 1e4, 1e4)
    L = 2700.0                                              # 1 500 de boucle + 2 x 600 d'accès
    p = Params(48.7, 2.27, L / 1000, time_s=6)
    dbg = {}
    access = make_access(lambda: trails + road, region, relief)
    P, g, res, info, acc = search_loop(trails, region, p, L, L * 1.05, relief, dbg, {}, access=access)
    assert acc is not None and info["access_m"] == 600
    assert "access" not in dbg["attempts"][0]               # pas de tableaux dans le debug
    xy, z, s, length, dplus = assemble(g, res, acc)
    assert np.hypot(*xy[0]) < 1.0 and np.allclose(xy[0], xy[-1])     # part du point cliqué, fermé
    assert abs(length - s[-1]) < 1.0 and L * 0.95 <= length <= L * 1.05
    assert abs(G.profile_dplus(z) - dplus) < 1e-6 * max(1.0, dplus)  # D+ profil = D+ annoncé
    assert np.hypot(*np.diff(xy, axis=0).T).max() < 6.0              # pas de saut


def test_no_access_falls_back_to_moved_start():
    from trailopt.pipeline import make_access
    trails = _square(600, 0, 375, 10)
    region = box(-1e4, -1e4, 1e4, 1e4)
    p = Params(48.7, 2.27, 1.5, time_s=6)
    access = make_access(lambda: trails, region, relief)     # aucune route vers le point cliqué
    P, g, res, info, acc = search_loop(trails, region, p, 1500, 1575, relief, {}, {}, access=access)
    assert acc is None and res.feasible and info["start_snap_m"] > 500
