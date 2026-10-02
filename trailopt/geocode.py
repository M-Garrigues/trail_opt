"""Recherche d'adresse, de commune ou de lieu (géocodage Géoplateforme, sans clé)."""
from __future__ import annotations

import re

import requests

URL = "https://data.geopf.fr/geocodage/search"
_COORD = re.compile(r"^\s*(-?\d+(?:[.,]\d+)?)\s*[,; ]\s*(-?\d+(?:[.,]\d+)?)\s*$")


def search(q: str, limit: int = 4) -> list[dict]:
    """Candidats triés par pertinence : [{label, lat, lon, zoom}]. « lat, lon » est accepté."""
    q = q.strip()
    m = _COORD.match(q)
    if m:
        lat, lon = (float(v.replace(",", ".")) for v in m.groups())
        if -90 <= lat <= 90 and -180 <= lon <= 180:
            return [{"label": f"{lat:.5f}, {lon:.5f}", "lat": lat, "lon": lon, "zoom": 15}]
    if len(q) < 3:
        return []
    out = []
    for index in ("address", "poi"):   # adresses et communes ; lieux-dits, cols, forêts...
        try:
            r = requests.get(URL, params={"q": q, "limit": limit, "index": index}, timeout=10,
                             headers={"User-Agent": "trailopt/1.0"})
            r.raise_for_status()
            feats = r.json().get("features", [])
        except Exception:
            continue
        for f in feats:
            p = f["properties"]
            lon, lat = f["geometry"]["coordinates"][:2]
            if index == "address":
                label = p.get("label", q)
                zoom = 13 if p.get("type") in ("municipality", "locality") else 16
            else:
                kinds = p.get("category") or []
                where = ", ".join(p.get("city") or []) if isinstance(p.get("city"), list) \
                    else (p.get("city") or "")
                label = f"{p.get('toponym', q)} ({kinds[0] if kinds else 'lieu'}"\
                        f"{', ' + where if where else ''})"
                zoom = 13 if "commune" in kinds else 15
            out.append({"label": label, "lat": lat, "lon": lon, "zoom": zoom,
                        "score": p.get("score", 0.0)})
    out.sort(key=lambda c: -c["score"])
    return out[: limit + 2]
