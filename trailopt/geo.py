"""Projection locale azimutale équidistante (mètres) centrée sur le départ."""
from __future__ import annotations

import numpy as np
from pyproj import Transformer


class LocalFrame:
    def __init__(self, lat: float, lon: float):
        self.lat, self.lon = float(lat), float(lon)
        crs = (f"+proj=aeqd +lat_0={self.lat} +lon_0={self.lon} +x_0=0 +y_0=0 "
               "+datum=WGS84 +units=m +no_defs")
        self._fwd = Transformer.from_crs("EPSG:4326", crs, always_xy=True)
        self._inv = Transformer.from_crs(crs, "EPSG:4326", always_xy=True)
        self._l93 = Transformer.from_crs(crs, "EPSG:2154", always_xy=True)

    def to_local(self, lon, lat) -> np.ndarray:
        x, y = self._fwd.transform(np.asarray(lon, float), np.asarray(lat, float))
        return np.column_stack([np.atleast_1d(x), np.atleast_1d(y)])

    def to_wgs(self, xy):
        xy = np.asarray(xy, float)
        return self._inv.transform(xy[:, 0], xy[:, 1])  # (lon, lat)

    def to_l93(self, xy):
        xy = np.asarray(xy, float)
        return self._l93.transform(xy[:, 0], xy[:, 1])
