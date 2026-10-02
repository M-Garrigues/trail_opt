"""Graphe, géométrie, élagage, profils, Hierholzer.

Toutes les coordonnées sont en mètres dans le repère local centré sur le départ.
"""
from __future__ import annotations

import heapq
from collections import defaultdict
from dataclasses import dataclass

import numpy as np
import shapely
from shapely.geometry import Point

INF = float("inf")
START = -1  # identifiant du nœud de départ quand il coupe une arête
# Autour du départ, on peut reprendre la même route (aller-retour sur l'accès)
# et repasser par les mêmes carrefours, même en mode « carrefours uniques ».
FREE_RADIUS = 200.0
# Couloirs parallèles (deux trottoirs, chemin large saisi deux fois) : deux tronçons sont
# « parallèles » s'ils restent à moins de PARALLEL_DIST l'un de l'autre, dans la même
# direction, sur au moins PARALLEL_MIN_LEN et PARALLEL_MIN_FRAC du plus court.
# Une boucle n'en emprunte qu'un. Deux petits bouts qui se touchent juste à un carrefour,
# ou qui ne se longent que partiellement, ne sont pas concernés.
PARALLEL_DIST = 15.0
PARALLEL_MIN_LEN = 30.0
PARALLEL_MIN_FRAC = 0.6
PARALLEL_COS = 0.9  # ~25°


@dataclass
class Edge:
    u: int
    v: int
    xy: np.ndarray          # (N,2), orienté u -> v
    s: np.ndarray           # abscisse curviligne cumulée (N,)
    flat: bool = False      # pont / tunnel : profil interpolé
    z: np.ndarray | None = None
    w: float = 0.0          # (montée + descente) / 2
    max_grade: float = 0.0
    twin: "Edge | None" = None  # original dont cette arête est la copie (rayon libre)

    @property
    def length(self) -> float:
        return float(self.s[-1])


class Graph:
    def __init__(self, edges: dict[int, Edge]):
        self.edges = edges
        self.adj: dict[int, list[tuple[int, int]]] = {}
        self.nxy: dict[int, np.ndarray] = {}
        for eid, e in edges.items():
            self.adj.setdefault(e.u, []).append((eid, e.v))
            if e.u != e.v:
                self.adj.setdefault(e.v, []).append((eid, e.u))
            self.nxy[e.u] = e.xy[0]
            self.nxy[e.v] = e.xy[-1]

    def sub(self, keep) -> "Graph":
        return Graph({k: self.edges[k] for k in keep})

    def reindexed(self) -> "Graph":
        return Graph({i: e for i, e in enumerate(self.edges.values())})


# ---------------------------------------------------------------------------
# Géométrie et région
# ---------------------------------------------------------------------------
def densify(xy, step: float):
    """Garde les sommets de la polyligne, ajoute des points espacés de <= step."""
    xy = np.asarray(xy, float)
    if len(xy) < 2:
        return None
    d = np.hypot(*np.diff(xy, axis=0).T)
    xy = xy[np.concatenate([[True], d > 1e-6])]
    if len(xy) < 2:
        return None
    seg = np.hypot(*np.diff(xy, axis=0).T)
    k = np.maximum(1, np.ceil(seg / step)).astype(int)
    parts = [xy[:1]]
    for p, q, kk in zip(xy[:-1], xy[1:], k):
        t = (np.arange(1, kk + 1) / kk)[:, None]
        parts.append(p + t * (q - p))
    xy = np.vstack(parts)
    s = np.concatenate([[0.0], np.cumsum(np.hypot(*np.diff(xy, axis=0).T))])
    return xy, s


def disk(R: float):
    """Disque de rayon R centré sur le départ (zone par défaut)."""
    return Point(0, 0).buffer(R, 64)


def clip_to_reach(poly, Lmax: float):
    """Une boucle de longueur <= Lmax ne s'éloigne jamais à plus de Lmax/2 du départ."""
    return poly.intersection(Point(0, 0).buffer(Lmax / 2.0, 128))


def clip_to_region(raw, poly, step: float = 5.0):
    """Garde la partie des arêtes brutes située dans le polygone. Une arête entièrement
    dedans est gardée telle quelle ; une arête qui traverse le bord est coupée au bord
    (à `step` près), au lieu d'être écartée en entier : un long tronçon qui dépasse à peine
    de la zone reste utilisable. Les coupes créent des nœuds (ids négatifs uniques)."""
    if not raw:
        return []
    allxy = np.vstack([e[2] for e in raw])
    inside = shapely.contains_xy(poly, allxy[:, 0], allxy[:, 1])
    starts = np.cumsum([0] + [len(e[2]) for e in raw[:-1]])
    all_in = np.logical_and.reduceat(inside, starts)
    any_in = np.logical_or.reduceat(inside, starts)
    out, cut_id = [], START - 10
    for (u, v, xy, flat), full, some in zip(raw, all_in, any_in):
        if full:
            out.append((u, v, xy, flat))
            continue
        d = densify(xy, step)
        if d is None or not (some or len(xy) >= 2):
            continue
        pts = d[0]
        m = shapely.contains_xy(poly, pts[:, 0], pts[:, 1])
        if not m.any():
            continue
        # plages consécutives de points intérieurs
        edges_ = np.flatnonzero(np.diff(np.concatenate([[False], m, [False]]).astype(int)))
        for a, b in zip(edges_[::2], edges_[1::2]):     # points a..b-1 dedans
            if b - a < 2:
                continue
            nu = u if a == 0 else cut_id
            cut_id -= a != 0
            nv = v if b == len(pts) else cut_id
            cut_id -= b != len(pts)
            out.append((nu, nv, pts[a:b], flat))
    return out


def contract_degree2(raw):
    """Fusionne les arêtes aux nœuds de degré 2 (pas des carrefours).
    Ne fusionne pas un pont/tunnel avec un tronçon normal."""
    E = {i: list(e) for i, e in enumerate(raw)}
    inc = defaultdict(list)
    for i, (u, v, _, _) in E.items():
        inc[u].append(i)
        inc[v].append(i)
    for n in list(inc):
        ids = inc[n]
        if len(ids) != 2 or ids[0] == ids[1]:
            continue
        i, j = ids
        ui, vi, xyi, fi = E[i]
        uj, vj, xyj, fj = E[j]
        if fi != fj:
            continue
        if vi != n:
            ui, vi, xyi = vi, ui, xyi[::-1]
        if uj != n:
            uj, vj, xyj = vj, uj, xyj[::-1]
        E[i] = [ui, vj, np.vstack([xyi, xyj[1:]]), fi]
        del E[j]
        inc[n] = []
        lst = inc[vj]
        lst[lst.index(j)] = i
    return [tuple(e) for e in E.values()]


# ---------------------------------------------------------------------------
# Algorithmes de graphe
# ---------------------------------------------------------------------------
def shortest(adj, cost, src, target=None, banned=None):
    """Dijkstra. `cost` est indexable par identifiant d'arête."""
    dist = {src: 0.0}
    prev = {}
    pq = [(0.0, src)]
    while pq:
        d, n = heapq.heappop(pq)
        if d > dist[n]:
            continue
        if n == target:
            break
        for eid, nb in adj[n]:
            if banned is not None and eid in banned:
                continue
            nd = d + cost[eid]
            if nd < dist.get(nb, INF):
                dist[nb] = nd
                prev[nb] = (eid, n)
                heapq.heappush(pq, (nd, nb))
    return dist, prev


def path_from(prev, src, tgt):
    out, n = [], tgt
    while n != src:
        eid, p = prev[n]
        out.append((eid, p, n))
        n = p
    out.reverse()
    return out


def find_bridges(g: Graph) -> set[int]:
    """Tarjan itératif sur identifiants d'arêtes (gère multi-arêtes et boucles)."""
    disc, low, bridges, t = {}, {}, set(), 0
    for root in g.adj:
        if root in disc:
            continue
        disc[root] = low[root] = t
        t += 1
        stack = [(root, None, iter(g.adj[root]))]
        while stack:
            node, pe, it = stack[-1]
            pushed = False
            for eid, nb in it:
                if eid == pe or nb == node:
                    continue
                if nb in disc:
                    low[node] = min(low[node], disc[nb])
                else:
                    disc[nb] = low[nb] = t
                    t += 1
                    stack.append((nb, eid, iter(g.adj[nb])))
                    pushed = True
                    break
            if not pushed:
                stack.pop()
                if stack:
                    parent = stack[-1][0]
                    low[parent] = min(low[parent], low[node])
                    if low[node] > disc[parent]:
                        bridges.add(pe)
    return bridges


def prune(g: Graph, s: int, Lmax: float) -> Graph:
    """Itère jusqu'à stabilité : retrait des ponts, composante de s, filtre de portée."""
    while True:
        m0 = len(g.edges)
        g = g.sub(set(g.edges) - find_bridges(g))
        if s not in g.adj:
            return Graph({})
        seen, st = {s}, [s]
        while st:
            n = st.pop()
            for _, nb in g.adj[n]:
                if nb not in seen:
                    seen.add(nb)
                    st.append(nb)
        g = g.sub({k for k, e in g.edges.items() if e.u in seen})
        dist, _ = shortest(g.adj, {k: e.length for k, e in g.edges.items()}, s)
        g = g.sub({k for k, e in g.edges.items()
                   if dist.get(e.u, INF) + e.length + dist.get(e.v, INF) <= Lmax})
        if len(g.edges) == m0:
            return g


def parallel_pairs(g: Graph, center=(0.0, 0.0)) -> dict[int, set[int]]:
    """Tronçons parallèles deux à deux (voir PARALLEL_*), hors rayon libre du départ.
    Points densifiés + cKDTree : tout est vectorisé."""
    from scipy.spatial import cKDTree
    cx, cy = center
    pts, own, tan, wgt, elen = [], [], [], [], {}
    for k, e in g.edges.items():
        if e.twin is not None or e.length < PARALLEL_MIN_LEN or len(e.xy) < 2:
            continue
        if np.hypot(e.xy[:, 0] - cx, e.xy[:, 1] - cy).max() <= FREE_RADIUS:
            continue
        d = np.gradient(e.xy, axis=0)
        tan.append(d / (np.hypot(d[:, 0], d[:, 1])[:, None] + 1e-12))
        w = np.gradient(e.s)  # longueur représentée par chaque point
        wgt.append(w)
        pts.append(e.xy)
        own.append(np.full(len(e.xy), k))
        elen[k] = e.length
    if len(pts) < 2:
        return {}
    P, E, T, W = np.vstack(pts), np.concatenate(own), np.vstack(tan), np.concatenate(wgt)
    pr = cKDTree(P).query_pairs(PARALLEL_DIST, output_type="ndarray")
    if len(pr) == 0:
        return {}
    a, b = pr[:, 0], pr[:, 1]
    keep = (E[a] != E[b]) & (np.abs((T[a] * T[b]).sum(1)) >= PARALLEL_COS)
    a, b = a[keep], b[keep]
    if len(a) == 0:
        return {}
    swap = E[a] > E[b]
    lo_pt, hi_pt = np.where(swap, b, a), np.where(swap, a, b)
    M, NP = int(E.max()) + 1, len(P)
    key = E[lo_pt].astype(np.int64) * M + E[hi_pt]

    def covered(pt):
        # longueur couverte, côté donné, par paire de tronçons (points distincts)
        u = np.unique(key * NP + pt)
        k, p = u // NP, u % NP
        keys, inv = np.unique(k, return_inverse=True)
        return keys, np.bincount(inv, weights=W[p])

    keys, cov_lo = covered(lo_pt)
    keys2, cov_hi = covered(hi_pt)
    assert np.array_equal(keys, keys2)
    out: dict[int, set[int]] = {}
    for kk, c1, c2 in zip(keys, cov_lo, cov_hi):
        e1, e2 = int(kk // M), int(kk % M)
        shortest_len = min(elen[e1], elen[e2])
        if min(c1, c2) >= max(PARALLEL_MIN_LEN, PARALLEL_MIN_FRAC * shortest_len):
            out.setdefault(e1, set()).add(e2)
            out.setdefault(e2, set()).add(e1)
    return out


def loop_components(g: Graph) -> list[set[int]]:
    """Composantes 2-arête-connexes (après retrait des ponts) : les seuls
    sous-réseaux où une boucle sans arête répétée peut exister."""
    h = g.sub(set(g.edges) - find_bridges(g))
    label = {}
    for root in h.adj:
        if root in label:
            continue
        label[root] = root
        stack = [root]
        while stack:
            n = stack.pop()
            for _, nb in h.adj[n]:
                if nb not in label:
                    label[nb] = root
                    stack.append(nb)
    comps = {}
    for k, e in h.edges.items():
        comps.setdefault(label[e.u], set()).add(k)
    return list(comps.values())


def euler_circuit(g: Graph, eids, s):
    """Hierholzer : circuit [(eid, de, vers), ...] partant de s."""
    adj = {}
    for e in eids:
        ed = g.edges[e]
        adj.setdefault(ed.u, []).append((e, ed.v))
        if ed.u != ed.v:
            adj.setdefault(ed.v, []).append((e, ed.u))
    if s not in adj:
        raise RuntimeError("le départ n'est pas dans le sous-graphe")
    used, ptr = set(), {n: 0 for n in adj}
    stack, circuit = [(s, None, None)], []
    while stack:
        n, e_in, frm = stack[-1]
        lst = adj[n]
        while ptr[n] < len(lst) and lst[ptr[n]][0] in used:
            ptr[n] += 1
        if ptr[n] < len(lst):
            e, nb = lst[ptr[n]]
            used.add(e)
            stack.append((nb, e, n))
        else:
            stack.pop()
            if e_in is not None:
                circuit.append((e_in, frm, n))
    circuit.reverse()
    if len(circuit) != len(set(eids)) or circuit[0][1] != s or circuit[-1][2] != s:
        raise RuntimeError("sous-graphe non eulérien ou non connexe")
    return circuit


# ---------------------------------------------------------------------------
# Construction
# ---------------------------------------------------------------------------
def insert_start(edges: dict[int, Edge], next_id: int, point=(0.0, 0.0), node: int = START):
    """Coupe l'arête la plus proche de `point` (défaut : le point cliqué, à l'origine) et y
    insère le nœud `node`. Renvoie (nœud, distance à `point`, position du nœud)."""
    if not edges:
        raise RuntimeError("aucune voie dans la zone")
    ids = list(edges)
    lens = [len(edges[k].xy) for k in ids]
    allxy = np.vstack([edges[k].xy for k in ids])
    d = np.hypot(allxy[:, 0] - point[0], allxy[:, 1] - point[1])
    j = int(np.argmin(d))
    starts = np.cumsum([0] + lens)
    k = int(np.searchsorted(starts, j, side="right") - 1)
    eid, i = ids[k], j - int(starts[k])
    e = edges[eid]
    pos = allxy[j].copy()
    if i == 0:
        return e.u, float(d[j]), pos
    if i == len(e.xy) - 1:
        return e.v, float(d[j]), pos
    edges[eid] = Edge(e.u, node, e.xy[: i + 1], e.s[: i + 1], e.flat)
    edges[next_id] = Edge(node, e.v, e.xy[i:], e.s[i:] - e.s[i], e.flat)
    return node, float(d[j]), pos


def access_bridges(g: Graph, s: int) -> set[int]:
    """Ponts « d'accès » : ceux qu'une boucle partant de s doit emprunter à l'aller ET au
    retour, faute d'autre chemin. Ce sont les ponts qui restent après effeuillage des
    culs-de-sac (un pont menant à une impasse ne mène à aucune boucle), s étant protégé."""
    bridges = find_bridges(g)
    deg = {n: len(a) for n, a in g.adj.items()}
    alive = set(g.edges)
    leaves = [n for n, d in deg.items() if d == 1 and n != s]
    while leaves:
        n = leaves.pop()
        if deg[n] != 1 or n == s:
            continue
        for eid, nb in g.adj[n]:
            if eid in alive:
                alive.discard(eid)
                deg[n] -= 1
                deg[nb] -= 1
                if deg[nb] == 1 and nb != s:
                    leaves.append(nb)
                break
    return bridges & alive


def duplicate_near_start(edges: dict[int, Edge], next_id: int, center=(0.0, 0.0), s: int = START,
                         radius: float | None = None) -> int:
    """Double (copie parallèle) les arêtes d'accès proches du départ : entièrement à moins de
    `radius` de `center`, et sans autre chemin possible (voir access_bridges). Une boucle
    sans arête répétée peut alors faire l'aller-retour sur un accès en impasse. Les autres
    arêtes proches ne sont pas doublées : sinon l'optimiseur multiplie les allers-retours
    autour du départ pour gagner du D+. Renvoie le nombre de copies."""
    radius = FREE_RADIUS if radius is None else radius
    cx, cy = center
    near = [k for k, e in edges.items()
            if np.hypot(e.xy[:, 0] - cx, e.xy[:, 1] - cy).max() <= radius]
    if not near:
        return 0
    forced = access_bridges(Graph(edges), s)
    near = [k for k in near if k in forced]
    for i, k in enumerate(near):
        e = edges[k]
        edges[next_id + i] = Edge(e.u, e.v, e.xy, e.s, e.flat, twin=e)
    return len(near)


def fill_nan(e: Edge, z: np.ndarray) -> np.ndarray:
    z = np.asarray(z, float).copy()
    bad = ~np.isfinite(z)
    if bad.all():
        return np.zeros_like(z) * np.nan
    if bad.any():
        z[bad] = np.interp(e.s[bad], e.s[~bad], z[~bad])
    return z


def compute_profile(e: Edge, z: np.ndarray, smooth: int = 3, grade_win: float = 25.0):
    """z doit être sans NaN et avoir les altitudes des nœuds aux extrémités."""
    if e.flat:
        z = np.interp(e.s, [0.0, e.length], [z[0], z[-1]])
    elif smooth >= 3 and len(z) > 2:
        k = smooth // 2
        zp = np.pad(z, k, mode="edge")
        ma = np.convolve(zp, np.ones(2 * k + 1) / (2 * k + 1), mode="valid")
        ma[0], ma[-1] = z[0], z[-1]  # extrémités fixées : altitude de nœud unique
        z = ma
    e.z = z
    e.w = float(np.abs(np.diff(z)).sum() / 2.0)
    if e.length <= grade_win:
        e.max_grade = abs(z[-1] - z[0]) / max(e.length, 1e-6)
    else:
        j = np.searchsorted(e.s, e.s + grade_win)
        ok = j < len(e.s)
        i, j = np.nonzero(ok)[0], j[ok]
        e.max_grade = float(np.max(np.abs(z[j] - z[i]) / (e.s[j] - e.s[i]))) if len(i) else 0.0


def assign_elevation(g: Graph, sampler, smooth=3, grade_win=25.0, node_z=None) -> int:
    """Échantillonne en un appel les arêtes sans profil, calcule leurs profils.
    `node_z` (partagé entre appels) garantit une altitude unique par nœud.
    Renvoie le nombre de points échantillonnés."""
    ids = [k for k, e in g.edges.items() if e.z is None]
    if not ids:
        return 0
    allxy = np.vstack([g.edges[k].xy for k in ids])
    z = np.asarray(sampler(allxy), float)
    if not np.isfinite(z).any():
        raise RuntimeError("aucune donnée d'altitude sur la zone (hors France ?)")
    zs, off = {}, 0
    for k in ids:
        n = len(g.edges[k].xy)
        zs[k] = fill_nan(g.edges[k], z[off: off + n])
        off += n
    # Altitude unique par nœud : première valeur finie rencontrée.
    node_z = {} if node_z is None else node_z
    for k in ids:
        e, zk = g.edges[k], zs[k]
        for n, val in ((e.u, zk[0]), (e.v, zk[-1])):
            if n not in node_z and np.isfinite(val):
                node_z[n] = float(val)
    fallback = float(np.nanmean(z))
    for k in ids:
        e, zk = g.edges[k], zs[k]
        zk[0] = node_z.get(e.u, fallback)
        zk[-1] = node_z.get(e.v, fallback)
        zk = fill_nan(e, zk)
        compute_profile(e, zk, smooth, grade_win)
    return len(allxy)


def route_geometry(g: Graph, circuit):
    """Polyligne (xy, z, abscisse) de la boucle, dans l'ordre de parcours."""
    xy, z = [], []
    for eid, a, b in circuit:
        e = g.edges[eid]
        exy, ez = e.xy, e.z
        if e.u != e.v and a != e.u:
            exy, ez = exy[::-1], ez[::-1]
        if xy:
            exy, ez = exy[1:], ez[1:]
        xy.append(exy)
        z.append(ez)
    xy, z = np.vstack(xy), np.concatenate(z)
    s = np.concatenate([[0.0], np.cumsum(np.hypot(*np.diff(xy, axis=0).T))])
    return xy, z, s


def profile_dplus(z) -> float:
    dz = np.diff(z)
    return float(dz[dz > 0].sum())


class Problem:
    """Instance réindexée prête pour les solveurs.

    mode "max"    : maximiser le D+ avec L dans [L(1-tol), L(1+tol)].
    mode "target" : minimiser |L-L*|/L* + |D-D*|/D*, avec L dans [0.7 L*, 1.3 L*].
    """

    def __init__(self, g: Graph, s: int, L: float, mode: str = "max", tol: float = 0.05,
                 D: float | None = None, node_simple: bool = False,
                 exclude_parallel: bool = True):
        if mode not in ("max", "target"):
            raise ValueError(mode)
        if mode == "target" and not D:
            raise ValueError("D+ cible requis en mode target")
        self.g, self.s, self.L, self.mode, self.D = g, s, float(L), mode, D
        lo, hi = (1 - tol, 1 + tol) if mode == "max" else (0.7, 1.3)
        self.Lmin, self.Lmax = L * lo, L * hi
        m = len(g.edges)
        self.len = np.array([g.edges[i].length for i in range(m)])
        self.w = np.array([g.edges[i].w for i in range(m)])
        self.adj = g.adj
        self.nodes = list(g.adj)
        self.nxy = g.nxy
        # node_simple : chaque nœud hors du rayon libre est traversé au plus une fois.
        self.node_simple = node_simple
        c = self.nxy[s]
        self.far = {n for n in self.nodes if np.hypot(*(self.nxy[n] - c)) > FREE_RADIUS}
        # parallel[e] : tronçons du même couloir que e (au plus un des deux dans la boucle)
        self.parallel = parallel_pairs(g, c) if exclude_parallel and len(g.edges) else {}

    def parallel_ok(self, ids) -> bool:
        ids = set(ids)
        return not any(self.parallel.get(e, set()) & ids for e in ids)

    def node_simple_ok(self, circuit) -> bool:
        seen = set()
        for _, _, b in circuit:
            if b in self.far:
                if b in seen:
                    return False
                seen.add(b)
        return True

    def stats(self, ids):
        ids = list(ids)
        return float(self.len[ids].sum()), float(self.w[ids].sum())

    def err(self, length, dplus):
        return abs(length - self.L) / self.L + abs(dplus - self.D) / self.D

    def score(self, ids) -> tuple[float, float, float, bool]:
        """(score à maximiser, longueur, D+, réalisable)."""
        length, dplus = self.stats(ids)
        feas = self.Lmin <= length <= self.Lmax
        if self.mode == "max":
            viol = max(0.0, self.Lmin - length, length - self.Lmax)
            return dplus - 0.5 * viol, length, dplus, feas
        return -self.err(length, dplus), length, dplus, feas

    def dplus_upper_bound(self) -> float:
        """Sac à dos fractionnaire (w/l décroissant, capacité Lmax) : borne sur le D+."""
        order = np.argsort(-self.w / np.maximum(self.len, 1e-9))
        cum = np.cumsum(self.len[order])
        k = int(np.searchsorted(cum, self.Lmax))
        ub = float(self.w[order[:k]].sum())
        if k < len(order):
            rest = self.Lmax - (cum[k - 1] if k else 0.0)
            ub += float(self.w[order[k]] * rest / max(self.len[order[k]], 1e-9))
        return ub
