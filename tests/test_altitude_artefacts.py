"""Pics d'altitude (D27, route de Villard-Notre-Dame dans l'Oisans) : falaise masquée, portail de
tunnel, pic isolé. Hors ligne."""
import numpy as np

from pipeline.build import tile_columns
from trailopt import elevation as el
from trailopt import graph, ign

IX, IY = 46, 322
X0, Y0 = IX * 200_000, IY * 200_000           # origine de dalle (dm)


def _edge(z, flat=False):
    n = len(z)
    return graph.Edge(0, 1, np.zeros((n, 2)), np.arange(n) * 5.0, flat)


def test_despike_keeps_slopes_removes_spikes():
    ramp = 800 + 5.0 * np.arange(30)                       # 100 % : relief réel, intact
    assert np.array_equal(graph.despike(_edge(ramp), ramp), ramp)
    z = 800 + 0.5 * np.arange(30)
    spike = z.copy()
    spike[14:16] += 150                                     # pic isolé de +150 m
    out = graph.despike(_edge(spike), spike)
    assert np.isnan(out[14:16]).all() and np.array_equal(out[:14], z[:14])
    portal = z.copy()
    portal[-4:] = [858, 900, 932, 936]                      # bout de tronçon dans la falaise
    assert np.isnan(graph.despike(_edge(portal), portal)[-4:]).all()
    assert np.array_equal(graph.despike(_edge(spike, flat=True), spike), spike)   # tunnel : inchangé


def test_tunnel_portal_takes_ground_altitude():
    """Le tunnel (id plus petit, donc vu en premier) passe sous 300 m de roche : son portail doit
    prendre l'altitude de la route au sol, pas celle du terrain au-dessus."""
    road = [(1000, 1000), (3000, 1000)]                     # route au sol, 500 m
    tunnel = [(3000, 1000), (6000, 1000)]                   # tunnel sous la montagne
    A = dict(n=np.array([2, 2], np.int32),
             nature=np.full(2, ign.NATURES.index("Route à 1 chaussée"), np.uint8),
             importance=np.full(2, 4, np.uint8), flat=np.array([True, False]), ok=np.ones(2, bool),
             ident=np.array([b"TRONROUT0000000000000001", b"TRONROUT0000000000000002"], dtype="S24"))
    xy = np.array(tunnel + road, np.int64)

    def sample(X, Y):                                       # portail (x = 300 m) dans la falaise
        x = X - X0 / 10
        return np.where(x >= 300, 800.0, 500.0)

    T, info = tile_columns(IX, IY, A, xy[:, 0] + X0, xy[:, 1] + Y0, sample)
    assert T["prof_z0_dm"].tolist() == [5000, 5000]         # portail = route, pas 800 m
    assert info["jump_gt10"] == 0 and info["node_fallback"] == 0


def test_cliff_road_falls_back_to_raw_dem(monkeypatch):
    """Route taillée dans la falaise : le test de saut (> 15 m entre pixels) invalide tout autour,
    dans les deux couches. On reprend alors le MNT brut (trous toujours exclus)."""
    grid = np.full((40, 40), 840.0)
    grid[:, 25:] = 1100.0                                    # falaise de 260 m à 25 m de la route
    bb = (0.0, 0.0, 200.0, 200.0)
    monkeypatch.setattr(el, "_tiles", lambda X, Y: iter([bb]))
    monkeypatch.setattr(el, "_fetch_tile", lambda layer, core, stats: (bb, None))
    monkeypatch.setattr(el, "_read", lambda p, mask=True, jumps=True: el.mask_unreliable(grid.copy(), jumps))
    X, Y = np.array([112.0, 60.0]), np.array([100.0, 100.0])  # route au pied de la falaise ; loin
    assert np.isnan(el.mask_unreliable(grid)[20, 22])         # masque complet : NaN au bord
    assert np.allclose(el.sample_l93(X, Y), [840.0, 840.0])
    hole = grid.copy()
    hole[:, :5] = np.nan
    assert np.isnan(el.mask_unreliable(hole, jumps=False)[20, 10])   # trous : toujours masqués


def test_road_grade_cap_bridges_cliff_steps():
    """Route en corniche (gorges de la Bourne) : le MNT saute de 85 m en 25 m. La marche devient une
    droite à ≤ 30 %, les extrémités (nœuds) et le reste du profil ne bougent pas."""
    s = np.arange(100) * 5.0
    z = np.where(s < 250, 573.0, 488.0)
    z[50:54] = [559, 537, 515, 498]
    out = graph.cap_grade(s, z, 0.30)
    assert (np.abs(np.diff(out)) <= 0.30 * 5 + 1e-9).all()
    assert out[0] == 573 and out[-1] == 488
    assert np.array_equal(out[:5], z[:5]) and np.array_equal(out[-5:], z[-5:])
    flat = 500 + 0.1 * s                                    # route normale : intacte
    assert np.array_equal(graph.cap_grade(s, flat, 0.30), flat)
