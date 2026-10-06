"""Liste des dalles tiles/1 d'une zone : python scripts/tile_list.py [fxx|re|gp|mq|gf|yt]

Contour : départements ADMINEXPRESS-COG-CARTO (WFS Géoplateforme) dans le repère de la zone
(pipeline.build.ZONES), élargi de 1 km ; une dalle de 20 km est gardée si elle touche ce contour (pas de
dalle 100 % mer ou étrangère). Sorties : pipeline/tiles_france.txt (fxx, clés `<ix>_<iy>`) et
pipeline/tiles_dom.txt (re, gp, mq, yt, gf : clés `<zone>/<ix>_<iy>`).
Guyane (83 500 km², forêt sans voies) : seules les dalles d'au moins GF_MIN_HITS tronçons de la BD TOPO
(WFS resultType=hits) sont gardées, soit la bande littorale habitée et les bourgs du Maroni/Oyapock."""
import re
import sys

import requests
import shapely
from pyproj import Transformer
from shapely.geometry import box, shape

sys.path.insert(0, ".")
from pipeline.build import ZONES  # noqa: E402

T = 20000
GF_MIN_HITS = 500
DEP = {"re": "974", "gp": "971", "mq": "972", "gf": "973", "yt": "976"}
WFS = "https://data.geopf.fr/wfs/ows"
zone = sys.argv[1] if len(sys.argv) > 1 else "fxx"
crs = ZONES[zone]["crs"]
r = requests.get(WFS, timeout=300, params=dict(
    SERVICE="WFS", VERSION="2.0.0", REQUEST="GetFeature", TYPENAMES="ADMINEXPRESS-COG-CARTO.LATEST:departement",
    OUTPUTFORMAT="application/json", SRSNAME="EPSG:4326", COUNT=200))
r.raise_for_status()
# métropole : code INSEE de 2 caractères (01…95, 2A, 2B) ; DOM : 971…976
keep = (lambda c: len(c) == 2) if zone == "fxx" else (lambda c: c == DEP[zone])
tr = Transformer.from_crs("EPSG:4326", crs, always_xy=True)
land = shapely.union_all([shapely.transform(shape(f["geometry"]), tr.transform, interleaved=False)
                          for f in r.json()["features"] if keep(f["properties"]["code_insee"])]).buffer(1000)


def hits(ix, iy):
    q = requests.get(WFS, timeout=120, params=dict(
        SERVICE="WFS", VERSION="2.0.0", REQUEST="GetFeature", TYPENAMES="BDTOPO_V3:troncon_de_route", RESULTTYPE="hits",
        BBOX=f"{ix * T},{iy * T},{(ix + 1) * T},{(iy + 1) * T},urn:ogc:def:crs:EPSG::{crs[5:]}"))
    q.raise_for_status()
    return int(re.search(r'numberMatched="(\d+)"', q.text).group(1))


x0, y0, x1, y1 = land.bounds
pre = "" if zone == "fxx" else f"{zone}/"
for ix in range(int(x0 // T), int(x1 // T) + 1):
    for iy in range(int(y0 // T), int(y1 // T) + 1):
        if land.intersects(box(ix * T, iy * T, (ix + 1) * T, (iy + 1) * T)) \
                and (zone != "gf" or hits(ix, iy) >= GF_MIN_HITS):
            print(f"{pre}{ix}_{iy}")
