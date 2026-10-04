"""Validation d'un dossier de dalles avant publication (data.yml, procédure trimestrielle).

Seuils par dalle, calés sur IdF + Isère 10b (max observés : nodata 0,32 %, sauts 0,18 %,
replis de nœud 0,10 %) avec une marge ; écart de tronçons vs la version précédente < 5 %."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path

from .load import FORMAT

MAX_NODATA = 0.01          # part des points de profil sans MNT avant comblement
MAX_JUMP = 0.005           # tronçons à saut > 10 m sur 5 m / n
MAX_FALLBACK = 0.002       # nœuds sans MNT (altitude du voisin) / n
MAX_DELTA_N = 0.05         # |n - n précédent| / max(n précédent, 1000)


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
    for k, t in sorted(tiles.items()):
        p = d / f"{k}.npz"
        if not p.exists() or hashlib.sha256(p.read_bytes()).hexdigest() != t["sha256"]:
            bad.append(f"{k} : fichier absent ou sha256 faux")
        n = max(1, t["n"])
        for key, lim in (("jump_gt10", MAX_JUMP), ("node_fallback", MAX_FALLBACK)):
            if key not in t or t[key] / n > lim:
                bad.append(f"{k} : {key} = {t.get(key)} sur {n} tronçons (> {lim:.1%})")
        if t["nodata_frac"] > MAX_NODATA:
            bad.append(f"{k} : nodata_frac = {t['nodata_frac']} (> {MAX_NODATA})")
    if previous:
        pt = previous.get("tiles", {})
        for k in sorted(pt.keys() - tiles.keys()):
            bad.append(f"{k} : présente dans la version précédente, absente ici")
        for k in sorted(pt.keys() & tiles.keys()):
            a, b = pt[k]["n"], tiles[k]["n"]
            if abs(b - a) > MAX_DELTA_N * max(a, 1000):
                bad.append(f"{k} : {a} -> {b} tronçons (écart > {MAX_DELTA_N:.0%})")
    return bad
