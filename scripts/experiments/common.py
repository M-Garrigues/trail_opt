"""Briques communes des expériences « grandes zones ». Rien ici ne modifie trailopt/."""
from __future__ import annotations

import gzip
import json
import pickle
import re
import resource
import sys
import time
from pathlib import Path

import numpy as np

from trailopt import cache, elevation, graph, ign

OUT = Path("/tmp/tp_exp")
OUT.mkdir(exist_ok=True)
MASSY = (48.7303, 2.2725)
MASSY_FILE = "/tmp/trailopt_cache/ign/troncons_1.929_48.504_2.616_48.956.json.gz"


def rss_mb():
    r = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    return r / 2**20 if sys.platform == "darwin" else r / 1024


class Timer:
    def __init__(self):
        self.t, self.T = time.time(), {}

    def lap(self, name):
        now = time.time()
        self.T[name] = round(now - self.t, 2)
        self.t = now


def load_feats(path=MASSY_FILE):
    return json.loads(gzip.decompress(Path(path).read_bytes()))


# ---------------------------------------------------------------------------
# Préparation rapide : géométrie 3D BD TOPO, aucune densification, aucun appel d'altitude
# ---------------------------------------------------------------------------
def fast_raw(feats, frame, roads="minor", radius=None, keep_fn=None):
    """Tronçons -> (u, v, xyz local, flat). Projection vectorisée (un seul appel pyproj),
    découpe au disque `radius` à la granularité du sommet."""
    keep_fn = keep_fn or (lambda p: ign.keep(p, roads))
    kept = [f for f in feats if len(f["c"]) >= 2 and keep_fn(f["p"])]
    n = np.fromiter((len(f["c"]) for f in kept), int, len(kept))
    C = np.array([q for f in kept for q in f["c"]], float)
    xy = frame.to_local(C[:, 0], C[:, 1])
    xyz = np.column_stack([xy, C[:, 2]])
    off = np.concatenate([[0], np.cumsum(n)])
    # nœuds = extrémités, identifiées par lon/lat arrondis à 1e-7
    ends = np.concatenate([off[:-1], off[1:] - 1])
    key = np.round(C[ends, :2] * 1e7).astype(np.int64)
    _, inv = np.unique(key[:, 0] * (1 << 32) + key[:, 1], return_inverse=True)
    m = len(kept)
    U, V = inv[:m], inv[m:]
    flat = np.fromiter((f["p"].get("position_par_rapport_au_sol") not in (None, "0") for f in kept),
                       bool, m)
    if radius is None:
        inside = np.ones(len(xyz), bool)
    else:
        inside = np.hypot(xyz[:, 0], xyz[:, 1]) <= radius
    all_in = np.logical_and.reduceat(inside, off[:-1])
    any_in = np.logical_or.reduceat(inside, off[:-1])
    raw, cut = [], -100
    for i in np.flatnonzero(any_in):
        a, b = off[i], off[i + 1]
        if all_in[i]:
            raw.append((int(U[i]), int(V[i]), xyz[a:b], bool(flat[i])))
            continue
        mk = inside[a:b]
        ed = np.flatnonzero(np.diff(np.concatenate([[False], mk, [False]]).astype(int)))
        for p, q in zip(ed[::2], ed[1::2]):
            if q - p < 2:
                continue
            nu = int(U[i]) if p == 0 else cut
            cut -= p != 0
            nv = int(V[i]) if q == b - a else cut
            cut -= q != b - a
            raw.append((nu, nv, xyz[a + p:a + q], bool(flat[i])))
    return raw, m


def bad_z(z):
    return ~np.isfinite(z) | (z < -100) | (z > 5000)


def topo_w(xyz, flat):
    """w = (montée + descente)/2 d'après les Z des sommets BD TOPO."""
    z = xyz[:, 2]
    b = bad_z(z)
    if b.all():
        return 0.0
    if b.any():
        s = np.concatenate([[0], np.cumsum(np.hypot(*np.diff(xyz[:, :2], axis=0).T))])
        z = np.interp(s, s[~b], z[~b])
    if flat:
        return abs(z[-1] - z[0]) / 2
    return float(np.abs(np.diff(z)).sum() / 2)


class G:
    """Graphe compact en tableaux. Nœuds 0..N-1, arêtes 0..M-1."""

    def __init__(self, u, v, ln, w, s, geom=None, parallel=None, twin=None):
        self.u, self.v = np.asarray(u, np.int64), np.asarray(v, np.int64)
        self.len, self.w = np.asarray(ln, float), np.asarray(w, float)
        self.s, self.geom = int(s), geom
        self.parallel = parallel or {}
        self.twin = twin
        self.N = int(max(self.u.max(), self.v.max())) + 1
        self.M = len(self.u)
        self._adj = None

    @property
    def adj(self):
        if self._adj is None:
            adj = [[] for _ in range(self.N)]
            for e, (a, b) in enumerate(zip(self.u.tolist(), self.v.tolist())):
                adj[a].append((e, b))
                if a != b:
                    adj[b].append((e, a))
            self._adj = adj
        return self._adj

    def sub(self, keep):
        """Sous-graphe (indices d'arêtes), nœuds renumérotés. Renvoie (G, indices d'origine)."""
        keep = np.asarray(sorted(keep), np.int64)
        nodes, inv = np.unique(np.concatenate([self.u[keep], self.v[keep]]), return_inverse=True)
        k = len(keep)
        new = {int(o): i for i, o in enumerate(keep)}
        par = {}
        for e, o in enumerate(keep.tolist()):
            ps = self.parallel.get(o)
            if ps:
                q = {new[x] for x in ps if x in new}
                if q:
                    par[e] = q
        s = int(np.searchsorted(nodes, self.s))
        assert nodes[s] == self.s, "départ hors du sous-graphe"
        g = G(inv[:k], inv[k:], self.len[keep], self.w[keep], s, None, par)
        g.nxy = self.nxy[nodes]
        g.orig = keep
        if hasattr(self, "ang_u"):
            g.ang_u, g.ang_v = self.ang_u[keep], self.ang_v[keep]
        return g


def sssp(g: G, weights=None, src=None, limit=np.inf):
    """Plus courts chemins depuis src (scipy, C)."""
    from scipy.sparse import coo_matrix
    from scipy.sparse.csgraph import dijkstra
    w = g.len if weights is None else weights
    # multi-arêtes : garder le minimum (coo -> csr additionne, on dédoublonne avant)
    a, b = np.minimum(g.u, g.v), np.maximum(g.u, g.v)
    order = np.lexsort((w, b, a))
    a, b, ww = a[order], b[order], w[order]
    first = np.concatenate([[True], (a[1:] != a[:-1]) | (b[1:] != b[:-1])]) & (a != b)
    A = coo_matrix((np.maximum(ww[first], 1e-9), (a[first], b[first])), shape=(g.N, g.N)).tocsr()
    return dijkstra(A, directed=False, indices=g.s if src is None else src, limit=limit)


def bridges(g: G, alive):
    """Ponts (Tarjan itératif) du sous-graphe des arêtes `alive` (masque booléen)."""
    adj = g.adj
    disc = [-1] * g.N
    low = [0] * g.N
    out, t = [], 0
    al = alive.tolist()
    for root in range(g.N):
        if disc[root] >= 0 or not adj[root]:
            continue
        disc[root] = low[root] = t
        t += 1
        stack = [(root, -1, iter(adj[root]))]
        while stack:
            node, pe, it = stack[-1]
            pushed = False
            for eid, nb in it:
                if not al[eid] or eid == pe or nb == node:
                    continue
                if disc[nb] >= 0:
                    if disc[nb] < low[node]:
                        low[node] = disc[nb]
                else:
                    disc[nb] = low[nb] = t
                    t += 1
                    stack.append((nb, eid, iter(adj[nb])))
                    pushed = True
                    break
            if not pushed:
                stack.pop()
                if stack:
                    parent = stack[-1][0]
                    if low[node] < low[parent]:
                        low[parent] = low[node]
                    if low[node] > disc[parent]:
                        out.append(pe)
    return out


def prune_mask(g: G, alive, Lmax):
    """Élagage sans perte (ponts, composante de s, portée) restreint à `alive`. Renvoie un masque."""
    from scipy.sparse import coo_matrix
    from scipy.sparse.csgraph import connected_components, dijkstra
    alive = alive.copy()
    while True:
        m0 = int(alive.sum())
        br = bridges(g, alive)
        alive[br] = False
        idx = np.flatnonzero(alive)
        if len(idx) == 0:
            return alive
        A = coo_matrix((np.maximum(g.len[idx], 1e-9), (g.u[idx], g.v[idx])), shape=(g.N, g.N))
        A = A.tocsr()
        A.data[:] = 1  # connexité seulement
        _, lab = connected_components(A, directed=False)
        alive &= lab[g.u] == lab[g.s]
        idx = np.flatnonzero(alive)
        sub = G.__new__(G)
        sub.u, sub.v, sub.len, sub.N, sub.s = g.u[idx], g.v[idx], g.len[idx], g.N, g.s
        d = sssp(sub)
        ok = d[g.u[idx]] + g.len[idx] + d[g.v[idx]] <= Lmax
        alive[idx[~ok]] = False
        if int(alive.sum()) == m0:
            return alive


def load_prod(name="massy25", clean=True) -> G:
    """Graphe de production sauvé par build_prod.py -> G (w LiDAR, parallèles, géométrie)."""
    d = pickle.load(open(OUT / f"{name}.pkl", "rb"))
    d["w_raw"] = d["w"]
    if clean:
        d["w"], d["z"], dirty = clean_w(d["off"], d["xy"], d["z"], d["w"], d["flat"])
        print(f"[clean] {len(dirty)} arêtes au profil LiDAR aberrant : w {d['w_raw'][dirty].sum():.0f} -> "
              f"{d['w'][dirty].sum():.0f} m (total {d['w_raw'].sum():.0f} -> {d['w'].sum():.0f})")
    nodes, inv = np.unique(np.concatenate([d["u"], d["v"]]), return_inverse=True)
    m = len(d["u"])
    s = int(np.searchsorted(nodes, d["s"]))
    g = G(inv[:m], inv[m:], d["len"], d["w"], s, (d["off"], d["xy"], d["z"]), d["parallel"], d["twin"])
    nxy = np.zeros((g.N, 2))
    nxy[g.u] = d["xy"][d["off"][:-1]]
    nxy[g.v] = d["xy"][d["off"][1:] - 1]
    g.nxy = nxy
    g.ang_u, g.ang_v = edge_angles(d["off"], d["xy"])
    g.w_raw = d["w_raw"]
    g.L = d["L"]
    g.meta = {k: d[k] for k in ("ways", "dbg", "info", "T", "net")}
    return g


def to_problem(g: G, L, tol=0.05):
    """G -> objet minimal compatible avec trailopt.solvers.anneal.Annealer (mode max)."""
    class P:
        pass
    P = P()
    P.s, P.L, P.mode, P.D = g.s, float(L), "max", None
    P.Lmin, P.Lmax = L * (1 - tol), L * (1 + tol)
    P.len, P.w = g.len, g.w
    P.nodes = list(range(g.N))
    P.adj = g.adj
    P.nxy = g.nxy
    P.node_simple, P.far = False, set()
    P.parallel = g.parallel

    def score(ids):
        ids = list(ids)
        length, dplus = float(P.len[ids].sum()), float(P.w[ids].sum())
        viol = max(0.0, P.Lmin - length, length - P.Lmax)
        return dplus - 0.5 * viol, length, dplus, P.Lmin <= length <= P.Lmax
    P.score = score
    P.parallel_ok = lambda ids: (lambda S: not any(P.parallel.get(e, set()) & S for e in S))(set(ids))
    return P


def check_route(g: G, route):
    ids = [e for e, _, _ in route]
    assert len(ids) == len(set(ids)), "arête répétée"
    assert route[0][1] == g.s and route[-1][2] == g.s, "boucle non fermée"
    assert all(a[2] == b[1] for a, b in zip(route, route[1:])), "discontinue"
    S = set(ids)
    assert not any(g.parallel.get(e, set()) & S for e in S), "couloir parallèle"
    return float(g.len[ids].sum()), float(g.w[ids].sum())


# ---------------------------------------------------------------------------
# Altitude depuis les dalles DÉJÀ en cache (aucune requête réseau)
# ---------------------------------------------------------------------------
class CachedDEM:
    def __init__(self):
        self.tiles = []
        for layer in elevation.LAYERS:
            lst = []
            for p in (cache.CACHE_DIR / "dem").glob(f"{layer[:40]}_*.tif.gz"):
                bb = tuple(float(x) for x in re.search(r"_(-?\d+)_(-?\d+)_(-?\d+)_(-?\d+)\.tif", p.name).groups())
                lst.append((bb, p))
            lst.sort(key=lambda t: -(t[0][2] - t[0][0]) * (t[0][3] - t[0][1]))  # grandes dalles d'abord
            self.tiles.append(lst)

    def __call__(self, X, Y):
        z = np.full(len(X), np.nan)
        m = elevation.MARGIN_PX * elevation.RES
        for lst in self.tiles:
            for bb, p in lst:
                todo = np.flatnonzero(np.isnan(z))
                if len(todo) == 0:
                    return z
                x, y = X[todo], Y[todo]
                sel = todo[(x >= bb[0] + m) & (x < bb[2] - m) & (y >= bb[1] + m) & (y < bb[3] - m)]
                if len(sel):
                    z[sel] = elevation.bilinear(elevation._read(p), bb, X[sel], Y[sel])
        return z


def cached_sampler(frame):
    dem = CachedDEM()

    def sample(xy):
        X, Y = frame.to_l93(xy)
        return dem(np.asarray(X), np.asarray(Y))
    return sample


# ---------------------------------------------------------------------------
# Nettoyage des artefacts LiDAR (pixels contaminés par le nodata côté serveur : z = -400..150
# au milieu d'un plateau à 177 m). Sans cela le maximiseur de D+ est attiré par des arêtes
# au D+ fictif (jusqu'à 2 800 m sur 500 m).
# ---------------------------------------------------------------------------
def clean_w(off, xy, z, w, flat, max_step_grade=1.0):
    """Recalcule w pour les arêtes dont le profil a un saut > 100 % entre deux points (5 m).
    Points fautifs = extrémités des sauts + tout ce qui est plus bas que le reste ; interpolés."""
    z = z.astype(float)
    ds = np.hypot(*np.diff(xy.astype(float), axis=0).T)
    dz = np.abs(np.diff(z))
    jump = dz > np.maximum(3.0, max_step_grade * ds)
    jump[off[1:-1] - 1] = False                      # pas entre deux arêtes
    eid = np.searchsorted(off, np.flatnonzero(jump), side="right") - 1
    w = w.copy()
    dirty = np.unique(eid)
    for e in dirty:
        a, b = off[e], off[e + 1]
        ze = z[a:b].copy()
        j = np.flatnonzero(jump[a:b - 1])
        bad = np.zeros(b - a, bool)
        bad[j] = bad[j + 1] = True
        if (~bad).any():
            ref = np.median(ze[~bad])
            bad |= np.abs(ze - ref) > 3 * (np.abs(ze[~bad] - ref).max() + 5.0)
            # entre deux sauts, un palier franchement plus bas que le reste est fautif
            lo = ze < ze[~bad].min() - 1.0
            bad |= lo
        if bad.all():
            w[e] = 0.0
            continue
        s = np.concatenate([[0], np.cumsum(ds[a:b - 1])])
        ze[bad] = np.interp(s[bad], s[~bad], ze[~bad])
        w[e] = abs(ze[-1] - ze[0]) / 2 if flat[e] else np.abs(np.diff(ze)).sum() / 2
        z[a:b] = ze
    return w, z, dirty


def edge_angles(off, xy):
    """Angle de départ de chaque arête à son nœud u et à son nœud v (pour l'ordre circulaire)."""
    a, b = off[:-1], off[1:] - 1
    d0 = xy[a + 1].astype(float) - xy[a]
    d1 = xy[b - 1].astype(float) - xy[b]
    return np.arctan2(d0[:, 1], d0[:, 0]), np.arctan2(d1[:, 1], d1[:, 0])


def faces(g: G, max_len):
    """Faces du plongement (ordre circulaire des arêtes à chaque nœud). Chaque face est une
    marche fermée ; on garde les cycles simples de longueur <= max_len.
    Renvoie (liste de faces = listes [(eid, de, vers)], faces_of_edge = liste de listes)."""
    M = g.M
    tail = np.empty(2 * M, np.int64)
    ang = np.empty(2 * M)
    tail[0::2], tail[1::2] = g.u, g.v
    ang[0::2], ang[1::2] = g.ang_u, g.ang_v
    order = np.lexsort((ang, tail))
    t = tail[order]
    first = np.concatenate([[True], t[1:] != t[:-1]])
    start = np.maximum.accumulate(np.where(first, np.arange(2 * M), 0))
    last = np.concatenate([first[1:], [True]])
    nxt_pos = np.where(last, start, np.arange(2 * M) + 1)
    nxt_out = np.empty(2 * M, np.int64)
    nxt_out[order] = order[nxt_pos]
    nf = nxt_out[np.arange(2 * M) ^ 1].tolist()      # demi-arête suivante sur la face
    head = tail[np.arange(2 * M) ^ 1].tolist()
    tl = tail.tolist()
    ln = g.len.tolist()
    seen = bytearray(2 * M)
    out, foe = [], [[] for _ in range(M)]
    stats = dict(total=0, too_long=0, non_simple=0)
    for h0 in range(2 * M):
        if seen[h0]:
            continue
        walk, h, L = [], h0, 0.0
        while not seen[h]:
            seen[h] = 1
            walk.append(h)
            L += ln[h >> 1]
            h = nf[h]
        stats["total"] += 1
        if L > max_len:
            stats["too_long"] += 1
            continue
        nodes = [tl[h] for h in walk]
        if len(set(nodes)) != len(nodes) or len({h >> 1 for h in walk}) != len(walk):
            stats["non_simple"] += 1
            continue
        fid = len(out)
        out.append([(h >> 1, tl[h], head[h]) for h in walk])
        for h in walk:
            foe[h >> 1].append(fid)
    return out, foe, stats
