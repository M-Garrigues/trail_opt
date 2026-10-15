"""Étiquettes `pipeline enrich` (calm, osm_hike, osm_water, colonnes vélo) sur deux mini-dalles synthétiques voisines, hors ligne."""
import json

import numpy as np
import pytest
from pyproj import Transformer

from pipeline.build import tile_columns
from pipeline.check import check
from pipeline.enrich import (F_COND, F_INFORMAL, F_ROUND, F_STEPS, NOCLASS, bike_level, calm, cycleway_level, derived_version,
                             enrich, maxspeed, oneway_dir, verify, way_class, _scale)
from trailopt import ign

A, B = "32_342", "33_342"                      # B est à l'est de A
X0, Y0 = 32 * 200_000, 342 * 200_000           # origine de A (dm)
E = 200_000                                    # bord est de A (dm)
# id -> (nature, importance, sommets en dm relatifs à l'origine de A)
TRONCONS = {
    1: ("Sentier", 6, [(1000, 1000), (6000, 1000)]),                 # longe la route 2 à 30 m
    2: ("Route à 1 chaussée", 2, [(1000, 1300), (6000, 1300)]),      # départementale
    3: ("Sentier", 6, [(1000, 60000), (6000, 60000)]),               # à 6 km de toute route
    4: ("Chemin", 5, [(E - 5000, 100000), (E - 100, 100000)]),       # finit à 20 m de la route 5 (dalle B)
    5: ("Route à 1 chaussée", 3, [(E + 100, 95000), (E + 100, 105000)]),
    6: ("Route à 1 chaussée", 5, [(1000, 80000), (6000, 80000)]),    # petite route tranquille
    7: ("Route à 1 chaussée", 4, [(1000, 90000), (6000, 90000)]),    # route de desserte, loin de tout
    8: ("Sentier", 6, [(6000, 1300), (6000, 3000)]),                 # part du bout de la route 2 (traversée)
}
SENTIER_PRES, ROUTE, SENTIER_LOIN, CHEMIN_BORD, PETITE_ROUTE, DESSERTE, DEPART = 0, 1, 2, 3, 4, 5, 6   # rangs dans A (par id)


def _write(d, key, ids):
    ix, iy = map(int, key.split("_"))
    n = np.array([len(TRONCONS[i][2]) for i in ids], np.int32)
    xy = np.array([p for i in ids for p in TRONCONS[i][2]], np.int64)
    T, info = tile_columns(ix, iy, dict(
        n=n, nature=np.array([ign.NATURES.index(TRONCONS[i][0]) for i in ids], np.uint8),
        importance=np.array([TRONCONS[i][1] for i in ids], np.uint8), flat=np.zeros(len(n), bool),
        ok=np.ones(len(n), bool), ident=np.array([f"TRONROUT{i:016d}" for i in ids], dtype="S24")),
        xy[:, 0] + X0, xy[:, 1] + Y0, lambda X, Y: np.full(len(X), 100.0))
    np.savez_compressed(d / f"{key}.npz", **T)
    return info


@pytest.fixture
def tiles(tmp_path):
    m = {"format": "tiles/1", "data_version": "bdtopo-wfs-2026-10e", "natures": ign.NATURES,
         "tiles": {A: _write(tmp_path, A, list(TRONCONS)), B: _write(tmp_path, B, list(TRONCONS))}}
    (tmp_path / "manifest.json").write_text(json.dumps(m))
    return tmp_path


TO_WGS = Transformer.from_crs("EPSG:2154", "EPSG:4326", always_xy=True)


def _opl(path, ways, rels, nodes=None):
    """ways : id -> (tags, sommets dm relatifs à A) ; rels : id -> (tags, [ids de ways]) ; nodes : id -> (tags, point)."""
    to_wgs = TO_WGS
    out = []
    for i, (tags, (x, y)) in (nodes or {}).items():
        lon, lat = to_wgs.transform((X0 + x) / 10, (Y0 + y) / 10)
        out.append(f"n{i} T{tags} x{lon:.7f} y{lat:.7f}")
    for i, (tags, pts) in ways.items():
        lon, lat = to_wgs.transform([(X0 + x) / 10 for x, _ in pts], [(Y0 + y) / 10 for _, y in pts])
        out.append(f"w{i} T{tags} N" + ",".join(f"n{k}x{a:.7f}y{b:.7f}" for k, (a, b) in enumerate(zip(lon, lat))))
    for i, (tags, mem) in rels.items():
        out.append(f"r{i} T{tags} M" + ",".join(f"{w}@" if isinstance(w, str) else f"w{w}@" for w in mem))
    path.write_text("\n".join(out) + "\n")
    return path


def test_calm_drops_near_major_road_and_sees_neighbour_tile(tiles):
    before = {k: np.load(tiles / f"{k}.npz") for k in (A, B)}
    m = enrich(tiles, log=lambda s: None)
    c = np.load(tiles / f"{A}.npz")["calm"]
    assert c.dtype == np.uint8 and c[SENTIER_LOIN] == 15 and c[PETITE_ROUTE] == 15 and c[DESSERTE] == 9
    assert c[SENTIER_PRES] == 2                       # 30 m sur 300 : round(15 × 0,1), à l'échantillonnage près
    assert c[ROUTE] == 0                              # route d'importance 2 : jamais calme
    assert 0 < c[CHEMIN_BORD] < 15                    # voit la route de la dalle voisine
    assert np.load(tiles / f"{B}.npz")["calm"].tolist() == [0]    # importance 3 : elle est sa propre route importante
    # colonnes d'origine intactes, manifeste à jour, version dérivée, rien d'OSM sans --osm
    for k, f in before.items():
        g = np.load(tiles / f"{k}.npz")
        assert set(g.files) == set(f.files) | {"calm", "ign_cross"} and all((g[x] == f[x]).all() for x in f.files)
    assert m["data_version"] == "bdtopo-wfs-2026-10e.1" and m["derived_from"] == "bdtopo-wfs-2026-10e"
    assert list(m["columns"]) == ["calm", "ign_cross"] and "IGN" in m["columns"]["calm"]["source"]
    assert m["totals"]["bytes"] == sum((tiles / f"{k}.npz").stat().st_size for k in (A, B))
    assert check(tiles) == []


def test_calm_independent_of_batch_and_checked_at_borders(tiles):
    """Même valeur que la dalle soit traitée seule ou avec les autres ; `check` voit une voisine manquante."""
    alone = calm(tiles, A, ign.NATURES)               # avant tout enrichissement, dalle par dalle
    enrich(tiles, log=lambda s: None)
    enrich(tiles, log=lambda s: None)                 # relançable : les colonnes sont remplacées
    assert np.load(tiles / f"{A}.npz")["calm"].tolist() == alone.tolist()
    m = json.loads((tiles / "manifest.json").read_text())
    assert m["data_version"] == "bdtopo-wfs-2026-10e.2" and m["derived_from"] == "bdtopo-wfs-2026-10e"
    del m["tiles"][B]
    (tiles / f"{B}.npz").unlink()
    (tiles / "manifest.json").write_text(json.dumps(m))
    assert any("calm différent" in b for b in check(tiles))
    assert calm(tiles, A, ign.NATURES)[CHEMIN_BORD] == 15   # sans la voisine, la route n'est pas vue


def test_osm_labels(tiles, tmp_path):
    osm = _opl(tmp_path / "x.opl", {
        11: ("highway=path", [(1000, 60050), (6000, 60050)]),             # même tracé que le sentier 3, à 5 m
        12: ("highway=path", [(3000, 0), (3000, 2000)]),                  # coupe le sentier 1 à angle droit
        13: ("highway=tertiary", [(1000, 80030), (6000, 80030)]),         # balisage sur la petite route 6
        14: ("highway=track", [(E - 5000, 100040), (E - 100, 100040)]),   # même tracé que le chemin 4
        21: ("waterway=river", [(1000, 60200), (6000, 60200)]),           # rivière à 20 m du sentier 3
        22: ("tunnel=culvert,waterway=river", [(1000, 1100), (6000, 1100)]),    # busée : ignorée
        23: ("intermittent=yes,waterway=river", [(1000, 1100), (6000, 1100)]),  # à sec : ignorée
        24: ("natural=water", [(1000, 900), (1100, 900), (1100, 1000), (1000, 900)]),   # mare de 50 m² : ignorée
        25: ("natural=water", [(E - 3000, 100300), (E - 1000, 100300), (E - 1000, 102000),
                               (E - 3000, 102000), (E - 3000, 100300)]),  # étang de 3,4 ha longé sur 200 m
    }, {1: ("network=nwn,route=hiking,type=route", [11, 12, 13]), 2: ("route=foot,type=route", [14]),
        3: ("route=bicycle,type=route", [12])})
    m = enrich(tiles, [osm], log=lambda s: None)
    f = np.load(tiles / f"{A}.npz")
    hike, water = f["osm_hike"], f["osm_water"]
    assert hike[SENTIER_LOIN] == 2 and hike[CHEMIN_BORD] == 1     # GR ; boucle locale
    assert hike[SENTIER_PRES] == 0                                # un tracé qui le coupe ne le balise pas
    assert hike[PETITE_ROUTE] == 0 and hike[ROUTE] == 0           # chemins seulement
    assert water[SENTIER_LOIN] == 15 and water[SENTIER_PRES] == 0 and water[ROUTE] == 0
    assert 5 <= water[CHEMIN_BORD] <= 10                          # ~300 m sur 490 à moins de 50 m de l'étang
    assert f["calm"][SENTIER_LOIN] == 15                          # l'eau n'entre pas dans le calme
    assert m["columns"]["osm_hike"]["license"] == "ODbL 1.0" and "OpenStreetMap" in m["columns"]["osm_water"]["source"]
    assert m["tiles"][A]["osm_hike_km"] == 1.0 and m["enriched"]["osm"] == ["x.opl"]
    assert check(tiles) == []
    # sans --osm ensuite : les colonnes OSM disparaissent avec leur mention au manifeste
    m = enrich(tiles, log=lambda s: None)
    assert "osm_hike" not in np.load(tiles / f"{A}.npz").files and list(m["columns"]) == ["calm", "ign_cross"]


def test_osm_class(tiles, tmp_path):
    """Classe de voie : 0 chemin naturel, 1 intermédiaire, 2 route ; NOCLASS sans appariement (repli IGN)."""
    osm = _opl(tmp_path / "c.opl", {
        31: ("highway=footway", [(1000, 1010), (6000, 1010)]),                  # sentier 1 = allée de ville
        32: ("highway=footway", [(1000, 1340), (6000, 1340)]),                  # longe la route 2 : pas une rue
        33: ("highway=track,surface=gravel", [(1000, 80040), (6000, 80040)]),   # route 6 non revêtue
        34: ("highway=pedestrian", [(1000, 90040), (6000, 90040)]),             # route 7 = rue piétonne
        35: ("highway=path,surface=asphalt", [(1000, 60050), (6000, 60050)]),   # sentier 3 revêtu
    }, {})
    m = enrich(tiles, [osm], log=lambda s: None)
    c = np.load(tiles / f"{A}.npz")["osm_class"]
    assert c[SENTIER_PRES] == 1 and c[SENTIER_LOIN] == 1 and c[PETITE_ROUTE] == 0 and c[DESSERTE] == 1
    assert c[ROUTE] == NOCLASS and c[CHEMIN_BORD] == NOCLASS
    assert m["columns"]["osm_class"]["values"] == [0, 1, NOCLASS] and m["tiles"][A]["osm_mid_km"] == 1.5
    assert check(tiles) == []
    assert way_class({"highway": "track"}) == 0 and way_class({"highway": "track", "tracktype": "grade1"}) == 1
    assert way_class({"highway": "steps"}) == 1 and way_class({"highway": "footway", "surface": "dirt"}) == 0
    assert way_class({"highway": "footway", "footway": "sidewalk"}) == NOCLASS
    assert way_class({"highway": "residential"}) == NOCLASS and way_class({"highway": "service", "surface": "gravel"}) == 0


def test_ign_cross(tiles):
    enrich(tiles, log=lambda s: None)
    c = np.load(tiles / f"{A}.npz")["ign_cross"]
    assert c[ROUTE] == 3 and c[DEPART] == 1 and c[SENTIER_PRES] == 0 and c[PETITE_ROUTE] == 0


def test_osm_ways_points_and_forest(tiles, tmp_path):
    from pipeline.enrich import F_DRINK, F_LIT, F_VIA, F_VIEW, HIGHWAYS, SURFACES
    osm = _opl(tmp_path / "w.opl", {
        41: ("highway=path,surface=rock,sac_scale=alpine_hiking,trail_visibility=bad", [(1000, 60050), (6000, 60050)]),
        42: ("highway=via_ferrata", [(E - 5000, 100040), (E - 100, 100040)]),
        43: ("highway=footway,lit=yes", [(1000, 1010), (6000, 1010)]),
        44: ("highway=residential", [(1000, 90040), (6000, 90040)]),
    }, {}, {51: ("amenity=drinking_water", (3000, 1100)), 52: ("tourism=viewpoint", (3000, 60300)),
            53: ("amenity=drinking_water,drinking_water=no", (3000, 80000))})
    ring = [TO_WGS.transform((X0 + x) / 10, (Y0 + y) / 10) for x, y in
            ((1000, 59000), (3500, 59000), (3500, 61000), (1000, 61000), (1000, 59000))]
    fo = tmp_path / "f.geojsonseq"
    fo.write_text(json.dumps({"type": "Feature", "properties": {}, "geometry": {"type": "Polygon", "coordinates": [ring]}}) + "\n")
    m = enrich(tiles, [osm, fo], log=lambda s: None)
    f = np.load(tiles / f"{A}.npz")
    assert f["osm_highway"][SENTIER_LOIN] == HIGHWAYS.index("path") and f["osm_surface"][SENTIER_LOIN] == SURFACES.index("rock")
    assert f["osm_rough"][SENTIER_LOIN] == 3 and f["osm_sac"][SENTIER_LOIN] == 4 and f["osm_visibility"][SENTIER_LOIN] == 4
    assert f["osm_flags"][CHEMIN_BORD] & F_VIA and f["osm_flags"][SENTIER_PRES] & F_LIT
    assert f["osm_highway"][DESSERTE] == HIGHWAYS.index("residential") and f["osm_class"][DESSERTE] == 255
    assert f["osm_flags"][SENTIER_PRES] & F_DRINK and f["osm_flags"][ROUTE] & F_DRINK       # à 10 et 20 m
    assert not f["osm_flags"][PETITE_ROUTE] & F_DRINK                                       # eau non potable
    assert f["osm_flags"][SENTIER_LOIN] & F_VIEW and not f["osm_flags"][SENTIER_PRES] & F_VIEW
    assert f["osm_forest"][SENTIER_LOIN] in (7, 8) and f["osm_forest"][SENTIER_PRES] == 0      # moitié sous la forêt
    assert SURFACES[m["columns"]["osm_surface"]["paved"][0]] == "asphalt" and SURFACES.index("rock") not in m["columns"]["osm_surface"]["paved"]
    assert m["columns"]["osm_highway"]["codes"][1] == "path" and m["tiles"][A]["osm_forest_km"] > 0
    assert check(tiles) == []


def test_osm_access(tiles, tmp_path):
    """D66 : fermé au piéton dès 20 m appariés ; foot=yes l'emporte ; trottoir privé le long d'une route : ignoré."""
    from pipeline.enrich import ACC_CLOSED, ACC_RESTRICTED, access_level
    osm = _opl(tmp_path / "a.opl", {
        61: ("access=private,highway=path", [(1000, 60050), (6000, 60050)]),              # sentier 3 privé
        62: ("access=private,foot=yes,highway=footway", [(1000, 1010), (6000, 1010)]),    # sentier 1 : piéton admis
        63: ("highway=tertiary", [(1000, 80030), (5750, 80030)]),                          # route 6 publique…
        64: ("access=private,highway=service", [(5750, 80030), (6000, 80030)]),           # … sauf ses 25 derniers m
        65: ("access=destination,highway=residential", [(1000, 90040), (6000, 90040)]),   # route 7 : riverains
        66: ("access=forestry,highway=track", [(E - 5000, 100040), (E - 100, 100040)]),   # chemin 4 : vise les véhicules
        67: ("access=private,highway=footway", [(1000, 1340), (6000, 1340)]),             # longe la route 2 : pas une rue
    }, {})
    m = enrich(tiles, [osm], log=lambda s: None)
    a = np.load(tiles / f"{A}.npz")["osm_access"]
    assert a[SENTIER_LOIN] == ACC_CLOSED and a[PETITE_ROUTE] == ACC_CLOSED and a[DESSERTE] == ACC_RESTRICTED
    assert a[SENTIER_PRES] == 0 and a[CHEMIN_BORD] == 0 and a[ROUTE] == 0
    assert m["tiles"][A]["osm_closed_path_km"] == 0.5 and m["tiles"][A]["osm_closed_road_km"] == 0.5
    assert m["columns"]["osm_access"]["range"] == [0, 2] and check(tiles) == []
    assert access_level({"highway": "path", "foot": "no"}) == ACC_CLOSED
    assert access_level({"highway": "service", "access": "no", "foot": "designated"}) == 0
    assert access_level({"highway": "track", "access": "agricultural"}) == 0
    assert access_level({"highway": "footway", "access": "private", "foot": "private"}) == ACC_CLOSED


def test_derived_version():
    assert derived_version("bdtopo-wfs-2026-10e") == "bdtopo-wfs-2026-10e.1"
    assert derived_version("bdtopo-wfs-2026-10e.9") == "bdtopo-wfs-2026-10e.10"


# ------------------------------------------------------------------------------------------------ vélo (D70)

def _lab(tiles, tmp_path, ways, rels=None, nodes=None, name="v"):
    """Étiquette le dossier avec un OPL synthétique ; renvoie (colonnes de la dalle A, manifeste)."""
    m = enrich(tiles, [_opl(tmp_path / f"{name}.opl", ways, rels or {}, nodes)], log=lambda s: None)
    return np.load(tiles / f"{A}.npz"), m


def test_bike_tag_rules():
    assert bike_level({"highway": "trunk"}) == 4 and bike_level({"highway": "bridleway"}) == 4
    assert bike_level({"highway": "residential", "motorroad": "yes"}) == 4
    assert bike_level({"highway": "footway"}) == 2 and bike_level({"highway": "footway", "bicycle": "designated"}) == 0
    assert bike_level({"highway": "trunk", "bicycle": "yes"}) == 0 and bike_level({"highway": "path"}) == 0
    assert bike_level({"highway": "steps"}) == 4 and bike_level({"highway": "steps", "ramp:bicycle": "yes"}) == 2
    assert bike_level({"highway": "path", "bicycle": "no"}) == 5 and bike_level({"highway": "path", "access": "private"}) == 5
    assert bike_level({"highway": "service", "vehicle": "no", "bicycle": "permissive"}) == 0
    assert bike_level({"highway": "primary", "bicycle": "use_sidepath"}) == 3
    assert bike_level({"highway": "path", "bicycle": "dismount"}) == 2
    assert bike_level({"highway": "residential", "access": "destination"}) == 1 and bike_level({"highway": "pedestrian"}) == 1
    assert bike_level({"highway": "residential"}) == 0 and bike_level({"bicycle": "no"}) == 0     # pas une voie
    assert cycleway_level({"highway": "residential", "cycleway:left": "lane", "cycleway:right": "track"}) == 3
    assert cycleway_level({"highway": "path", "bicycle": "designated"}) == 4 and cycleway_level({"cyclestreet": "yes"}) == 5
    assert oneway_dir({"highway": "residential", "oneway": "-1"}) == 2 and oneway_dir({"highway": "motorway"}) == 1
    assert oneway_dir({"highway": "residential", "oneway": "yes", "oneway:bicycle": "no"}) == 3
    assert oneway_dir({"highway": "residential", "oneway:bicycle": "no"}) == 0
    assert bike_level({"highway": "service", "access": "no", "vehicle": "yes"}) == 0                # vehicle plus précis
    assert bike_level({"highway": "service", "access": "no", "vehicle": "destination"}) == 1
    assert oneway_dir({"highway": "residential", "junction": "circular"}) == 1
    assert oneway_dir({"highway": "residential", "oneway": "yes", "cycleway:left": "lane", "cycleway:left:oneway": "-1"}) == 3
    assert oneway_dir({"highway": "residential", "oneway": "yes", "cycleway:right": "lane", "cycleway:right:oneway": "yes"}) == 1
    assert oneway_dir({"highway": "residential", "oneway": "-1", "cycleway:right:oneway": "yes"}) == 3
    assert _scale("2+", 6) == 3 and _scale("0", 6) == 1 and _scale("x", 6) == 0 and _scale("6", 5) == 0
    assert maxspeed({"maxspeed": "30"}) == 30 and maxspeed({"maxspeed": "FR:zone30"}) == 30
    assert maxspeed({"maxspeed": "50%20%mph"}) == 80 and maxspeed({"zone:maxspeed": "FR:30"}) == 30
    assert maxspeed({"maxspeed": "none", "zone:maxspeed": "FR:30"}) == 0 and maxspeed({"maxspeed": "999"}) == 254


def test_osm_bike_worst_on_20m(tiles, tmp_path):
    """Pire valeur dès 20 m appariés (25 m de bicycle=no : 5 ; 15 m : 0) ; défauts France ; bicycle=yes l'emporte."""
    f, m = _lab(tiles, tmp_path, {
        11: ("highway=path,bicycle=no", [(1000, 60030), (1250, 60030)]),                   # 25 m sur le sentier 3
        12: ("highway=path", [(1250, 60030), (6000, 60030)]),
        13: ("highway=track,bicycle=no", [(E - 5000, 100030), (E - 4850, 100030)]),        # 15 m sur le chemin 4
        14: ("highway=track", [(E - 4850, 100030), (E - 100, 100030)]),
        15: ("highway=trunk", [(1000, 1330), (6000, 1330)]),                                # route 2
        16: ("highway=footway", [(1000, 1010), (6000, 1010)]),                              # sentier 1 : pied à terre
        17: ("highway=bridleway", [(1000, 80030), (6000, 80030)]),                          # route 6 : non appariée (pas une rue)
        18: ("highway=unclassified,bicycle=yes", [(1000, 90030), (6000, 90030)]),
    })
    b = f["osm_bike"]
    assert b[SENTIER_LOIN] == 5 and b[CHEMIN_BORD] == 0 and b[ROUTE] == 4 and b[SENTIER_PRES] == 2
    assert b[PETITE_ROUTE] == 0 and b[DESSERTE] == 0
    t = m["tiles"][A]
    assert t["osm_bike5_km"] == 0.5 and t["osm_bike2_km"] == 0.5 and t["osm_bike_closed_road_km"] == 0.5
    assert t["osm_matched_km"] > 0 and m["totals"]["osm_bike4_km"] == 0.5 and check(tiles) == []
    # chemin barré par bridleway à pied d'œuvre : un bridleway sur un CHEMIN vaut 4
    f, _ = _lab(tiles, tmp_path, {21: ("highway=bridleway", [(1000, 60030), (6000, 60030)])}, name="w")
    assert f["osm_bike"][SENTIER_LOIN] == 4


def test_osm_oneway_oriented(tiles, tmp_path):
    """Sens unique vélo orienté u→v du tronçon, quel que soit le sens de numérisation du way."""
    fwd, rev = [(1000, 80030), (6000, 80030)], [(6000, 80030), (1000, 80030)]
    f, m = _lab(tiles, tmp_path, {
        31: ("highway=residential,oneway=yes", fwd),                                        # route 6 : 1
        32: ("highway=residential,oneway=yes", [(6000, 90030), (1000, 90030)]),              # route 7, numérisée à l'envers : 2
        33: ("highway=residential,oneway=yes,oneway:bicycle=no", [(1000, 1330), (6000, 1330)]),     # route 2 : 3
    })
    o = f["osm_oneway"]
    assert (o[PETITE_ROUTE], o[DESSERTE], o[ROUTE]) == (1, 2, 3) and f["osm_oneway"].dtype == np.uint8
    assert m["tiles"][A]["osm_oneway_km"] == 1.0 and m["tiles"][A]["osm_oneway3_km"] == 0.5
    f, _ = _lab(tiles, tmp_path, {
        34: ("highway=residential,oneway=-1", fwd),                                         # 2
        35: ("highway=residential,junction=roundabout", [(1000, 90030), (6000, 90030)]),     # rond-point implicite : 1 + bit
        36: ("highway=residential,oneway=yes,cycleway:left=opposite_lane", [(6000, 1330), (1000, 1330)]),   # 3
    }, name="w")
    assert (f["osm_oneway"][PETITE_ROUTE], f["osm_oneway"][DESSERTE], f["osm_oneway"][ROUTE]) == (2, 1, 3)
    assert f["osm_flags"][DESSERTE] & F_ROUND and not f["osm_flags"][PETITE_ROUTE] & F_ROUND


def test_osm_networks(tiles, tmp_path):
    """Véloroute : niveau max du réseau, sur chemins ET routes, hérité à travers plusieurs étages de relations
    (et sans boucle infinie si deux relations se contiennent) ; route=mtb ; GR hérité de la même façon."""
    f, m = _lab(tiles, tmp_path, {
        41: ("highway=path", [(1000, 60030), (6000, 60030)]),
        42: ("highway=residential", [(1000, 80030), (6000, 80030)]),
        43: ("highway=path", [(1000, 1030), (6000, 1030)]),
        44: ("highway=residential", [(1000, 90030), (6000, 90030)]),
        45: ("highway=track", [(E - 5000, 100030), (E - 100, 100030)]),
    }, {
        1: ("route=bicycle,network=icn,type=route", ["r2"]),                                  # EuroVelo -> étape -> sous-étape -> way
        2: ("route=bicycle,network=ncn,type=route", ["r3"]),
        3: ("route=bicycle,network=lcn,type=route", [41]),
        4: ("route=bicycle,type=route", [42]),                                                # sans network : local
        5: ("route=mtb,network=rcn,type=route", [43]),
        6: ("route=mtb,type=route", [44]),
        7: ("route=bicycle,network=ncn,type=route", ["r8", 45]),                              # deux relations qui se contiennent
        8: ("route=bicycle,network=lcn,type=route", ["r7"]),
        9: ("route=hiking,network=rwn,type=route", ["r10"]),                                  # GR sur deux étages
        10: ("route=hiking,type=route", ["r11"]),
        11: ("route=hiking,type=route", [41]),
    })
    assert f["osm_bike_route"][SENTIER_LOIN] == 4 and f["osm_bike_route"][PETITE_ROUTE] == 1
    assert f["osm_bike_route"][CHEMIN_BORD] == 3 and f["osm_bike_route"][ROUTE] == 0
    assert f["osm_mtb_route"][SENTIER_PRES] == 2 and f["osm_mtb_route"][DESSERTE] == 1 and f["osm_mtb_route"][SENTIER_LOIN] == 0
    assert f["osm_hike"][SENTIER_LOIN] == 2
    t = m["tiles"][A]
    assert t["osm_bike_route4_km"] == 0.5 and t["osm_bike_route1_km"] == 0.5 and t["osm_mtb_route_km"] == 1.0
    assert m["columns"]["osm_bike_route"]["range"] == [0, 4] and check(tiles) == []


def test_osm_cycleway_mtb_and_attributes(tiles, tmp_path):
    f, m = _lab(tiles, tmp_path, {
        51: ("highway=cycleway", [(1000, 60030), (6000, 60030)]),                           # sentier 3 = voie dédiée
        52: ("highway=path,mtb:scale=2+,mtb:scale:uphill=1,smoothness=very_bad,informal=yes", [(1000, 1010), (1120, 1010)]),
        53: ("highway=steps", [(1120, 1010), (6000, 1010)]),
        54: ("highway=track,tracktype=grade3,surface=gravel", [(E - 5000, 100010), (E - 4920, 100010)]),   # 8 m d'escalier plus bas
        55: ("highway=tertiary,maxspeed=FR:urban,cycleway:right=lane", [(1000, 80030), (6000, 80030)]),
        56: ("highway=residential,maxspeed=50%20%mph,bicycle_road=yes", [(1000, 90030), (6000, 90030)]),
        57: ("highway=primary,vehicle:conditional=no%20%@%20%(Nov%20%1-Apr%20%1)", [(1000, 1330), (6000, 1330)]),
        58: ("highway=track,tracktype=grade3", [(E - 4920, 100010), (E - 100, 100010)]),
    })
    assert f["osm_cycleway"][SENTIER_LOIN] == 4 and f["osm_bike"][SENTIER_LOIN] == 0
    assert f["osm_mtb"][SENTIER_PRES] == 0                                  # 12 m de mtb:scale, puis escalier : < 20 m
    assert f["osm_flags"][SENTIER_PRES] & F_STEPS and not f["osm_flags"][CHEMIN_BORD] & F_STEPS
    assert f["osm_tracktype"][CHEMIN_BORD] == 3 and f["osm_flags"][SENTIER_PRES] & F_INFORMAL == 0
    assert (f["osm_cycleway"][PETITE_ROUTE], f["osm_maxspeed"][PETITE_ROUTE]) == (2, 50)
    assert (f["osm_cycleway"][DESSERTE], f["osm_maxspeed"][DESSERTE]) == (5, 80)
    assert f["osm_flags"][ROUTE] & F_COND and not f["osm_flags"][PETITE_ROUTE] & F_COND
    assert m["columns"]["osm_flags"]["range"] == [0, 255] and m["tiles"][A]["osm_cycleway_km"] == 1.5 and check(tiles) == []
    f, m = _lab(tiles, tmp_path, {                                          # 25 m d'un chemin : mtb:scale, smoothness, informal
        61: ("highway=path,mtb:scale=2+,mtb:scale:uphill=1,smoothness=very_bad,informal=yes", [(1000, 60030), (1250, 60030)]),
        62: ("highway=path", [(1250, 60030), (6000, 60030)]),
    }, name="w")
    assert (f["osm_mtb"][SENTIER_LOIN], f["osm_mtb_up"][SENTIER_LOIN]) == (3, 2) and f["osm_flags"][SENTIER_LOIN] & F_INFORMAL
    assert f["osm_smooth"][SENTIER_LOIN] == 0                               # smoothness : majoritaire, 25 m sur 500 m
    assert m["tiles"][A]["osm_mtb_km"] == 0.5


def test_osm_lateral_cycleway(tiles, tmp_path):
    """Route IGN : way cyclable parallèle à ≤ 15 m sur ≥ 60 % -> 3 ; à 25 m, ou trottoir non désigné -> 0."""
    f, _ = _lab(tiles, tmp_path, {
        71: ("highway=residential", [(1000, 1330), (6000, 1330)]),
        72: ("highway=cycleway", [(1000, 1380), (6000, 1380)]),                             # à 8 m de la route 2
        73: ("highway=tertiary", [(1000, 80030), (6000, 80030)]),
        74: ("highway=cycleway", [(1000, 80250), (6000, 80250)]),                           # à 25 m de la route 6
        75: ("highway=residential", [(1000, 90030), (6000, 90030)]),
        76: ("highway=footway", [(1000, 90080), (6000, 90080)]),                            # trottoir non désigné
    })
    c = f["osm_cycleway"]
    assert c[ROUTE] == 3 and c[PETITE_ROUTE] == 0 and c[DESSERTE] == 0


def test_bike_columns_absent_without_osm_and_verify(tiles, tmp_path):
    m = enrich(tiles, log=lambda s: None)
    assert not any(k.startswith("osm_") for k in np.load(tiles / f"{A}.npz").files) and "osm_bike" not in m["columns"]
    f, m = _lab(tiles, tmp_path, {81: ("highway=path", [(1000, 60030), (6000, 60030)])})
    assert verify(tiles, m) == []
    T = {k: v for k, v in np.load(tiles / f"{A}.npz").items()}
    T["osm_mtb"] = T["osm_mtb"].copy()
    T["osm_mtb"][ROUTE] = 1                                                  # mtb:scale sur une route : impossible
    np.savez_compressed(tiles / f"{A}.npz", **T)
    assert any("hors chemin" in b for b in verify(tiles, m))


def test_pn_core(tiles, tmp_path):
    """pn_core : au moins la moitié du tronçon dans un cœur terrestre (CTPN) ; aire d'adhésion, cœur marin ignorés ; sans fichier, colonne absente."""
    def feat(code, x0, x1, y0, y1):
        ring = [TO_WGS.transform((X0 + x) / 10, (Y0 + y) / 10) for x, y in ((x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0))]
        return {"type": "Feature", "properties": {"code_r_enp": code}, "geometry": {"type": "MultiPolygon", "coordinates": [[ring]]}}
    pn = tmp_path / "pn.geojson"
    pn.write_text(json.dumps({"type": "FeatureCollection", "features": [
        feat("CTPN", 0, 10000, 59000, 61000),        # sentier 3 entièrement dedans
        feat("CTPN", 0, 3400, 79000, 81000),         # route 6 : 48 % seulement
        feat("CTPN", 0, 4000, 89000, 91000),         # route 7 : 60 %
        feat("AAPN", 0, 10000, 0, 2000),             # aire d'adhésion sur sentier 1 et route 2 : ignorée
        feat("CMPN", 0, 10000, 99000, 101000)]}))
    m = enrich(tiles, pn=pn, log=lambda s: None)
    c = np.load(tiles / f"{A}.npz")["pn_core"]
    assert c.dtype == np.uint8 and c[SENTIER_LOIN] == 1 and c[DESSERTE] == 1
    assert c[PETITE_ROUTE] == 0 and c[SENTIER_PRES] == 0 and c[ROUTE] == 0 and c[CHEMIN_BORD] == 0
    assert m["tiles"][A]["pn_core_km"] == 1.0 and m["totals"]["pn_core_km"] == 1.0
    assert m["columns"]["pn_core"]["license"] == "Licence Ouverte 2.0" and m["enriched"]["pn"] == "pn.geojson" and check(tiles) == []
    m = enrich(tiles, log=lambda s: None)
    assert "pn_core" not in np.load(tiles / f"{A}.npz").files and "pn_core" not in m["columns"]
