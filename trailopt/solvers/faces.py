"""Recuit « par faces » pour les grands graphes.

Idée (Gemsa, Pajor, Wagner, Zündorf, « Efficient Computation of Jogging Routes », SEA 2013) :
une boucle est un élément de l'espace des cycles. On la modifie par différence symétrique avec
une face du graphe (un « pâté de maisons ») : les degrés restent pairs par construction, sans
aucun plus court chemin. Un mouvement coûte O(taille de la face), donc le solveur ne se dégrade
pas quand la zone grandit, contrairement au recuit par remplacement de segment.

Départs multiples : plus petite face au départ, et couloirs aller-retour vers les secteurs les
plus denses en D+. Sans garantie d'optimalité, comme le recuit classique.
"""
from __future__ import annotations

import math
import random
import time

import numpy as np

from ..graph import Problem, euler_circuit, path_from
from .anneal import Annealer, shortest

INF = float("inf")
TARGET_SCALE = 1000.0   # mode cible : erreur relative -> unités comparables à des mètres de D+


def build_faces(u, v, ang_u, ang_v, length, max_len):
    """Faces du plongement donné par l'ordre circulaire des arêtes autour de chaque nœud.
    Chaque face est une marche fermée ; on garde les cycles simples de longueur <= max_len.
    Renvoie (faces = listes [(eid, de, vers)], faces de chaque arête)."""
    M = len(u)
    tail = np.empty(2 * M, np.int64)
    ang = np.empty(2 * M)
    tail[0::2], tail[1::2] = u, v
    ang[0::2], ang[1::2] = ang_u, ang_v
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
    seen = bytearray(2 * M)
    out, foe = [], [[] for _ in range(M)]
    for h0 in range(2 * M):
        if seen[h0]:
            continue
        walk, h, total = [], h0, 0.0
        while not seen[h]:
            seen[h] = 1
            walk.append(h)
            total += length[h >> 1]
            h = nf[h]
        if total > max_len:
            continue
        nodes = [tl[h] for h in walk]
        if len(set(nodes)) != len(nodes) or len({h >> 1 for h in walk}) != len(walk):
            continue                                  # marche non simple : ignorée
        fid = len(out)
        out.append([(h >> 1, tl[h], head[h]) for h in walk])
        for h in walk:
            foe[h >> 1].append(fid)
    return out, foe


class FaceSearch:
    def __init__(self, P: Problem, seed: int = 0):
        self.P = P
        self.rng = random.Random(seed)
        m = len(P.len)
        idx = {n: i for i, n in enumerate(P.nodes)}
        self.N = len(P.nodes)
        E = P.g.edges
        self.eu = [idx[E[e].u] for e in range(m)]
        self.ev = [idx[E[e].v] for e in range(m)]
        self.s = idx[P.s]
        self.len, self.w = P.len.tolist(), P.w.tolist()
        self.adj = [[] for _ in range(self.N)]
        for e in range(m):
            self.adj[self.eu[e]].append((e, self.ev[e]))
            if self.eu[e] != self.ev[e]:
                self.adj[self.ev[e]].append((e, self.eu[e]))
        d0 = np.array([E[e].xy[1] - E[e].xy[0] for e in range(m)])
        d1 = np.array([E[e].xy[-2] - E[e].xy[-1] for e in range(m)])
        self.F, self.foe = build_faces(np.array(self.eu), np.array(self.ev),
                                       np.arctan2(d0[:, 1], d0[:, 0]), np.arctan2(d1[:, 1], d1[:, 0]),
                                       self.len, P.Lmax)
        self.par = P.parallel
        self.far = {idx[n] for n in P.far} if P.node_simple else None
        self.nxy = np.array([P.nxy[n] for n in P.nodes])
        self.iterations = 0

    # ---------------------------------------------------------------- boucles initiales
    def start_face(self):
        """Plus petite face passant par le départ (boucle initiale minimale)."""
        best = None
        for e, _ in self.adj[self.s]:
            for f in self.foe[e]:
                ids = {x for x, _, _ in self.F[f]}
                if any(self.par.get(x, set()) & ids for x in ids):
                    continue        # face qui longe son propre couloir
                ln = sum(self.len[x] for x in ids)
                if best is None or ln < best[0]:
                    best = (ln, f)
        return [x for x, _, _ in self.F[best[1]]] if best else None

    def relief_targets(self, k=6, cell=1000.0, min_sep=3000.0):
        """Nœuds cibles : cellules de 1 km les plus denses en D+ (moyenne 3×3), atteignables
        (aller-retour <= 60 % de L), espacées d'au moins min_sep."""
        from scipy.sparse import coo_matrix
        from scipy.sparse.csgraph import dijkstra
        P, u, v = self.P, np.array(self.eu), np.array(self.ev)
        ln, w = P.len, P.w
        a, b = np.minimum(u, v), np.maximum(u, v)
        o = np.lexsort((ln, b, a))
        first = np.concatenate([[True], (a[o][1:] != a[o][:-1]) | (b[o][1:] != b[o][:-1])]) & (a[o] != b[o])
        i = o[first]
        A = coo_matrix((np.maximum(ln[i], 1e-9), (a[i], b[i])), shape=(self.N, self.N)).tocsr()
        d = dijkstra(A, directed=False, indices=self.s)
        xy = self.nxy - self.nxy[self.s]
        ci, cj = np.floor(xy[:, 0] / cell).astype(int), np.floor(xy[:, 1] / cell).astype(int)
        i0, j0 = ci.min(), cj.min()
        W = np.zeros((cj.max() - j0 + 1, ci.max() - i0 + 1))
        Ln = np.zeros_like(W)
        np.add.at(W, (cj[u] - j0, ci[u] - i0), w)
        np.add.at(Ln, (cj[u] - j0, ci[u] - i0), ln)

        def smooth(z):
            p = np.pad(z, 1)
            return sum(p[1 + dj: 1 + dj + z.shape[0], 1 + di: 1 + di + z.shape[1]]
                       for dj in (-1, 0, 1) for di in (-1, 0, 1))
        Ws, Ls = smooth(W), smooth(Ln)
        dens = np.where(Ls > 5 * cell, Ws / np.maximum(Ls, 1.0), 0.0)
        rep = {}
        for n in np.argsort(d).tolist():              # représentant d'une cellule : le plus proche du départ
            if np.isfinite(d[n]):
                rep.setdefault((int(cj[n] - j0), int(ci[n] - i0)), n)
        L = P.L
        cands = sorted((((L - 2 * d[n]) * dens[j, i], n) for (j, i), n in rep.items()
                        if 2 * d[n] <= 0.6 * L and dens[j, i] > 0), reverse=True)
        out = []
        for _, n in cands:
            if (np.hypot(*xy[n]) >= min_sep
                    and all(np.hypot(*(self.nxy[n] - self.nxy[q])) >= min_sep for q in out)):
                out.append(n)
                if len(out) == k:
                    break
        return out

    def corridor(self, t):
        """Aller-retour s -> t -> s par deux chemins sans arête commune ni couloir parallèle
        (ni carrefour commun en mode carrefours uniques)."""
        d1, p1 = shortest(self.adj, self.len, self.s, t)
        if d1[t] == INF:
            return None
        a = path_from(p1, self.s, t)
        banned = set()
        for e, _, _ in a:
            banned.add(e)
            banned.update(self.par.get(e, ()))
        bn = None
        if self.far is not None:
            bn = {x for _, _, x in a if x in self.far and x != t}
        d2, p2 = shortest(self.adj, self.len, t, self.s, banned, bn)
        if d2[self.s] == INF:
            return None
        ids = [e for e, _, _ in a] + [e for e, _, _ in path_from(p2, t, self.s)]
        return ids if self.P.parallel_ok(ids) else None

    # ---------------------------------------------------------------- recherche locale
    def run(self, init, budget, T0=20.0, T1=0.3, lam=0.02, pen=0.5, cancel=None):
        """Recuit depuis la boucle `init` (ids d'arêtes). Renvoie (ids de la meilleure boucle
        dans les bornes, ou de la dernière ; λ final).
        Mode max : score lagrangien D+ - λ·L, λ asservi pour que L reste dans [Lmin, Lmax] :
        la boucle ne grossit que par des faces assez denses en D+. Mode cible : -erreur."""
        P, rng, F, foe, adj = self.P, self.rng, self.F, self.foe, self.adj
        ln, w, eu, ev, par, far = self.len, self.w, self.eu, self.ev, self.par, self.far
        Lmin, Lmax, s, target = P.Lmin, P.Lmax, self.s, P.mode == "target"
        Lt, Dt = P.L, P.D
        inX = bytearray(len(ln))
        deg = [0] * self.N
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
        lam_ = lam
        Lmid = 0.5 * (Lmin + Lmax)

        def score(L_, W_):
            if target:      # erreur, plus une forte pénalité hors des bornes de distance
                out = Lmin - L_ if L_ < Lmin else (L_ - Lmax if L_ > Lmax else 0.0)
                return -TARGET_SCALE * (abs(L_ - Lt) / Lt + abs(W_ - Dt) / Dt + 3.0 * out / Lt)
            return W_ - lam_ * L_ - (pen * (L_ - Lmax) if L_ > Lmax else 0.0)

        def value(L_, W_):          # ce qu'on cherche à maximiser parmi les boucles dans les bornes
            return score(L_, W_) if target else W_

        def toggle(f):
            nonlocal nact
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

        def connected():
            seen, st = {s}, [s]
            while st:
                n = st.pop()
                for e, nb in adj[n]:
                    if inX[e] and nb not in seen:
                        seen.add(nb)
                        st.append(nb)
            return len(seen) == nact

        cur = score(curL, curW)
        ok0 = Lmin <= curL <= Lmax
        best, bestX = (value(curL, curW) if ok0 else -INF), list(X)
        t0 = time.time()
        it = 0
        rnd, exp = rng.random, math.exp
        temp = T0
        while X:
            if it & 255 == 0:
                el = time.time() - t0
                if el >= budget or (cancel is not None and cancel.is_set()):
                    break
                temp = T0 * (T1 / T0) ** (el / budget) if budget > 0 else T1
                if not target:
                    lam_ *= 1.01 if curL > Lmid else 0.99
                    cur = score(curL, curW)
            it += 1
            e = X[int(rnd() * len(X))]
            an = adj[eu[e] if rnd() < 0.5 else ev[e]]
            fl = foe[an[int(rnd() * len(an))][0]]
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
            if par:   # une arête ajoutée ne doit pas longer une arête qui reste ou qui arrive
                added = {x for x, _, _ in f if not inX[x]}
                fset = {x for x, _, _ in f}
                if any((p in added) or (inX[p] and p not in fset)
                       for x in added for p in par.get(x, ())):
                    continue
            # connexité : cas rapide (une seule plage commune, nœuds intérieurs de degré 2)
            need_full = k == nf
            if k and not need_full:
                sh = [inX[x] for x, _, _ in f]
                if sum(1 for i in range(nf) if sh[i] and not sh[i - 1]) != 1:
                    need_full = True
                else:
                    need_full = any(sh[i] and sh[i - 1] and deg[f[i][1]] != 2 for i in range(nf))
            toggle(f)
            ok = deg[s] > 0 and bool(X)
            if ok and far is not None:      # carrefours uniques : degré <= 2 hors du rayon libre
                ok = not any(deg[a] > 2 and a in far for _, a, _ in f)
            if ok and need_full:
                ok = connected()
            if not ok:
                toggle(f)                   # la différence symétrique est involutive
                continue
            curL += dL
            curW += dW
            cur = new
            if Lmin <= curL <= Lmax:
                val = value(curL, curW)
                if val > best:
                    best, bestX = val, list(X)
        self.iterations += it
        return (bestX if best > -INF else list(X)), lam_

    def solve(self, budget, cancel=None, k=6, probe=0.07, warm=None):
        """Sondes courtes depuis plusieurs boucles initiales, puis le reste du budget sur la
        meilleure. `warm` : boucle supplémentaire à essayer (ids d'arêtes).
        Renvoie (circuit [(eid, de, vers)], détail) ou (None, détail)."""
        P, t = self.P, time.time()
        inits = []
        if warm:
            inits.append(("recuit", list(warm)))
        f0 = self.start_face()
        if f0:
            inits.append(("face", f0))
        for n in self.relief_targets(k):
            c = self.corridor(n)
            if c:
                inits.append((f"couloir {np.hypot(*(self.nxy[n] - self.nxy[self.s])) / 1000:.1f} km", c))
        if not inits:           # réseau sans face simple au départ : boucle par waypoints
            a = Annealer(P, seed=self.rng.randrange(1 << 30))
            r = None
            while r is None and time.time() - t < 0.5 * budget:
                r = a.construct()
            if r is None:
                return None, {"faces": len(self.F), "depart": None}
            inits.append(("waypoints", [e for e, _, _ in r]))
        left = budget - (time.time() - t)
        res = []
        for name, x0 in inits:
            ids, lam = self.run(x0, probe * left if len(inits) > 1 else 0.0, T0=10.0, T1=1.0, cancel=cancel)
            sc = P.score(ids)
            res.append(((sc[3], sc[0]), name, ids, lam))
        res.sort(key=lambda x: x[0], reverse=True)
        _, name, ids, lam = res[0]
        ids, _ = self.run(ids, max(0.5, budget - (time.time() - t)), T0=3.0, T1=0.2, lam=lam, cancel=cancel)
        return euler_circuit(P.g, ids, P.s), {
            "faces": len(self.F), "depart": name, "departs_essayes": [n for _, n, _, _ in res],
            "iterations": self.iterations}
