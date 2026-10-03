"""optimize() : choix du solveur selon la taille du graphe, rapport de debug."""
from __future__ import annotations

import os
import time
from dataclasses import dataclass, field

from ..graph import Problem, euler_circuit
from . import exact
from .anneal import Annealer
from .faces import FaceSearch

# Seuil calibré par scripts/bench_exact.py (voir README).
EXACT_MAX_EDGES = 1000
# Au-delà d'EXACT_MAX_EDGES : recuit par faces. Jusqu'à FACES_ONLY_EDGES, il part de la solution
# du recuit classique (meilleur en réseau clairsemé, type montagne) ; au-delà, il travaille
# seul, avec départs multiples (le recuit classique se noie dans les grands graphes).
FACES_ONLY_EDGES = 5000
ANNEAL_SHARE = 0.6


@dataclass
class SolveResult:
    circuit: list            # [(eid, de, vers), ...] depuis s
    length: float            # m
    dplus: float             # m
    feasible: bool           # distance dans les bornes
    method: str
    debug: dict = field(default_factory=dict)
    alternatives: list = field(default_factory=list)   # autres boucles : (circuit, longueur, D+)


def optimize(P: Problem, budget: float = 20.0, exact_max_edges: int = EXACT_MAX_EDGES,
             seed: int = 0, workers: int = 2, solver: str = "auto", cancel=None,
             n_candidates: int = 1) -> SolveResult:
    """solver : "auto", "exact" (warm-start + CP-SAT), "anneal" (recuit classique seul),
    "faces" (recuit par faces seul) ou "rust" (moteur Rust, repli sur "auto" s'il échoue ;
    aussi activé en mode "auto" par TRAILOPT_SOLVER=rust)."""
    rust_err = None
    if solver == "rust" or (solver == "auto" and os.environ.get("TRAILOPT_SOLVER") == "rust"):
        from . import rust
        try:
            return rust.optimize(P, budget, seed, n_candidates, cancel)
        except Exception as ex:     # binaire absent, échec, sortie invalide : repli Python
            if cancel is not None and cancel.is_set():
                raise RuntimeError("calcul annulé") from None
            rust_err, solver = f"{type(ex).__name__}: {ex}"[:300], "auto"
    m = len(P.len)
    # Plusieurs candidats : chaque boucle en plus reçoit une demi-part du temps de la principale
    # (le budget conseillé grandit d'autant, voir pipeline.suggested_time).
    total_budget, budget = budget, budget / (1 + 0.5 * (max(1, n_candidates) - 1))
    fs = None
    ann = Annealer(P, seed=seed)
    use_exact = solver == "exact" or (solver == "auto" and m <= exact_max_edges)
    dbg = {"edges": m, "nodes": len(P.nodes), "exact_max_edges": exact_max_edges,
           "solver_reason": (f"{m} arêtes {'≤' if m <= exact_max_edges else '>'} "
                             f"seuil {exact_max_edges}") if solver == "auto" else f"forcé : {solver}"}
    if rust_err:
        dbg["rust_repli"] = rust_err
    t0 = time.time()
    if use_exact:
        th = max(1.0, 0.2 * budget)
        hroute = ann.run(th, cancel)
        dbg["anneal_iterations"] = ann.iterations
        dbg["anneal_time_s"] = round(time.time() - t0, 2)
        r = exact.solve(P, budget - th, hint=hroute, workers=workers, cancel=cancel)
        dbg.update(cpsat_status=r["status"], cpsat_bound=r["bound"],
                   cpsat_time_s=round(r["wall"], 2))
        h_sc = ann.evaluate(hroute) if hroute else None
        cands = []
        if r["chosen"]:
            cands.append((P.score(r["chosen"]), "cpsat", r["chosen"]))
        if h_sc:
            cands.append((h_sc, "anneal", hroute))
        if not cands:
            raise RuntimeError(f"aucune boucle trouvée (CP-SAT : {r['status']})")
        # CP-SAT gagne à égalité ; on préfère le réalisable.
        cands.sort(key=lambda c: (c[0][3], c[0][0], c[1] == "cpsat"), reverse=True)
        sc, who, sol = cands[0]
        if who == "cpsat":
            circuit = euler_circuit(P.g, sol, P.s)
            method = f"CP-SAT ({r['status']})"
        else:
            circuit = sol
            method = (f"recuit simulé (warm-start meilleur que CP-SAT {r['status']})"
                      if r["chosen"] else f"recuit simulé (CP-SAT : {r['status']})")
        dbg["solver"] = "CP-SAT"
    else:
        use_faces = solver == "faces" or solver == "auto"
        # Mode cible : le recuit par faces seul vise mal un couple (distance, D+) ; il ne fait
        # qu'affiner la solution du recuit classique, quelle que soit la taille du graphe.
        share = 1.0 if not use_faces else (
            ANNEAL_SHARE if (solver == "auto" and (m <= FACES_ONLY_EDGES or P.mode == "target"))
            else 0.0)
        circuit = ann.run(share * budget, cancel) if share > 0 else None
        dbg["anneal_iterations"] = ann.iterations
        dbg["solver"] = "recuit simulé"
        method = f"recuit simulé ({ann.iterations} itérations)"
        if use_faces:
            fs = FaceSearch(P, seed)
            c2, st = fs.solve(max(1.0, budget - (time.time() - t0)), cancel,
                              warm=[e for e, _, _ in circuit] if circuit else None)
            dbg["faces"] = st
            if c2 is not None:
                sc2 = P.score([e for e, _, _ in c2])
                sc1 = ann.evaluate(circuit) if circuit else None
                if sc1 is None or (sc2[3], sc2[0]) >= (sc1[3], sc1[0]):
                    circuit = c2
                    dbg["solver"] = "recuit par faces"
                    method = (f"recuit par faces ({st['iterations']} itérations, "
                              f"départ : {st['depart']})")
            if circuit is None:     # aucune face exploitable : recuit classique sur le temps restant
                circuit = ann.run(max(1.0, budget - (time.time() - t0)), cancel)
                method = f"recuit simulé ({ann.iterations} itérations)"
        if circuit is None:
            raise RuntimeError("aucune boucle trouvée")
    dbg["solve_time_s"] = round(time.time() - t0, 2)

    ids = [e for e, _, _ in circuit]
    assert len(ids) == len(set(ids)), "arête répétée"
    assert circuit[0][1] == P.s and circuit[-1][2] == P.s, "boucle non fermée"
    assert all(a[2] == b[1] for a, b in zip(circuit, circuit[1:])), "boucle discontinue"
    if P.node_simple:
        assert P.node_simple_ok(circuit), "carrefour repassé"
    assert P.parallel_ok(ids), "couloir parallèle emprunté deux fois"
    dbg["parallel_pairs"] = sum(len(v) for v in P.parallel.values()) // 2
    dbg["node_simple"] = P.node_simple
    _, length, dplus, feas = P.score(ids)
    if use_exact and dbg.get("cpsat_bound") is not None:
        b = dbg["cpsat_bound"]
        dbg["cpsat_gap"] = ((b - dplus) / max(b, 1e-9) if P.mode == "max"
                            else P.err(length, dplus) - b)
    alts = []
    if n_candidates > 1:
        fs = fs or FaceSearch(P, seed)
        for c in fs.alternates([ids], n_candidates - 1, max(1.0, total_budget - (time.time() - t0)), cancel):
            cids = [e for e, _, _ in c]
            assert len(cids) == len(set(cids)) and c[0][1] == P.s and c[-1][2] == P.s
            assert P.parallel_ok(cids) and (not P.node_simple or P.node_simple_ok(c))
            _, cl, cd, _ = P.score(cids)
            alts.append((c, cl, cd))
        dbg["candidates"] = [{"km": round(length / 1000, 2), "dplus": round(dplus)}] + [
            {"km": round(cl / 1000, 2), "dplus": round(cd), "commun_avec_1": round(fs.overlap(cids_, ids), 2)}
            for (c_, cl, cd) in alts for cids_ in [[e for e, _, _ in c_]]]
        dbg["candidates_essais"] = fs.alt_log
        dbg["solve_time_s"] = round(time.time() - t0, 2)
    return SolveResult(circuit, length, dplus, feas, method, dbg, alts)
