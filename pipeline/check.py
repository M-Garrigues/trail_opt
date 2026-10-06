"""Validation d'un dossier de dalles avant publication (data.yml, procédure trimestrielle).

Seuils par dalle, calés sur IdF + Isère 10b (max observés : nodata 0,32 %, sauts 0,18 %,
replis de nœud 0,10 %) avec une marge ; écart de tronçons vs la version précédente < 5 %."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path

import numpy as np

from .load import FORMAT, read_tile

MAX_NODATA = 0.01          # part des points de profil sans MNT avant comblement
MAX_JUMP = 0.005           # tronçons à saut > 10 m sur 5 m / n
MAX_FALLBACK = 0.002       # nœuds sans MNT (altitude du voisin) / n
MAX_DELTA_N = 0.05         # |n - n précédent| / max(n précédent, 1000)


def node_conflicts(d: Path, tiles) -> list[str]:
    """Altitude unique par clé de nœud sur l'ensemble des dalles (D+ additif du moteur) : une même
    extrémité (x_dm, y_dm) doit avoir la même altitude, d'une dalle à l'autre. Une zone = un repère."""
    bad = []
    zones = {k.rpartition("/")[0] for k in tiles}
    for zone in sorted(zones):
        K, Z = [], []
        for k in sorted(t for t in tiles if t.rpartition("/")[0] == zone):
            ix, iy = map(int, k.rpartition("/")[2].split("_"))
            if not (d / f"{k}.npz").exists():
                continue                      # déjà signalée
            t = read_tile(d / f"{k}.npz", ix, iy)
            if not len(t["n"]):
                continue
            last = np.cumsum(t["n"]) - 1
            first = last - t["n"] + 1
            pl = np.cumsum(t["pn"]) - 1
            pf = pl - t["pn"] + 1
            x, y = t["x_dm"].astype(np.int64), t["y_dm"].astype(np.int64)
            for g, p in ((first, pf), (last, pl)):
                K.append((x[g] << 32) | y[g])
                Z.append(np.round(t["z"][p] * 10).astype(np.int64))
        if not K:
            continue
        k_, z_ = np.concatenate(K), np.concatenate(Z)
        o = np.argsort(k_, kind="stable")
        k_, z_ = k_[o], z_[o]
        dif = (k_[1:] == k_[:-1]) & (z_[1:] != z_[:-1])
        n = int(dif.sum())
        if n:
            i = int(np.flatnonzero(dif)[0])
            bad.append(f"{zone or 'fxx'} : {n} extrémités à altitude non unique (ex. clé {k_[i]} : "
                       f"{z_[i]} et {z_[i + 1]} dm)")
    return bad


def check(tiles_dir, expected: list[str] | None = None, previous: dict | None = None) -> list[str]:
    """Liste des problèmes (vide = publiable)."""
    d = Path(tiles_dir)
    m = json.loads((d / "manifest.json").read_text())
    bad = []
    if m.get("format") != FORMAT:
        bad.append(f"format {m.get('format')} != {FORMAT}")
    tiles = m.get("tiles", {})
    for k in sorted(set(expected or ()) - tiles.keys()):
        bad.append(f"{k} : absente du manifeste")
    broken = set()   # fichiers illisibles : écartés du contrôle des altitudes de nœud
    for k, t in sorted(tiles.items()):
        p = d / f"{k}.npz"
        if not p.exists() or hashlib.sha256(p.read_bytes()).hexdigest() != t["sha256"]:
            bad.append(f"{k} : fichier absent ou sha256 faux")
            broken.add(k)
        n = max(1, t["n"])
        for key, lim in (("jump_gt10", MAX_JUMP), ("node_fallback", MAX_FALLBACK)):
            if key not in t or t[key] / n > lim:
                bad.append(f"{k} : {key} = {t.get(key)} sur {n} tronçons (> {lim:.1%})")
        if t["nodata_frac"] > MAX_NODATA:
            bad.append(f"{k} : nodata_frac = {t['nodata_frac']} (> {MAX_NODATA})")
    bad += node_conflicts(d, [k for k in tiles if k not in broken])
    po = m.get("pois")
    if po:
        p = d / po["file"]
        if not p.exists() or hashlib.sha256(p.read_bytes()).hexdigest() != po["sha256"]:
            bad.append(f"{po['file']} : fichier absent ou sha256 faux")
        elif sorted(json.loads(p.read_text())["tiles"]) != sorted(tiles):
            bad.append(f"{po['file']} : dalles différentes du manifeste (relancer pipeline pois)")
    if previous:
        pt = previous.get("tiles", {})
        for k in sorted(pt.keys() - tiles.keys()):
            bad.append(f"{k} : présente dans la version précédente, absente ici")
        for k in sorted(pt.keys() & tiles.keys()):
            a, b = pt[k]["n"], tiles[k]["n"]
            if abs(b - a) > MAX_DELTA_N * max(a, 1000):
                bad.append(f"{k} : {a} -> {b} tronçons (écart > {MAX_DELTA_N:.0%})")
    return bad
