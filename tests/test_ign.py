"""Source IGN BD TOPO : filtrage par nature et topologie par extrémités."""
import numpy as np
import pytest

from trailopt import ign
from trailopt.geo import LocalFrame

BASE = {"etat_de_l_objet": "En service", "prive": False, "acces_pieton": None,
        "position_par_rapport_au_sol": "0"}


def P(nature, importance="5", **kw):
    return {**BASE, "nature": nature, "importance": importance, **kw}


@pytest.mark.parametrize("props,roads,expected", [
    (P("Sentier", "6"), "unpaved", True),
    (P("Route empierrée"), "unpaved", True),
    (P("Escalier", "6"), "unpaved", False),
    (P("Escalier", "6"), "pedestrian", True),
    (P("Route à 1 chaussée", "5"), "pedestrian", False),
    (P("Route à 1 chaussée", "5"), "minor", True),
    (P("Route à 1 chaussée", "3"), "minor", False),
    (P("Route à 1 chaussée", "3"), "all", True),
    (P("Route à 2 chaussées", "2"), "all", True),
    (P("Type autoroutier", "1"), "all", False),
    (P("Sentier", "6", prive=True), "all", False),
    (P("Sentier", "6", acces_pieton="Restreint aux ayants droit"), "all", False),
    (P("Sentier", "6", etat_de_l_objet="En construction"), "all", False),
])
def test_keep(props, roads, expected):
    assert ign.keep(props, roads) is expected


def test_to_edges_topology_and_bridges():
    frame = LocalFrame(48.73, 2.27)
    a, b, c = [2.2700, 48.7300, 80.0], [2.2710, 48.7300, 81.0], [2.2710, 48.7310, 85.0]
    feats = [
        {"p": P("Sentier", "6"), "c": [a, b]},
        {"p": P("Sentier", "6", position_par_rapport_au_sol="1"), "c": [b, c]},   # pont
        {"p": P("Route à 1 chaussée", "5"), "c": [c, a]},                          # filtrée en piéton
    ]
    raw, n = ign.to_edges(feats, frame, "pedestrian")
    assert n == 2
    (u0, v0, xy0, flat0), (u1, v1, xy1, flat1) = raw
    assert v0 == u1 and u0 != v1            # carrefour partagé par coordonnées
    assert (flat0, flat1) == (False, True)
    assert np.allclose(xy0[0], [0, 0], atol=1.0)
    raw_all, n_all = ign.to_edges(feats, frame, "minor")
    assert n_all == 3 and raw_all[2][1] == raw_all[0][0]   # la boucle se referme


def test_geocode_parses_coordinates_without_network():
    from trailopt import geocode
    assert geocode.search("48.7303, 2.2725")[0]["lat"] == pytest.approx(48.7303)
    assert geocode.search("45,92 ; 6,87")[0]["lon"] == pytest.approx(6.87)
    assert geocode.search("zz") == []
