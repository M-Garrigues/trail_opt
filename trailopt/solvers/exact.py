"""Modèle CP-SAT exact : sous-graphe eulérien connexe contenant s."""
from __future__ import annotations

import math
import threading

from ..graph import Problem

OBJ_SCALE = 1e9  # mode cible : erreur relative * OBJ_SCALE (entiers)


def solve(P: Problem, budget: float, hint=None, workers: int = 2, cancel=None):
    """Renvoie dict(chosen, status, bound, wall) ; chosen=None si rien trouvé.

    bound : borne supérieure du D+ (mode max) ou borne inférieure de l'erreur (cible).
    """
    from ortools.sat.python import cp_model

    m, nodes, N = len(P.len), P.nodes, len(P.nodes)
    mdl = cp_model.CpModel()
    x = [mdl.NewBoolVar(f"x{e}") for e in range(m)]
    z = {v: mdl.NewBoolVar(f"z{v}") for v in nodes}
    mdl.Add(z[P.s] == 1)
    inc = {v: ([], []) for v in nodes}
    fin = {v: [] for v in nodes}
    fout = {v: [] for v in nodes}
    flow = {}
    for e in range(m):
        u, v = P.g.edges[e].u, P.g.edges[e].v
        mdl.AddImplication(x[e], z[u])
        mdl.AddImplication(x[e], z[v])
        if u == v:
            inc[u][0].append(x[e])
            inc[u][1].append(2)
            continue
        inc[u][0].append(x[e])
        inc[u][1].append(1)
        inc[v][0].append(x[e])
        inc[v][1].append(1)
        f1 = mdl.NewIntVar(0, N - 1, "")
        f2 = mdl.NewIntVar(0, N - 1, "")
        mdl.Add(f1 + f2 <= (N - 1) * x[e])
        fout[u].append(f1)
        fin[v].append(f1)
        fout[v].append(f2)
        fin[u].append(f2)
        flow[e] = (f1, f2)
    # Bris de symétrie : une copie (rayon libre) n'est prise que si l'original l'est.
    pos = {id(P.g.edges[e]): e for e in range(m)}
    for e in range(m):
        t = P.g.edges[e].twin
        if t is not None and id(t) in pos:
            mdl.AddImplication(x[e], x[pos[id(t)]])
    # Couloirs parallèles : au plus un des deux tronçons.
    for e1, others in P.parallel.items():
        for e2 in others:
            if e1 < e2:
                mdl.Add(x[e1] + x[e2] <= 1)
    kvar, kcap = {}, {}
    for v in nodes:
        vs, cs = inc[v]
        deg = cp_model.LinearExpr.WeightedSum(vs, cs)
        # carrefours uniques : degré <= 2 hors du rayon libre autour du départ
        kmax = 1 if (P.node_simple and v in P.far) else sum(cs) // 2
        kv = mdl.NewIntVar(0, kmax, "")
        kvar[v], kcap[v] = kv, kmax
        mdl.Add(deg == 2 * kv)                       # parité
        mdl.Add(deg >= 2).OnlyEnforceIf(z[v])
        if v != P.s:
            mdl.Add(sum(fin[v]) - sum(fout[v]) == z[v])   # connexité par flot

    Ld = [int(round(10 * l)) for l in P.len]     # décimètres
    Wc = [int(round(100 * w)) for w in P.w]      # centimètres
    tot = cp_model.LinearExpr.WeightedSum(x, Ld)
    dplus = cp_model.LinearExpr.WeightedSum(x, Wc)
    mdl.Add(tot >= int(math.ceil(10 * P.Lmin)))
    mdl.Add(tot <= int(math.floor(10 * P.Lmax)))
    if P.mode == "max":
        mdl.Maximize(dplus)
    else:
        Lt, Dt = int(round(10 * P.L)), int(round(100 * P.D))
        dL = mdl.NewIntVar(-sum(Ld), sum(Ld), "dL")
        dD = mdl.NewIntVar(-max(sum(Wc), Dt), max(sum(Wc), Dt), "dD")
        aL = mdl.NewIntVar(0, max(sum(Ld), Lt), "aL")
        aD = mdl.NewIntVar(0, max(sum(Wc), Dt), "aD")
        mdl.Add(dL == tot - Lt)
        mdl.Add(dD == dplus - Dt)
        mdl.AddAbsEquality(aL, dL)
        mdl.AddAbsEquality(aD, dD)
        cL, cD = round(OBJ_SCALE / Lt), round(OBJ_SCALE / Dt)
        mdl.Minimize(cL * aL + cD * aD)

    if hint:
        used = {e for e, _, _ in hint}
        for e in list(used):  # copie sans son original : échange (arêtes identiques)
            t = P.g.edges[e].twin
            if t is not None and id(t) in pos and pos[id(t)] not in used:
                used.discard(e)
                used.add(pos[id(t)])
        for e in range(m):
            mdl.AddHint(x[e], e in used)
        deg = {v: 0 for v in nodes}
        for e in used:
            ed = P.g.edges[e]
            deg[ed.u] += 1
            deg[ed.v] += 1
        for v in nodes:
            mdl.AddHint(z[v], deg[v] > 0 or v == P.s)
            mdl.AddHint(kvar[v], min(deg[v] // 2, kcap[v]))
        # flot cohérent : arbre BFS depuis s, flot = taille du sous-arbre
        adj = {}
        for e in used:
            ed = P.g.edges[e]
            if ed.u != ed.v:
                adj.setdefault(ed.u, []).append((e, ed.v))
                adj.setdefault(ed.v, []).append((e, ed.u))
        order, parent, seen = [P.s], {}, {P.s}
        for n in order:
            for e, nb in adj.get(n, ()):
                if nb not in seen:
                    seen.add(nb)
                    parent[nb] = (e, n)
                    order.append(nb)
        size = {n: 1 for n in order}
        for n in reversed(order[1:]):
            size[parent[n][1]] += size[n]
        fval = {}
        for n in order[1:]:
            e, p = parent[n]
            fval[(e, 0 if P.g.edges[e].u == p else 1)] = size[n]
        for e, (f1, f2) in flow.items():
            mdl.AddHint(f1, fval.get((e, 0), 0))
            mdl.AddHint(f2, fval.get((e, 1), 0))

    solver = cp_model.CpSolver()
    solver.parameters.max_time_in_seconds = max(1.0, budget)
    solver.parameters.num_workers = workers
    done = threading.Event()
    if cancel is not None:   # annulation : un guetteur arrête la recherche CP-SAT
        def watch():
            while not done.wait(0.2):
                if cancel.is_set():
                    solver.StopSearch()
                    return
        threading.Thread(target=watch, daemon=True).start()
    st = solver.Solve(mdl)
    done.set()
    name = solver.StatusName(st)
    out = {"chosen": None, "status": name, "bound": None, "wall": solver.WallTime()}
    if st in (cp_model.OPTIMAL, cp_model.FEASIBLE):
        out["chosen"] = [e for e in range(m) if solver.Value(x[e])]
        b = solver.BestObjectiveBound()
        out["bound"] = b / 100.0 if P.mode == "max" else b / OBJ_SCALE
    return out
