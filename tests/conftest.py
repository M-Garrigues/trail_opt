"""Graphe synthétique en grille + relief analytique, sans réseau."""
from __future__ import annotations

import numpy as np
import pytest
from shapely.geometry import box

from trailopt import graph
from trailopt.pipeline import prepare_graph


def relief(xy):
    x, y = xy[:, 0], xy[:, 1]
    return 40 * np.sin(x / 250) + 30 * np.cos(y / 180) + 0.05 * x + 100


def grid_raw(n=7, h=100.0):
    """Grille n x n (pas h), départ hors nœud ; un pont, une arête parallèle,
    une impasse et une chaîne de degré 2 pour exercer élagage et simplification."""
    off = np.array([13.0, 37.0])

    def p(i, j):
        return np.array([(j - n // 2) * h, (i - n // 2) * h]) + off

    raw = []
    for i in range(n):
        for j in range(n):
            a = i * n + j
            if j + 1 < n:  # horizontale avec un sommet intermédiaire légèrement décalé
                mid = (p(i, j) + p(i, j + 1)) / 2 + [0, 8]
                raw.append((a, a + 1, np.array([p(i, j), mid, p(i, j + 1)]), False))
            if i + 1 < n:
                raw.append((a, a + n, np.array([p(i, j), p(i + 1, j)]), (i, j) == (1, 1)))
    # parallèle courbe entre deux nœuds voisins
    raw.append((0, 1, np.array([p(0, 0), p(0, 0) + [50, -40], p(0, 1)]), False))
    # impasse (pont au sens de Tarjan) : doit être élaguée
    raw.append((n * n - 1, 10_000, np.array([p(n - 1, n - 1), p(n - 1, n - 1) + [80, 80]]), False))
    # chaîne de degré 2 : deux tronçons à contracter, en parallèle d'une arête de grille
    q = p(n - 1, 0) + [50, 60]
    raw.append((n * (n - 1), 20_000, np.array([p(n - 1, 0), q]), False))
    raw.append((20_000, n * (n - 1) + 1, np.array([q, p(n - 1, 1)]), False))
    return raw


def build(L, mode="max", tol=0.05, D=None, n=7, max_grade=None, node_simple=False, raw=None):
    raw = grid_raw(n) if raw is None else raw
    region = box(-1e4, -1e4, 1e4, 1e4)
    Lmax = L * (1 + tol) if mode == "max" else 1.3 * L
    dbg = {}
    g, s = prepare_graph(raw, region, Lmax, relief, max_grade, dbg)
    return graph.Problem(g, s, L, mode, tol, D, node_simple), dbg


def check_loop(P, circuit, tol=1e-6):
    """Boucle fermée en s, sans arête répétée, continue, parité, D+ profil = Σw."""
    ids = [e for e, _, _ in circuit]
    assert ids, "boucle vide"
    assert len(ids) == len(set(ids)), "arête répétée"
    assert circuit[0][1] == P.s and circuit[-1][2] == P.s, "boucle non fermée en s"
    for (e1, _, b), (_, a, _) in zip(circuit, circuit[1:]):
        assert b == a, "discontinuité"
    for e, a, b in circuit:
        ed = P.g.edges[e]
        assert {a, b} == {ed.u, ed.v}
    deg = {}
    for e in ids:
        ed = P.g.edges[e]
        deg[ed.u] = deg.get(ed.u, 0) + 1
        deg[ed.v] = deg.get(ed.v, 0) + 1
    assert all(d % 2 == 0 for d in deg.values()), "degré impair"
    xy, z, s = graph.route_geometry(P.g, circuit)
    assert np.allclose(xy[0], xy[-1]), "géométrie non fermée"
    steps = np.hypot(*np.diff(xy, axis=0).T)
    assert steps.max() < 6.0, "saut dans la géométrie"
    if P.node_simple:
        assert P.node_simple_ok(circuit), "carrefour repassé hors du rayon libre"
    assert P.parallel_ok(ids), "couloir parallèle emprunté deux fois"
    sum_w = float(P.w[ids].sum())
    assert abs(graph.profile_dplus(z) - sum_w) <= tol * max(1.0, sum_w), "D+ profil != Σw"
    return float(P.len[ids].sum()), sum_w


@pytest.fixture
def problem_factory():
    return build


def dead_end_raw():
    """Départ sur une impasse de 200 m reliée à un carré de 400 m de côté :
    la seule boucle possible fait l'aller-retour sur l'impasse."""
    A, B = np.array([0.0, -50.0]), np.array([0.0, 150.0])
    C, D, E = np.array([400.0, 150.0]), np.array([400.0, 550.0]), np.array([0.0, 550.0])
    return [(1, 2, np.array([A, B]), False),
            (2, 3, np.array([B, C]), False), (3, 4, np.array([C, D]), False),
            (4, 5, np.array([D, E]), False), (5, 2, np.array([E, B]), False),
            (3, 5, np.array([C, (C + E) / 2 + [30, 0], E]), False)]  # diagonale (carrefours)
