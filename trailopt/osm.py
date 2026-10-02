"""Overpass (JSON brut) -> arêtes brutes (u, v, xy local, flat).

Pas d'osmnx en production : trop lourd en RAM.
"""
from __future__ import annotations

import gzip
import hashlib
import json
import math
import os
import time
from collections import Counter

import numpy as np
import requests

from . import cache

PEDESTRIAN_HW = ["path", "footway", "track", "bridleway", "steps", "pedestrian", "cycleway"]
MINOR_ROADS = ["living_street", "residential", "unclassified", "service", "road",
               "tertiary", "tertiary_link"]
MAJOR_ROADS = ["secondary", "secondary_link", "primary", "primary_link"]
# unpaved : sentiers au sens strict (surface non revêtue), y compris chemins/routes en gravier.
ROADS = ("unpaved", "pedestrian", "minor", "all")
UNPAVED_SURFACES = {"unpaved", "compacted", "fine_gravel", "gravel", "pebblestone", "dirt",
                    "earth", "ground", "grass", "mud", "sand", "woodchips", "rock", "clay",
                    "soil", "stone"}


def is_unpaved(tags: dict) -> bool:
    """Surface non revêtue : tag `surface` s'il existe, sinon path/bridleway/track
    (sauf track grade1, revêtu) sont présumés non revêtus."""
    surface = tags.get("surface", "").split(";")[0].strip()
    if surface:
        return surface in UNPAVED_SURFACES
    hw = tags.get("highway")
    if hw == "track":
        return tags.get("tracktype") != "grade1"
    return hw in ("path", "bridleway")

ENDPOINTS = os.environ.get(
    "TRAILOPT_OVERPASS_URLS",
    "https://overpass.openstreetmap.fr/api/interpreter,https://overpass-api.de/api/interpreter",
).split(",")
USER_AGENT = "trailopt/1.0 (boucle de trail a D+ optimal; open-source)"


def osm_filter(roads: str) -> str:
    hw = PEDESTRIAN_HW + (MINOR_ROADS if roads in ("unpaved", "minor", "all") else []) \
        + (MAJOR_ROADS if roads == "all" else [])
    return (f'["highway"~"^({"|".join(hw)})$"]["area"!~"yes"]'
            '["access"!~"^(private|no)$"]["foot"!~"^no$"]'
            '["service"!~"^(parking_aisle|driveway)$"]')


def build_query(bbox, roads: str) -> str:
    """bbox = (sud, ouest, nord, est) en degrés, arrondie vers l'extérieur à 1e-3°."""
    s, w, n, e = bbox
    s, w = math.floor(s * 1000) / 1000, math.floor(w * 1000) / 1000
    n, e = math.ceil(n * 1000) / 1000, math.ceil(e * 1000) / 1000
    return (f"[out:json][timeout:60];way{osm_filter(roads)}"
            f"({s:.3f},{w:.3f},{n:.3f},{e:.3f});out body geom qt;")


def count(bbox, roads: str, stats: dict | None = None) -> int | None:
    """Nombre de ways de la requête, sans les télécharger (`out count`). None si échec."""
    stats = stats if stats is not None else cache.new_stats()
    q = build_query(bbox, roads).replace("out body geom qt;", "out count;").replace("timeout:60", "timeout:25")
    if cache.offline():
        return None
    for url in ENDPOINTS:
        try:
            stats["requests"] += 1
            r = requests.post(url, data={"data": q}, timeout=(10, 30), headers={"User-Agent": USER_AGENT})
            r.raise_for_status()
            return int(r.json()["elements"][0]["tags"]["ways"])
        except Exception:
            continue
    return None


def fetch(query: str, stats: dict | None = None, retries: int = 3) -> dict:
    stats = stats if stats is not None else cache.new_stats()
    key = hashlib.sha1(query.encode()).hexdigest()[:20]
    p = cache.path("osm", f"{key}.json.gz")
    if p.exists():
        stats["cache_hits"] += 1
        return json.loads(gzip.decompress(p.read_bytes()))
    if cache.offline():
        raise RuntimeError(f"hors ligne et réponse Overpass absente du cache ({p})")
    last = None
    for attempt in range(retries):
        for url in ENDPOINTS:
            try:
                stats["requests"] += 1
                r = requests.post(url, data={"data": query}, timeout=(10, 75),
                                  headers={"User-Agent": USER_AGENT})
                r.raise_for_status()
                data = r.json()
                if "runtime error" in data.get("remark", ""):
                    raise RuntimeError(data["remark"])
                cache.write_atomic(p, gzip.compress(json.dumps(data).encode()))
                return data
            except Exception as ex:  # réseau, 429, 504, JSON invalide...
                last = ex
                stats.setdefault("errors", []).append(f"{url.split('/')[2]}: {str(ex)[:120]}")
        time.sleep(2 ** attempt)
    raise RuntimeError(f"Overpass indisponible : {last}")


def _tag_on(tags: dict, key: str, ignore=("no",)) -> bool:
    v = tags.get(key)
    return v is not None and v not in ignore


def ways_to_edges(data: dict, frame, roads: str = "pedestrian"):
    """Nœuds = extrémités de ways + nœuds partagés ; arêtes = tronçons entre eux.
    Renvoie (liste de (u, v, xy (N,2), flat), nombre de ways)."""
    ways = [el for el in data.get("elements", [])
            if el.get("type") == "way" and len(el.get("nodes", ())) >= 2
            and el.get("geometry") and all(g is not None for g in el["geometry"])
            and (roads != "unpaved" or is_unpaved(el.get("tags", {})))]
    count = Counter(n for w in ways for n in w["nodes"])
    raw = []
    for w in ways:
        nodes = w["nodes"]
        lon = [g["lon"] for g in w["geometry"]]
        lat = [g["lat"] for g in w["geometry"]]
        xy = frame.to_local(lon, lat)
        tags = w.get("tags", {})
        # oneway ignoré : à pied tout est bidirectionnel
        flat = _tag_on(tags, "bridge") or _tag_on(tags, "tunnel", ("no", "culvert"))
        cut = [i for i, n in enumerate(nodes) if i == 0 or i == len(nodes) - 1 or count[n] >= 2]
        for a, b in zip(cut[:-1], cut[1:]):
            raw.append((int(nodes[a]), int(nodes[b]), xy[a:b + 1], flat))
    return raw, len(ways)
