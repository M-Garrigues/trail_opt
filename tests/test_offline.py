"""Chaîne complète sur fixtures réelles (Overpass + dalle LiDAR HD, Massy), sans réseau."""
from pathlib import Path

import gpxpy
import pytest

from trailopt.pipeline import Params, UserError, plan_loop

FIX = Path(__file__).parent / "fixtures" / "cache"
START = (48.7303, 2.2725)


@pytest.fixture(autouse=True)
def offline(monkeypatch):
    monkeypatch.setenv("TRAILOPT_CACHE_DIR", str(FIX))
    monkeypatch.setenv("TRAILOPT_OFFLINE", "1")


@pytest.mark.parametrize("source,roads,mode", [
    ("osm", "pedestrian", "max"), ("osm", "pedestrian", "target"), ("ign", "minor", "max")])
def test_real_fixture_end_to_end(source, roads, mode):
    p = Params(*START, distance_km=3, time_s=5, mode=mode, source=source, roads=roads,
               target_dplus=60 if mode == "target" else None)
    r = plan_loop(p)
    assert r.debug["network"]["overpass"]["requests"] == 0
    assert r.debug["network"]["wfs_ign"]["requests"] == 0
    assert r.debug["network"]["wms_r"]["requests"] == 0
    assert abs(r.debug["profile_dplus_m"] - r.dplus) <= 1e-6 * max(1, r.dplus)
    assert not [w for w in r.warnings if "Incohérence" in w]
    if mode == "max":
        assert r.feasible and 2850 <= r.length <= 3150
        assert r.dplus > 50
    gpx = gpxpy.parse(r.gpx)
    pts = gpx.tracks[0].segments[0].points
    assert len(pts) > 100 and all(pt.elevation is not None for pt in pts)
    assert pts[0].distance_2d(pts[-1]) < 1.0
    # le tracé part du point de départ (voie la plus proche)
    assert abs(pts[0].latitude - START[0]) < 0.001 and abs(pts[0].longitude - START[1]) < 0.001


def test_start_outside_polygon_rejected():
    poly = [(2.30, 48.75), (2.31, 48.75), (2.31, 48.76), (2.30, 48.76)]
    with pytest.raises(UserError, match="départ"):
        plan_loop(Params(*START, distance_km=3, polygon=poly))


def test_limits_rejected():
    with pytest.raises(UserError):
        plan_loop(Params(*START, distance_km=40))
    with pytest.raises(UserError):
        plan_loop(Params(*START, distance_km=10, time_s=120))
    big = [(2.0, 48.6), (2.5, 48.6), (2.5, 48.9), (2.0, 48.9)]
    with pytest.raises(UserError, match="trop grande"):
        plan_loop(Params(*START, distance_km=25, polygon=big))


def test_pedestrian_always_uses_osm():
    r = plan_loop(Params(*START, distance_km=3, time_s=5, roads="pedestrian", source="ign"))
    assert r.debug["source"] == "OpenStreetMap"
    assert r.debug["network"]["wfs_ign"] == {"requests": 0, "cache_hits": 0}


def test_defaults_are_ign_minor_roads():
    p = Params(*START, distance_km=3)
    assert (p.source, p.roads) == ("ign", "minor")
    assert plan_loop(Params(*START, distance_km=3, time_s=5)).debug["source"] == "IGN BD TOPO"
