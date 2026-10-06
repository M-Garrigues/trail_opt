"""Assemblage des dalles construites par shards (data.yml) en un seul dossier publiable.

Chaque shard dépose dans le même dossier ses `.npz` (sous-dossier de zone compris) plus
`manifest-<k>.json` et `pois-<k>.json` (pipeline pois sur ses seules dalles). `merge` les fusionne en
`manifest.json` + `pois.json` et supprime les fichiers par shard. Aucune dalle n'est relue : leur sha256
sont vérifiés ensuite par `check`, qui contrôle aussi l'altitude de nœud entre dalles de shards voisins."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path

from trailopt import cache

from .build import totals
from .pois import FORMAT as POIS_FORMAT


def merge(tiles_dir) -> dict:
    d = Path(tiles_dir)
    parts = sorted(d.glob("manifest-*.json"))
    if not parts:
        raise RuntimeError(f"{d} : aucun manifest-<shard>.json")
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
    cache.write_atomic(d / "manifest.json", json.dumps(out, ensure_ascii=False, indent=1).encode())
    for p in parts:
        p.unlink()
        p.with_name(p.name.replace("manifest-", "pois-")).unlink(missing_ok=True)
    return out
