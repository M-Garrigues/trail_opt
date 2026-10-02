"""Couloirs parallèles : deux trottoirs ou un chemin saisi deux fois = un seul couloir."""
import numpy as np
import pytest

from trailopt import graph
from trailopt.solvers import optimize

from conftest import build, check_loop


def _g(lines):
    """lines : liste de polylignes ; chaque extrémité a un id unique sauf si partagée."""
    edges, ids = {}, {}
    for k, xy in enumerate(lines):
        xy = np.asarray(xy, float)
        u = ids.setdefault(tuple(xy[0]), len(ids))
        v = ids.setdefault(tuple(xy[-1]), len(ids))
        p, s = graph.densify(xy, 5.0)
        edges[k] = graph.Edge(u, v, p, s)
    return graph.Graph(edges)


FAR = 1000.0  # loin du départ (rayon libre exempté)


@pytest.mark.parametrize("lines,expected", [
    # deux trottoirs de 400 m à 10 m : même couloir
    ([[(FAR, 0), (FAR + 400, 0)], [(FAR, 10), (FAR + 400, 10)]], {0: {1}, 1: {0}}),
    # sens de saisie opposés : toujours le même couloir
    ([[(FAR, 0), (FAR + 400, 0)], [(FAR + 400, 10), (FAR, 10)]], {0: {1}, 1: {0}}),
    # se longent sur 20 m seulement
    ([[(FAR, 0), (FAR + 400, 0)], [(FAR + 380, 10), (FAR + 780, 10)]], {}),
    # perpendiculaires qui se croisent
    ([[(FAR, 0), (FAR + 400, 0)], [(FAR + 200, -200), (FAR + 200, 200)]], {}),
    # deux petits bouts de 20 m côte à côte : sous le seuil de 30 m
    ([[(FAR, 0), (FAR + 20, 0)], [(FAR, 10), (FAR + 20, 10)]], {}),
    # petit bout de 25 m à côté d'un long tronçon : trop court
    ([[(FAR, 0), (FAR + 400, 0)], [(FAR + 100, 8), (FAR + 125, 8)]], {}),
    # bout de 40 m entièrement le long d'un long tronçon : même couloir
    ([[(FAR, 0), (FAR + 400, 0)], [(FAR + 100, 8), (FAR + 140, 8)]], {0: {1}, 1: {0}}),
    # tronçons consécutifs alignés (se touchent au carrefour seulement)
    ([[(FAR, 0), (FAR + 400, 0)], [(FAR + 400, 0), (FAR + 800, 0)]], {}),
    # deux chemins en V qui partent du même carrefour
    ([[(FAR, 0), (FAR + 400, 60)], [(FAR, 0), (FAR + 400, -60)]], {}),
    # à 25 m : deux voies distinctes
    ([[(FAR, 0), (FAR + 400, 0)], [(FAR, 25), (FAR + 400, 25)]], {}),
])
def test_parallel_detection(lines, expected):
    assert graph.parallel_pairs(_g(lines)) == expected


def test_free_radius_exempt():
    lines = [[(0, 0), (150, 0)], [(0, 10), (150, 10)]]
    assert graph.parallel_pairs(_g(lines)) == {}


def _ladder_raw():
    """Grand carré de 1,6 km dont un côté est doublé par un trottoir parallèle à 10 m,
    relié par deux traversées : sans la règle, l'aller-retour par les deux trottoirs
    permet une boucle plus longue."""
    A, B, C, D = (0, -100), (400, -100), (400, 300), (0, 300)
    B2, C2 = (410, -100), (410, 300)
    seg = lambda p, q: np.array([p, q], float)
    return [(1, 2, seg(A, B), False), (2, 3, seg(B, C), False), (3, 4, seg(C, D), False),
            (4, 1, seg(D, A), False), (2, 5, seg(B, B2), False), (5, 6, seg(B2, C2), False),
            (6, 3, seg(C2, C), False)]


@pytest.mark.parametrize("solver", ["exact", "anneal"])
def test_solvers_respect_parallel(solver):
    P, _ = build(1600, raw=_ladder_raw(), tol=0.2)
    assert P.parallel, "les deux trottoirs doivent être détectés"
    res = optimize(P, budget=3.0, solver=solver)
    check_loop(P, res.circuit)


def test_grid_has_no_false_parallel():
    P, _ = build(2000)   # grille de 100 m : aucun couloir parallèle
    assert P.parallel == {}
