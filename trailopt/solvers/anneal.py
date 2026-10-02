"""Construction par waypoints + recuit simulé par remplacement de segment."""
from __future__ import annotations

import heapq
import math
import random
import time

import numpy as np

from ..graph import Problem, path_from

INF = float("inf")


def shortest(adj, cost, src, target=None, banned=None, banned_nodes=None):
    """Dijkstra sur ids de nœuds contigus (listes, pas de dicts) : le goulot du recuit.
    banned_nodes : nœuds interdits comme étapes intermédiaires (la cible reste permise)."""
    n = len(adj)
    dist = [INF] * n
    prev = [None] * n
    dist[src] = 0.0
    pq = [(0.0, src)]
    pop, push = heapq.heappop, heapq.heappush
    while pq:
        d, u = pop(pq)
        if d > dist[u]:
            continue
        if u == target:
            break
        for eid, nb in adj[u]:
            if banned is not None and eid in banned:
                continue
            if banned_nodes is not None and nb in banned_nodes and nb != target:
                continue
            nd = d + cost[eid]
            if nd < dist[nb]:
                dist[nb] = nd
                prev[nb] = (eid, u)
                push(pq, (nd, nb))
    return dist, prev


class Annealer:
    def __init__(self, P: Problem, seed: int = 0):
        self.P = P
        self.rng = random.Random(seed)
        self.nrng = np.random.default_rng(seed)
        g = P.w / np.maximum(P.len, 1.0)
        q = float(np.quantile(g, 0.95)) if len(g) else 1.0
        self.gn = g / max(q, 1e-6)
        # Ids de nœuds contigus 0..N-1 en interne ; reconvertis en sortie de run().
        idx = {n: i for i, n in enumerate(P.nodes)}
        self.adj = [[(eid, idx[nb]) for eid, nb in P.adj[n]] for n in P.nodes]
        self.s = idx[P.s]
        self.node_xy = np.array([P.nxy[n] for n in P.nodes])
        self.dist_s, _ = shortest(self.adj, P.len.tolist(), self.s)
        # Mode carrefours uniques : nœuds (ids internes) à ne traverser qu'une fois.
        self.far = {idx[n] for n in P.far} if P.node_simple else None
        self.par = P.parallel  # ids d'arêtes : inchangés en interne
        # α < 0 pousse vers le plat : utile en mode cible quand le D+ est trop haut.
        self.alpha_lo = 0.0 if P.mode == "max" else -0.95
        self.iterations = 0

    def cost(self, alpha, sigma):
        c = self.P.len * np.maximum(0.05, 1.0 - alpha * self.gn)
        if sigma > 0:
            c = c * np.exp(sigma * self.nrng.standard_normal(len(c)))
        return c.tolist()

    def with_parallel(self, eids):
        """Arêtes + leurs tronçons parallèles (à bannir ensemble)."""
        out = set(eids)
        for e in eids:
            out.update(self.par.get(e, ()))
        return out

    def evaluate(self, route):
        return self.P.score([e for e, _, _ in route])

    def construct(self):
        P, rng = self.P, self.rng
        cands = [n for n, d in enumerate(self.dist_s) if 0.12 * P.L <= d <= 0.42 * P.L]
        if not cands:
            cands = [n for n, d in enumerate(self.dist_s) if d < INF and n != self.s]
        if not cands:
            return None
        k = rng.choice([1, 2, 2, 3, 3, 4])
        wps = rng.sample(cands, min(k, len(cands)))
        xy = self.node_xy
        wps.sort(key=lambda n: math.atan2(xy[n][1], xy[n][0]),
                 reverse=rng.random() < 0.5)
        cost = self.cost(rng.uniform(self.alpha_lo, 0.95), 0.3)
        route, used, cur = [], set(), self.s
        far, visited = self.far, set()
        for t in wps + [self.s]:
            if far is not None and t in visited:
                continue  # waypoint déjà traversé
            dist, prev = shortest(self.adj, cost, cur, t, used, visited if far is not None else None)
            if dist[t] == INF:
                return None
            seg = path_from(prev, cur, t)
            route += seg
            used |= self.with_parallel(e for e, _, _ in seg)
            if far is not None:
                visited.update(b for _, _, b in seg if b in far)
            cur = t
        if self.par and not self.P.parallel_ok(e for e, _, _ in route):
            return None
        return route or None

    def random_node_near(self, c, r):
        d = np.hypot(self.node_xy[:, 0] - c[0], self.node_xy[:, 1] - c[1])
        idx = np.nonzero(d <= r)[0]
        if len(idx) == 0:
            return self.rng.randrange(len(self.node_xy))
        return int(idx[self.rng.randrange(len(idx))])

    def mutate(self, route, length):
        """Remplace route[i:j] par un plus court chemin a->b (direct ou via c) qui
        évite les arêtes du reste de la boucle : la boucle reste valide."""
        P, rng = self.P, self.rng
        k = len(route)
        nodes = [route[0][1]] + [b for _, _, b in route]
        seg = rng.randint(1, min(k, 25))
        i = rng.randint(0, k - seg)
        j = i + seg
        a, b = nodes[i], nodes[j]
        banned = self.with_parallel([e for e, _, _ in route[:i]] + [e for e, _, _ in route[j:]])
        bn = None
        if self.far is not None:  # nœuds du reste de la boucle : interdits en transit
            bn = {n for n in nodes[: i + 1] if n in self.far}
            bn.update(n for n in nodes[j:] if n in self.far)
        cost = self.cost(rng.uniform(self.alpha_lo, 0.95), rng.uniform(0, 0.5))
        lo, hi = (P.Lmin, P.Lmax) if P.mode == "max" else (0.97 * P.L, 1.03 * P.L)
        p_via = 0.75 if length < lo else (0.2 if length > hi else 0.45)
        if rng.random() < p_via:
            mid = (self.node_xy[a] + self.node_xy[b]) / 2
            c = self.random_node_near(mid, rng.uniform(80, max(100.0, 0.2 * P.L)))
            if bn is not None and c in bn:
                return None
            d1, p1 = shortest(self.adj, cost, a, c, banned, bn)
            if d1[c] == INF:
                return None
            leg1 = path_from(p1, a, c)
            bn2 = None if bn is None else bn | {x for _, _, x in leg1 if x in self.far} | (
                {a} if a in self.far else set())
            d2, p2 = shortest(self.adj, cost, c, b,
                              banned | self.with_parallel(e for e, _, _ in leg1), bn2)
            if d2[b] == INF:
                return None
            new = leg1 + path_from(p2, c, b)
        else:
            d, p = shortest(self.adj, cost, a, b, banned, bn)
            if d[b] == INF:
                return None
            new = path_from(p, a, b)
        if self.par:  # le nouveau segment ne doit pas longer son propre couloir
            ids = {e for e, _, _ in new}
            if any(self.par.get(e, set()) & ids for e in ids):
                return None
        return (route[:i] + new + route[j:]) or None

    def run(self, budget: float, cancel=None):
        """`cancel` (threading.Event) : arrêt anticipé, on rend le meilleur trouvé."""
        stop = cancel.is_set if cancel is not None else (lambda: False)
        """Renvoie la meilleure boucle (réalisable si possible) ou None."""
        P = self.P
        t0 = time.time()
        pool = []
        while time.time() - t0 < 0.15 * budget and len(pool) < 300 and not stop():
            r = self.construct()
            if r:
                pool.append((self.evaluate(r), r))
        if not pool:
            return None
        pool.sort(key=lambda x: x[0][0], reverse=True)
        (cur_sc, cur_len, cur_dp, _), cur = pool[0]
        best = (cur_sc, cur)
        best_feas = None
        for (sc, _, _, fe), r in pool:
            if fe and (best_feas is None or sc > best_feas[0]):
                best_feas = (sc, r)
        if P.mode == "max":
            T0, T1 = 0.05 * max(cur_dp, 10.0), 0.002 * max(cur_dp, 10.0)
        else:
            T0, T1 = 0.03, 0.001
        t1 = time.time()
        dur = max(0.0, budget - (t1 - t0))
        it = stall = 0
        while True:
            el = time.time() - t1
            if el >= dur or stop():
                break
            temp = T0 * (T1 / T0) ** (el / dur)
            it += 1
            stall += 1
            new = self.mutate(cur, cur_len)
            if new is None:
                continue
            sc, ln, dp, fe = self.evaluate(new)
            if sc >= cur_sc or self.rng.random() < math.exp((sc - cur_sc) / temp):
                cur, cur_sc, cur_len = new, sc, ln
                if sc > best[0]:
                    best, stall = (sc, new), 0
                if fe and (best_feas is None or sc > best_feas[0]):
                    best_feas, stall = (sc, new), 0
            if stall > 1500:  # retour au meilleur connu
                cur = best_feas[1] if best_feas else best[1]
                cur_sc, cur_len, _, _ = self.evaluate(cur)
                stall = 0
        self.iterations = it
        route = best_feas[1] if best_feas else best[1]
        nodes = P.nodes
        return [(e, nodes[a], nodes[b]) for e, a, b in route]
