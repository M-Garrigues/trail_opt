"""CLI : python -m pipeline build --tiles <32_342,33_342 | sud,ouest,nord,est> --out <dossier>"""
from __future__ import annotations

import argparse
import json

from .build import build


def main(argv=None):
    ap = argparse.ArgumentParser(prog="python -m pipeline", description=__doc__)
    sub = ap.add_subparsers(dest="cmd", required=True)
    b = sub.add_parser("build", help="construit des dalles et met à jour manifest.json")
    b.add_argument("--tiles", required=True,
                   help="liste 'ix_iy,…' (grille L93 20 km) ou bbox WGS 'sud,ouest,nord,est'")
    b.add_argument("--out", required=True, help="dossier des dalles (créé si besoin)")
    b.add_argument("--force", action="store_true", help="refaire les dalles déjà écrites et intactes")
    a = ap.parse_args(argv)
    m = build(a.tiles, a.out, log=lambda s: print(s, flush=True), force=a.force)
    print(json.dumps(m.get("totals", {}), ensure_ascii=False))


if __name__ == "__main__":
    main()
