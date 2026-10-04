"""Parité des modes D19 (contracts/modes.md) : moteur Rust contre le prototype du lead algo.

Instances : pickles du prototype (scripts/experiments/algo/*.pkl, non versionnés), exportées en
JSON problem.md v2 (mode min_distance, q, climbs) puis résolues par `engine solve`. Chaque
boucle est revérifiée en Python (`check_loop`). Références : handoff 2026-10-03-algo-modes.md.

    PYTHONPATH=. .venv/bin/python scripts/parity_modes.py [--out DIR] [--time S] [--seeds N] [--tiles DIR]

Deuxième partie (T28, I2) : `engine plan` sur les dalles avec le filtre de pente PAR DÉFAUT de l'API
(60 % sur 50 m, D31) contre sans limite, distance max par défaut (selon le relief) : longueur, D+
et dépassement de X. Échec si `dplus_not_reached` avec le défaut (la distance max serait trop
serrée) ; le dépassement de X est rapporté, pas jugé (boucle la plus courte du réseau du départ).
"""
from __future__ import annotations

import argparse
import json
import pickle
import statistics
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, "scripts/experiments/algo")
sys.path.insert(0, "tests")
import climbs as C  # noqa: E402  (prototype, lecture seule)
from conftest import check_loop  # noqa: E402

from trailopt.graph import euler_circuit  # noqa: E402
from trailopt.solvers.rust import to_dict  # noqa: E402

ALGO = Path("scripts/experiments/algo")
ENGINE = Path("engine/target/release/engine")
# (instance, X, longueur de référence en m : optimum CP-SAT (Oisans) ou prototype (Massy))
CASES = [("alpes15", 500, 6260), ("alpes15", 1000, 9590), ("alpes15", 1400, 12870),
         ("massy15", 150, 6600), ("massy15", 300, 9100), ("massy15", 500, 12700)]
# (lat, lon, X) min_distance depuis les dalles : Massy (plaine), Bourg-d'Oisans (montagne)
PLAN_CASES = [(48.7309, 2.2713, x) for x in (150, 300, 500)] + [(45.0555, 6.0310, x) for x in (500, 1000, 1400)]
API_GRADE = 0.60  # défaut de l'API (api.md, D31)
# préférence de montées en mode max (Massy 10 km, Oisans 25 km) : effet sur la montée max
CLIMB_CASES = ["massy10", "alpes25"]


def knap_lb(P, X):
    """Longueur indicative du prototype (`as_minlen`) : sac à dos inverse seul."""
    import numpy as np
    r = P.w / np.maximum(P.len, 1e-9)
    o = np.argsort(-r)
    cw = np.cumsum(P.w[o])
    k = int(np.searchsorted(cw, X))
    prev = cw[k - 1] if k else 0.0
    return float(P.len[o[:k]].sum() + P.len[o[k]] * (X - prev) / max(P.w[o[k]], 1e-9))


def export(P, path, mode="max", X=None, gamma=0, q=None):
    d = to_dict(P)
    d["version"] = 2
    if mode == "min_distance":
        d.update(mode=mode, D=float(X), Lmin=0.0, L=min(P.Lmax, 2 * knap_lb(P, X)))
    if gamma:
        d.update(climbs=gamma, q=[float(x) for x in q])
    path.write_text(json.dumps(d))


def solve(path, time_s, seed):
    out = subprocess.run([str(ENGINE), "solve", "--problem", str(path), "--time", str(time_s),
                          "--seed", str(seed)], capture_output=True, text=True)
    r = json.loads(out.stdout)
    if "error" in r:
        raise RuntimeError(r["error"])
    return r


def plan_md(tiles, lat, lon, X, grade, out):
    req = dict(lat=lat, lon=lon, mode="min_distance", target_dplus=float(X), max_grade=grade,
               n_candidates=1, node_simple=True, max_compute_s=15.0)
    f = out / f"plan_{lat}_{lon}_{X}_{grade}.json"
    f.write_text(json.dumps(req))
    r = json.loads(subprocess.run([str(ENGINE), "plan", "--tiles", str(tiles), "--request", str(f)],
                                  capture_output=True, text=True).stdout)
    if "error" in r:
        return dict(error=r["error"]["code"])
    c = r["candidates"][0]
    return dict(L_km=round(c["length_m"] / 1000, 2), dplus=round(c["dplus_m"]),
                over_pct=round(100 * (c["dplus_m"] / X - 1), 1), lb_km=r["lower_bound_m"] and round(r["lower_bound_m"] / 1000, 2),
                grade_pct=c["max_grade_pct"], warnings=[w["code"] for w in r["warnings"]], compute_s=round(r["compute_s"], 1))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default="scripts/experiments/cto_modes")
    ap.add_argument("--time", type=float, default=20.0)  # budget « Python » par défaut du moteur (suggested_time)
    ap.add_argument("--seeds", type=int, default=3)
    ap.add_argument("--tiles", default="scripts/experiments/tiles_v1")
    a = ap.parse_args()
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    rows, ok = [], True
    for inst, X, ref in CASES:
        P = pickle.load(open(ALGO / f"{inst}.pkl", "rb"))
        f = out / f"{inst}_md{X}.json"
        export(P, f, "min_distance", X)
        Ls, ts = [], []
        for seed in range(a.seeds):
            r = solve(f, a.time, seed)
            L, W = check_loop(P, euler_circuit(P.g, r["edges"], P.s))
            assert W >= X - 1e-6 and L <= P.Lmax + 1e-6, (inst, X, L, W)
            Ls.append(L)
            ts.append(r["solve_s"])
        med = statistics.median(Ls)
        good = med <= 1.03 * ref
        ok &= good
        rows.append(dict(inst=inst, X=X, ref_m=ref, median_m=round(med), lengths=[round(x) for x in Ls],
                         solve_s=round(statistics.median(ts), 2), ok=good))
        print(rows[-1], flush=True)
    for inst in CLIMB_CASES:
        P = pickle.load(open(ALGO / f"{inst}.pkl", "rb"))
        q = C.edge_q(P)
        for name, g in (("balanced", 0), ("short", -1), ("long", 1)):
            f = out / f"{inst}_{name}.json"
            export(P, f, gamma=g, q=q)
            st = []
            for seed in range(a.seeds):
                r = solve(f, a.time, seed)
                c = C.orient(P, euler_circuit(P.g, r["edges"], P.s), g)
                L, W = check_loop(P, c)
                s = C.loop_stats(P, c)
                st.append((L, W, s["Gbar"], s["Gmax"]))
            m = [statistics.median(x) for x in zip(*st)]
            row = dict(inst=inst, climbs=name, L_km=round(m[0] / 1000, 2), dplus=round(m[1]),
                       gbar=round(m[2]), gmax=round(m[3]))
            rows.append(row)
            print(row, flush=True)
    if Path(a.tiles, "manifest.json").exists():
        for lat, lon, X in PLAN_CASES:
            row = dict(lat=lat, lon=lon, X=X)
            for name, g in (("default60", API_GRADE), ("nolimit", None)):
                row[name] = plan_md(a.tiles, lat, lon, X, g, out)
            ok &= "dplus_not_reached" not in row["default60"].get("warnings", ["error"])
            rows.append(row)
            print(row, flush=True)
    else:
        print(f"dalles absentes ({a.tiles}) : partie plan sautée", flush=True)
    (out / "parity_modes.json").write_text(json.dumps(rows, indent=1))
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
