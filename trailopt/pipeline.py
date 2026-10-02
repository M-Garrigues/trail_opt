"""Chaîne complète hors Streamlit : zone -> OSM -> graphe -> altitude -> solveur -> GPX."""
from __future__ import annotations

import math
import resource
import sys
import time
from dataclasses import dataclass, field

import numpy as np
import shapely
from shapely.geometry import Point, Polygon

from . import cache, elevation, graph, ign, osm
from .geo import LocalFrame
from .gpx import to_gpx
from .solvers import EXACT_MAX_EDGES, optimize

# Plafonds anti-abus (app publique, Streamlit Community Cloud : 2 cœurs, 2,7 Go).
DIST_KM = (2.0, 100.0)
MAX_AREA_KM2 = 1965.0     # disque de 25 km de rayon
# La zone est en plus réduite (disque plus petit autour du départ) tant qu'elle contient trop
# de voies : au-delà, le chargement prend des minutes, la mémoire explose et le recuit se
# noie dans le graphe. Mesuré à Massy : 414 000 tronçons IGN sur 25 km de rayon = 5 min 30,
# 2,0 Go, et un D+ plus faible qu'avec une zone de 7 km de rayon.
MAX_WAYS = {"ign": 110_000, "osm": 130_000}
# Source IGN en mode max : les tronçons arrivent en tableaux compacts et le graphe est réduit
# avant l'altitude fine ; un rayon de 25 km en zone dense reste traitable.
MAX_WAYS_IGN_MAX = 650_000
FALLBACK_AREA_KM2 = 150.0  # plafond d'aire si le comptage des voies est indisponible
LONG_KM = 25.0            # au-delà : avertissement (chargement et calcul plus longs)


def suggested_time(distance_km: float) -> float:
    """Budget solveur conseillé selon la distance : 20 s jusqu'à 10 km, puis une montée
    régulière jusqu'à 60 s pour 100 km, arrondie à 5 s. Les mesures ne justifient pas plus :
    sur 100 km à Massy, 60 s de recuit par faces ne donnent qu'environ 1 % de D+ de plus que
    20 s, et 5 s en donnent déjà 96 %. Le budget reste réglable jusqu'à TIME_S[1]."""
    t = 20.0 + max(0.0, distance_km - 10.0) * 40.0 / 90.0
    return float(min(TIME_S[1], max(20.0, 5.0 * round(t / 5.0))))
TIME_S = (5.0, 180.0)
START_BUFFER_M = 50.0
OSM_MARGIN_M = 200.0


class UserError(ValueError):
    """Entrée refusée : message affichable tel quel."""


class Cancelled(UserError):
    """Calcul annulé par l'utilisateur."""


@dataclass
class Params:
    lat: float
    lon: float
    distance_km: float
    polygon: list | None = None       # [(lon, lat), ...] ; None = disque centré sur le départ
    mode: str = "max"                 # "max" | "target"
    target_dplus: float | None = None
    max_grade: float | None = None    # fraction (0.35 = 35 %)
    time_s: float | None = None       # None : budget conseillé selon la distance
    tol: float = 0.05
    roads: str = "minor"              # "unpaved" | "pedestrian" | "minor" | "all"
    seed: int = 0
    solver: str = "auto"
    exact_max_edges: int = EXACT_MAX_EDGES
    workers: int = 2
    node_simple: bool = False         # ne jamais repasser par un carrefour (hors 200 m du départ)
    enforce_limits: bool = True
    source: str = "ign"               # "ign" (BD TOPO) | "osm" ; "pedestrian" force "osm"


@dataclass
class LoopResult:
    lon: np.ndarray
    lat: np.ndarray
    ele: np.ndarray
    dist: np.ndarray                  # abscisse (m)
    length: float
    dplus: float
    method: str
    feasible: bool
    gpx: str
    region_lonlat: list               # anneaux [(lon, lat), ...] de la zone effective
    warnings: list = field(default_factory=list)
    debug: dict = field(default_factory=dict)


def peak_rss_mb() -> float:
    r = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    return r / 2**20 if sys.platform == "darwin" else r / 1024


def default_radius(distance_m: float) -> float:
    """Rayon du disque par défaut : D/2, plafonné pour respecter MAX_AREA_KM2 (marge 1 %)."""
    return min(distance_m / 2, math.sqrt(0.99 * MAX_AREA_KM2 * 1e6 / math.pi))


def _rings(poly, frame):
    geoms = getattr(poly, "geoms", [poly])
    out = []
    for gm in geoms:
        if not hasattr(gm, "exterior"):
            continue
        lon, lat = frame.to_wgs(np.asarray(gm.exterior.coords))
        out.append(list(zip(lon, lat)))
    return out


def build_region(p: Params, frame: LocalFrame, Lmax: float):
    """Polygone local effectif, coupé au disque de portée Lmax/2 autour du départ."""
    if p.polygon:
        poly = Polygon(frame.to_local(*zip(*p.polygon)))
        if not poly.is_valid:
            poly = shapely.make_valid(poly).buffer(0)
        if poly.is_empty or poly.area <= 0:
            raise UserError("Zone invalide : dessine un polygone non dégénéré.")
        if not poly.buffer(START_BUFFER_M).contains(Point(0, 0)):
            raise UserError("Le départ doit être dans la zone dessinée (tolérance 50 m).")
    else:
        poly = graph.disk(default_radius(p.distance_km * 1000))
    return graph.clip_to_reach(poly, Lmax)


def validate(p: Params):
    if p.mode not in ("max", "target"):
        raise UserError("Mode inconnu.")
    if p.roads not in osm.ROADS:
        raise UserError("Type de voies inconnu.")
    if p.source not in ("ign", "osm"):
        raise UserError("Source de chemins inconnue.")
    if p.mode == "target" and not (p.target_dplus and p.target_dplus > 0):
        raise UserError("Indique un D+ cible positif.")
    if not (0.0 < p.tol <= 0.5):
        raise UserError("Tolérance de distance entre 0 et 50 %.")
    if p.max_grade is not None and p.max_grade <= 0:
        raise UserError("Pente max doit être positive.")
    if not p.enforce_limits:
        return
    if not DIST_KM[0] <= p.distance_km <= DIST_KM[1]:
        raise UserError(f"Distance entre {DIST_KM[0]:g} et {DIST_KM[1]:g} km.")
    if not TIME_S[0] <= p.time_s <= TIME_S[1]:
        raise UserError(f"Temps de calcul entre {TIME_S[0]:g} et {TIME_S[1]:g} s.")


def base_edges(raw, region, dbg=None) -> dict:
    """Arêtes brutes -> arêtes de la zone, simplifiées, densifiées (repère local)."""
    dbg = {} if dbg is None else dbg
    dbg["raw_edges"] = len(raw)
    raw = graph.clip_to_region(raw, region, elevation.RES)
    dbg["edges_in_zone"] = len(raw)
    raw = graph.contract_degree2(raw)
    dbg["edges_simplified"] = len(raw)
    edges, eid = {}, 0
    # Géométrie brute : la densification (5 m) n'a lieu qu'après élagage et réduction, sur
    # les arêtes réellement gardées (assign_elevation s'en charge).
    for u, v, xy, flat in raw:
        r = graph.polyline(xy)
        if r is not None:
            edges[eid] = graph.Edge(u, v, r[0], r[1], flat)
            eid += 1
    if not edges:
        raise UserError("Aucune voie praticable dans la zone.")
    return edges


REDUCE_K = 8.0            # longueur d'arêtes pentues gardées, en multiples de Lmax
REDUCE_MIN_EDGES = 5000   # en dessous, pas de réduction (le recuit classique s'en sort)


def build_candidate(edges, Lmin, Lmax, sampler, max_grade=None, node_z=None, info=None,
                    step=lambda s: None, access=None, coarse=None):
    """Graphe de boucles depuis le point de `edges` le plus proche du point cliqué.
    Renvoie (graphe réindexé, nœud de départ) ou None si aucune boucle de longueur
    >= Lmin n'est possible (test nécessaire, sans altitude). `info` reçoit le détail."""
    info = {} if info is None else info
    edges_in = edges
    edges = dict(edges)  # insert_start remplace des arêtes : ne pas toucher l'original
    point, by_road = (0.0, 0.0), False
    nearest = min(float(np.hypot(e.xy[:, 0], e.xy[:, 1]).min()) for e in edges.values())
    if access is not None and nearest > ACCESS_MIN_M:
        # Réseau éloigné du point cliqué : y entrer par le point le plus proche par la route.
        ends = [xy for e in edges.values() for xy in (e.xy[0], e.xy[-1])]
        entry = access.best_entry(ends)
        if entry is not None:
            point, by_road = entry, True
    s, snap, pos = graph.insert_start(edges, max(edges) + 1, point=point)
    info["start_snap_m"] = round(float(np.hypot(*pos)), 1)
    if access is not None and nearest > ACCESS_MIN_M:
        acc = access(pos)
        if acc is not None and 2 * acc["length"] <= ACCESS_MAX_SHARE * Lmin:
            Lmin, Lmax = Lmin - 2 * acc["length"], Lmax - 2 * acc["length"]
            info["access"], info["access_m"] = acc, round(acc["length"])
        elif by_road:               # accès inutilisable : départ au plus proche à vol d'oiseau
            edges = dict(edges_in)
            s, snap, pos = graph.insert_start(edges, max(edges) + 1)
            info["start_snap_m"] = round(snap, 1)
    info["edges_doubled_near_start"] = graph.duplicate_near_start(edges, max(edges) + 1, pos, s)
    g = graph.prune(graph.Graph(edges), s, Lmax)
    info["edges_pruned"] = len(g.edges)
    total = sum(e.length for e in g.edges.values())
    info["network_km"] = round(total / 1000, 2)
    if total < Lmin:
        info["status"] = "réseau trop court"
        return None
    if coarse is not None and len(g.edges) > REDUCE_MIN_EDGES and total > 1.25 * REDUCE_K * Lmax:
        # Grand graphe : on ne garde que les arêtes les plus pentues et de quoi les relier,
        # repérées sur un modèle de terrain grossier (une seule requête). Le LiDAR fin n'est
        # ensuite échantillonné que sur ce graphe réduit. Heuristique, voir steep_reduction.
        step("criblage")
        keep = graph.steep_reduction(g, s, Lmax, graph.screening_w(g, coarse), REDUCE_K)
        g = graph.prune(g.sub(keep), s, Lmax)
        info["edges_reduced"] = len(g.edges)
        if sum(e.length for e in g.edges.values()) < Lmin:
            info["status"] = "réseau trop court après réduction"
            return None
    step("altitude")
    info["elevation_points"] = graph.assign_elevation(g, sampler, node_z=node_z)
    step("élagage")
    if max_grade is not None:
        steep = {k for k, e in g.edges.items() if e.max_grade > max_grade}
        g = graph.prune(g.sub(set(g.edges) - steep), s, Lmax)
        if sum(e.length for e in g.edges.values()) < Lmin:
            info["status"] = "trop court après filtre de pente"
            return None
    info["edges_after_grade"] = len(g.edges)
    return g.reindexed(), s


def prepare_graph(raw, region, Lmax, sampler, max_grade=None, dbg=None):
    """Graphe depuis le point cliqué, sans repli (tests)."""
    dbg = {} if dbg is None else dbg
    r = build_candidate(base_edges(raw, region, dbg), 0.0, Lmax, sampler, max_grade, info=dbg)
    if r is None or not r[0].edges:
        raise UserError("Aucune boucle possible depuis ce départ dans cette zone.")
    return r


def candidate_sets(edges):
    """Départ cliqué d'abord, puis chaque sous-réseau bouclable trié par distance."""
    yield "départ", edges
    mind = {k: float(np.hypot(e.xy[:, 0], e.xy[:, 1]).min()) for k, e in edges.items()}
    comps = graph.loop_components(graph.Graph(edges))
    comps.sort(key=lambda c: min(mind[k] for k in c))
    for c in comps:
        yield "repli", {k: edges[k] for k in c}


ACCESS_MIN_M = 30.0        # au-delà, on cherche un accès par la route depuis le point cliqué
ACCESS_MAX_START_M = 100.0  # le point cliqué doit être à moins de ça d'une voie quelconque
ACCESS_MAX_SHARE = 0.6     # l'aller-retour d'accès prend au plus cette part de la distance


class Access:
    """Accès par aller-retour : plus courts chemins, par toutes les voies de la zone, du
    point cliqué (origine) vers le réseau bouclable. La boucle part alors vraiment du point
    choisi. Construit à la demande, une seule fois (un Dijkstra depuis le point cliqué)."""

    def __init__(self, raw_access, region, sampler):
        self._raw, self._region, self._sampler = raw_access, region, sampler
        self._ready = False
        self.g = None

    def _prepare(self):
        if self._ready:
            return
        self._ready = True
        try:
            edges = dict(base_edges(self._raw(), self._region))
        except Exception:           # réseau d'accès indisponible : on s'en passe
            return
        self.s0, self.snap0, _ = graph.insert_start(edges, max(edges) + 1)
        if self.snap0 > ACCESS_MAX_START_M:
            return
        self.edges = edges
        self.g = graph.Graph(edges)
        self.dist, self.prev = graph.shortest(
            self.g.adj, {k: e.length for k, e in edges.items()}, self.s0)
        # position (arrondie au dm) -> nœud du réseau d'accès : les jonctions sentier/route
        # ont les mêmes coordonnées dans les deux réseaux.
        self.by_xy = {(round(float(xy[0]), 1), round(float(xy[1]), 1)): n
                      for n, xy in self.g.nxy.items() if n in self.dist}

    def best_entry(self, points):
        """Parmi `points` (extrémités de tronçons du réseau bouclable), celui qui est le
        plus proche du point cliqué par la route. None si aucun n'est relié."""
        self._prepare()
        if self.g is None:
            return None
        best = None
        for xy in points:
            n = self.by_xy.get((round(float(xy[0]), 1), round(float(xy[1]), 1)))
            if n is not None and n != self.s0 and (best is None or self.dist[n] < best[0]):
                best = (self.dist[n], xy)
        return None if best is None else np.asarray(best[1], float)

    def __call__(self, pos):
        """Chemin d'accès jusqu'à `pos`, ou None."""
        self._prepare()
        if self.g is None:
            return None
        g, prev, t = self.g, self.prev, self.by_xy.get((round(float(pos[0]), 1), round(float(pos[1]), 1)))
        if t is None:               # `pos` n'est pas un nœud : on l'insère et on recalcule
            edges = dict(self.edges)
            t, snap_t, _ = graph.insert_start(edges, max(edges) + 1, point=pos, node=graph.START - 1)
            if snap_t > 10.0:
                return None
            g = graph.Graph(edges)
            dist, prev = graph.shortest(g.adj, {k: e.length for k, e in edges.items()}, self.s0, t)
            if t not in dist:
                return None
        if t == self.s0:
            return None
        path = graph.path_from(prev, self.s0, t)
        sub = graph.Graph({k: g.edges[k] for k, _, _ in path})
        graph.assign_elevation(sub, self._sampler)
        xy, z, s = graph.route_geometry(sub, path)
        return {"length": float(s[-1]), "xy": xy, "z": z, "snap": self.snap0,
                "updown": float(np.abs(np.diff(z)).sum())}


def make_access(raw_access, region, sampler) -> Access:
    return Access(raw_access, region, sampler)


def assemble(g, res, acc=None):
    """Tracé complet (xy, z, abscisse), longueur et D+ : la boucle, précédée et suivie de
    l'aller-retour d'accès s'il y en a un."""
    xy, z, _ = graph.route_geometry(g, res.circuit)
    length, dplus = res.length, res.dplus
    if acc is not None:
        az = acc["z"].copy()
        az[-1] = z[0]                       # raccord exact au départ de la boucle
        xy = np.vstack([acc["xy"], xy[1:], acc["xy"][::-1][1:]])
        z = np.concatenate([az, z[1:], az[::-1][1:]])
        length += 2 * acc["length"]
        dplus += float(np.abs(np.diff(az)).sum())   # montée à l'aller + montée au retour
    s = np.concatenate([[0.0], np.cumsum(np.hypot(*np.diff(xy, axis=0).T))])
    return xy, z, s, length, dplus


MIN_ATTEMPT_S = 3.0   # budget minimal pour tenter un sous-réseau de repli
MAX_ATTEMPTS = 8      # sous-réseaux résolus au plus (les trop courts ne comptent pas)


def search_loop(raw, region, p: Params, L, Lmax, sampler, dbg, timings, step=lambda s: None,
                cancel=None, access=None, coarse=None):
    """Premier sous-réseau, du plus proche au plus lointain, qui donne une boucle
    à la bonne distance. Renvoie (Problem, graphe, SolveResult, info du candidat, accès).
    `access` : voir make_access ; l'accès retenu est None si la boucle part du point cliqué."""
    t = time.time()
    edges = base_edges(raw, region, dbg)
    Lmin = L * (1 - p.tol) if p.mode == "max" else 0.7 * L
    node_z, tried_ids, attempts, first = {}, set(), [], None
    remaining = p.time_s
    timings["prep_s"] = time.time() - t
    for kind, sub in candidate_sets(edges):
        if remaining < MIN_ATTEMPT_S or sum(a["solved"] for a in attempts) >= MAX_ATTEMPTS:
            break
        if kind == "repli" and any(id(e) in tried_ids for e in sub.values()):
            continue  # déjà couvert par un candidat précédent
        t = time.time()
        # Avec accès par aller-retour d'abord ; s'il rend la boucle impossible (il mange de la
        # distance), même sous-réseau avec le départ déplacé.
        for acc_fn in ((access, None) if access is not None else (None,)):
            info = {"kind": kind, "solved": False}
            attempts.append(info)
            r = build_candidate(sub, Lmin, Lmax, sampler, p.max_grade, node_z, info, step, acc_fn,
                                coarse)
            acc = info.pop("access", None)      # tableaux : hors du rapport de debug
            if r is not None or acc is None:
                break
            info["status"] += " avec l'aller-retour d'accès"
        timings["prep_s"] += time.time() - t
        if r is None:
            continue
        g, s = r
        tried_ids.update(id(e) for e in g.edges.values())
        # L'aller-retour d'accès est retranché de ce que la boucle doit faire.
        L_eff = L - (2 * acc["length"] if acc else 0.0)
        D_eff = p.target_dplus
        if acc and D_eff:
            D_eff = max(1.0, D_eff - acc["updown"])
        P = graph.Problem(g, s, L_eff, p.mode, p.tol * L / L_eff, D_eff, p.node_simple)
        step("solveur")
        t = time.time()
        try:
            res = optimize(P, remaining, p.exact_max_edges, p.seed, p.workers, p.solver, cancel)
        except RuntimeError:
            if cancel is not None and cancel.is_set():
                raise Cancelled("Calcul annulé.") from None
            raise
        if cancel is not None and cancel.is_set():
            raise Cancelled("Calcul annulé.")
        dt = time.time() - t
        timings["solver_s"] = timings.get("solver_s", 0.0) + dt
        remaining -= dt
        info.update(solved=True, feasible=res.feasible, method=res.method,
                    status="boucle trouvée" if res.feasible else "distance hors bornes")
        if first is None:
            first = (P, g, res, info, acc)
        if res.feasible:
            first = (P, g, res, info, acc)
            break
    dbg["attempts"] = attempts
    if first is None:
        raise UserError("Aucune boucle de cette distance n'est possible dans la zone "
                        "(réseau trop court ou trop morcelé). Agrandis la zone ou réduis la distance.")
    return first


def plan_loop(p: Params, progress=None, cancel=None) -> LoopResult:
    """progress(étape: str) est appelé au début de chaque étape.
    cancel (threading.Event) : lève Cancelled dès que possible une fois positionné."""
    def step(name):
        if cancel is not None and cancel.is_set():
            raise Cancelled("Calcul annulé.")
        if progress:
            progress(name)
    if p.time_s is None:
        p.time_s = suggested_time(p.distance_km)
    validate(p)
    dbg, warns, timings = {}, [], {}
    if p.distance_km > LONG_KM:
        warns.append(f"Distance de plus de {LONG_KM:g} km : le chargement des données et le calcul "
                     f"sont plus longs.")
    net = {"overpass": cache.new_stats(), "wfs_ign": cache.new_stats(), "wms_r": cache.new_stats()}
    L = p.distance_km * 1000.0
    frame = LocalFrame(p.lat, p.lon)
    Lmax = L * (1 + p.tol) if p.mode == "max" else 1.3 * L
    region = build_region(p, frame, Lmax)
    area_km2 = region.area / 1e6
    dbg["zone_km2"] = round(area_km2, 2)
    if p.enforce_limits and area_km2 > MAX_AREA_KM2:
        raise UserError(f"Zone utile trop grande : {area_km2:.0f} km² (max {MAX_AREA_KM2:g}).")

    # Réseau de chemins : BD TOPO IGN ou OpenStreetMap
    step("chemins")
    t = time.time()
    # La BD TOPO n'a ni trottoirs ni petites voies piétonnes : ce mode passe toujours par OSM.
    source = "osm" if p.roads == "pedestrian" else p.source

    def bbox_of(reg):
        minx, miny, maxx, maxy = reg.bounds
        m = OSM_MARGIN_M
        corners = np.array([[minx - m, miny - m], [maxx + m, miny - m],
                            [minx - m, maxy + m], [maxx + m, maxy + m]])
        lon, lat = frame.to_wgs(corners)
        return (min(lat), min(lon), max(lat), max(lon))

    bbox = bbox_of(region)
    if p.enforce_limits and area_km2 > FALLBACK_AREA_KM2:
        # Grande zone : on compte les voies avant de télécharger, et on réduit si c'est trop dense.
        radius0 = radius = float(np.hypot(*np.asarray(region.exterior.coords).T).max()) \
            if hasattr(region, "exterior") else math.sqrt(region.area / math.pi)
        cap = MAX_WAYS_IGN_MAX if (source == "ign" and p.mode == "max") else MAX_WAYS[source]
        for _ in range(4):
            n = (ign.count(bbox, net["wfs_ign"]) if source == "ign"
                 else osm.count(bbox, p.roads, net["overpass"]))
            if n is None:       # comptage indisponible : on retombe sur un plafond d'aire prudent
                radius = min(radius, math.sqrt(FALLBACK_AREA_KM2 * 1e6 / math.pi))
            elif n <= cap:
                break
            else:
                radius *= 0.95 * math.sqrt(cap / n)
            region = region.intersection(graph.disk(radius))
            bbox = bbox_of(region)
            if n is None:
                break
        if radius < radius0 - 1.0:
            dbg["zone_km2"] = round(region.area / 1e6, 2)
            warns.append(f"Zone trop dense en voies : réduite à un rayon de {radius / 1000:.1f} km "
                         f"autour du départ ({region.area / 1e6:.0f} km²) pour garder un calcul rapide.")
    if source == "ign":
        data = ign.fetch(bbox, net["wfs_ign"])
        raw, nways = ign.to_edges(data, frame, p.roads)

        def raw_access():
            return ign.to_edges(data, frame, "all")[0]
    else:
        data = osm.fetch(osm.build_query(bbox, p.roads), net["overpass"])
        raw, nways = osm.ways_to_edges(data, frame, p.roads)

        def raw_access():   # voies piétonnes : les routes ne sont pas dans la réponse, on les demande
            d2 = (osm.fetch(osm.build_query(bbox, "minor"), net["overpass"])
                  if p.roads == "pedestrian" else data)
            return osm.ways_to_edges(d2, frame, "all" if p.roads == "all" else "minor")[0]
    timings["network_fetch_s"] = time.time() - t
    sampler = elevation.sampler_for(frame, net["wms_r"])

    dbg["source"] = "IGN BD TOPO" if source == "ign" else "OpenStreetMap"
    dbg["source_ways"] = nways
    # Criblage par relief : en mode max seulement (en mode cible, le plat peut être utile).
    coarse = elevation.coarse_sampler_for(frame, net["wms_r"]) if p.mode == "max" else None
    P, g, res, info, acc = search_loop(raw, region, p, L, Lmax, sampler, dbg, timings, step,
                                       cancel, make_access(raw_access, region, sampler), coarse)
    del raw, data
    for k in ("start_snap_m", "edges_doubled_near_start", "edges_pruned", "edges_reduced",
              "edges_after_grade"):
        dbg[k] = info.get(k)
    dbg["elevation_points"] = sum(a.get("elevation_points", 0) for a in dbg["attempts"])
    snap = info["start_snap_m"]
    if acc is not None:
        dbg["start_snap_m"], dbg["access_m"] = round(acc["snap"], 1), round(acc["length"])
        warns.append(f"Pas de boucle possible sur ce type de voies depuis le point choisi : la "
                     f"boucle en part quand même, avec un aller-retour de {acc['length']:.0f} m "
                     f"par les autres voies pour rejoindre le réseau.")
    elif info["kind"] == "repli":
        warns.append(f"Pas de boucle possible depuis le point choisi : départ déplacé à "
                     f"{snap:.0f} m, au point le plus proche qui permet une boucle.")
    elif snap > 150:
        warns.append(f"Départ à {snap:.0f} m de la voie la plus proche.")
    dbg["nodes"] = len(P.nodes)
    ub = P.dplus_upper_bound()
    dbg["dplus_upper_bound_m"] = round(ub)
    dbg.update(res.debug)

    used = {id(g.edges[e]) for e, _, _ in res.circuit}
    dbg["edges_used_twice_near_start"] = sum(
        1 for e, _, _ in res.circuit if g.edges[e].twin is not None and id(g.edges[e].twin) in used)
    xy, z, s, length, dplus = assemble(g, res, acc)
    lon, lat = frame.to_wgs(xy)
    prof = graph.profile_dplus(z)
    dbg["profile_dplus_m"] = prof
    dbg["sum_w_m"] = dplus
    if abs(prof - dplus) > 1e-6 * max(1.0, dplus):
        warns.append(f"Incohérence D+ profil ({prof:.2f}) / Σw ({dplus:.2f}).")

    if res.debug.get("cpsat_status") == "INFEASIBLE":
        warns.append(f"Prouvé par CP-SAT : aucune boucle de {P.Lmin / 1000:.1f} à "
                     f"{P.Lmax / 1000:.1f} km dans cette zone. Boucle la plus proche affichée.")
    if p.mode == "max":
        if not res.feasible:
            warns.append("Distance hors tolérance : aucune boucle trouvée dans les bornes.")
        desc = f"{length / 1000:.2f} km, D+ {dplus:.0f} m"
    else:
        eL = (length - L) / L
        eD = (dplus - p.target_dplus) / p.target_dplus
        dbg["err_distance"], dbg["err_dplus"] = eL, eD
        if p.target_dplus > ub:
            warns.append(f"D+ cible hors de portée : le maximum théorique est {ub:.0f} m "
                         f"pour {Lmax / 1000:.1f} km dans cette zone.")
        elif res.debug.get("cpsat_bound") is not None and res.debug["cpsat_bound"] > 0.05:
            warns.append(f"Cible inatteignable : erreur minimale prouvée "
                         f"{res.debug['cpsat_bound']:.1%}.")
        elif abs(eD) > 0.10:
            warns.append("D+ cible probablement hors de portée dans cette zone.")
        desc = (f"{length / 1000:.2f} km, D+ {dplus:.0f} m "
                f"(cible {p.distance_km:g} km / {p.target_dplus:.0f} m)")

    dbg["timings_s"] = {k: round(v, 2) for k, v in timings.items()}
    dbg["network"] = net
    dbg["peak_rss_mb"] = round(peak_rss_mb(), 1)
    gpx = to_gpx(lon, lat, z, f"Boucle {p.distance_km:g} km", f"{desc}, {res.method}")
    return LoopResult(np.asarray(lon), np.asarray(lat), z, s, length, dplus,
                      res.method, res.feasible, gpx, _rings(region, frame), warns, dbg)
