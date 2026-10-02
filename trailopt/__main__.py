"""CLI : python -m trailopt --start LAT,LON --distance 10 --out boucle.gpx"""
from __future__ import annotations

import argparse
import json
import sys

from .pipeline import Params, UserError, plan_loop


def main(argv=None):
    ap = argparse.ArgumentParser(prog="python -m trailopt", description=__doc__)
    ap.add_argument("--start", required=True, help="départ 'lat,lon'")
    ap.add_argument("--distance", type=float, required=True, help="distance (km)")
    ap.add_argument("--polygon", help="fichier GeoJSON (Polygon) de la zone ; défaut disque centré sur le départ")
    ap.add_argument("--mode", choices=["max", "target"], default="max")
    ap.add_argument("--dplus", type=float, help="D+ cible (m), mode target")
    ap.add_argument("--max-grade", type=float, help="pente max en %% (ex. 35)")
    ap.add_argument("--time", type=float, default=None,
                    help="budget solveur (s) ; défaut : selon la distance (20 s à 10 km, 60 s à 100 km)")
    ap.add_argument("--tol", type=float, default=0.05)
    ap.add_argument("--roads", choices=["unpaved", "pedestrian", "minor", "all"], default="minor",
                    help="unpaved = sentiers non revêtus ; pedestrian = voies piétonnes "
                         "(toujours via OSM) ; minor = + petites routes (défaut)")
    ap.add_argument("--source", choices=["ign", "osm"], default="ign",
                    help="réseau de chemins : BD TOPO IGN (défaut) ou OpenStreetMap")
    ap.add_argument("--solver", choices=["auto", "exact", "anneal", "faces"], default="auto")
    ap.add_argument("--exact-max-edges", type=int)
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--candidates", type=int, default=1,
                    help="nombre de boucles réellement différentes (moitié de temps en plus par boucle)")
    ap.add_argument("--no-revisit", action="store_true",
                    help="ne jamais repasser par un carrefour (hors 200 m du départ)")
    ap.add_argument("--no-limits", action="store_true", help="ignore les plafonds de l'app")
    ap.add_argument("--debug", action="store_true", help="affiche le dict debug (JSON)")
    ap.add_argument("--out", default="boucle.gpx")
    a = ap.parse_args(argv)

    lat, lon = map(float, a.start.split(","))
    poly = None
    if a.polygon:
        gj = json.load(open(a.polygon))
        gj = gj.get("geometry", gj) if gj.get("type") == "Feature" else gj
        poly = [tuple(c) for c in gj["coordinates"][0]]
    p = Params(lat, lon, a.distance, poly, a.mode, a.dplus,
               a.max_grade / 100 if a.max_grade else None, a.time, a.tol, a.roads, a.seed,
               a.solver, node_simple=a.no_revisit, source=a.source, n_candidates=a.candidates, enforce_limits=not a.no_limits)
    if a.exact_max_edges is not None:
        p.exact_max_edges = a.exact_max_edges
    try:
        r = plan_loop(p, lambda s: print(f"[{s}]", file=sys.stderr, flush=True))
    except UserError as ex:
        sys.exit(f"Erreur : {ex}")
    with open(a.out, "w", encoding="utf-8") as f:
        f.write(r.gpx)
    print(f"Méthode   : {r.method}")
    print(f"Distance  : {r.length / 1000:.2f} km" + ("" if r.feasible else "  (HORS BORNES)"))
    print(f"D+        : {r.dplus:.0f} m ({r.dplus / r.length * 1000:.0f} m/km)")
    if a.mode == "target":
        print(f"Erreurs   : distance {r.debug['err_distance']:+.1%}, D+ {r.debug['err_dplus']:+.1%}")
    for w in r.warnings:
        print(f"Attention : {w}")
    print(f"GPX       : {a.out}")
    for i, c in enumerate(r.candidates[1:], 2):
        path = a.out.replace(".gpx", f"_{i}.gpx")
        with open(path, "w", encoding="utf-8") as f:
            f.write(c.gpx)
        print(f"Variante {i} : {c.length / 1000:.2f} km, D+ {c.dplus:.0f} m -> {path}")
    if a.debug:
        print(json.dumps(r.debug, indent=1, default=str))


if __name__ == "__main__":
    main()
