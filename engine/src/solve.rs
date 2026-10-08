//! Routage des solveurs selon la taille du graphe et le mode (port de `solvers.optimize`).
//! Pas de CP-SAT (décision D6) : sur les petits graphes, le recuit classique puis les faces
//! qui partent de son résultat le remplacent.
use std::time::Instant;

use rayon::prelude::*;
use serde_json::json;

use crate::codes::{Code, Msg};
use crate::faces::key_cmp;
use crate::problem::length_lower_bound;
use crate::{Annealer, FaceSearch, Problem};

/// Jusqu'ici (ou en mode cible), recuit classique d'abord, puis faces depuis son résultat ;
/// au-delà, faces seules (le recuit classique se noie dans les grands graphes).
pub const FACES_ONLY_EDGES: usize = 5000;
/// Recherches indépendantes sur les petits graphes (voir `optimize`).
const RESTARTS: u64 = 2;
/// Voir `search` : part des itérations de faces faite sans la prime de type de voie.
const SURF_FIRST: f64 = 0.6;
/// Mode cible : écart relatif max (distance ou D+) de la sortie avant la recherche de secours sans
/// confort (`optimize_on`, D63).
const TARGET_KEEP: f64 = 0.05;

pub struct Budget {
    /// Itérations du recuit par faces (boucle principale).
    pub iters: u64,
    /// Itérations (mutations) du recuit classique.
    pub anneal_iters: u64,
    pub seed: u64,
    /// Nombre de boucles voulues ; chaque boucle en plus reçoit une demi-part de `iters`.
    pub candidates: usize,
    /// Plafond de temps, sécurité seulement (le calcul n'est plus déterministe s'il est atteint).
    pub deadline: Option<Instant>,
}

/// Itérations par seconde de budget Python (faces, recuit classique), calibrées sur Massy.
const FACES_PER_S: f64 = 3e5;
/// Recuit classique : une mutation coûte ~ un Dijkstra, donc ∝ arêtes. Travail fixe
/// (mutations × arêtes), borné : ~30 000 mutations sous 1 000 arêtes, ~9 000 à Massy 10 km.
/// Moins de mutations laisse des graines médiocres sur les petits graphes (corpus de parité).
const ANNEAL_WORK: f64 = 3.5e7;
const ANNEAL_MIN_MAX: (f64, f64) = (5e3, 3e4);

impl Budget {
    /// Budget pour `time_s` secondes de budget Python (contrat problem.md, CLI) ; la boucle
    /// principale reçoit time_s / (1 + 0,5 (K - 1)). `deadline` reste à fixer par l'appelant.
    pub fn from_time(time_s: f64, candidates: usize, n_edges: usize, seed: u64) -> Budget {
        let main_s = time_s / (1.0 + 0.5 * (candidates.max(1) - 1) as f64);
        let (lo, hi) = ANNEAL_MIN_MAX;
        // au-delà de 10 000 arêtes (mode cible en ville, toutes les routes dans le graphe depuis
        // api.md v1.7), le plancher baisse d'autant : temps du recuit constant (Croix-Rousse cible,
        // 18 000 arêtes : 3,9 s → 2,7 s, cible tenue pareil)
        let lo = lo * (1e4 / n_edges.max(1) as f64).min(1.0);
        let anneal = (ANNEAL_WORK / n_edges.max(1) as f64).clamp(lo, hi) * (main_s / 5.0).min(1.0);
        Budget {
            iters: (FACES_PER_S * main_s) as u64,
            anneal_iters: anneal as u64,
            seed,
            candidates: candidates.max(1),
            deadline: None,
        }
    }
}

pub struct Output {
    pub ids: Vec<usize>,
    pub length: f64,
    pub dplus: f64,
    pub feasible: bool,
    /// "faces" ou "recuit".
    pub method: &'static str,
    pub depart: Option<String>,
    pub alternatives: Vec<Vec<usize>>,
    pub face_iterations: u64,
    pub anneal_iterations: u64,
    pub warnings: Vec<Msg>,
}

/// Résultat d'une recherche : (boucle, méthode, départ, itérations faces, mutations recuit).
type Found = (Option<Vec<usize>>, &'static str, Option<String>, u64, u64);

/// Une recherche complète (recuit classique éventuel, puis faces) avec la graine `seed`.
fn search(p: &Problem, b: &Budget, seed: u64, use_anneal: bool) -> Found {
    let mut fs = FaceSearch::new(p);
    fs.deadline = b.deadline;
    let mut ann = Annealer::new(p, seed);
    ann.deadline = b.deadline;
    let ids = |r: Vec<crate::faces::Step>| r.into_iter().map(|s| s.0).collect::<Vec<_>>();
    let mut route = if use_anneal {
        ann.run(b.anneal_iters).map(ids)
    } else {
        None
    };
    let (mut method, mut depart, mut face_iterations) = ("recuit", None, 0);
    // Type de voie en mode max : `SURF_FIRST` des itérations sans la prime, pour partir de la boucle
    // de plus fort D+ et l'amener vers le bon type (une seule recherche avec la prime, lancée de la
    // face du départ, reste parfois sur des chemins plats : Massy, 136 m de D+ au lieu de 380).
    // Pareil en min_distance avec les grands axes (D50) : Massy X = 250, 5 graines, 9,0–9,4 km et
    // 0–97 m de grand axe, contre 9,1–11,7 km et 97–438 m d'une seule recherche avec le coût.
    let mut iters = b.iters;
    if !p.off.is_empty() && !p.target() {
        let plain = Problem {
            off: Vec::new(),
            ..p.clone()
        };
        let mut f0 = FaceSearch::new(&plain);
        f0.deadline = b.deadline;
        let first = (SURF_FIRST * iters as f64) as u64;
        if let Some(sol) = f0.solve(first, seed, route.as_deref()) {
            face_iterations = sol.iterations;
            if route
                .as_ref()
                .is_none_or(|r| key_cmp(key(&plain, &sol.ids), key(&plain, r)).is_ge())
            {
                route = Some(sol.ids);
            }
        }
        iters -= first;
    }
    // min_distance avec préférence de montées : la clé (−L) ignore le bonus B, on garde les
    // faces (prototype `solve_minlen`).
    let prefer_faces = p.min_distance() && p.climbs < 0;
    if let Some(sol) = fs.solve(iters, seed, route.as_deref()) {
        face_iterations += sol.iterations;
        if route
            .as_ref()
            .is_none_or(|r| prefer_faces || key_cmp(key(p, &sol.ids), key(p, r)).is_ge())
        {
            (route, method, depart) = (Some(sol.ids), "faces", Some(sol.depart));
        }
    }
    if !use_anneal && route.as_ref().is_none_or(|r| !key(p, r).0) {
        // aucune face exploitable, ou boucle hors bornes (faces coincées : aucun couloir
        // depuis un départ relié au relief par un passage étroit, Grenoble T34) : recuit
        // classique (waypoints), gardé s'il est meilleur
        if let Some(r) = ann.run(b.anneal_iters).map(ids)
            && route
                .as_ref()
                .is_none_or(|x| key_cmp(key(p, &r), key(p, x)).is_gt())
        {
            (route, method, depart) = (Some(r), "recuit", None);
        }
    }
    (route, method, depart, face_iterations, ann.iterations)
}

/// (réalisable, score) : ordre de préférence entre boucles.
fn key(p: &Problem, x: &[usize]) -> (bool, f64) {
    let sc = p.score(x);
    (sc.3, sc.0)
}

/// Mode min_distance : refus immédiat si la borne inférieure de distance dépasse Lmax
/// (ou si Σw < X), aucun calcul lancé. Renvoie la borne (m).
pub fn md_lower_bound(p: &Problem) -> Result<f64, Msg> {
    let x = p.d.unwrap_or(0.0);
    let lb = length_lower_bound(&p.len, &p.w, &p.reach(), x);
    match lb {
        Some(lb) if lb <= p.lmax => Ok(lb),
        _ => Err(Msg::with_detail(
            Code::DplusUnreachableProven,
            json!({"dplus_m": x.round(), "min_km": lb.map(|l| (l / 100.0).round() / 10.0)}),
            format!("length lower bound {lb:?} > {:.0}", p.lmax),
        )),
    }
}

/// Résout `p`. Préférence de montées en modes max et cible : la recherche se fait sur le
/// poids w + γβq/H, le D+ rapporté sur le vrai w (modes.md B). En mode min_distance, le bonus
/// passe dans B (`FaceSearch`), la contrainte reste sur w.
pub fn optimize(p: &Problem, b: &Budget) -> Result<Output, Msg> {
    if p.min_distance() {
        md_lower_bound(p)?;
    }
    let Some(ps) = p.search_problem() else {
        return optimize_on(p, b);
    };
    let mut out = optimize_on(&ps, b)?;
    (_, out.length, out.dplus, out.feasible) = p.score(&out.ids);
    out.alternatives
        .sort_by(|x, y| p.stats(y).1.total_cmp(&p.stats(x).1));
    Ok(out)
}

fn optimize_on(p: &Problem, b: &Budget) -> Result<Output, Msg> {
    // Mode cible : les faces seules visent mal un couple (distance, D+) ; elles affinent.
    let use_anneal = p.n_edges() <= FACES_ONLY_EDGES || p.target();
    // Petits graphes : RESTARTS recherches indépendantes en parallèle, on garde la meilleure
    // (une graine isolée finit parfois bas : corpus de parité, Massy 10 km et réels 3 km).
    let restarts = if use_anneal { RESTARTS } else { 1 };
    let found: Vec<Found> = (0..restarts)
        .into_par_iter()
        .map(|k| {
            search(
                p,
                b,
                b.seed.wrapping_mul(RESTARTS).wrapping_add(k),
                use_anneal,
            )
        })
        .collect();
    let face_iterations: u64 = found.iter().map(|f| f.3).sum();
    let anneal_iterations: u64 = found.iter().map(|f| f.4).sum();
    // premier maximum
    let best = found.into_iter().filter(|f| f.0.is_some()).reduce(|a, c| {
        if key_cmp(key(p, c.0.as_ref().unwrap()), key(p, a.0.as_ref().unwrap())).is_gt() {
            c
        } else {
            a
        }
    });
    let (route, method, depart) = best.map_or((None, "recuit", None), |f| (f.0, f.1, f.2));
    let mut face_iterations = face_iterations;
    let mut fs = FaceSearch::new(p);
    fs.deadline = b.deadline;
    let mut main = route.ok_or_else(|| Msg::error(Code::NoLoopFound, "no initial loop"))?;
    // D63 : le confort, fort en cible, ne doit jamais faire rater la cible. Si la sortie s'écarte de
    // plus de 5 % en distance ou en D+, même recherche sans confort, gardée si elle s'approche plus
    // de la demande (Bourg 12 km / 500 m, graine 1 : 418 m de D+ avec le confort, 491 m sans).
    let dist = |x: &[usize]| {
        let (l, d) = p.stats(x);
        let dt = p.d.unwrap_or(1.0);
        ((l - p.l) / p.l).abs().max(((d - dt) / dt).abs())
    };
    if p.target() && !p.off.is_empty() && dist(&main) > TARGET_KEEP {
        let plain = Problem {
            off: Vec::new(),
            ..p.clone()
        };
        let again: Vec<Found> = (0..restarts)
            .into_par_iter()
            .map(|k| {
                search(
                    &plain,
                    b,
                    b.seed.wrapping_mul(RESTARTS).wrapping_add(k),
                    use_anneal,
                )
            })
            .collect();
        face_iterations += again.iter().map(|f| f.3).sum::<u64>();
        if let Some(x) = again
            .into_iter()
            .filter_map(|f| f.0)
            .min_by(|x, y| dist(x).total_cmp(&dist(y)))
            && dist(&x) < dist(&main)
        {
            main = x;
        }
    }
    p.check(&main)
        .map_err(|e| Msg::error(Code::InvariantViolated, e))?;
    let mut alternatives = Vec::new();
    if b.candidates > 1 {
        let extra = (b.candidates - 1) as u64;
        let (alts, it) = fs.alternates(
            std::slice::from_ref(&main),
            b.candidates - 1,
            b.iters / 2 * extra,
            b.seed,
        );
        face_iterations += it;
        for a in alts {
            p.check(&a)
                .map_err(|e| Msg::error(Code::InvariantViolated, e))?;
            alternatives.push(a);
        }
        // Triées par D+ (min_distance et cible : par score, grands axes compris) ; la principale
        // reste en tête.
        if p.min_distance() || p.target() {
            alternatives.sort_by(|x, y| p.score(y).0.total_cmp(&p.score(x).0));
        } else {
            alternatives.sort_by(|x, y| p.stats(y).1.total_cmp(&p.stats(x).1));
        }
    }
    let (_, length, dplus, feasible) = p.score(&main);
    let mut warnings = Vec::new();
    if p.min_distance() && !feasible {
        warnings.push(Msg::new(
            Code::DplusNotReached,
            json!({"dplus_m": p.d.unwrap_or(0.0).round(), "best_dplus_m": dplus.round(),
                   "max_km": (p.lmax / 100.0).round() / 10.0}),
        ));
    } else if !p.target() && !feasible {
        warnings.push(Msg::new(Code::DistanceOutOfTolerance, json!({})));
    }
    Ok(Output {
        ids: main,
        length,
        dplus,
        feasible,
        method,
        depart,
        alternatives,
        face_iterations,
        anneal_iterations,
        warnings,
    })
}
