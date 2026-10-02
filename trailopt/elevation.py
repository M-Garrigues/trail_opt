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
TILE_PX = 500      # dalles de 2,5 km, <= MaxWidth/MaxHeight annoncés (5010)
MARGIN_PX = 2      # recouvrement pour le bilinéaire en bord de dalle
RES_COARSE = 50.0  # modèle de terrain grossier, pour le criblage des grands graphes
COARSE_SNAP = 5000.0
USER_AGENT = "trailopt/1.0"


def _tiles(X, Y):
    """Dalles d'une grille FIXE (Lambert-93, pas de TILE_PX × RES) qui contiennent des points.
    Fixe, donc réutilisable en cache d'un calcul à l'autre, quelle que soit la zone."""
    span = TILE_PX * RES
    cells = np.unique(np.column_stack([np.floor(X / span), np.floor(Y / span)]).astype(np.int64), axis=0)
    for ix, iy in cells.tolist():
        yield (ix * span, iy * span, (ix + 1) * span, (iy + 1) * span)


def _fetch_tile(layer, core, stats, res=RES, margin_px=MARGIN_PX):
    """Renvoie (bbox, chemin du GeoTIFF gzip en cache) de la dalle `core` élargie de la marge."""
    m = margin_px * res
    bb = (core[0] - m, core[1] - m, core[2] + m, core[3] + m)
    tag = "" if res == RES else f"r{res:.0f}_"
    p = cache.path("dem", f"{layer[:40]}_{tag}{bb[0]:.0f}_{bb[1]:.0f}_{bb[2]:.0f}_{bb[3]:.0f}.tif.gz")
    if p.exists():
        stats["cache_hits"] += 1
        return bb, p
    if cache.offline():
        raise RuntimeError(f"hors ligne et dalle d'altitude absente du cache ({p})")
    params = dict(SERVICE="WMS", VERSION="1.3.0", REQUEST="GetMap", LAYERS=layer, STYLES="",
                  CRS="EPSG:2154", BBOX=",".join(f"{v:.2f}" for v in bb),
                  WIDTH=round((bb[2] - bb[0]) / res), HEIGHT=round((bb[3] - bb[1]) / res),
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


def _read(p, mask=True):
    from rasterio.io import MemoryFile
    with MemoryFile(gzip.decompress(p.read_bytes())) as mf, mf.open() as ds:
        a = ds.read(1).astype(np.float64)
        nd = ds.nodata
    if nd is not None:
        a[a == nd] = np.nan
    a[a < -1000] = np.nan
    return mask_unreliable(a) if mask else a


# En bordure d'une zone sans données (base militaire, plan d'eau, limite de couverture), le
# serveur rééchantillonne en mélangeant la valeur « vide » (-9999) avec les vraies altitudes :
# on obtient une rampe de valeurs fausses mais d'apparence plausible, sur ~100 m. Mesuré au
# nord de la base de Villacoublay : -9473, -7972, -3709, -1205... puis 54, 28 m au lieu de 177.
HOLE_MARGIN_PX = 30      # 150 m autour d'un trou : valeurs non fiables
MAX_STEP_M = 15.0        # saut d'altitude impossible entre deux pixels voisins (pente 300 %)


def mask_unreliable(a: np.ndarray) -> np.ndarray:
    """Met à NaN les pixels proches d'un trou et ceux pris dans une pente impossible.
    Ils seront comblés par la couche de repli (RGE ALTI), puis par interpolation."""
    from scipy import ndimage
    hole = ~np.isfinite(a)
    bad = hole.copy()
    if hole.any() and not hole.all():
        bad |= ndimage.distance_transform_edt(~hole) <= HOLE_MARGIN_PX
    # Rampe dont le trou est hors de la dalle : repérée par ses sauts d'altitude.
    f = np.where(hole, 0.0, a)
    jump = np.zeros(a.shape, bool)
    dy = (np.abs(np.diff(f, axis=0)) > MAX_STEP_M) & ~hole[1:] & ~hole[:-1]
    dx = (np.abs(np.diff(f, axis=1)) > MAX_STEP_M) & ~hole[:, 1:] & ~hole[:, :-1]
    jump[1:] |= dy
    jump[:-1] |= dy
    jump[:, 1:] |= dx
    jump[:, :-1] |= dx
    if jump.any():
        bad |= ndimage.binary_dilation(jump, iterations=2)
    if bad.any():
        a = a.copy()
        a[bad] = np.nan
    return a


def bilinear(grid, bb, X, Y, res=RES):
    """Interpolation bilinéaire, centres de pixels ; NaN si un voisin manque."""
    h, w = grid.shape
    c = (X - bb[0]) / res - 0.5
    r = (bb[3] - Y) / res - 0.5
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
        with ThreadPoolExecutor(6) as ex:  # téléchargements en parallèle, lecture séquentielle (RAM)
            files = list(ex.map(lambda t: _fetch_tile(layer, t[0], stats), tiles))
        for (core, sel), (bb, p) in zip(tiles, files):
            z[sel] = bilinear(_read(p), bb, X[sel], Y[sel])
    return z


def sample_coarse_l93(X, Y, stats: dict | None = None) -> np.ndarray:
    """Altitudes approchées (RGE ALTI rééchantillonné à 50 m) : UNE requête pour toute la zone.
    Sert au criblage des grands graphes, jamais au D+ final."""
    stats = stats if stats is not None else cache.new_stats()
    X, Y = np.asarray(X, float), np.asarray(Y, float)
    g = COARSE_SNAP
    core = (math.floor(X.min() / g) * g, math.floor(Y.min() / g) * g,
            math.ceil(X.max() / g) * g + g, math.ceil(Y.max() / g) * g + g)
    bb, p = _fetch_tile(LAYERS[1], core, stats, res=RES_COARSE, margin_px=1)
    return bilinear(_read(p, mask=False), bb, X, Y, res=RES_COARSE)


def coarse_sampler_for(frame, stats: dict | None = None):
    def sample(xy):
        X, Y = frame.to_l93(xy)
        return sample_coarse_l93(X, Y, stats)
    return sample


def sampler_for(frame, stats: dict | None = None):
    """Fonction xy local (N,2) -> z (N,)."""
    def sample(xy):
        X, Y = frame.to_l93(xy)
        return sample_l93(X, Y, stats)
    return sample
