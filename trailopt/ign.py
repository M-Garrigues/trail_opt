"""BD TOPO IGN (tronçons de route, WFS Géoplateforme) -> arêtes brutes (u, v, xy local, flat).

Avantages sur OSM : ponts/tunnels fiables (position par rapport au sol), géométrie métrique,
cohérence avec le Plan IGN. Limite : pas de trottoirs ni de petites voies piétonnes urbaines.
"""
from __future__ import annotations

import gzip
import json
import math
import re
import time
from concurrent.futures import ThreadPoolExecutor

import requests

from . import cache

WFS_URL = "https://data.geopf.fr/wfs/ows"
LAYER = "BDTOPO_V3:troncon_de_route"
PROPS = "nature,importance,position_par_rapport_au_sol,acces_pieton,prive,etat_de_l_objet,geometrie"
PAGE = 5000  # maximum servi par requête
USER_AGENT = "trailopt/1.0"

# Natures BD TOPO par type de voies (cumulatif, comme pour OSM).
UNPAVED = {"Sentier", "Chemin", "Route empierrée"}
PEDESTRIAN = UNPAVED | {"Escalier", "Piste cyclable"}
ROADS = {"Route à 1 chaussée", "Route à 2 chaussées", "Rond-point", "Bretelle"}
MINOR_IMPORTANCE = {"4", "5", "6"}       # dessertes locales ; 3, 2, 1 : liaisons majeures
EXCLUDED = {"Type autoroutier", "Bac ou liaison maritime"}


def keep(props: dict, roads: str) -> bool:
    nature = props.get("nature")
    if (nature in EXCLUDED or props.get("prive") is True
            or props.get("etat_de_l_objet") not in (None, "En service")
            or props.get("acces_pieton") == "Restreint aux ayants droit"):
        return False
    if roads == "unpaved":
        return nature in UNPAVED
    if nature in PEDESTRIAN:
        return True
    if roads == "pedestrian" or nature not in ROADS:
        return False
    return roads == "all" or props.get("importance") in MINOR_IMPORTANCE


def _page(bbox_str: str, start: int, stats: dict) -> dict:
    params = dict(SERVICE="WFS", VERSION="2.0.0", REQUEST="GetFeature", TYPENAMES=LAYER,
                  BBOX=f"{bbox_str},urn:ogc:def:crs:OGC:1.3:CRS84", PROPERTYNAME=PROPS,
                  OUTPUTFORMAT="application/json", COUNT=PAGE, STARTINDEX=start, SORTBY="cleabs")
    last = None
    for attempt in range(4):
        try:
            stats["requests"] += 1
            r = requests.get(WFS_URL, params=params, timeout=(10, 75),
                             headers={"User-Agent": USER_AGENT})
            r.raise_for_status()
            return r.json()
        except Exception as ex:
            last = ex
            stats.setdefault("errors", []).append(f"wfs: {str(ex)[:120]}")
            time.sleep(2 ** attempt)
    raise RuntimeError(f"WFS BD TOPO indisponible : {last}")


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


def fetch(bbox, stats: dict | None = None) -> list:
    """Tous les tronçons de la bbox (sud, ouest, nord, est), arrondie à 1e-3°.
    Le cache ne dépend pas du type de voies : le filtrage est local."""
    stats = stats if stats is not None else cache.new_stats()
    bbox_str = _round_bbox(bbox)
    p = cache.path("ign", f"troncons_{bbox_str.replace(',', '_')}.json.gz")
    if p.exists():
        stats["cache_hits"] += 1
        return json.loads(gzip.decompress(p.read_bytes()))
    if cache.offline():
        raise RuntimeError(f"hors ligne et tronçons BD TOPO absents du cache ({p})")
    first = _page(bbox_str, 0, stats)
    feats = first["features"]
    total = first.get("numberMatched") or len(feats)
    starts = list(range(PAGE, total, PAGE))
    with ThreadPoolExecutor(4) as ex:
        for page in ex.map(lambda st: _page(bbox_str, st, stats), starts):
            feats += page["features"]
    # On ne garde que l'utile : le cache et la RAM restent petits.
    out = [{"p": f["properties"], "c": f["geometry"]["coordinates"]}
           for f in feats if f.get("geometry") and f["geometry"]["type"] == "LineString"]
    cache.write_atomic(p, gzip.compress(json.dumps(out).encode(), 3))
    return out


def to_edges(feats: list, frame, roads: str = "pedestrian"):
    """Les tronçons BD TOPO sont déjà coupés aux carrefours : nœuds = extrémités,
    identifiées par leurs coordonnées. Renvoie (arêtes brutes, nombre de tronçons retenus)."""
    nodes: dict[tuple, int] = {}
    raw = []
    for f in feats:
        if not keep(f["p"], roads) or len(f["c"]) < 2:
            continue
        c = f["c"]
        xy = frame.to_local([q[0] for q in c], [q[1] for q in c])
        u = nodes.setdefault((round(c[0][0], 7), round(c[0][1], 7)), len(nodes))
        v = nodes.setdefault((round(c[-1][0], 7), round(c[-1][1], 7)), len(nodes))
        flat = f["p"].get("position_par_rapport_au_sol") not in (None, "0")  # pont ou tunnel
        raw.append((u, v, xy, flat))
    return raw, len(raw)
