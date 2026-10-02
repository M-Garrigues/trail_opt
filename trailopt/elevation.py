"""Altitude : grilles GeoTIFF float32 du WMS-R Géoplateforme, interpolation bilinéaire.

Couche prioritaire : MNT LiDAR HD ; repli sur RGE ALTI là où le LiDAR a des trous.
Noms vérifiés par GetCapabilities (https://data.geopf.fr/wms-r, octobre 2026).
"""
from __future__ import annotations

import gzip
import math
import time
from concurrent.futures import ThreadPoolExecutor

import numpy as np
import requests

from . import cache

WMS_URL = "https://data.geopf.fr/wms-r"
LAYERS = [
    "IGNF_LIDAR-HD_MNT_ELEVATION.ELEVATIONGRIDCOVERAGE.LAMB93",  # nodata -9999
    "ELEVATION.ELEVATIONGRIDCOVERAGE.HIGHRES",                    # RGE ALTI, nodata -99999
]
RES = 5.0          # m / pixel
TILE_PX = 1000     # <= MaxWidth/MaxHeight annoncés (5010)
SNAP = 100.0       # bbox arrondie vers l'extérieur à 100 m (clé de cache)
MARGIN_PX = 2      # recouvrement pour le bilinéaire en bord de dalle
USER_AGENT = "trailopt/1.0"


def _tiles(X, Y):
    """Découpe la bbox (arrondie) des points en dalles de <= TILE_PX pixels."""
    x0, y0 = math.floor(X.min() / SNAP) * SNAP, math.floor(Y.min() / SNAP) * SNAP
    x1, y1 = math.ceil(X.max() / SNAP) * SNAP + SNAP, math.ceil(Y.max() / SNAP) * SNAP + SNAP
    span = TILE_PX * RES
    nx, ny = math.ceil((x1 - x0) / span), math.ceil((y1 - y0) / span)
    for i in range(nx):
        for j in range(ny):
            yield (x0 + i * span, y0 + j * span,
                   min(x1, x0 + (i + 1) * span), min(y1, y0 + (j + 1) * span))


def _fetch_tile(layer, core, stats):
    """Renvoie les octets GeoTIFF (gzip) de la dalle `core` élargie de MARGIN_PX."""
    m = MARGIN_PX * RES
    bb = (core[0] - m, core[1] - m, core[2] + m, core[3] + m)
    p = cache.path("dem", f"{layer[:40]}_{bb[0]:.0f}_{bb[1]:.0f}_{bb[2]:.0f}_{bb[3]:.0f}.tif.gz")
    if p.exists():
        stats["cache_hits"] += 1
        return bb, p
    if cache.offline():
        raise RuntimeError(f"hors ligne et dalle d'altitude absente du cache ({p})")
    params = dict(SERVICE="WMS", VERSION="1.3.0", REQUEST="GetMap", LAYERS=layer, STYLES="",
                  CRS="EPSG:2154", BBOX=",".join(f"{v:.2f}" for v in bb),
                  WIDTH=round((bb[2] - bb[0]) / RES), HEIGHT=round((bb[3] - bb[1]) / RES),
                  FORMAT="image/geotiff")
    last = None
    for attempt in range(4):
        try:
            stats["requests"] += 1
            r = requests.get(WMS_URL, params=params, timeout=(10, 60),
                             headers={"User-Agent": USER_AGENT})
            r.raise_for_status()
            if "tiff" not in r.headers.get("content-type", ""):
                raise RuntimeError(r.text[:200])
            cache.write_atomic(p, gzip.compress(r.content, 3))
            return bb, p
        except Exception as ex:
            last = ex
            stats.setdefault("errors", []).append(f"wms-r: {str(ex)[:120]}")
            time.sleep(2 ** attempt)
    raise RuntimeError(f"WMS-R altitude indisponible : {last}")


def _read(p):
    from rasterio.io import MemoryFile
    with MemoryFile(gzip.decompress(p.read_bytes())) as mf, mf.open() as ds:
        a = ds.read(1).astype(np.float64)
        nd = ds.nodata
    if nd is not None:
        a[a == nd] = np.nan
    a[a < -1000] = np.nan
    return a


def bilinear(grid, bb, X, Y):
    """Interpolation bilinéaire, centres de pixels ; NaN si un voisin manque."""
    h, w = grid.shape
    c = (X - bb[0]) / RES - 0.5
    r = (bb[3] - Y) / RES - 0.5
    c0 = np.clip(np.floor(c).astype(int), 0, w - 2)
    r0 = np.clip(np.floor(r).astype(int), 0, h - 2)
    fc = np.clip(c - c0, 0.0, 1.0)
    fr = np.clip(r - r0, 0.0, 1.0)
    z00, z01 = grid[r0, c0], grid[r0, c0 + 1]
    z10, z11 = grid[r0 + 1, c0], grid[r0 + 1, c0 + 1]
    return (z00 * (1 - fc) * (1 - fr) + z01 * fc * (1 - fr)
            + z10 * (1 - fc) * fr + z11 * fc * fr)


def sample_l93(X, Y, stats: dict | None = None) -> np.ndarray:
    """Altitudes (m) aux points Lambert-93 ; NaN là où aucune couche ne couvre."""
    stats = stats if stats is not None else cache.new_stats()
    X, Y = np.asarray(X, float), np.asarray(Y, float)
    z = np.full(len(X), np.nan)
    for layer in LAYERS:
        todo = np.nonzero(np.isnan(z))[0]
        if len(todo) == 0:
            break
        tiles = []
        for core in _tiles(X[todo], Y[todo]):
            sel = todo[(X[todo] >= core[0]) & (X[todo] < core[2])
                       & (Y[todo] >= core[1]) & (Y[todo] < core[3])]
            if len(sel):
                tiles.append((core, sel))
        with ThreadPoolExecutor(4) as ex:  # téléchargements en parallèle, lecture séquentielle (RAM)
            files = list(ex.map(lambda t: _fetch_tile(layer, t[0], stats), tiles))
        for (core, sel), (bb, p) in zip(tiles, files):
            z[sel] = bilinear(_read(p), bb, X[sel], Y[sel])
    return z


def sampler_for(frame, stats: dict | None = None):
    """Fonction xy local (N,2) -> z (N,)."""
    def sample(xy):
        X, Y = frame.to_l93(xy)
        return sample_l93(X, Y, stats)
    return sample
