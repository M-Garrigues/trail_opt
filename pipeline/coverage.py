"""Contour de la couverture (union des dalles du manifeste) en GeoJSON WGS84, pour le front."""
from __future__ import annotations

import json
from pathlib import Path

import numpy as np
import shapely
from shapely.geometry import box, mapping
from shapely.geometry.polygon import orient

from .build import _TO_WGS
from .load import TILE_M

STEP_M = 5000.0   # densification avant reprojection : un bord L93 droit est courbe en WGS84
DIGITS = 5        # ~1 m


def coverage(tiles_dir) -> dict:
    """FeatureCollection (une MultiPolygon) des dalles listées dans tiles_dir/manifest.json."""
    names = json.loads((Path(tiles_dir) / "manifest.json").read_text())["tiles"]
    cells = [box(ix * TILE_M, iy * TILE_M, (ix + 1) * TILE_M, (iy + 1) * TILE_M)
             for ix, iy in (map(int, n.split("_")) for n in names)]
    u = shapely.segmentize(shapely.union_all(cells), STEP_M)
    u = shapely.transform(u, lambda c: np.column_stack(_TO_WGS.transform(c[:, 0], c[:, 1])))
    u = shapely.set_precision(u, 10 ** -DIGITS)
    polys = getattr(u, "geoms", [u])
    geom = mapping(shapely.MultiPolygon([orient(p, 1.0) for p in polys]))  # RFC 7946 : extérieur anti-horaire
    return {"type": "FeatureCollection",
            "features": [{"type": "Feature", "properties": {"tiles": len(names)}, "geometry": geom}]}


def write(tiles_dir, out) -> int:
    data = json.dumps(coverage(tiles_dir), separators=(",", ":"))
    Path(out).parent.mkdir(parents=True, exist_ok=True)
    Path(out).write_text(data)
    return len(data)
