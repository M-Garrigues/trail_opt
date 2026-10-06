"""Repères cols et sommets (BD TOPO, détail orographique) des dalles d'un dossier -> `pois.json`.

Repères affichés seulement (D34) : nom, nature, position L93 au dm, altitude MNT au point. Une
requête WFS par dalle du manifeste (pas de cache : ~90 requêtes, petites), altitudes via le cache
MNT de `trailopt.elevation`. À relancer après tout `build` qui ajoute des dalles.
"""
from __future__ import annotations

import fcntl
import hashlib
import json
import time
from pathlib import Path

import numpy as np
import requests

from trailopt import cache, ign

from .build import _TO_L93, sampler, split_key, transformers
from .load import TILE_M

FORMAT = "pois/1"
LAYER = "BDTOPO_V3:detail_orographique"
NATURES = ("Col", "Pic", "Sommet")
SOURCE = "BD TOPO® IGN (détail orographique), RGE ALTI®, LiDAR HD — Etalab 2.0"
PEAK_RADIUS_M = 100.0
_g = np.arange(-PEAK_RADIUS_M, PEAK_RADIUS_M + 1, 10.0)
OFFSETS = np.column_stack([a.ravel() for a in np.meshgrid(_g, _g)])
OFFSETS = OFFSETS[np.hypot(OFFSETS[:, 0], OFFSETS[:, 1]) <= PEAK_RADIUS_M]
OFFSETS = OFFSETS[np.argsort(np.hypot(OFFSETS[:, 0], OFFSETS[:, 1]), kind="stable")]   # (0, 0) en premier


def fetch(bbox) -> list:
    """Points du détail orographique dans la bbox WGS (sud, ouest, nord, est), paginés."""
    s, w, n, e = bbox
    feats, start = [], 0
    while True:
        for attempt in range(4):
            try:
                r = requests.get(ign.WFS_URL, timeout=(10, 75), headers={"User-Agent": ign.USER_AGENT}, params=dict(
                    SERVICE="WFS", VERSION="2.0.0", REQUEST="GetFeature", TYPENAMES=LAYER,
                    BBOX=f"{w:.5f},{s:.5f},{e:.5f},{n:.5f},urn:ogc:def:crs:OGC:1.3:CRS84",
                    PROPERTYNAME="cleabs,nature,toponyme,importance,geometrie", OUTPUTFORMAT="application/json",
                    COUNT=ign.PAGE, STARTINDEX=start, SORTBY="cleabs"))
                r.raise_for_status()
                page = r.json()["features"]
                break
            except Exception as ex:
                last = ex
                time.sleep(2 ** attempt)
        else:
            raise RuntimeError(f"WFS {LAYER} indisponible : {last}")
        feats += page
        if len(page) < ign.PAGE:
            return feats
        start += ign.PAGE


def pois_from_features(feats, tiles: set[str], sample, to_crs=_TO_L93) -> list[dict]:
    """Garde Col/Pic/Sommet nommés dont le point est dans une dalle de `tiles` (noms `<ix>_<iy>` d'une
    même zone, repère `to_crs`) ; sample(X, Y) -> z (m). Rangés par id, sans doublon."""
    rows = {}
    for f in feats:
        p, g = f.get("properties") or {}, f.get("geometry") or {}
        name = (p.get("toponyme") or "").strip()
        cle = str(p.get("cleabs") or "")
        if p.get("nature") not in NATURES or not name or g.get("type") != "Point" or not cle[8:].isdigit():
            continue
        x, y = to_crs.transform(*g["coordinates"][:2])
        x_dm, y_dm = round(x * 10), round(y * 10)
        if f"{x_dm // (TILE_M * 10)}_{y_dm // (TILE_M * 10)}" in tiles:
            rows[int(cle[8:])] = dict(id=int(cle[8:]), nature=p["nature"], name=name,
                                      importance=int(p.get("importance") or 0), x_dm=x_dm, y_dm=y_dm)
    out = [rows[k] for k in sorted(rows)]
    # Col : MNT au point. Sommet/pic : point du toponyme souvent à côté du sommet (précision 5–30 m,
    # ex. Mont Aiguille 1 959 m au point, 2 055 m au max à 100 m, 2 085 m officiel) -> max du MNT à 100 m.
    if not out:
        return out
    k = np.array([1 if r["nature"] == "Col" else len(OFFSETS) for r in out])
    j = np.concatenate([np.arange(n) for n in k])
    xy = np.repeat(np.array([[r["x_dm"], r["y_dm"]] for r in out]) / 10.0, k, axis=0) + OFFSETS[j]
    z = np.asarray(sample(xy[:, 0], xy[:, 1]), float)
    for r, v in zip(out, np.split(z, np.cumsum(k)[:-1])):
        r["z_dm"] = int(round(float(np.nanmax(v)) * 10)) if np.isfinite(v).any() else None
    return out


def build_pois(tiles_dir, log=print) -> dict:
    """Écrit tiles_dir/pois.json pour toutes les dalles du manifeste et l'inscrit dans le manifeste."""
    d = Path(tiles_dir)
    m = json.loads((d / "manifest.json").read_text())
    tiles = sorted(m["tiles"])
    pois = []
    for zone in sorted({split_key(k)[0] for k in tiles}):       # DOM : repère de la zone, champ `zone` en plus
        to_wgs, to_crs = transformers(zone)
        feats, names = [], set()
        for k in tiles:
            z, ix, iy = split_key(k)
            if z != zone:
                continue
            names.add(f"{ix}_{iy}")
            lon, lat = to_wgs.transform([ix * TILE_M, (ix + 1) * TILE_M] * 2,
                                        [iy * TILE_M] * 2 + [(iy + 1) * TILE_M] * 2)
            feats += fetch((min(lat), min(lon), max(lat), max(lon)))
        rows = pois_from_features(feats, names, sampler(zone), to_crs)
        pois += rows if zone == "fxx" else [dict(r, zone=zone) for r in rows]
    pois.sort(key=lambda r: r["id"])
    doc = dict(format=FORMAT, data_version=m["data_version"], source=SOURCE, natures=list(NATURES),
               tiles=tiles, pois=pois)
    data = (json.dumps(doc, ensure_ascii=False, separators=(",", ":")) + "\n").encode()
    cache.write_atomic(d / "pois.json", data)
    info = dict(file="pois.json", format=FORMAT, n=len(pois), bytes=len(data),
                sha256=hashlib.sha256(data).hexdigest(),
                no_z=sum(p["z_dm"] is None for p in pois))
    with open(d / "manifest.lock", "w") as lock:   # même verrou que build
        fcntl.flock(lock, fcntl.LOCK_EX)
        m = json.loads((d / "manifest.json").read_text())
        m["pois"] = info
        cache.write_atomic(d / "manifest.json", json.dumps(m, ensure_ascii=False, indent=1).encode())
    log(f"pois.json : {json.dumps(info, ensure_ascii=False)}")
    return info
