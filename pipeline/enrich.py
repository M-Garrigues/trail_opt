"""Étiquettes d'agrément ajoutées aux dalles `tiles/1` DÉJÀ construites (post-traitement, aucune requête IGN).

Chaque `.npz` du dossier est réécrit avec des colonnes en plus (les colonnes existantes sont recopiées
telles quelles), le manifeste reçoit sha256/tailles, `columns` (source et licence par colonne),
`derived_from` et une `data_version` dérivée `<base>.<n>`. Contrat : .team/contracts/tiles.md § Étiquettes.

- `calm` (uint8, 0–15) : éloignement moyen de la route importante la plus proche × nature propre du
  tronçon. Calculé depuis les seules dalles (la dalle et ses 8 voisines), donc sans licence nouvelle.
- `osm_hike` (uint8, 0 aucun / 1 balisé / 2 balisé régional ou plus), chemins seulement, et
  `osm_water` (uint8, 0–15 = part de la longueur à moins de 50 m de l'eau) : seulement avec `--osm`.
  `osm_class` (uint8, classe de voie 0 chemin naturel / 1 intermédiaire, 255 = rien appris d'OSM : le
  moteur prend alors la classe de la nature IGN, 2 route compris) : seulement avec `--osm`.
  © les contributeurs d'OpenStreetMap, ODbL. Étiquettes seulement : aucune géométrie OSM n'entre dans
  la dalle, et l'absence d'étiquette est neutre (0 = « rien de connu », jamais une pénalité).
- `osm_access` (uint8, D66) : 2 = fermé au piéton (`foot=private|no`, ou `access=private|no` sans `foot`
  autorisant), 1 = restreint (customers, destination…), posé dès 20 m appariés ; le moteur EXCLUT 2 (seule
  étiquette qui retire une voie, comme la via ferrata de `osm_flags`). Barrières des nœuds non lues (voir contrat).

- Colonnes vélo (D70, `--osm` seulement, 0 = inconnu = neutre) : `osm_bike` (accès vélo 0–5, pire valeur sur ≥ 20 m),
  `osm_cycleway` (aménagement 0–5), `osm_oneway` (sens unique vélo ORIENTÉ u→v, 0–3), `osm_bike_route` / `osm_mtb_route`
  (réseaux `route=bicycle` / `route=mtb`), `osm_mtb` / `osm_mtb_up` (`mtb:scale`), `osm_tracktype`, `osm_smooth`,
  `osm_maxspeed` ; bits 16–128 d'`osm_flags` (escalier, informal, fermeture conditionnelle, rond-point).

Chaque valeur ne dépend que du tronçon et de ce qui l'entoure à 300 m au plus : le résultat est le même
quel que soit l'ordre ou le lot de traitement, pourvu que les dalles voisines soient dans le dossier.
"""
from __future__ import annotations

import datetime
import functools
import hashlib
import io
import json
import re
import resource
import subprocess
import sys
import time
from array import array
from pathlib import Path

import numpy as np
from scipy.spatial import cKDTree

from trailopt import cache

from .build import ZONES, split_key, totals, transformers
from .load import FORMAT, TILE_M, _undelta

CALM_MAX_M = 300.0         # au-delà, la route ne s'entend plus : note pleine
HIKE_BUF_M = 15.0          # écart toléré entre le tracé IGN et le tracé OSM du même chemin
HIKE_COS = 0.8             # ... et directions compatibles (|cos|)
HIKE_FRAC = 0.6            # part de la longueur appariée pour poser l'étiquette
SPOT_M = 20.0              # difficulté, visibilité, via ferrata : posées dès 20 m appariés (ou la moitié du tronçon)
WATER_M = 50.0             # « au bord de l'eau »
WATER_MIN_AREA_M2 = 5000.0  # plan d'eau d'un seul tenant plus petit (mare, bassin) : ignoré
DRINK_M = 100.0            # point d'eau potable à moins de 100 m
VIEW_M = 50.0              # sommet, col ou point de vue à moins de 50 m
FOREST_PX = 10.0           # maille (m) du raster de couvert forestier
ROADS = ("Route à 1 chaussée", "Route à 2 chaussées", "Rond-point", "Bretelle")
PATHS = ("Sentier", "Chemin", "Route empierrée", "Escalier", "Piste cyclable")
MOTORWAY = "Type autoroutier"
CROSS_IMPORTANCE = (1, 2)  # ign_cross : routes d'importance 1-2 (axes principaux)
IGN = dict(source="dérivé des dalles (BD TOPO® IGN)", license="Licence Ouverte Etalab 2.0")
OSM = dict(source="© les contributeurs d'OpenStreetMap", license="ODbL 1.0")
PN = dict(source="Parcs nationaux de France, contours des 11 parcs nationaux (cœurs terrestres), data.gouv.fr",
          license="Licence Ouverte 2.0")
NOCLASS = 255              # osm_class : rien appris d'OSM, repli moteur sur la nature IGN
OTHER = 255                # osm_highway, osm_surface : valeur OSM hors table
# Tables figées (on n'ajoute qu'en fin de liste) ; code = rang, 0 = aucun way apparié ou tag absent.
HIGHWAYS = ("", "path", "track", "bridleway", "footway", "pedestrian", "steps", "cycleway", "living_street",
            "corridor", "via_ferrata", "service", "residential", "unclassified", "tertiary", "tertiary_link",
            "secondary", "secondary_link", "primary", "primary_link", "trunk", "trunk_link", "motorway",
            "motorway_link", "road", "busway", "construction", "proposed", "platform", "raceway", "elevator")
PAVED = ("asphalt", "paved", "concrete", "concrete:plates", "concrete:lanes", "paving_stones", "sett",
         "cobblestone", "unhewn_cobblestone", "bricks", "metal", "wood", "chipseal", "tartan", "rubber")
SEMI = ("compacted", "fine_gravel", "grass_paver")                 # allée stabilisée
NATURAL = ("unpaved", "gravel", "dirt", "earth", "ground", "grass", "mud", "sand", "rock", "pebblestone",
           "woodchips", "stone")
SURFACES = ("",) + PAVED + SEMI + NATURAL + ("scree", "clay", "artificial_turf", "metal_grid", "shells", "snow", "ice")
SAC = {"hiking": 1, "mountain_hiking": 2, "demanding_mountain_hiking": 3, "alpine_hiking": 4,
       "demanding_alpine_hiking": 5, "difficult_alpine_hiking": 6}
VISIBILITY = {"excellent": 1, "good": 2, "intermediate": 3, "bad": 4, "horrible": 5, "no": 6}
SMOOTH = {"excellent": 1, "good": 1, "intermediate": 1, "bad": 2, "very_bad": 3, "horrible": 3,
          "very_horrible": 3, "impassable": 3}
ROUGH = {**{s: 1 for s in PAVED + ("compacted", "fine_gravel", "dirt", "earth", "ground", "clay", "tartan")},
         **{s: 2 for s in ("gravel", "pebblestone", "grass", "grass_paver", "sand", "mud", "unpaved", "woodchips",
                           "shells", "snow")},
         **{s: 3 for s in ("rock", "stone", "scree", "ice")}}
F_VIA, F_LIT, F_DRINK, F_VIEW = 1, 2, 4, 8     # bits de osm_flags
F_STEPS, F_INFORMAL, F_COND, F_ROUND = 16, 32, 64, 128   # escalier, informal=yes, fermeture conditionnelle, rond-point
STEPS_M = 10.0             # un escalier court (une volée) marque déjà le tronçon
ACC_RESTRICTED, ACC_CLOSED = 1, 2              # osm_access (D66) : restreint, fermé au piéton
# Vélo (D70). Accès : valeurs de `bicycle` qui autorisent, qui restreignent ; défauts France par type de voie.
BIKE_OK = ("yes", "designated", "permissive")
BIKE_RESTRICTED = ("destination", "customers", "delivery", "permit", "agricultural", "forestry")
BIKE_BANNED = ("motorway", "motorway_link", "trunk", "trunk_link", "bridleway", "via_ferrata")
CYCLEWAY = {"shared_lane": 1, "share_busway": 1, "lane": 2, "opposite_lane": 2, "track": 3, "opposite_track": 3}
CYCLEWAY_KEYS = ("cycleway", "cycleway:both", "cycleway:left", "cycleway:right")
ROUNDABOUT = ("roundabout", "circular")     # `junction` : sens unique implicite, bit F_ROUND
BIKE_NETWORK = {"lcn": 1, "rcn": 2, "ncn": 3, "icn": 4}
TRACKTYPE = {f"grade{i}": i for i in range(1, 6)}
SMOOTHNESS = {s: i for i, s in enumerate(("excellent", "good", "intermediate", "bad", "very_bad", "horrible",
                                          "very_horrible", "impassable"), 1)}
SPEED = {"FR:urban": 50, "FR:rural": 80, "FR:zone30": 30, "FR:motorway": 130, "walk": 6}
ROUTE_TYPE = {"hiking": "hike", "foot": "hike", "bicycle": "broute", "mtb": "mroute"}
TRAILS = ("path", "track", "bridleway")
PEDESTRIAN = ("pedestrian", "footway", "steps", "cycleway", "living_street", "corridor")
NOT_STREET = ("path", "bridleway", "footway", "steps", "cycleway", "corridor", "via_ferrata")   # jamais appariés à une route IGN
COLUMNS = {
    "calm": dict(dtype="uint8", range=[0, 15], **IGN),
    "ign_cross": dict(dtype="uint8", range=[0, 3], **IGN),
    "osm_hike": dict(dtype="uint8", range=[0, 2], **OSM),
    "osm_water": dict(dtype="uint8", range=[0, 15], **OSM),
    "osm_class": dict(dtype="uint8", range=[0, 255], values=[0, 1, NOCLASS], **OSM),
    "osm_highway": dict(dtype="uint8", range=[0, 255], codes=list(HIGHWAYS), **OSM),
    "osm_surface": dict(dtype="uint8", range=[0, 255], codes=list(SURFACES),
                        paved=[SURFACES.index(v) for v in PAVED + ("artificial_turf", "metal_grid")], **OSM),
    "osm_rough": dict(dtype="uint8", range=[0, 3], **OSM),
    "osm_sac": dict(dtype="uint8", range=[0, 6], **OSM),
    "osm_visibility": dict(dtype="uint8", range=[0, 6], **OSM),
    "osm_flags": dict(dtype="uint8", range=[0, 255], **OSM),
    "osm_forest": dict(dtype="uint8", range=[0, 15], **OSM),
    "osm_access": dict(dtype="uint8", range=[0, 2], **OSM),
    "osm_bike": dict(dtype="uint8", range=[0, 5], **OSM),
    "osm_cycleway": dict(dtype="uint8", range=[0, 5], **OSM),
    "osm_oneway": dict(dtype="uint8", range=[0, 3], **OSM),
    "osm_bike_route": dict(dtype="uint8", range=[0, 4], **OSM),
    "osm_mtb_route": dict(dtype="uint8", range=[0, 2], **OSM),
    "osm_mtb": dict(dtype="uint8", range=[0, 7], **OSM),
    "osm_mtb_up": dict(dtype="uint8", range=[0, 6], **OSM),
    "osm_tracktype": dict(dtype="uint8", range=[0, 5], **OSM),
    "osm_smooth": dict(dtype="uint8", range=[0, 8], **OSM),
    "osm_maxspeed": dict(dtype="uint8", range=[0, 254], **OSM),
    "pn_core": dict(dtype="uint8", range=[0, 1], **PN),
}
PN_CORE = "CTPN"           # code_r_enp des cœurs terrestres (CMPN cœur marin, AAPN aire d'adhésion, AMAPN aire maritime : ignorés)
ADDED = tuple(COLUMNS)
# Extrait OSM réduit à ce qui sert : `osmium tags-filter` (ways porteurs de leurs coordonnées ensuite).
OSM_FILTER = ("r/route=hiking,foot,bicycle,mtb", "w/waterway=river,canal", "wr/natural=water", "w/natural=coastline",
              "w/highway", "w/via_ferrata_scale", "n/amenity=drinking_water", "n/drinking_water=yes",
              "n/natural=peak,saddle", "n/mountain_pass=yes", "n/tourism=viewpoint")
FOREST_FILTER = ("wr/landuse=forest", "wr/natural=wood")
METRO = (-6.0, 41.0, 10.5, 51.6)   # ouest, sud, est, nord : ways OSM projetés en Lambert-93


def _points(x, y, n, step, sel=None):
    """Milieux de pas d'au plus `step` m le long de chaque ligne (x, y concaténés, n sommets par ligne).
    Renvoie px, py, ux, uy (direction unitaire), rang de la ligne, longueur du pas."""
    n = np.asarray(n, np.int64)
    sx, sy = np.diff(x), np.diff(y)
    L = np.hypot(sx, sy)
    owner = np.repeat(np.arange(len(n)), n)[:-1]
    k = np.maximum(1, np.ceil(L / step)).astype(np.int64)
    k[np.cumsum(n)[:-1] - 1] = 0                   # pas de segment entre deux lignes
    if sel is not None:
        k[~sel[owner]] = 0
    seg = np.repeat(np.arange(len(L)), k)
    r = (np.arange(len(seg)) - np.repeat(np.cumsum(k) - k, k) + 0.5) / k[seg]
    Ls = np.maximum(L[seg], 1e-9)
    return x[seg] + r * sx[seg], y[seg] + r * sy[seg], sx[seg] / Ls, sy[seg] / Ls, owner[seg], L[seg] / k[seg]


def _share(owner, w, mask, n):
    """Part de la longueur de chaque ligne dont les pas vérifient `mask`."""
    return np.bincount(owner, weights=w * mask, minlength=n) / np.maximum(np.bincount(owner, weights=w, minlength=n), 1e-9)


def _geom(path: Path, ix: int, iy: int) -> dict:
    """Ce qu'il faut d'une dalle pour l'étiqueter : sommets absolus (m), nature, importance."""
    with np.load(path) as f:
        n = f["geom_n"].astype(np.int64)
        return dict(n=n, x=_undelta(f["geom_x"], n, ix * TILE_M * 10) / 10.0,
                    y=_undelta(f["geom_y"], n, iy * TILE_M * 10) / 10.0,
                    nature=f["nature"], importance=f["importance"], flags=f["flags"])


def _codes(natures, names):
    return [natures.index(k) for k in names if k in natures]


@functools.lru_cache(maxsize=512)
def _major(d: Path, key: str, natures: tuple) -> np.ndarray:
    """Points (pas de 20 m) des routes importantes d'une dalle : carrossables d'importance 1 à 3, ou
    type autoroutier. Dalle absente du dossier = aucune voie (mer, étranger)."""
    p = d / f"{key}.npz"
    if not p.exists():
        return np.zeros((0, 2))
    _, ix, iy = split_key(key)
    g = _geom(p, ix, iy)
    sel = (np.isin(g["nature"], _codes(natures, ROADS)) & np.isin(g["importance"], (1, 2, 3))) \
        | np.isin(g["nature"], _codes(natures, [MOTORWAY]))
    px, py, *_ = _points(g["x"], g["y"], g["n"], 20.0, sel)
    return np.column_stack([px, py])


def calm(d: Path, key: str, natures) -> np.ndarray:
    """round(15 × min(distance moyenne à la route importante la plus proche / 300 m, 1) × propre), où
    propre = 0,6 sur une route d'importance 4, 1 sinon. Une route importante est à distance nulle
    d'elle-même : importance 1 à 3 et autoroutes valent donc 0."""
    d, natures = Path(d), tuple(natures)
    zone, ix, iy = split_key(key)
    pre = "" if zone == "fxx" else f"{zone}/"
    g = _geom(d / f"{key}.npz", ix, iy)
    ns = len(g["n"])
    Q = np.vstack([_major(d, f"{pre}{i}_{j}", natures) for i in (ix - 1, ix, ix + 1) for j in (iy - 1, iy, iy + 1)])
    px, py, _, _, ow, w = _points(g["x"], g["y"], g["n"], 10.0)
    dist = np.full(len(px), CALM_MAX_M)
    if len(Q) and len(px):
        dist = np.minimum(cKDTree(Q).query(np.column_stack([px, py]), distance_upper_bound=CALM_MAX_M, workers=-1)[0], CALM_MAX_M)
    own = np.where(np.isin(g["nature"], _codes(natures, ROADS)) & (g["importance"] == 4), 0.6, 1.0)
    return np.round(15 * _share(ow, w, dist / CALM_MAX_M, ns) * own).astype(np.uint8)


def _ends(g: dict) -> tuple[np.ndarray, np.ndarray]:
    """Clés de nœud (x_dm << 32 | y_dm) des extrémités u et v de chaque tronçon."""
    k = (np.round(g["x"] * 10).astype(np.int64) << 32) | np.round(g["y"] * 10).astype(np.int64)
    last = np.cumsum(g["n"]) - 1
    return k[last - g["n"] + 1], k[last]


@functools.lru_cache(maxsize=512)
def _major_nodes(d: Path, key: str, natures: tuple) -> np.ndarray:
    """Nœuds (extrémités, hors ponts et tunnels) des routes d'importance CROSS_IMPORTANCE d'une dalle."""
    p = d / f"{key}.npz"
    if not p.exists():
        return np.zeros(0, np.int64)
    g = _geom(p, *split_key(key)[1:])
    sel = np.isin(g["nature"], _codes(natures, ROADS)) & np.isin(g["importance"], CROSS_IMPORTANCE) & (g["flags"] & 1 == 0)
    u, v = _ends(g)
    return np.unique(np.concatenate([u[sel], v[sel]]))


def cross(d: Path, key: str, natures) -> np.ndarray:
    """ign_cross : bit 1 = l'extrémité u touche (à niveau : même nœud) une route d'importance 1-2, bit 2 = v.
    Les routes concernées sont lues dans la dalle et ses 8 voisines (nœud de bord)."""
    d, natures = Path(d), tuple(natures)
    zone, ix, iy = split_key(key)
    pre = "" if zone == "fxx" else f"{zone}/"
    Q = np.concatenate([_major_nodes(d, f"{pre}{i}_{j}", natures) for i in (ix - 1, ix, ix + 1) for j in (iy - 1, iy, iy + 1)])
    u, v = _ends(_geom(d / f"{key}.npz", ix, iy))
    return (np.isin(u, Q) * 1 + np.isin(v, Q) * 2).astype(np.uint8)

# ---------------------------------------------------------------------------------------------- OSM

def filter_pbf(pbf: Path) -> tuple[Path, Path]:
    """Extrait .osm.pbf (Geofabrik) -> (texte OPL réduit à OSM_FILTER, ways porteurs de leurs coordonnées ;
    .pbf réduit aux forêts). Demande `osmium` (paquet osmium-tool) ; résultats gardés à côté de l'extrait,
    qui peut ensuite être supprimé (disque)."""
    out, tmp, forest = pbf.with_suffix(".opl"), pbf.with_suffix(".filtre.pbf"), pbf.with_suffix(".foret.pbf")
    if not out.exists() or out.stat().st_mtime < pbf.stat().st_mtime:
        subprocess.run(["osmium", "tags-filter", "-O", "-o", str(tmp), str(pbf), *OSM_FILTER], check=True)
        subprocess.run(["osmium", "add-locations-to-ways", "-O", "-f", "opl,add_metadata=false",
                        "-o", str(out), str(tmp)], check=True)
        tmp.unlink()
    if not forest.exists() or forest.stat().st_mtime < pbf.stat().st_mtime:
        subprocess.run(["osmium", "tags-filter", "-O", "-o", str(forest), str(pbf), *FOREST_FILTER], check=True)
    return out, forest


def _sources(paths) -> tuple[list[Path], list[Path]]:
    """Fichiers OSM -> (OPL des ways et points, sources de forêt .foret.pbf ou .geojsonseq)."""
    opl, forest = [], []
    for p in map(Path, paths):
        if p.name.endswith(".foret.pbf") or p.suffix == ".geojsonseq":
            forest.append(p)
        elif p.suffix == ".pbf":
            o, f = filter_pbf(p)
            opl.append(o)
            forest.append(f)
        else:
            opl.append(p)
    return opl, forest


_XY = re.compile(r"x(-?[0-9.]+)y(-?[0-9.]+)")
_LEVEL = {"rwn": 2, "nwn": 2, "iwn": 2}


def way_class(tags: dict) -> int:
    """Classe de voie d'un way OSM d'après `highway`, `surface` (et `tracktype`), NOCLASS si rien à en dire."""
    h, s = tags.get("highway"), tags.get("surface")
    if h is None or tags.get("footway") in ("sidewalk", "crossing") or tags.get("area") == "yes":
        return NOCLASS                  # trottoirs et passages piétons : collés à la route, pas une voie à part
    if h in TRAILS:
        return 1 if s in PAVED or (s is None and tags.get("tracktype") == "grade1") else 0
    if h in PEDESTRIAN:
        return 0 if s in NATURAL else 1
    return 0 if s in SEMI + NATURAL else NOCLASS        # route non revêtue


def _code(table, v) -> int:
    return 0 if v is None else table.index(v) if v in table else OTHER


def access_level(tags: dict) -> int:
    """Accès piéton d'un way (D66) : ACC_CLOSED si `foot=private|no`, ou `access=private|no` sans `foot`
    autorisant (yes, designated, permissive) ; ACC_RESTRICTED si `access=customers|destination|delivery|permit` ;
    0 sinon (rien de connu, ou piéton admis ; `access=agricultural|forestry` ne vise pas le piéton)."""
    a, f = tags.get("access"), tags.get("foot")
    if f in ("yes", "designated", "permissive"):
        return 0
    if f in ("private", "no") or a in ("private", "no"):
        return ACC_CLOSED
    return ACC_RESTRICTED if a in ("customers", "destination", "delivery", "permit") else 0


def _unescape(v: str) -> str:
    """Valeur de tag OPL : les caractères spéciaux y sont écrits %hex% (espace = %20%)."""
    return re.sub(r"%([0-9a-f]+)%", lambda m: chr(int(m[1], 16)), v) if "%" in v else v


def bike_level(tags: dict) -> int:
    """Accès vélo d'un way (D70) : 5 interdit (`bicycle=no|private`, ou `access`/`vehicle=no|private`), 4 interdit par
    défaut en France (autoroute, voie rapide, bridleway, via ferrata, `motorroad`, escalier sans rampe vélo), 3
    `bicycle=use_sidepath`, 2 pied à terre (`dismount`, footway, escalier à rampe), 1 restreint (destination,
    customers… ; aire piétonne), 0 admis ou rien de connu. `bicycle=yes|designated|permissive` l'emporte (0)."""
    h, b = tags.get("highway"), tags.get("bicycle")
    if h is None or b in BIKE_OK:
        return 0
    if b in ("no", "private"):
        return 5
    if b == "use_sidepath":
        return 3
    if b == "dismount":
        return 2
    acc = tags.get("vehicle") or tags.get("access")     # la clé la plus précise l'emporte (access=no + vehicle=yes : admis)
    if b in BIKE_RESTRICTED:
        return 1
    if acc in ("no", "private"):
        return 5
    if h in BIKE_BANNED or tags.get("motorroad") == "yes":
        return 4
    if h == "steps":
        return 2 if tags.get("ramp:bicycle") == "yes" else 4
    if h == "footway":
        return 2
    return int(h == "pedestrian" or acc in BIKE_RESTRICTED)


def cycleway_level(tags: dict) -> int:
    """Aménagement cyclable propre au way : 1 voie partagée, 2 bande, 3 piste latérale (max des côtés), 4 voie dédiée
    (`cycleway`, ou path|track|footway `bicycle=designated`), 5 vélorue."""
    lv = max(CYCLEWAY.get(tags.get(k), 0) for k in CYCLEWAY_KEYS)
    if tags.get("highway") == "cycleway" or (tags.get("highway") in ("path", "track", "footway")
                                             and tags.get("bicycle") == "designated"):
        lv = 4
    return 5 if "yes" in (tags.get("bicycle_road"), tags.get("cyclestreet")) else lv


def oneway_dir(tags: dict) -> int:
    """Sens unique vélo RELATIF au way (le sens du tronçon IGN est posé à l'appariement) : 0 double sens ou inconnu,
    1 vélo seulement dans le sens du way, 2 seulement à contre-sens, 3 voitures à sens unique mais vélo dans les deux
    sens (`oneway:bicycle=no`, `cycleway*=opposite*`, ou `cycleway*:oneway` = `no` / à contre-sens des voitures).
    Rond-point (`junction=roundabout|circular`) et autoroute impliquent `oneway=yes`."""
    h, o = tags.get("highway"), tags.get("oneway")
    if h is None:
        return 0
    if o is None and (tags.get("junction") in ROUNDABOUT or h in ("motorway", "motorway_link")):
        o = "yes"
    car = 1 if o in ("yes", "true", "1") else 2 if o in ("-1", "reverse") else 0
    ob = tags.get("oneway:bicycle")
    if ob == "no":
        return 3 if car else 0
    if ob in ("yes", "true", "1"):
        return 1
    if ob in ("-1", "reverse"):
        return 2
    contra = ("no", "-1" if car == 1 else "yes")        # piste ou bande à double sens, ou à contre-sens des voitures
    return 3 if car and any(tags.get(k, "").startswith("opposite") or tags.get(k + ":oneway") in contra
                            for k in CYCLEWAY_KEYS) else car


def _scale(v, top: int) -> int:
    """`mtb:scale` « 2+ » -> 3 : premier chiffre (0 à `top`) + 1 ; 0 si absent ou invalide."""
    return int(v[0]) + 1 if v and v[0] in "0123456"[:top + 1] else 0


def maxspeed(tags: dict) -> int:
    """km/h (mph converti ; FR:urban, FR:rural… ; repli `zone:maxspeed`, « FR:30 » compris) ; 0 si inconnu, none,
    signals, variable ; plafonné à 254."""
    v = _unescape(tags.get("maxspeed") or tags.get("zone:maxspeed") or "")
    if v in SPEED:
        return SPEED[v]
    m = re.fullmatch(r"(?:[A-Z]{2}:)?(\d+)(?: (mph|km/h))?", v)
    return min(254, round(int(m[1]) * (1.609 if m[2] == "mph" else 1))) if m else 0


def _way(tags: dict) -> tuple:
    """Attributs d'un way (ordre de WAY_KEYS) : highway, surface (codes), classe, rue ?, roulant, sac, visibilité,
    bits d'osm_flags, accès piéton, puis vélo : accès, aménagement, sens, mtb, mtb montée, tracktype, smoothness, vitesse."""
    h, s = tags.get("highway"), tags.get("surface")
    rough = SMOOTH.get(tags.get("smoothness"), ROUGH.get(s, 0)) if h else 0
    via = h == "via_ferrata" or "via_ferrata_scale" in tags
    cond = any("no @" in _unescape(tags.get(k + ":conditional", "")) for k in ("access", "vehicle", "bicycle", "motor_vehicle"))
    wfl = F_VIA * via | F_LIT * (tags.get("lit") == "yes") | F_STEPS * (h == "steps") \
        | F_INFORMAL * (tags.get("informal") == "yes") | F_COND * cond | F_ROUND * (tags.get("junction") in ROUNDABOUT)
    return (_code(HIGHWAYS, h), _code(SURFACES, s) if h else 0, way_class(tags),
            h is not None and h not in NOT_STREET, rough, SAC.get(tags.get("sac_scale"), 0),
            VISIBILITY.get(tags.get("trail_visibility"), 0), wfl, access_level(tags) if h else 0,
            bike_level(tags), cycleway_level(tags), oneway_dir(tags), _scale(tags.get("mtb:scale"), 6),
            _scale(tags.get("mtb:scale:uphill"), 5), TRACKTYPE.get(tags.get("tracktype"), 0) if h else 0,
            SMOOTHNESS.get(tags.get("smoothness"), 0) if h else 0, maxspeed(tags) if h else 0)


WAY_KEYS = ("highway", "surface", "cls", "street", "rough", "sac", "vis", "wfl", "acc",
            "bike", "cyc", "dir", "mtb", "mtbu", "trk", "smo", "spd")


def _spread(routes: dict, subs: dict, rel_ways: dict) -> dict:
    """way -> niveau le plus haut d'une relation de route qui le contient, directement ou par une sur-relation (une
    véloroute ou un GR est souvent une relation de relations, à plusieurs étages). Niveaux décroissants : une relation
    déjà atteinte à un niveau égal ou supérieur n'est pas reparcourue, ce qui coupe aussi les cycles."""
    eff = {}
    for rid, lv in sorted(routes.items(), key=lambda kv: -kv[1]):
        stack = [rid]
        while stack:
            r = stack.pop()
            if eff.get(r, 0) < lv:
                eff[r] = lv
                stack += subs.get(r, ())
    ways = {}
    for r, lv in eff.items():
        for i in rel_ways.get(r, ()):
            ways[i] = max(ways.get(i, 0), lv)
    return ways


def read_osm(paths) -> dict:
    """Ways et points utiles des fichiers OPL : lon, lat concaténés (float32, ~0,5 m), n sommets par way,
    `hike` (0, 1 balisé, 2 régional ou plus), `broute` (réseau vélo 0–4), `mroute` (réseau VTT 0–2), `water` (0, 1 rivière, canal, côte ou contour de multipolygone
    d'eau, 3 plan d'eau d'un seul tenant, à trier par surface), attributs `_way` ; points `p_lon`, `p_lat`,
    `p_kind` (F_DRINK eau potable, F_VIEW sommet, col, point de vue)."""
    lon, lat, n, wid, water = array("f"), array("f"), array("i"), array("q"), array("b")
    att = {k: array("B") for k in WAY_KEYS}
    plon, plat, pkind = array("d"), array("d"), array("B")
    routes = {t: {} for t in ROUTE_TYPE.values()}      # type -> relation -> niveau de réseau
    lake, rel_ways, subs = set(), {}, {}                # ways de contour d'eau ; relation -> ways, -> sous-relations
    for path in paths:
        with open(path, encoding="utf-8") as f:
            for line in f:
                kind = line[0]
                if kind not in "nwr":
                    continue
                ident = int(line[1:line.index(" ")])
                fld = {p[0]: p[1:] for p in line.rstrip("\n").split(" ")[1:] if p}
                tags = dict(kv.split("=", 1) for kv in fld.get("T", "").split(",") if "=" in kv)
                if kind == "n":
                    k = F_DRINK * ((tags.get("amenity") == "drinking_water" and tags.get("drinking_water") != "no")
                                   or tags.get("drinking_water") == "yes") \
                        | F_VIEW * (tags.get("natural") in ("peak", "saddle") or tags.get("mountain_pass") == "yes"
                                    or tags.get("tourism") == "viewpoint")
                    if k and fld.get("x") and fld.get("y"):
                        plon.append(float(fld["x"]))
                        plat.append(float(fld["y"]))
                        pkind.append(k)
                    continue
                if kind == "w":
                    xy = _XY.findall(fld.get("N", ""))
                    if len(xy) < 2:
                        continue
                    c = 0
                    if tags.get("waterway") in ("river", "canal"):      # pas `stream` : en montagne tout sentier en croise
                        c = int(tags.get("intermittent") != "yes" and tags.get("tunnel", "no") == "no")
                    elif tags.get("natural") == "coastline":
                        c = 1
                    elif tags.get("natural") == "water" and tags.get("intermittent") != "yes" \
                            and tags.get("water") not in ("basin", "wastewater"):
                        c = 3
                    wid.append(ident)
                    water.append(c)
                    for k, v in zip(WAY_KEYS, _way(tags)):
                        att[k].append(v)
                    n.append(len(xy))
                    lon.extend(float(a) for a, _ in xy)
                    lat.extend(float(b) for _, b in xy)
                    continue
                mem = [m.split("@")[0] for m in fld.get("M", "").split(",")]
                rel_ways[ident] = ways = [int(m[1:]) for m in mem if m[:1] == "w"]
                if any(m[:1] == "r" for m in mem):
                    subs[ident] = [int(m[1:]) for m in mem if m[:1] == "r"]
                if tags.get("route") in ROUTE_TYPE:
                    net = tags.get("network")
                    t = ROUTE_TYPE[tags["route"]]
                    routes[t][ident] = _LEVEL.get(net, 1) if t == "hike" else BIKE_NETWORK.get(net, 1) if t == "broute" \
                        else 2 if net in ("rcn", "ncn", "icn") else 1
                elif tags.get("natural") == "water" and tags.get("intermittent") != "yes":
                    lake.update(ways)
    wid_a, wat = np.array(wid, np.int64), np.array(water, np.uint8)
    net = {t: _spread(r, subs, rel_ways) for t, r in routes.items()}
    net = {t: np.array([m.get(i, 0) for i in wid], np.uint8) for t, m in net.items()}
    wat[(wat == 0) & np.isin(wid_a, np.fromiter(lake, np.int64, len(lake)))] = 1
    return dict(lon=np.frombuffer(lon, np.float32), lat=np.frombuffer(lat, np.float32), n=np.array(n, np.int64),
                hike=net["hike"], broute=net["broute"], mroute=net["mroute"], water=wat, **{k: np.array(v, np.uint8) for k, v in att.items()},
                p_lon=np.array(plon), p_lat=np.array(plat), p_kind=np.array(pkind, np.uint8))


def _project(osm: dict, zone: str) -> dict:
    """Ways et points de la zone dans son repère métrique, avec l'emprise des ways ; petits plans d'eau écartés."""
    n = osm["n"]
    first = np.cumsum(n) - n
    w, s, e, nn = ZONES[zone].get("bbox", METRO)
    lo, la = osm["lon"][first], osm["lat"][first]
    keep = (lo >= w) & (lo <= e) & (la >= s) & (la <= nn)
    pts = np.repeat(keep, n)
    n = n[keep]
    # float32 (~0,1 m en Lambert-93) et par morceaux : la France entière fait ~170 M de sommets
    lon, lat, t = osm["lon"][pts], osm["lat"][pts], transformers(zone)[1]
    x, y = np.empty(len(lon), np.float32), np.empty(len(lon), np.float32)
    for i in range(0, len(lon), 10_000_000):
        x[i:i + 10_000_000], y[i:i + 10_000_000] = t.transform(lon[i:i + 10_000_000].astype(np.float64),
                                                               lat[i:i + 10_000_000].astype(np.float64))
    att = {k: osm[k][keep] for k in ("hike", "broute", "mroute") + WAY_KEYS}
    water = osm["water"][keep].copy()
    if len(n):
        lake = water == 3
        if lake.any():      # surface du contour fermé (lacet), en float64 relatif au 1er sommet
            lx, ly, ln = _pick(dict(x=x, y=y, n=n), lake)
            owner = np.repeat(np.arange(len(ln)), ln)
            f0 = np.repeat(np.cumsum(ln) - ln, ln)
            fx, fy = lx.astype(np.float64) - lx[f0], ly.astype(np.float64) - ly[f0]
            cross_ = fx[:-1] * fy[1:] - fx[1:] * fy[:-1]
            cross_[np.cumsum(ln)[:-1] - 1] = 0.0
            area = np.abs(np.bincount(owner[:-1], weights=cross_, minlength=len(ln))) / 2
            water[np.flatnonzero(lake)[area < WATER_MIN_AREA_M2]] = 0
            water[water == 3] = 1
        ends = np.cumsum(n)
        box = [f.reduceat(v, ends - n) for v in (x, y) for f in (np.minimum, np.maximum)]
    else:
        box = [np.zeros(0)] * 4
    pk = (osm["p_lon"] >= w) & (osm["p_lon"] <= e) & (osm["p_lat"] >= s) & (osm["p_lat"] <= nn)
    px, py = transformers(zone)[1].transform(osm["p_lon"][pk], osm["p_lat"][pk])
    return dict(x=x, y=y, n=n, water=water, box=box, **att,
                P=np.column_stack([np.asarray(px), np.asarray(py)]).reshape(-1, 2), p_kind=osm["p_kind"][pk])


def _pick(W: dict, sel) -> tuple:
    pts = np.repeat(sel, W["n"])
    return W["x"][pts], W["y"][pts], W["n"][sel]


def _matched(g: dict, W: dict, ww, sel, value):
    """Tronçons `sel` : points tous les 5 m, ways OSM `ww` à 15 m de même direction (|cos| ≥ 0,8, rangés du
    plus proche au plus loin) ; renvoie (valeur `value` du way par voisin et par point, 0 sans way, voisin
    valide ?, rang du tronçon, longueur du pas, way dans le sens du tronçon (u→v) ?)."""
    ox, oy, oux, ouy, oo, _ = _points(*_pick(W, ww), 5.0)
    px, py, ux, uy, ow, w = _points(g["x"], g["y"], g["n"], 5.0, sel)
    D, J = cKDTree(np.column_stack([ox, oy])).query(np.column_stack([px, py]), k=min(6, len(ox)),
                                                    distance_upper_bound=HIKE_BUF_M, workers=-1)
    D, J = D.reshape(len(px), -1), J.reshape(len(px), -1)
    Jc = np.where(np.isfinite(D), J, 0)
    dot = oux[Jc] * ux[:, None] + ouy[Jc] * uy[:, None]
    ok = np.isfinite(D) & (np.abs(dot) >= HIKE_COS)
    return np.where(ok, value[ww][oo[Jc]], 0), ok, ow, w, dot > 0


def _majority(code, ow, w, ns) -> np.ndarray:
    """Code (> 0) le plus long par tronçon si les points à code > 0 font au moins HIKE_FRAC de sa longueur, 0 sinon."""
    out = np.zeros(ns, np.int64)
    m = code > 0
    if not m.any():
        return out
    k, inv = np.unique(ow[m] * 256 + code[m], return_inverse=True)
    s = np.bincount(inv, weights=w[m])
    o = np.lexsort((-s, k >> 8))
    first = o[np.r_[True, (k[o][1:] >> 8) != (k[o][:-1] >> 8)]]
    tot = np.bincount(ow, weights=w, minlength=ns)
    got = np.bincount(ow[m], weights=w[m], minlength=ns)
    t = k[first] >> 8
    keep = got[t] >= HIKE_FRAC * tot[t]
    out[t[keep]] = k[first][keep] & 255
    return out


def _spot(level, ow, w, ns, metres=SPOT_M) -> np.ndarray:
    """Plus haut niveau atteint sur au moins `metres` (ou la moitié du tronçon) de longueur appariée."""
    out = np.zeros(ns, np.uint8)
    need = np.minimum(metres, 0.5 * np.bincount(ow, weights=w, minlength=ns))
    for lv in range(1, int(level.max(initial=0)) + 1):
        out[np.bincount(ow, weights=w * (level >= lv), minlength=ns) >= np.maximum(need, 1e-9)] = lv
    return out


def osm_ways(g: dict, W: dict, natures, near) -> dict:
    """Colonnes tirées du way OSM apparié le plus proche (voir _matched) : chemins IGN (PATHS) avec tous les
    ways `highway`, routes IGN (ROADS) avec les seuls ways de type rue (pas un trottoir ni une piste cyclable
    voisine). Autres natures : 0 (osm_class : NOCLASS). Vélo : `osm_bike`, `osm_mtb*` et les bits d'escalier / informal
    sont de la règle « pire sur ≥ 20 m » (escalier : 10 m), le reste est majoritaire ; `osm_oneway` est orienté
    (sens du way contre sens du tronçon) ; une route IGN reçoit 3 à `osm_cycleway` si un way cyclable lui est parallèle."""
    ns = len(g["n"])
    out = {k: np.zeros(ns, np.uint8) for k in ("osm_highway", "osm_surface", "osm_rough", "osm_sac", "osm_visibility",
                                               "osm_flags", "osm_access", "osm_bike", "osm_cycleway", "osm_oneway",
                                               "osm_mtb", "osm_mtb_up", "osm_tracktype", "osm_smooth", "osm_maxspeed")}
    out["osm_class"] = np.full(ns, NOCLASS, np.uint8)
    if not ns or not len(W["n"]):
        return out
    nat = list(natures)
    idx = np.arange(1, len(W["n"]) + 1)
    for sel, ww, trail in ((np.isin(g["nature"], _codes(nat, PATHS)), near & (W["highway"] > 0), True),
                           (np.isin(g["nature"], _codes(nat, ROADS)), near & W["street"].astype(bool), False)):
        if not (sel.any() and ww.any()):
            continue
        val, ok, ow, w, same = _matched(g, W, ww, sel, idx)
        first = ok.argmax(axis=1)
        j = val[np.arange(len(val)), first] - 1                  # way apparié le plus proche, -1 sans
        a = {k: np.where(j >= 0, W[k][j], 0).astype(np.int64) for k in WAY_KEYS}
        for col, k in (("osm_highway", "highway"), ("osm_surface", "surface"), ("osm_rough", "rough"),
                       ("osm_cycleway", "cyc"), ("osm_tracktype", "trk"), ("osm_smooth", "smo"), ("osm_maxspeed", "spd")):
            out[col][sel] = _majority(a[k], ow, w, ns)[sel]
        c = _majority(np.where((j >= 0) & (a["cls"] != NOCLASS), a["cls"] + 1, 0), ow, w, ns)
        out["osm_class"][sel & (c > 0)] = (c - 1)[sel & (c > 0)]
        for col, k in (("osm_sac", "sac"), ("osm_visibility", "vis"), ("osm_access", "acc"), ("osm_bike", "bike")):
            out[col][sel] = _spot(a[k], ow, w, ns)[sel]               # le plus fermé / difficile sur ≥ 20 m appariés
        d, fwd = a["dir"], same[np.arange(len(same)), first]
        out["osm_oneway"][sel] = _majority(np.where(d == 3, 3, np.where(d == 0, 0, 1 + ((d == 1) != fwd))), ow, w, ns)[sel]
        lit = _share(ow, w, a["wfl"] & F_LIT > 0, ns) >= HIKE_FRAC
        bits = lambda bit, m=SPOT_M: _spot((a["wfl"] & bit) > 0, ow, w, ns, m) > 0
        f = F_VIA * bits(F_VIA) | F_LIT * lit | F_COND * bits(F_COND) | F_ROUND * bits(F_ROUND)
        if trail:
            out["osm_mtb"][sel] = _spot(a["mtb"], ow, w, ns)[sel]
            out["osm_mtb_up"][sel] = _spot(a["mtbu"], ow, w, ns)[sel]
            f = f | F_STEPS * bits(F_STEPS, STEPS_M) | F_INFORMAL * bits(F_INFORMAL)
        else:
            cw = near & (W["cyc"] == 4)                              # piste latérale : way cyclable à ≤ 15 m, ≥ 60 %
            if cw.any():
                _, ok2, ow2, w2, _ = _matched(g, W, cw, sel, idx)
                lat = _share(ow2, w2, ok2.any(axis=1), ns) >= HIKE_FRAC
                out["osm_cycleway"][sel] = np.maximum(out["osm_cycleway"], 3 * lat)[sel]
        out["osm_flags"][sel] |= f[sel].astype(np.uint8)
    return out


def osm_labels(g: dict, W: dict, natures) -> dict:
    """Colonnes OSM (hors forêt) des tronçons `g` d'une dalle, d'après les ways et points OSM projetés `W`."""
    ns = len(g["n"])
    net = {c: np.zeros(ns, np.uint8) for c in ("osm_hike", "osm_bike_route", "osm_mtb_route")}
    water = np.zeros(ns, np.uint8)
    if not ns:
        return dict(osm_ways(g, W, natures, None), osm_water=water, **net)
    m = max(WATER_M, DRINK_M) + 10
    near = (W["box"][1] >= g["x"].min() - m) & (W["box"][0] <= g["x"].max() + m) \
        & (W["box"][3] >= g["y"].min() - m) & (W["box"][2] <= g["y"].max() + m)
    out = osm_ways(g, W, natures, near)
    # Réseaux : points tous les 5 m, au moins un way membre à 15 m de même direction. Balisage de randonnée : chemins
    # seulement ; véloroutes et itinéraires VTT passent aussi sur route.
    nat = list(natures)
    path, road = np.isin(g["nature"], _codes(nat, PATHS)), np.isin(g["nature"], _codes(nat, ROADS))
    for col, key, top, sel in (("osm_hike", "hike", 2, path), ("osm_bike_route", "broute", 4, path | road),
                               ("osm_mtb_route", "mroute", 2, path | road)):
        hw = near & (W[key] > 0)
        if hw.any() and sel.any():
            lv, _, ow, w, _ = _matched(g, W, hw, sel, W[key])
            lv = lv.max(axis=1)
            for k in range(1, top + 1):
                net[col][_share(ow, w, lv >= k, ns) >= HIKE_FRAC] = k
    px, py, _, _, ow, w = _points(g["x"], g["y"], g["n"], 10.0)
    Q = np.column_stack([px, py])
    # Eau : part de la longueur à moins de 50 m d'une rivière, d'un canal, d'un plan d'eau ou de la côte.
    ww = near & (W["water"] == 1)
    if ww.any():
        ox, oy, *_ = _points(*_pick(W, ww), 10.0)
        D, _ = cKDTree(np.column_stack([ox, oy])).query(Q, distance_upper_bound=WATER_M, workers=-1)
        water = np.round(15 * _share(ow, w, np.isfinite(D), ns)).astype(np.uint8)
    # Points : eau potable à 100 m, sommet / col / point de vue à 50 m d'un point du tronçon.
    for bit, r in ((F_DRINK, DRINK_M), (F_VIEW, VIEW_M)):
        P = W["P"][(W["p_kind"] & bit) > 0]
        if len(P):
            D, _ = cKDTree(P).query(Q, distance_upper_bound=r, workers=-1)
            out["osm_flags"][np.bincount(ow, weights=np.isfinite(D), minlength=ns) > 0] |= bit
    return dict(out, osm_water=water, **net)


def _features(paths):
    """Polygones GeoJSON des sources de forêt (.foret.pbf via `osmium export`, ou .geojsonseq)."""
    for p in paths:
        if p.suffix == ".geojsonseq":
            f = open(p, encoding="utf-8")
        else:
            proc = subprocess.Popen(["osmium", "export", "-f", "geojsonseq", "-x", "print_record_separator=false",
                                     "--geometry-types=polygon", "-o", "-", str(p)], stdout=subprocess.PIPE, text=True)
            f = proc.stdout
        with f:
            for line in f:
                line = line.strip("\x1e \n")
                if line:
                    yield json.loads(line)["geometry"]


def _zone_of(lon: float, lat: float) -> str:
    return next((z for z, v in ZONES.items() if "bbox" in v and v["bbox"][0] <= lon <= v["bbox"][2]
                 and v["bbox"][1] <= lat <= v["bbox"][3]), "fxx")


def forest_rasters(paths, keys) -> dict:
    """Raster (FOREST_PX, bits tassés par ligne, nord en haut) du couvert forestier OSM (landuse=forest,
    natural=wood) de chaque dalle `keys`."""
    import shapely
    from rasterio.features import rasterize
    from rasterio.transform import Affine
    from shapely.geometry import shape
    want, side, R, buf = set(keys), int(TILE_M / FOREST_PX), {}, {}

    def flush(k):
        _, ix, iy = split_key(k)
        a = rasterize(buf.pop(k), out_shape=(side, side), dtype=np.uint8,
                      transform=Affine(FOREST_PX, 0, ix * TILE_M, 0, -FOREST_PX, (iy + 1) * TILE_M))
        R[k] = R[k] | np.packbits(a, axis=1) if k in R else np.packbits(a, axis=1)

    for geom in _features(paths):
        g = shape(geom)
        if g.is_empty:
            continue
        c = shapely.get_coordinates(g)
        zone = _zone_of(*c[0])
        g = shapely.transform(g, lambda c, t=transformers(zone)[1]: np.column_stack(t.transform(c[:, 0], c[:, 1])))
        x0, y0, x1, y1 = g.bounds
        pre = "" if zone == "fxx" else f"{zone}/"
        for i in range(int(x0 // TILE_M), int(x1 // TILE_M) + 1):
            for j in range(int(y0 // TILE_M), int(y1 // TILE_M) + 1):
                k = f"{pre}{i}_{j}"
                if k in want:
                    buf.setdefault(k, []).append(g)
                    if len(buf[k]) >= 2000:
                        flush(k)
    for k in list(buf):
        flush(k)
    return R


def forest(g: dict, R: dict, zone: str) -> np.ndarray:
    """osm_forest : round(15 × part de la longueur sous couvert forestier), points tous les 10 m."""
    ns = len(g["n"])
    px, py, _, _, ow, w = _points(g["x"], g["y"], g["n"], 10.0)
    inside = np.zeros(len(px), bool)
    i, j = (px // TILE_M).astype(np.int64), (py // TILE_M).astype(np.int64)
    pre = "" if zone == "fxx" else f"{zone}/"
    side = int(TILE_M / FOREST_PX)
    for a, b in set(zip(i.tolist(), j.tolist())):
        r = R.get(f"{pre}{a}_{b}")
        if r is None:
            continue
        s = (i == a) & (j == b)
        row = np.clip(((b + 1) * TILE_M - py[s]) // FOREST_PX, 0, side - 1).astype(np.int64)
        col = np.clip((px[s] - a * TILE_M) // FOREST_PX, 0, side - 1).astype(np.int64)
        inside[s] = (r[row, col >> 3] >> (7 - (col & 7))) & 1
    return np.round(15 * _share(ow, w, inside, ns)).astype(np.uint8)


def pn_cores(path) -> dict:
    """Cœurs terrestres de parc national du GeoJSON des contours des 11 parcs (champ `code_r_enp` = PN_CORE), projetés
    dans le repère métrique de leur zone et préparés : zone -> polygones."""
    import shapely
    from shapely.geometry import shape
    out = {}
    for ft in json.loads(Path(path).read_text(encoding="utf-8"))["features"]:
        if ft["properties"].get("code_r_enp") != PN_CORE:
            continue
        g = shape(ft["geometry"])
        zone = _zone_of(*shapely.get_coordinates(g)[0])
        g = shapely.transform(g, lambda c, t=transformers(zone)[1]: np.column_stack(t.transform(c[:, 0], c[:, 1])))
        shapely.prepare(g)
        out.setdefault(zone, []).append(g)
    return out


def pn_core(g: dict, cores: list) -> np.ndarray:
    """pn_core = 1 si au moins la moitié de la longueur du tronçon (points tous les 10 m) est dans un cœur de parc national."""
    import shapely
    px, py, _, _, ow, w = _points(g["x"], g["y"], g["n"], 10.0)
    inside = np.zeros(len(px), bool)
    for poly in cores:
        x0, y0, x1, y1 = poly.bounds
        if len(px) and px.max() >= x0 and px.min() <= x1 and py.max() >= y0 and py.min() <= y1:
            inside |= shapely.contains_xy(poly, px, py)
    return (_share(ow, w, inside, len(g["n"])) >= 0.5).astype(np.uint8)


# ----------------------------------------------------------------------------------------- dossier

STATS = ("path_km", "osm_hike_km", "osm_water_km", "osm_mid_km", "osm_forest_km", "osm_closed_path_km",
         "osm_closed_road_km", "osm_matched_km", "osm_bike_closed_path_km", "osm_bike_closed_road_km",
         "osm_cycleway_km", "osm_oneway_km", "osm_oneway3_km", "osm_mtb_km", "osm_mtb_route_km", "osm_maxspeed_km",
         "osm_steps_km", "pn_core_km") + tuple(f"osm_bike{c}_km" for c in range(1, 6)) + tuple(f"osm_bike_route{c}_km" for c in range(1, 5))


def bike_stats(T: dict, km, path, road) -> dict:
    """Km du manifeste pour les colonnes vélo : part appariée, accès par code, aménagement, sens uniques, réseaux, escaliers."""
    def k(mask):
        return round(float(km[mask].sum()), 1)
    return dict({f"osm_bike{c}_km": k(T["osm_bike"] == c) for c in range(1, 6)},
                **{f"osm_bike_route{c}_km": k(T["osm_bike_route"] == c) for c in range(1, 5)},
                osm_matched_km=k(T["osm_highway"] > 0), osm_cycleway_km=k(T["osm_cycleway"] > 0),
                osm_bike_closed_path_km=k(path & (T["osm_bike"] >= 4)), osm_bike_closed_road_km=k(road & (T["osm_bike"] >= 4)),
                osm_oneway_km=k((T["osm_oneway"] == 1) | (T["osm_oneway"] == 2)), osm_oneway3_km=k(T["osm_oneway"] == 3),
                osm_mtb_km=k(T["osm_mtb"] > 0), osm_mtb_route_km=k(T["osm_mtb_route"] > 0),
                osm_maxspeed_km=k(T["osm_maxspeed"] > 0), osm_steps_km=k(T["osm_flags"] & F_STEPS > 0))


def derived_version(v: str) -> str:
    """'bdtopo-wfs-2026-10e' -> '…-10e.1' ; '…-10e.1' -> '…-10e.2'."""
    base, _, k = v.partition(".")
    return f"{base}.{int(k or 0) + 1}"


def enrich(tiles_dir, osm_paths=(), version: str | None = None, log=print, pn=None) -> dict:
    d = Path(tiles_dir)
    m = json.loads((d / "manifest.json").read_text())
    if m.get("format") != FORMAT:
        raise RuntimeError(f"format {m.get('format')}, attendu {FORMAT}")
    natures = m["natures"]
    _major.cache_clear()
    _major_nodes.cache_clear()
    t0 = time.time()
    opl, forest_src = _sources(osm_paths)
    osm = read_osm(opl) if opl else None
    if osm:
        log(f"OSM : {len(osm['n'])} ways, {len(osm['lon'])} sommets, {int((osm['hike'] > 0).sum())} balisés, "
            f"{int((osm['broute'] > 0).sum())} véloroutes, {int((osm['mroute'] > 0).sum())} VTT, "
            f"{int((osm['water'] > 0).sum())} d'eau, {len(osm['p_kind'])} points, lus en {time.time() - t0:.0f} s")
    cores = pn_cores(pn) if pn else None
    R = forest_rasters(forest_src, m["tiles"]) if forest_src else None
    if R is not None:
        log(f"forêt : {len(R)} dalles avec du couvert, {time.time() - t0:.0f} s")
    W = {}
    cols = ["calm", "ign_cross"] + ([c for c in COLUMNS if c.startswith("osm_") and c != "osm_forest"] if osm else []) \
        + (["osm_forest"] if R is not None else []) + (["pn_core"] if cores is not None else [])
    for key, info in sorted(m["tiles"].items()):
        t = time.time()
        zone, ix, iy = split_key(key)
        p = d / f"{key}.npz"
        with np.load(p) as f:
            T = {k: f[k] for k in f.files if k not in ADDED}
        T["calm"] = calm(d, key, natures)
        T["ign_cross"] = cross(d, key, natures)
        g = _geom(p, ix, iy)
        if osm:
            if zone not in W:
                W[zone] = _project(osm, zone)
            T.update(osm_labels(g, W[zone], natures))
        if R is not None:
            T["osm_forest"] = forest(g, R, zone)
        if cores is not None:
            T["pn_core"] = pn_core(g, cores.get(zone, ()))
        buf = io.BytesIO()
        np.savez_compressed(buf, **T)
        data = buf.getvalue()
        cache.write_atomic(p, data)
        km = T["len_dm"] / 1e4
        info.update(sha256=hashlib.sha256(data).hexdigest(), bytes=len(data),
                    o_par_troncon=round(len(data) / max(1, info["n"]), 1),
                    calm_km=round(float((km * T["calm"]).sum() / 15), 1))
        for k in STATS:
            info.pop(k, None)
        if R is not None:
            info["osm_forest_km"] = round(float((km * T["osm_forest"]).sum() / 15), 1)
        if cores is not None:
            info["pn_core_km"] = round(float(km[T["pn_core"] > 0].sum()), 1)
        if osm:
            path = np.isin(T["nature"], _codes(natures, PATHS))
            info.update(path_km=round(float(km[path].sum()), 1), osm_hike_km=round(float(km[T["osm_hike"] > 0].sum()), 1),
                        osm_water_km=round(float((km * T["osm_water"]).sum() / 15), 1),
                        osm_mid_km=round(float(km[T["osm_class"] == 1].sum()), 1),
                        osm_closed_path_km=round(float(km[path & (T["osm_access"] == ACC_CLOSED)].sum()), 1),
                        osm_closed_road_km=round(float(km[np.isin(T["nature"], _codes(natures, ROADS))
                                                          & (T["osm_access"] == ACC_CLOSED)].sum()), 1),
                        **bike_stats(T, km, path, np.isin(T["nature"], _codes(natures, ROADS))))
        log(f"{key} : {info['n']} tronçons, {len(data)} octets, {time.time() - t:.1f} s")
    m["derived_from"] = m.get("derived_from") or m["data_version"]
    m["data_version"] = version or derived_version(m["data_version"])
    m["columns"] = {k: COLUMNS[k] for k in cols}
    m["enriched"] = dict(date=datetime.date.today().isoformat(), osm=sorted(Path(p).name for p in osm_paths),
                         **({"pn": Path(pn).name} if pn else {}))
    m["totals"] = totals(m["tiles"])
    for k in ("calm_km",) + STATS:
        if any(k in t for t in m["tiles"].values()):
            m["totals"][k] = round(sum(t.get(k, 0) for t in m["tiles"].values()), 1)
    cache.write_atomic(d / "manifest.json", json.dumps(m, ensure_ascii=False, indent=1).encode())
    rss = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / (1e6 if sys.platform == "darwin" else 1e3)
    log(f"{len(m['tiles'])} dalles, version {m['data_version']} (depuis {m['derived_from']}), {time.time() - t0:.0f} s, "
        f"mémoire max {rss:.0f} Mo (hors osmium)")
    return m


def verify(tiles_dir, m: dict, skip=()) -> list[str]:
    """Contrôle des colonnes annoncées par `columns` : présentes, de la taille de la dalle, dans leurs
    bornes ; `calm` recalculé depuis le dossier = `calm` stocké (une dalle étiquetée sans ses voisines,
    ou des voisines changées depuis, donnent un écart près des bords)."""
    d, bad = Path(tiles_dir), []
    _major.cache_clear()
    cols = m.get("columns") or {}
    if not cols:
        return bad                      # dossier non enrichi : rien à contrôler
    for key in sorted(k for k in m.get("tiles", {}) if k not in skip):
        with np.load(d / f"{key}.npz") as f:
            n = len(f["len_dm"])
            got = {k: f[k] for k in cols if k in f.files}
            path = np.isin(f["nature"], _codes(m["natures"], PATHS))
            road = np.isin(f["nature"], _codes(m["natures"], ROADS))
            km = f["len_dm"] / 1e4
        for k, c in cols.items():
            a = got.get(k)
            if a is None or len(a) != n or str(a.dtype) != c["dtype"]:
                bad.append(f"{key} : colonne {k} absente, de mauvaise taille ou de mauvais type")
            elif n and (int(a.max()) > c["range"][1] or not np.isin(a, c.get("values", a)).all()):
                bad.append(f"{key} : {k} = {int(a.max())} hors bornes {c['range']}")
        if "osm_hike" in got and len(got["osm_hike"]) == n and got["osm_hike"][~path].any():
            bad.append(f"{key} : osm_hike posé hors chemin")
        if "osm_class" in got and len(got["osm_class"]) == n and \
                (got["osm_class"][~(path | road)] != NOCLASS).any():
            bad.append(f"{key} : osm_class posé hors chemins et routes")
        if all(len(got.get(k, ())) == n for k in ("osm_mtb", "osm_mtb_up", "osm_flags")):
            if got["osm_mtb"][~path].any() or got["osm_mtb_up"][~path].any() or (got["osm_flags"][~path] & F_INFORMAL).any():
                bad.append(f"{key} : osm_mtb, osm_mtb_up ou bit informal posé hors chemin")
        if n > 1000 and "osm_highway" in got and len(got["osm_highway"]) == n:      # avertissements : règle cassée ?
            if not got["osm_highway"].any():
                print(f"avertissement {key} : aucun tronçon apparié à OSM sur {n} tronçons", file=sys.stderr)
            if "osm_bike" in got and len(got["osm_bike"]) == n and km[got["osm_bike"] >= 4].sum() > 0.5 * km.sum():
                print(f"avertissement {key} : osm_bike >= 4 sur plus de la moitié des km", file=sys.stderr)
        if "calm" in got and len(got["calm"]) == n:
            dif = int((calm(d, key, m["natures"]) != got["calm"]).sum())
            if dif:
                bad.append(f"{key} : calm différent du recalcul sur {dif} tronçons (dalles voisines absentes ou changées ?)")
    return bad
