"""Découpe de dalles tiles/1 : garde les tronçons dont le 1er sommet est dans un des disques
(lat, lon WGS84, rayon en m). Sert à la dalle de test versionnée (engine/tests/data/tiles, T29) : même format,
même data_version ; les parallèles vers un tronçon écarté sont retirés (relation restée symétrique)."""
from __future__ import annotations

import hashlib
import io
import json
from pathlib import Path

import numpy as np
from pyproj import Transformer

from .load import TILE_M, profile_counts

_TO_L93 = Transformer.from_crs("EPSG:4326", "EPSG:2154", always_xy=True)


def _load(path) -> dict:
    with np.load(path) as f:
        return {k: f[k] for k in f.files}


def clip(src, out, disks: list[tuple[float, float, float]]) -> dict:
    src, out = Path(src), Path(out)
    m = json.loads((src / "manifest.json").read_text())
    X, Y = _TO_L93.transform([d[1] for d in disks], [d[0] for d in disks])
    R = np.array([d[2] for d in disks])
    P = np.column_stack([X, Y]) * 10                          # dm
    keys = {f"{ix}_{iy}" for x, y, r in zip(X, Y, R)
            for ix in range(int((x - r) // TILE_M), int((x + r) // TILE_M) + 1)
            for iy in range(int((y - r) // TILE_M), int((y + r) // TILE_M) + 1)}
    tiles, kept_ids = {}, []
    for k in sorted(keys & m["tiles"].keys()):
        ix, iy = map(int, k.split("_"))
        T = _load(src / f"{k}.npz")
        first = np.cumsum(T["geom_n"]) - T["geom_n"]
        fx = T["geom_x"][first].astype(np.int64) + ix * TILE_M * 10
        fy = T["geom_y"][first].astype(np.int64) + iy * TILE_M * 10
        keep = ((fx[:, None] - P[:, 0]) ** 2 + (fy[:, None] - P[:, 1]) ** 2 < (R * 10) ** 2).any(axis=1)
        if keep.any():
            tiles[k] = (T, keep)
            kept_ids.append(np.cumsum(T["id_d"])[keep])
    kept_ids = np.concatenate(kept_ids) if kept_ids else np.zeros(0, np.int64)

    out.mkdir(parents=True, exist_ok=True)
    infos = {}
    for k, (T, keep) in tiles.items():
        n = len(keep)
        pn = profile_counts(T["len_dm"])
        owner = np.repeat(np.arange(n), T["par_n"])
        pk = keep[owner] & np.isin(T["par_id"], kept_ids)
        C = {c: T[c][keep] for c in ("len_dm", "dplus_dm", "dminus_dm", "max_grade_pm", "nature",
                                      "importance", "flags", "geom_n", "prof_z0_dm")}
        C.update(id_d=np.diff(np.cumsum(T["id_d"])[keep], prepend=0).astype(np.int64),
                 geom_x=T["geom_x"][np.repeat(keep, T["geom_n"])],
                 geom_y=T["geom_y"][np.repeat(keep, T["geom_n"])],
                 prof_d=T["prof_d"][np.repeat(keep, pn)],
                 par_n=np.bincount(owner[pk], minlength=n)[keep].astype(np.uint16),
                 par_id=T["par_id"][pk])
        buf = io.BytesIO()
        np.savez_compressed(buf, **{c: C[c] for c in T})        # même ordre de colonnes
        data = buf.getvalue()
        (out / f"{k}.npz").write_bytes(data)
        nk = int(keep.sum())
        starts = np.cumsum(profile_counts(C["len_dm"])) - profile_counts(C["len_dm"])
        jump = (np.abs(C["prof_d"].astype(np.int64)) > 100).astype(np.int64)  # 1er point : 0
        steep = int((C["max_grade_pm"] > 600).sum())
        info = dict(m["tiles"][k])                            # nodata_frac, node_fallback : dalle entière
        info.update(n=nk, km=round(int(C["len_dm"].sum()) / 1e4, 1), steep_gt60=steep,
                    steep_frac=round(steep / max(1, nk), 5), par_links=int(pk.sum()),
                    jump_gt10=int((np.add.reduceat(jump, starts) > 0).sum()) if nk else 0,
                    sha256=hashlib.sha256(data).hexdigest(), bytes=len(data),
                    o_par_troncon=round(len(data) / max(1, nk), 1))
        infos[k] = info
    tot = infos.values()
    n = sum(t["n"] for t in tot)
    man = {k: v for k, v in m.items() if k not in ("tiles", "totals")}
    man.update(clip=dict(disks=[list(d) for d in disks],
                         note="extrait de dalles complètes ; nodata_frac et node_fallback = dalle entière"),
               tiles=infos,
               totals=dict(tiles=len(infos), n=n, bytes=sum(t["bytes"] for t in tot),
                           steep_gt60=sum(t["steep_gt60"] for t in tot),
                           jump_gt10=sum(t["jump_gt10"] for t in tot),
                           node_fallback=sum(t.get("node_fallback", 0) for t in tot),
                           nodata_frac=round(sum(t["nodata_frac"] * t["n"] for t in tot) / max(1, n), 5)))
    if m.get("pois"):                                         # repères dans les disques
        doc = json.loads((src / m["pois"]["file"]).read_text())
        doc["pois"] = [q for q in doc["pois"]
                       if ((q["x_dm"] - P[:, 0]) ** 2 + (q["y_dm"] - P[:, 1]) ** 2 < (R * 10) ** 2).any()]
        doc["tiles"] = sorted(infos)
        data = (json.dumps(doc, ensure_ascii=False, separators=(",", ":")) + "\n").encode()
        (out / m["pois"]["file"]).write_bytes(data)
        man["pois"] = dict(m["pois"], n=len(doc["pois"]), bytes=len(data), sha256=hashlib.sha256(data).hexdigest(),
                           no_z=sum(q["z_dm"] is None for q in doc["pois"]))
    (out / "manifest.json").write_text(json.dumps(man, ensure_ascii=False, indent=1) + "\n")
    return man
