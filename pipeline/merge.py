"""Assemblage des dalles construites par shards (data.yml) en un seul dossier publiable.

Chaque shard dépose dans le même dossier ses `.npz` (sous-dossier de zone compris) plus
`manifest-<k>.json` et `pois-<k>.json` (pipeline pois sur ses seules dalles). `merge` les fusionne en
`manifest.json` + `pois.json` et supprime les fichiers par shard, puis harmonise l'altitude des nœuds
partagés par plusieurs dalles (`harmonize`, seules les dalles touchées sont réécrites). Les sha256 sont
vérifiés ensuite par `check`, qui contrôle aussi l'altitude de nœud entre dalles."""
from __future__ import annotations

import hashlib
import io
import json
import shutil
from pathlib import Path

import numpy as np

from trailopt import cache

from .build import split_key, totals
from .check import BAC
from .load import profile_counts, read_tile
from .pois import FORMAT as POIS_FORMAT


def harmonize(d: Path, m: dict, backup=None, log=print) -> list[str]:
    """Altitude unique par nœud d'une dalle à l'autre. Un nœud sans MNT prend dans chaque dalle
    l'altitude d'un voisin qui peut différer (bac dont l'autre bout sort de la zone chargée, côte en
    bord de dalle). Valeur retenue : la plus basse parmi celles des tronçons hors bacs, sinon la plus
    basse. Chaque tronçon concerné reçoit une rampe linéaire le long du profil (0 à l'autre bout),
    D+, D−, pente max et sauts recalculés. Met `m` à jour ; `backup` : copie des dalles avant réécriture."""
    bac = m["natures"].index(BAC)
    out = []
    for zone in sorted({k.rpartition("/")[0] for k in m["tiles"]}):
        keys = sorted(k for k in m["tiles"] if k.rpartition("/")[0] == zone)
        K, Z, B, W = [], [], [], []
        for ti, k in enumerate(keys):
            t = read_tile(d / f"{k}.npz", *split_key(k)[1:])
            last = np.cumsum(t["n"]) - 1
            pl = np.cumsum(t["pn"]) - 1
            x, y = t["x_dm"].astype(np.int64), t["y_dm"].astype(np.int64)
            for end, (g, p) in enumerate(((last - t["n"] + 1, pl - t["pn"] + 1), (last, pl))):
                K.append((x[g] << 32) | y[g])
                Z.append(np.round(t["z"][p] * 10).astype(np.int64))
                B.append(t["nature"] == bac)
                W.append((ti << 32) | (np.arange(len(g), dtype=np.int64) << 1) | end)
        if not K:
            continue
        K, Z, B, W = map(np.concatenate, (K, Z, B, W))
        o = np.lexsort((Z, B, K))               # par nœud : hors bacs d'abord, puis la plus basse
        K, Z, W = K[o], Z[o], W[o]
        head = np.flatnonzero(np.r_[True, K[1:] != K[:-1]])
        dz = np.repeat(Z[head], np.diff(np.r_[head, len(K)])) - Z
        fix = np.flatnonzero(dz)
        log(f"{zone or 'fxx'} : {len(fix)} extrémités recalées sur {len(np.unique(K[fix]))} nœuds")
        for ti in np.unique(W[fix] >> 32).tolist():
            sel = fix[(W[fix] >> 32) == ti]
            k = keys[ti]
            if backup:
                (Path(backup) / f"{k}.npz").parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(d / f"{k}.npz", Path(backup) / f"{k}.npz")
            m["tiles"][k].update(_ramp(d / f"{k}.npz", (W[sel] & 0xFFFFFFFF) >> 1, W[sel] & 1, dz[sel]))
            out.append(k)
    m["totals"] = totals(m["tiles"])
    return out


def _ramp(p: Path, edge, end, dz) -> dict:
    """Ajoute dz (dm) à l'extrémité `end` (0 = u, 1 = v) des tronçons `edge` de la dalle p, en rampe
    linéaire ; réécrit le .npz (autres colonnes inchangées). Renvoie les champs du manifeste à jour."""
    with np.load(p) as f:
        T = {k: f[k] for k in f.files}
    pn = profile_counts(T["len_dm"])
    start = np.cumsum(pn) - pn
    z = np.cumsum(T["prof_d"].astype(np.int64))
    z += np.repeat(T["prof_z0_dm"].astype(np.int64) - z[start], pn)     # profil absolu (dm)
    du, dv = np.zeros(len(pn)), np.zeros(len(pn))
    np.add.at(du, edge[end == 0], dz[end == 0])
    np.add.at(dv, edge[end == 1], dz[end == 1])
    for i in np.unique(edge).tolist():
        L = int(T["len_dm"][i])
        s = np.minimum(np.arange(pn[i]) * 50, L) / 10.0                # abscisses (m)
        zi = z[start[i]:start[i] + pn[i]] + np.round(du[i] + (dv[i] - du[i]) * s * 10 / L).astype(np.int64)
        z[start[i]:start[i] + pn[i]] = zi
        d = np.diff(zi)
        T["dplus_dm"][i], T["dminus_dm"][i] = d[d > 0].sum(), -d[d < 0].sum()
        zm = zi / 10.0
        if s[-1] <= 25.0:                                             # comme graph.compute_profile
            g = abs(zm[-1] - zm[0]) / max(s[-1], 1e-6)
        else:
            j = np.searchsorted(s, s + 25.0)
            a = np.flatnonzero(j < len(s))
            g = float(np.max(np.abs(zm[j[a]] - zm[a]) / (s[j[a]] - s[a]))) if len(a) else 0.0
        T["max_grade_pm"][i] = min(65535, round(g * 1000))
    d = np.diff(z, prepend=0)
    d[start] = 0
    if np.abs(d).max(initial=0) > 32767:
        raise ValueError(f"{p} : saut de profil > 3 276 m sur 5 m après harmonisation")
    T["prof_d"], T["prof_z0_dm"] = d.astype(np.int16), z[start].astype(np.int32)
    buf = io.BytesIO()
    np.savez_compressed(buf, **T)
    data = buf.getvalue()
    cache.write_atomic(p, data)
    jump = np.maximum.reduceat(np.abs(d), start) > 100 if len(start) else np.zeros(0, bool)
    return dict(sha256=hashlib.sha256(data).hexdigest(), bytes=len(data), jump_gt10=int(jump.sum()),
                o_par_troncon=round(len(data) / max(1, len(pn)), 1))


def merge(tiles_dir, backup=None) -> dict:
    """Fusionne les manifestes de shards puis harmonise ; sans manifest-<k>.json, harmonise seulement
    le manifest.json existant (dossier déjà fusionné)."""
    d = Path(tiles_dir)
    parts = sorted(d.glob("manifest-*.json"))
    if not parts:
        if not (d / "manifest.json").exists():
            raise RuntimeError(f"{d} : aucun manifest-<shard>.json")
        m = json.loads((d / "manifest.json").read_text())
        harmonize(d, m, backup)
        cache.write_atomic(d / "manifest.json", json.dumps(m, ensure_ascii=False, indent=1).encode())
        return m
    out, pois = None, {}
    for p in parts:
        m = json.loads(p.read_text())
        out = out or {k: v for k, v in m.items() if k not in ("tiles", "totals", "pois", "zones")}
        out.setdefault("zones", {}).update(m.get("zones", {}))   # union : un shard n'a pas toutes les zones
        for k in ("format", "data_version", "source", "natures"):
            if m.get(k) != out.get(k):
                raise RuntimeError(f"{p.name} : `{k}` diffère des autres shards")
        dup = out.setdefault("tiles", {}).keys() & m["tiles"].keys()
        if dup:
            raise RuntimeError(f"{p.name} : dalles déjà dans un autre shard : {sorted(dup)[:3]}")
        out["tiles"].update(m["tiles"])
        q = p.with_name(p.name.replace("manifest-", "pois-"))
        if m.get("pois") and q.exists():
            doc = json.loads(q.read_text())
            if hashlib.sha256(q.read_bytes()).hexdigest() != m["pois"]["sha256"]:
                raise RuntimeError(f"{q.name} : sha256 faux")
            pois.update({r["id"]: r for r in doc["pois"]})
            out["_pois_doc"] = {k: v for k, v in doc.items() if k not in ("tiles", "pois")}
        elif m["tiles"]:
            raise RuntimeError(f"{q.name} : repères absents (pipeline pois oublié ?)")
    out["tiles"] = dict(sorted(out["tiles"].items()))
    if not out["zones"]:
        del out["zones"]                                          # métropole seule : manifeste inchangé
    out["totals"] = totals(out["tiles"])
    doc = dict(out.pop("_pois_doc"), tiles=list(out["tiles"]), pois=[pois[k] for k in sorted(pois)])
    data = (json.dumps(doc, ensure_ascii=False, separators=(",", ":")) + "\n").encode()
    cache.write_atomic(d / "pois.json", data)
    out["pois"] = dict(file="pois.json", format=POIS_FORMAT, n=len(pois), bytes=len(data),
                       sha256=hashlib.sha256(data).hexdigest(),
                       no_z=sum(r["z_dm"] is None for r in pois.values()))
    harmonize(d, out, backup)
    cache.write_atomic(d / "manifest.json", json.dumps(out, ensure_ascii=False, indent=1).encode())
    for p in parts:
        p.unlink()
        p.with_name(p.name.replace("manifest-", "pois-")).unlink(missing_ok=True)
    return out
