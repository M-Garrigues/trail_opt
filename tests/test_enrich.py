"""Étiquettes `pipeline enrich` (calm, osm_hike, osm_water) sur deux mini-dalles synthétiques voisines, hors ligne."""
import json

import numpy as np
import pytest
from pyproj import Transformer

from pipeline.build import tile_columns
from pipeline.check import check
from pipeline.enrich import NOCLASS, calm, derived_version, enrich, way_class
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
        out.append(f"r{i} T{tags} M" + ",".join(f"w{w}@" for w in mem))
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
    assert m["columns"]["osm_highway"]["codes"][1] == "path" and m["tiles"][A]["osm_forest_km"] > 0
    assert check(tiles) == []


def test_derived_version():
    assert derived_version("bdtopo-wfs-2026-10e") == "bdtopo-wfs-2026-10e.1"
    assert derived_version("bdtopo-wfs-2026-10e.9") == "bdtopo-wfs-2026-10e.10"
