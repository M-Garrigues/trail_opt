"""Lecture des dalles `tiles/1` (contrat .team/contracts/tiles.md, copie dans ce module).

`TileStore` remplace, pour la source IGN, le WFS BD TOPO (`fetch`, `count`) et le WMS-R
d'altitude (`sampler_for`) : `trailopt.pipeline` l'utilise si TRAILOPT_TILES_DIR est défini.
Les tronçons sont rendus au format de `trailopt.ign.fetch`, donc `ign.to_edges` puis
`base_edges` reconstruisent exactement les mêmes arêtes que depuis le WFS.
"""
from __future__ import annotations

import json
import math
from pathlib import Path

import numpy as np
from pyproj import Transformer

FORMAT = "tiles/1"
TILE_M = 20000             # dalle Lambert-93 de 20 km
STEP_DM = 50               # profil : un point tous les 5 m
MARGIN_M = 1000.0          # un tronçon peut déborder de sa dalle
_TO_WGS = Transformer.from_crs("EPSG:2154", "EPSG:4326", always_xy=True)
_TO_L93 = Transformer.from_crs("EPSG:4326", "EPSG:2154", always_xy=True)


def lengths(x, y, n):
    """Longueur 2D (m) de chaque tronçon ; x, y : sommets concaténés (m), n : sommets par tronçon."""
    n = np.asarray(n, np.int64)
    d = np.hypot(np.diff(x), np.diff(y))
    seg_of = np.repeat(np.arange(len(n)), n)[:-1]
    d[np.cumsum(n)[:-1] - 1] = 0.0                 # pas de segment entre deux tronçons
    return np.bincount(seg_of, weights=d, minlength=len(n))


def profile_counts(len_dm):
    """Nombre de points de profil : ceil(len_dm / 50) + 1 (calcul entier, identique en Rust)."""
    return (np.asarray(len_dm, np.int64) + STEP_DM - 1) // STEP_DM + 1


def profile_points(x, y, n, len_dm):
    """Points du profil : abscisses 0, 5, 10 … puis l'extrémité du tronçon. Renvoie (px, py)."""
    n = np.asarray(n, np.int64)
    off = np.concatenate([[0], np.cumsum(n)])
    d = np.hypot(np.diff(x), np.diff(y))
    d[off[1:-1] - 1] = 1.0                         # écart entre tronçons : abscisse croissante
    S = np.concatenate([[0.0], np.cumsum(d)])
    pn = profile_counts(len_dm)
    start = np.repeat(S[off[:-1]], pn)
    rank = np.arange(int(pn.sum())) - np.repeat(np.cumsum(pn) - pn, pn)
    t = start + 5.0 * rank
    last = np.cumsum(pn) - 1
    t[last] = S[off[1:] - 1]
    return np.interp(t, S, x), np.interp(t, S, y)


def _undelta(d, n, base=0):
    """Deltas par tronçon (1er élément absolu - base) -> valeurs absolues."""
    c = np.cumsum(d.astype(np.int64))
    first = np.cumsum(n) - n
    return c - np.repeat(c[first] - d[first].astype(np.int64), n) + base


def read_tile(path, ix: int, iy: int) -> dict:
    """Décode une dalle : sommets L93 en dm absolus, profil z (m) concaténé, colonnes."""
    with np.load(path) as f:
        T = {k: f[k] for k in f.files}
    n = T["geom_n"].astype(np.int64)
    pn = profile_counts(T["len_dm"])
    if pn.sum() != len(T["prof_d"]):
        raise ValueError(f"{path} : profil incohérent avec len_dm")
    z = _undelta(T["prof_d"], pn) + np.repeat(T["prof_z0_dm"].astype(np.int64), pn)
    return dict(
        id=np.cumsum(T["id_d"]), n=n,
        x_dm=_undelta(T["geom_x"], n, ix * TILE_M * 10), y_dm=_undelta(T["geom_y"], n, iy * TILE_M * 10),
        z=z / 10.0, pn=pn,
        **{k: T[k] for k in ("len_dm", "dplus_dm", "dminus_dm", "max_grade_pm", "nature",
                             "importance", "flags", "par_n", "par_id")})


class TileStore:
    """Dalles d'un dossier (manifest.json + <ix>_<iy>.npz), lues à la demande et gardées."""

    def __init__(self, root):
        self.root = Path(root)
        m = json.loads((self.root / "manifest.json").read_text())
        if m.get("format") != FORMAT:
            raise RuntimeError(f"format de dalles inconnu : {m.get('format')} (attendu {FORMAT})")
        self.manifest = m
        self._cache, self._loaded, self._kd = {}, [], None

    def keys(self, bbox):
        """Dalles du manifeste qui touchent la bbox WGS (sud, ouest, nord, est), marge comprise.
        Une dalle absente du manifeste n'a aucune voie."""
        X, Y = self.l93_box(bbox, MARGIN_M)
        return [f"{ix}_{iy}"
                for ix in range(math.floor(X[0] / TILE_M), math.floor(X[1] / TILE_M) + 1)
                for iy in range(math.floor(Y[0] / TILE_M), math.floor(Y[1] / TILE_M) + 1)
                if f"{ix}_{iy}" in self.manifest["tiles"]]

    @staticmethod
    def l93_box(bbox, margin=0.0):
        s, w, n, e = bbox
        X, Y = _TO_L93.transform([w, e, w, e], [s, s, n, n])
        return (min(X) - margin, max(X) + margin), (min(Y) - margin, max(Y) + margin)

    def tile(self, key):
        if key not in self._cache:
            ix, iy = map(int, key.split("_"))
            self._cache[key] = read_tile(self.root / f"{key}.npz", ix, iy)
        return self._cache[key]

    def count(self, bbox) -> int:
        """Tronçons dont le premier sommet est dans la bbox (remplace le comptage WFS)."""
        (x0, x1), (y0, y1) = self.l93_box(bbox)
        tot = 0
        for k in self.keys(bbox):
            T = self.tile(k)
            first = np.cumsum(T["n"]) - T["n"]
            x, y = T["x_dm"][first] / 10.0, T["y_dm"][first] / 10.0
            tot += int(((x >= x0) & (x < x1) & (y >= y0) & (y < y1)).sum())
        return tot

    def fetch(self, bbox) -> dict:
        """Tronçons des dalles qui touchent la bbox, au format de `trailopt.ign.fetch`."""
        self._loaded, self._kd = [self.tile(k) for k in self.keys(bbox)], None
        if not self._loaded:
            from trailopt import ign
            return ign._concat([])
        cat = {k: np.concatenate([T[k] for T in self._loaded])
               for k in ("x_dm", "y_dm", "n", "nature", "importance", "flags", "id")}
        lon, lat = _TO_WGS.transform(cat["x_dm"] / 10.0, cat["y_dm"] / 10.0)
        return dict(lon=np.asarray(lon), lat=np.asarray(lat), n=cat["n"].astype(np.int32),
                    nature=cat["nature"], importance=cat["importance"],
                    flat=(cat["flags"] & 1).astype(bool), ok=(cat["flags"] & 2).astype(bool),
                    ident=cat["id"])

    def _index(self):
        from scipy.spatial import cKDTree
        px, py, z, first, last = [], [], [], [], []
        for T in self._loaded:
            x, y = profile_points(T["x_dm"] / 10.0, T["y_dm"] / 10.0, T["n"], T["len_dm"])
            px.append(x)
            py.append(y)
            z.append(T["z"])
            f = np.zeros(len(x), bool)
            f[np.cumsum(T["pn"]) - T["pn"]] = True
            first.append(f)
            last.append(np.roll(f, -1))            # dernier point = juste avant un premier
        # Ponts et tunnels hors index : à un croisement, le point le plus proche en plan peut être
        # le tunnel, 100 m plus bas. Leur profil (linéaire) ne dépend que des nœuds, sur le sol.
        keep = ~np.concatenate([np.repeat((T["flags"] & 1).astype(bool), T["pn"]) for T in self._loaded])
        self._P = np.column_stack([np.concatenate(px), np.concatenate(py)])[keep]
        self._Z = np.concatenate(z)[keep]
        self._first, self._last = np.concatenate(first)[keep], np.concatenate(last)[keep]
        self._kd = cKDTree(self._P)

    def sample_l93(self, X, Y) -> np.ndarray:
        """Altitude (m) aux points L93 : interpolée sur le segment de profil le plus proche
        (parmi ceux des 4 points de profil les plus proches). NaN sans dalle chargée."""
        Q = np.column_stack([np.asarray(X, float), np.asarray(Y, float)])
        if not self._loaded:
            return np.full(len(Q), np.nan)
        if self._kd is None:
            self._index()
        P, Z, M = self._P, self._Z, len(self._P)
        _, J = self._kd.query(Q, k=min(4, M))
        J = J.reshape(len(Q), -1)
        best, z = np.full(len(Q), np.inf), np.full(len(Q), np.nan)
        for j in J.T:
            for a, b, ok in ((j - 1, j, ~self._first[j]), (j, np.minimum(j + 1, M - 1), ~self._last[j])):
                a = np.maximum(a, 0)
                ab = P[b] - P[a]
                t = np.clip(((Q - P[a]) * ab).sum(1) / np.maximum((ab * ab).sum(1), 1e-12), 0.0, 1.0)
                d = np.hypot(*(Q - P[a] - t[:, None] * ab).T)
                take = ok & (d < best)
                best[take] = d[take]
                z[take] = (Z[a] + t * (Z[b] - Z[a]))[take]
        return z

    def sampler_for(self, frame):
        """Fonction xy local (N,2) -> z (N,), comme `elevation.sampler_for`."""
        def sample(xy):
            return self.sample_l93(*frame.to_l93(xy))
        sample.smooth = 1          # profil déjà lissé dans la dalle : pas de second lissage
        return sample
