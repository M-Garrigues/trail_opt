"""coverage.geojson : union des dalles du manifeste, WGS84, extérieurs anti-horaires."""
import json

from shapely.geometry import Point, shape

from pipeline.__main__ import main
from pipeline.build import _TO_WGS


def test_coverage(tmp_path):
    tiles = ["32_342", "33_342", "32_343", "40_330"]  # un L de 3 dalles + une isolée
    (tmp_path / "manifest.json").write_text(json.dumps({"tiles": {t: {} for t in tiles}}))
    out = tmp_path / "web" / "coverage.geojson"
    main(["coverage", "--tiles", str(tmp_path), "--out", str(out)])

    fc = json.loads(out.read_text())
    g = shape(fc["features"][0]["geometry"])
    assert fc["features"][0]["properties"]["tiles"] == 4
    assert g.is_valid and len(g.geoms) == 2
    assert all(p.exterior.is_ccw for p in g.geoms)
    centre = Point(*_TO_WGS.transform(32.5 * 20000, 342.5 * 20000))
    trou = Point(*_TO_WGS.transform(33.5 * 20000, 343.5 * 20000))   # coin manquant du L
    assert g.contains(centre) and not g.contains(trou)
    assert out.stat().st_size < 50_000
