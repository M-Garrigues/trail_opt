//! Routage des solveurs selon la taille du graphe et le mode (port de `solvers.optimize`).
//! Pas de CP-SAT (décision D6) : sur les petits graphes, le recuit classique puis les faces
//! qui partent de son résultat le remplacent.
use std::time::Instant;

use rayon::prelude::*;
use serde_json::json;

use crate::codes::{Code, Msg};
use crate::faces::key_cmp;
use crate::{Annealer, FaceSearch, Problem};

/// Jusqu'ici (ou en mode cible), recuit classique d'abord, puis faces depuis son résultat ;
/// au-delà, faces seules (le recuit classique se noie dans les grands graphes).
pub const FACES_ONLY_EDGES: usize = 5000;
/// Recherches indépendantes sur les petits graphes (voir `optimize`).
const RESTARTS: u64 = 2;

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
    if let Some(sol) = fs.solve(b.iters, seed, route.as_deref()) {
        face_iterations = sol.iterations;
        if route
            .as_ref()
            .is_none_or(|r| key_cmp(key(p, &sol.ids), key(p, r)).is_ge())
        {
            (route, method, depart) = (Some(sol.ids), "faces", Some(sol.depart));
        }
    }
    if route.is_none() && !use_anneal {
        // aucune face exploitable : recuit classique
        route = ann.run(b.anneal_iters).map(ids);
    }
    (route, method, depart, face_iterations, ann.iterations)
}

/// (réalisable, score) : ordre de préférence entre boucles.
fn key(p: &Problem, x: &[usize]) -> (bool, f64) {
    let sc = p.score(x);
    (sc.3, sc.0)
}

pub fn optimize(p: &Problem, b: &Budget) -> Result<Output, Msg> {
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
    let main = route.ok_or_else(|| Msg::error(Code::NoLoopFound, "no initial loop"))?;
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
        // Les boucles sont triées par D+ (la principale reste en tête).
        alternatives.sort_by(|x, y| p.stats(y).1.total_cmp(&p.stats(x).1));
    }
    let (_, length, dplus, feasible) = p.score(&main);
    let mut warnings = Vec::new();
    if !p.target() && !feasible {
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
