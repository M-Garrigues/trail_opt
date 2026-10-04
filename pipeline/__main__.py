"""CLI : python -m pipeline build --tiles <32_342,33_342 | sud,ouest,nord,est> --out <dossier>
         python -m pipeline coverage --tiles <dossier des dalles> --out <coverage.geojson>
         python -m pipeline check --tiles <dossier> [--list tiles.txt] [--previous manifest.json]
         python -m pipeline clip --tiles <dossier> --out <dossier> --disk lat,lon,rayon_m [--disk …]"""
from __future__ import annotations

import argparse
import json

import sys
from pathlib import Path

from .build import build
from .check import check
from .clip import clip
from .coverage import write


def main(argv=None):
    ap = argparse.ArgumentParser(prog="python -m pipeline", description=__doc__)
    sub = ap.add_subparsers(dest="cmd", required=True)
    b = sub.add_parser("build", help="construit des dalles et met à jour manifest.json")
    b.add_argument("--tiles", required=True,
                   help="liste 'ix_iy,…' (grille L93 20 km) ou bbox WGS 'sud,ouest,nord,est'")
    b.add_argument("--out", required=True, help="dossier des dalles (créé si besoin)")
    b.add_argument("--force", action="store_true", help="refaire les dalles déjà écrites et intactes")
    c = sub.add_parser("coverage", help="écrit le contour GeoJSON WGS84 des dalles du manifeste")
    c.add_argument("--tiles", required=True, help="dossier des dalles (manifest.json)")
    c.add_argument("--out", required=True, help="fichier GeoJSON (ex. web/public/coverage.geojson)")
    k = sub.add_parser("clip", help="extrait les tronçons de quelques disques (dalle de test)")
    k.add_argument("--tiles", required=True, help="dossier source (manifest.json)")
    k.add_argument("--out", required=True, help="dossier de sortie")
    k.add_argument("--disk", required=True, action="append",
                   type=lambda s: tuple(map(float, s.split(","))), help="lat,lon,rayon_m (répétable)")
    v = sub.add_parser("check", help="valide un dossier de dalles (code de sortie 1 si problème)")
    v.add_argument("--tiles", required=True, help="dossier des dalles (manifest.json)")
    v.add_argument("--list", help="fichier des dalles attendues (une 'ix_iy' par ligne)")
    v.add_argument("--previous", help="manifest.json de la version précédente (écart de tronçons)")
    a = ap.parse_args(argv)
    if a.cmd == "check":
        bad = check(a.tiles, a.list and Path(a.list).read_text().split(),
                    a.previous and json.loads(Path(a.previous).read_text()))
        print("\n".join(bad) or "OK")
        sys.exit(1 if bad else 0)
    if a.cmd == "clip":
        print(json.dumps(clip(a.tiles, a.out, a.disk)["totals"], ensure_ascii=False))
        return
    if a.cmd == "coverage":
        print(f"{a.out} : {write(a.tiles, a.out)} octets")
        return
    m = build(a.tiles, a.out, log=lambda s: print(s, flush=True), force=a.force)
    print(json.dumps(m.get("totals", {}), ensure_ascii=False))


if __name__ == "__main__":
    main()
