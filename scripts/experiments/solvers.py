"""Prototypes de solveurs et de réductions pour grands graphes (aucune modification de trailopt/).

- Baseline   : trailopt.solvers.anneal.Annealer tel quel.
- AnnealPlus : même recuit, vecteurs de coûts tirés d'un petit stock précalculé, Dijkstra sur
               dictionnaires (pas d'allocation O(N) par appel), point de passage tiré par KD-tree
               dans un rayon plafonné.
- FaceLS     : recuit dans l'espace des cycles. Mouvement = différence symétrique de la boucle
               avec une face du graphe (îlot / maille). Parité conservée par construction, aucun
               Dijkstra ; delta de longueur et de D+ en O(taille de la face).
               Idée reprise de Gemsa, Pajor, Wagner, Zündorf, « Efficient Computation of Jogging
               Routes » (SEA 2013, « Greedy Faces »), ici en recuit et sans imposer un cycle simple.
"""
from __future__ import annotations

import heapq
import math
import random
import time

import numpy as np

import common as C
from trailopt.solvers import anneal as _an

INF = float("inf")


# ---------------------------------------------------------------------------
def run_baseline(g, L, budget, seed=0):
    P = C.to_problem(g, L)
    a = _an.Annealer(P, seed)
    t = time.time()
    r = a.run(budget)
    return r, dict(iters=a.iterations, wall=round(time.time() - t, 1))


def shortest_dict(adj, cost, src, target=None, banned=None, banned_nodes=None):
    """Dijkstra à dictionnaires : coût proportionnel à la zone explorée, pas à N."""
    dist = {src: 0.0}
    prev = {}
    pq = [(0.0, src)]
    pop, push = heapq.heappop, heapq.heappush
    get = dist.get
    while pq:
        d, u = pop(pq)
        if d > dist[u]:
            continue
        if u == target:
            break
        for eid, nb in adj[u]:
            if banned is not None and eid in banned:
                continue
            nd = d + cost[eid]
            if nd < get(nb, INF):
                dist[nb] = nd
                prev[nb] = (eid, u)
                push(pq, (nd, nb))
    return _D(dist), prev


class _D(dict):
    def __missing__(self, k):
        return INF


class AnnealPlus(_an.Annealer):
    POOL, VIA_CAP = 24, 3000.0

    def __init__(self, P, seed=0):
        super().__init__(P, seed)
        from scipy.spatial import cKDTree
        self.tree = cKDTree(self.node_xy)
        self.pool = [super(AnnealPlus, self).cost(a, s) for a, s in
                     [(self.rng.uniform(0, 0.95), self.rng.uniform(0, 0.5)) for _ in range(self.POOL)]]
        self.in_mutate = False

    def cost(self, alpha, sigma):
        return self.pool[self.rng.randrange(self.POOL)]

    def random_node_near(self, c, r):
        idx = self.tree.query_ball_point(c, min(r, self.VIA_CAP))
        if not idx:
            return int(self.tree.query(c)[1])
        return idx[self.rng.randrange(len(idx))]

    def mutate(self, route, length):
        _an.shortest = shortest_dict
        try:
            return super().mutate(route, length)
        finally:
            _an.shortest = _ORIG


_ORIG = _an.shortest


def run_plus(g, L, budget, seed=0):
    P = C.to_problem(g, L)
    a = AnnealPlus(P, seed)
    t = time.time()
    r = a.run(budget)
    return r, dict(iters=a.iterations, wall=round(time.time() - t, 1))


# ---------------------------------------------------------------------------
def euler(g, eids, s):
    """Hierholzer : circuit [(eid, de, vers)] depuis s."""
    adj = {}
    for e in eids:
        a, b = int(g.u[e]), int(g.v[e])
        adj.setdefault(a, []).append((e, b))
        if a != b:
            adj.setdefault(b, []).append((e, a))
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
    assert len(circuit) == len(set(eids)), "sous-graphe non eulérien ou non connexe"
    return circuit


class FaceLS:
    def __init__(self, g, L, tol=0.05, seed=0, max_face=None):
        self.g, self.L = g, L
        self.Lmin, self.Lmax = L * (1 - tol), L * (1 + tol)
        self.rng = random.Random(seed)
        t = time.time()
        self.F, self.foe, self.fstats = C.faces(g, max_face or min(0.2 * L, 15000.0))
        self.t_faces = time.time() - t
        self.len, self.w = g.len.tolist(), g.w.tolist()
        self.eu, self.ev = g.u.tolist(), g.v.tolist()
        self.adj = g.adj
        self.par = g.parallel

    def start_face(self):
        """Plus petite face passant par le départ (boucle initiale minimale)."""
        best = None
        for e, _ in self.adj[self.g.s]:
            for f in self.foe[e]:
                ln = sum(self.len[x] for x, _, _ in self.F[f])
                if best is None or ln < best[0]:
                    best = (ln, f)
        return [x for x, _, _ in self.F[best[1]]] if best else None

    def _connected(self, inX, deg, nact):
        s = self.g.s
        if deg[s] == 0:
            return False
        seen, st, adj = {s}, [s], self.adj
        while st:
            n = st.pop()
            for e, nb in adj[n]:
                if inX[e] and nb not in seen:
                    seen.add(nb)
                    st.append(nb)
        return len(seen) == nact

    def run(self, init, budget, T0=20.0, T1=0.3, pen=0.5, cancel=None):
        rng, F, foe, adj = self.rng, self.F, self.foe, self.adj
        ln, w, eu, ev, par = self.len, self.w, self.eu, self.ev, self.par
        Lmin, Lmax, s = self.Lmin, self.Lmax, self.g.s
        M = len(ln)
        inX = bytearray(M)
        deg = [0] * self.g.N
        X, pos = [], {}
        curL = curW = 0.0
        for e in init:
            inX[e] = 1
            pos[e] = len(X)
            X.append(e)
            deg[eu[e]] += 1
            deg[ev[e]] += 1
            curL += ln[e]
            curW += w[e]
        nact = sum(1 for d in deg if d)

        def score(L_, W_):
            return W_ - pen * (Lmin - L_ if L_ < Lmin else (L_ - Lmax if L_ > Lmax else 0.0))
        cur = score(curL, curW)
        best, bestX = (cur if Lmin <= curL <= Lmax else -INF), list(X)
        t0 = time.time()
        it = acc = full = 0
        rnd, exp = rng.random, math.exp
        temp = T0
        while True:
            if it & 255 == 0:
                el = time.time() - t0
                if el >= budget or (cancel is not None and cancel.is_set()):
                    break
                temp = T0 * (T1 / T0) ** (el / budget)
            it += 1
            e = X[int(rnd() * len(X))]
            n = eu[e] if rnd() < 0.5 else ev[e]
            an = adj[n]
            e2 = an[int(rnd() * len(an))][0]
            fl = foe[e2]
            if not fl:
                continue
            f = F[fl[int(rnd() * len(fl))]]
            dL = dW = 0.0
            k = 0
            for x, _, _ in f:
                if inX[x]:
                    k += 1
                    dL -= ln[x]
                    dW -= w[x]
                else:
                    dL += ln[x]
                    dW += w[x]
            new = score(curL + dL, curW + dW)
            if new < cur and rnd() >= exp((new - cur) / temp):
                continue
            nf = len(f)
            # couloirs parallèles : une arête ajoutée ne doit pas longer une arête qui reste / arrive
            if par:
                added = {x for x, _, _ in f if not inX[x]}
                bad = False
                for x in added:
                    ps = par.get(x)
                    if ps:
                        for p in ps:
                            if (inX[p] and not any(p == y for y, _, _ in f)) or p in added:
                                bad = True
                                break
                    if bad:
                        break
                if bad:
                    continue
            # connexité : cas rapide (une seule plage commune, nœuds intérieurs de degré 2)
            need_full = False
            if k == nf:
                need_full = True
            elif k:
                sh = [inX[x] for x, _, _ in f]
                runs = sum(1 for i in range(nf) if sh[i] and not sh[i - 1])
                if runs != 1:
                    need_full = True
                else:
                    for i in range(nf):
                        if sh[i] and sh[i - 1] and deg[f[i][1]] != 2:
                            need_full = True
                            break
            # application
            for x, a, b in f:
                if inX[x]:
                    inX[x] = 0
                    p = pos.pop(x)
                    lastx = X.pop()
                    if lastx != x:
                        X[p] = lastx
                        pos[lastx] = p
                    for q in (a, b):
                        deg[q] -= 1
                        if deg[q] == 0:
                            nact -= 1
                else:
                    inX[x] = 1
                    pos[x] = len(X)
                    X.append(x)
                    for q in (a, b):
                        if deg[q] == 0:
                            nact += 1
                        deg[q] += 1
            ok = deg[s] > 0 and bool(X)
            if ok and need_full:
                full += 1
                ok = self._connected(inX, deg, nact)
            if not ok:                      # annulation : la différence symétrique est involutive
                for x, a, b in f:
                    if inX[x]:
                        inX[x] = 0
                        p = pos.pop(x)
                        lastx = X.pop()
                        if lastx != x:
                            X[p] = lastx
                            pos[lastx] = p
                        for q in (a, b):
                            deg[q] -= 1
                            if deg[q] == 0:
                                nact -= 1
                    else:
                        inX[x] = 1
                        pos[x] = len(X)
                        X.append(x)
                        for q in (a, b):
                            if deg[q] == 0:
                                nact += 1
                            deg[q] += 1
                continue
            acc += 1
            curL += dL
            curW += dW
            cur = new
            if cur > best and Lmin <= curL <= Lmax:
                best, bestX = cur, list(X)
        self.stats = dict(iters=it, accepted=acc, full_checks=full, faces=len(F),
                          t_faces=round(self.t_faces, 2))
        return euler(self.g, bestX, s)


def run_faces(g, L, budget, seed=0, init="construct", init_share=0.15, **kw):
    """init : "construct" (meilleure boucle par waypoints de l'Annealer), "plus" (AnnealPlus
    pendant init_share du budget), "face" (plus petite face au départ)."""
    t = time.time()
    fl = FaceLS(g, L, seed=seed)
    P = C.to_problem(g, L)
    if init == "face":
        x0 = fl.start_face()
    elif init == "plus":
        a = AnnealPlus(P, seed)
        x0 = [e for e, _, _ in a.run(init_share * budget)]
    else:
        a = _an.Annealer(P, seed)
        pool, t1 = [], time.time()
        while (time.time() - t1 < init_share * budget and len(pool) < 40) or not pool:
            r = a.construct()
            if r:
                pool.append((a.evaluate(r)[0], r))
            if time.time() - t1 > budget:
                break
        x0 = [e for e, _, _ in max(pool, key=lambda x: x[0])[1]]
    t_init = time.time() - t
    r = fl.run(x0, max(1.0, budget - t_init), **kw)
    return r, dict(fl.stats, t_init=round(t_init, 1), wall=round(time.time() - t, 1))


# ---------------------------------------------------------------------------
# Réductions
# ---------------------------------------------------------------------------
def by_radius(g, R, Lmax):
    d = np.hypot(g.nxy[:, 0], g.nxy[:, 1])
    alive = (d[g.u] <= R) & (d[g.v] <= R)
    return g.sub(np.flatnonzero(C.prune_mask(g, alive, Lmax)))


def by_steep(g, Lmax, k=4.0, w=None, n_trees=2, penal=3.0):
    """HEURISTIQUE : garde les arêtes les plus pentues (w/l décroissant) jusqu'à k × Lmax de
    longueur cumulée, plus des connecteurs : pour chaque arête gardée, son chemin vers le départ
    dans `n_trees` arbres de plus courts chemins (le 2e pénalise les arêtes du 1er, pour offrir
    un aller et un retour distincts). Puis élagage sans perte habituel."""
    from scipy.sparse import coo_matrix
    from scipy.sparse.csgraph import dijkstra
    w = g.w if w is None else w
    ratio = w / np.maximum(g.len, 1.0)
    order = np.argsort(-ratio)
    cut = int(np.searchsorted(np.cumsum(g.len[order]), k * Lmax)) + 1
    keep = np.zeros(g.M, bool)
    keep[order[:cut]] = True
    steep = keep.copy()
    cost = g.len.copy()
    for _ in range(n_trees):
        a, b = np.minimum(g.u, g.v), np.maximum(g.u, g.v)
        o = np.lexsort((cost, b, a))
        first = np.concatenate([[True], (a[o][1:] != a[o][:-1]) | (b[o][1:] != b[o][:-1])]) & (a[o] != b[o])
        idx = o[first]
        A = coo_matrix((np.maximum(cost[idx], 1e-9), (a[idx], b[idx])), shape=(g.N, g.N)).tocsr()
        _, pred = dijkstra(A, directed=False, indices=g.s, return_predecessors=True)
        # arête (min coût) entre un nœud et son prédécesseur
        emap = {}
        for e in idx.tolist():
            emap[(int(a[e]), int(b[e]))] = e
        mark = np.zeros(g.N, bool)
        mark[g.s] = True
        pl = pred.tolist()
        tree_edges = []
        for n in np.unique(np.concatenate([g.u[steep], g.v[steep]])).tolist():
            while not mark[n] and pl[n] >= 0:
                mark[n] = True
                p = pl[n]
                tree_edges.append(emap[(n, p) if n < p else (p, n)])
                n = p
        keep[tree_edges] = True
        cost[tree_edges] *= penal
    return g.sub(np.flatnonzero(C.prune_mask(g, keep, Lmax)))


def lift(sub, route):
    """Boucle sur un sous-graphe -> ids d'arêtes du graphe parent."""
    return [int(sub.orig[e]) for e, _, _ in route]
