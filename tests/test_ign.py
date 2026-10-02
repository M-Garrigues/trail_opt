"""Source IGN BD TOPO : filtrage par nature et topologie par extrémités."""
import numpy as np
import pytest

from trailopt import ign
from trailopt.geo import LocalFrame

BASE = {"etat_de_l_objet": "En service", "prive": False, "acces_pieton": None,
        "position_par_rapport_au_sol": "0"}
LINE = [[2.27, 48.73, 80.0], [2.271, 48.73, 81.0]]


def P(nature, importance="5", **kw):
    return {**BASE, "nature": nature, "importance": importance, **kw}


def F(props, coords=LINE, ident="A"):
    return {"properties": {"cleabs": ident, **props}, "geometry": {"type": "LineString", "coordinates": coords}}


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
    (P("Nature inconnue", "5"), "all", False),
    (P("Sentier", "6", prive=True), "all", False),
    (P("Sentier", "6", acces_pieton="Restreint aux ayants droit"), "all", False),
    (P("Sentier", "6", etat_de_l_objet="En construction"), "all", False),
])
def test_keep_mask(props, roads, expected):
    assert bool(ign.keep_mask(ign.to_arrays([F(props)]), roads)[0]) is expected


def test_to_edges_topology_and_bridges():
    frame = LocalFrame(48.73, 2.27)
    a, b, c = [2.2700, 48.7300, 80.0], [2.2710, 48.7300, 81.0], [2.2710, 48.7310, 85.0]
    A = ign.to_arrays([
        F(P("Sentier", "6"), [a, b], "T1"),
        F(P("Sentier", "6", position_par_rapport_au_sol="1"), [b, c], "T2"),     # pont
        F(P("Route à 1 chaussée", "5"), [c, a], "T3"),                            # filtrée en piéton
    ])
    raw, n = ign.to_edges(A, frame, "pedestrian")
    assert n == 2
    (u0, v0, xy0, flat0), (u1, v1, xy1, flat1) = raw
    assert v0 == u1 and u0 != v1            # carrefour partagé par coordonnées
    assert (flat0, flat1) == (False, True)
    assert np.allclose(xy0[0], [0, 0], atol=1.0)
    raw_all, n_all = ign.to_edges(A, frame, "minor")
    assert n_all == 3 and raw_all[2][1] == raw_all[0][0]   # la boucle se referme


def test_tiles_are_merged_without_duplicates():
    """Un tronçon à cheval sur deux dalles est servi par les deux : il ne doit compter qu'une fois."""
    t1 = ign.to_arrays([F(P("Sentier", "6"), LINE, "A"), F(P("Chemin", "6"), LINE, "B")])
    t2 = ign.to_arrays([F(P("Chemin", "6"), LINE, "B"), F(P("Escalier", "6"), LINE, "C")])
    A = ign._concat([t1, t2])
    assert sorted(A["ident"].tolist()) == [b"A", b"B", b"C"]
    assert len(A["lon"]) == int(A["n"].sum()) == 6
    assert ign._concat([])["n"].size == 0


def test_geocode_parses_coordinates_without_network():
    from trailopt import geocode
    assert geocode.search("48.7303, 2.2725")[0]["lat"] == pytest.approx(48.7303)
    assert geocode.search("45,92 ; 6,87")[0]["lon"] == pytest.approx(6.87)
    assert geocode.search("zz") == []
