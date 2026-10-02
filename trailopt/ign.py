"""BD TOPO IGN (tronçons de route, WFS Géoplateforme) -> arêtes brutes (u, v, xy local, flat).

Avantages sur OSM : ponts/tunnels fiables (position par rapport au sol), géométrie métrique,
cohérence avec le Plan IGN. Limite : pas de trottoirs ni de petites voies piétonnes urbaines.

Téléchargement en dalles FIXES (grille en degrés) : le serveur répond environ dix fois plus
vite par tronçon sur une petite emprise que sur une grande emprise paginée, et le cache est
réutilisable d'un calcul à l'autre, quelle que soit la zone. Chaque dalle est convertie
aussitôt en tableaux numpy compacts : des centaines de milliers de tronçons tiennent en
quelques dizaines de Mo, contre plus d'un Go en listes Python.
"""
from __future__ import annotations

import io
import math
import re
import time
from concurrent.futures import ThreadPoolExecutor

import numpy as np
import requests

from . import cache

WFS_URL = "https://data.geopf.fr/wfs/ows"
LAYER = "BDTOPO_V3:troncon_de_route"
PROPS = ("cleabs,nature,importance,position_par_rapport_au_sol,acces_pieton,prive,"
         "etat_de_l_objet,geometrie")
PAGE = 5000              # maximum servi par requête
TILE_DEG = (0.06, 0.04)  # dalle d'environ 4,4 km de côté (plus petit : trop de requêtes, refus 429)
USER_AGENT = "trailopt/1.0"

# Natures BD TOPO par type de voies (cumulatif, comme pour OSM).
UNPAVED = {"Sentier", "Chemin", "Route empierrée"}
PEDESTRIAN = UNPAVED | {"Escalier", "Piste cyclable"}
ROADS = {"Route à 1 chaussée", "Route à 2 chaussées", "Rond-point", "Bretelle"}
MINOR_IMPORTANCE = (4, 5, 6)             # dessertes locales ; 3, 2, 1 : liaisons majeures
EXCLUDED = {"Type autoroutier", "Bac ou liaison maritime"}
NATURES = sorted(UNPAVED | PEDESTRIAN | ROADS | EXCLUDED)
_CODE = {k: i for i, k in enumerate(NATURES)}
UNKNOWN = 255
_FIELDS = ("lon", "lat", "n", "nature", "importance", "flat", "ok", "ident")


def _round_bbox(bbox) -> str:
    s, w, n, e = bbox
    s, w = math.floor(s * 1000) / 1000, math.floor(w * 1000) / 1000
    n, e = math.ceil(n * 1000) / 1000, math.ceil(e * 1000) / 1000
    return f"{w:.3f},{s:.3f},{e:.3f},{n:.3f}"


def count(bbox, stats: dict | None = None) -> int | None:
    """Nombre de tronçons dans la bbox, sans les télécharger (WFS resultType=hits).
    None si le service ne répond pas."""
    stats = stats if stats is not None else cache.new_stats()
    if cache.offline():
        return None
    try:
        stats["requests"] += 1
        r = requests.get(WFS_URL, timeout=(10, 30), headers={"User-Agent": USER_AGENT}, params=dict(
            SERVICE="WFS", VERSION="2.0.0", REQUEST="GetFeature", TYPENAMES=LAYER, RESULTTYPE="hits",
            BBOX=f"{_round_bbox(bbox)},urn:ogc:def:crs:OGC:1.3:CRS84"))
        r.raise_for_status()
        m = re.search(r'numberMatched="(\d+)"', r.text)
        return int(m.group(1)) if m else None
    except Exception:
        return None


def to_arrays(feats: list) -> dict:
    """Tronçons GeoJSON -> tableaux compacts. `ok` : ni privé, ni réservé, ni hors service."""
    feats = [f for f in feats if f.get("geometry") and f["geometry"]["type"] == "LineString"
             and len(f["geometry"]["coordinates"]) >= 2]
    P = [f["properties"] for f in feats]
    C = np.array([q[:2] for f in feats for q in f["geometry"]["coordinates"]], float).reshape(-1, 2)
    return dict(
        lon=C[:, 0], lat=C[:, 1],
        n=np.fromiter((len(f["geometry"]["coordinates"]) for f in feats), np.int32, len(feats)),
        nature=np.fromiter((_CODE.get(p.get("nature"), UNKNOWN) for p in P), np.uint8, len(P)),
        importance=np.fromiter((int(p.get("importance") or 0) for p in P), np.uint8, len(P)),
        flat=np.fromiter((p.get("position_par_rapport_au_sol") not in (None, "0") for p in P), bool, len(P)),
        ok=np.fromiter((not (p.get("prive") is True or p.get("etat_de_l_objet") not in (None, "En service")
                             or p.get("acces_pieton") == "Restreint aux ayants droit") for p in P), bool, len(P)),
        ident=np.array([str(p.get("cleabs", i)) for i, p in enumerate(P)], dtype="S24"),
    )


def _concat(parts: list) -> dict:
    """Assemble des dalles et retire les doublons (un tronçon à cheval est dans chaque dalle)."""
    parts = [a for a in parts if len(a["n"])]
    if not parts:
        return {k: np.zeros(0, t) for k, t in zip(_FIELDS, (float, float, np.int32, np.uint8, np.uint8,
                                                              bool, bool, "S24"))}
    A = {k: np.concatenate([a[k] for a in parts]) for k in _FIELDS}
    _, first = np.unique(A["ident"], return_index=True)
    if len(first) == len(A["n"]):
        return A
    keep = np.zeros(len(A["n"]), bool)
    keep[first] = True
    vk = np.repeat(keep, A["n"])
    return {"lon": A["lon"][vk], "lat": A["lat"][vk],
            **{k: A[k][keep] for k in ("n", "nature", "importance", "flat", "ok", "ident")}}


def _tile(ix: int, iy: int, stats: dict) -> dict:
    """Une dalle de la grille : depuis le cache, sinon téléchargée (paginée si elle est pleine)."""
    p = cache.path("ign", f"t6x4_{ix}_{iy}.npz")
    if p.exists():
        stats["cache_hits"] += 1
        with np.load(p) as z:
            return {k: z[k] for k in _FIELDS}
    if cache.offline():
        raise RuntimeError(f"hors ligne et dalle BD TOPO absente du cache ({p})")
    w, s = ix * TILE_DEG[0], iy * TILE_DEG[1]
    bbox = f"{w:.5f},{s:.5f},{w + TILE_DEG[0]:.5f},{s + TILE_DEG[1]:.5f},urn:ogc:def:crs:OGC:1.3:CRS84"
    feats, start = [], 0
    while True:
        params = dict(SERVICE="WFS", VERSION="2.0.0", REQUEST="GetFeature", TYPENAMES=LAYER, BBOX=bbox,
                      PROPERTYNAME=PROPS, OUTPUTFORMAT="application/json", COUNT=PAGE, STARTINDEX=start,
                      SORTBY="cleabs")
        last = None
        for attempt in range(4):
            try:
                stats["requests"] += 1
                r = requests.get(WFS_URL, params=params, timeout=(10, 75), headers={"User-Agent": USER_AGENT})
                r.raise_for_status()
                page = r.json()["features"]
                break
            except Exception as ex:
                last = ex
                stats.setdefault("errors", []).append(f"wfs: {str(ex)[:120]}")
                time.sleep(2 ** attempt)
        else:
            raise RuntimeError(f"WFS BD TOPO indisponible : {last}")
        feats += page
        if len(page) < PAGE:
            break
        start += PAGE
    A = to_arrays(feats)
    buf = io.BytesIO()
    np.savez_compressed(buf, **A)
    cache.write_atomic(p, buf.getvalue())
    return A


def fetch(bbox, stats: dict | None = None) -> dict:
    """Tous les tronçons des dalles qui couvrent la bbox (sud, ouest, nord, est), en tableaux.
    Le cache ne dépend ni de la zone exacte ni du type de voies : le filtrage est local."""
    stats = stats if stats is not None else cache.new_stats()
    s, w, n, e = bbox
    cells = [(ix, iy)
             for ix in range(math.floor(w / TILE_DEG[0]), math.floor(e / TILE_DEG[0]) + 1)
             for iy in range(math.floor(s / TILE_DEG[1]), math.floor(n / TILE_DEG[1]) + 1)]
    with ThreadPoolExecutor(4) as ex:    # au-delà, le serveur refuse une partie des requêtes
        parts = list(ex.map(lambda c: _tile(c[0], c[1], stats), cells))
    return _concat(parts)


def keep_mask(A: dict, roads: str) -> np.ndarray:
    """Tronçons retenus pour un type de voies."""
    def isin(names):
        return np.isin(A["nature"], [_CODE[k] for k in names])
    m = A["ok"] & ~isin(EXCLUDED) & (A["nature"] != UNKNOWN) & (A["n"] >= 2)
    if roads == "unpaved":
        return m & isin(UNPAVED)
    ped = isin(PEDESTRIAN)
    if roads == "pedestrian":
        return m & ped
    road = isin(ROADS)
    if roads == "all":
        return m & (ped | road)
    return m & (ped | (road & np.isin(A["importance"], MINOR_IMPORTANCE)))


def to_edges(A: dict, frame, roads: str = "minor"):
    """Les tronçons BD TOPO sont déjà coupés aux carrefours : nœuds = extrémités, identifiées
    par leurs coordonnées. Une seule projection pour tous les sommets.
    Renvoie (arêtes brutes (u, v, xy, flat), nombre de tronçons retenus)."""
    if not len(A["n"]):
        return [], 0
    keep = keep_mask(A, roads)
    off = np.concatenate([[0], np.cumsum(A["n"])])
    xy = frame.to_local(A["lon"], A["lat"])
    ends = np.concatenate([off[:-1], off[1:] - 1])
    key = (np.round(A["lon"][ends] * 1e7).astype(np.int64) << 32) + np.round(A["lat"][ends] * 1e7).astype(np.int64)
    _, inv = np.unique(key, return_inverse=True)
    m = len(A["n"])
    U, V, flat = inv[:m].tolist(), inv[m:].tolist(), A["flat"].tolist()
    offl = off.tolist()
    raw = [(U[i], V[i], xy[offl[i]:offl[i + 1]], flat[i]) for i in np.flatnonzero(keep).tolist()]
    return raw, len(raw)
