//! CLI du moteur.
//!   engine solve --problem f.json [--time S] [--iters N] [--anneal-iters M] [--seed S] [--candidates K]
//!   engine codes          liste des codes d'erreur et d'avertissement (JSON)
//! `solve` écrit le résultat en JSON sur la sortie standard ; en cas d'échec
//! {"error": {"code", "params", "detail"}} et code de sortie 1.
//! Budget : --time S fixe le plafond de temps et, par défaut, les itérations (S en secondes
//! de budget Python : le moteur vise ~1/10 de ce temps) ; --iters / --anneal-iters les forcent.
use std::process::exit;
use std::time::{Duration, Instant};

use engine::{Budget, Code, Msg, Problem, codes, optimize};

const USAGE: &str = "usage : engine solve --problem f.json [--time S] [--iters N] [--anneal-iters M] [--seed S] [--candidates K] | engine codes";
/// Itérations par seconde de budget Python (faces, recuit classique), calibrées sur Massy.
const FACES_PER_S: f64 = 3e5;
/// Recuit classique : une mutation coûte ~ un Dijkstra, donc ∝ arêtes. Travail fixe
/// (mutations × arêtes), borné : ~30 000 mutations sous 1 000 arêtes, ~9 000 à Massy 10 km.
/// Moins de mutations laisse des graines médiocres sur les petits graphes (corpus de parité).
const ANNEAL_WORK: f64 = 3.5e7;
const ANNEAL_MIN_MAX: (f64, f64) = (5e3, 3e4);

fn fail(msg: Msg) -> ! {
    println!("{}", serde_json::json!({ "error": msg }));
    exit(1)
}

fn usage(detail: String) -> ! {
    eprintln!("{detail}\n{USAGE}");
    fail(Msg::error(Code::InvalidProblem, detail))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("codes") => {
            println!(
                "{}",
                serde_json::to_string_pretty(&codes::export()).unwrap()
            );
            return;
        }
        Some("solve") => {}
        _ => usage("commande attendue : solve ou codes".into()),
    }
    let (mut path, mut time, mut iters, mut anneal_iters, mut seed, mut candidates) =
        (None, None, None, None, 0u64, 1usize);
    let mut it = args[1..].iter();
    while let Some(k) = it.next() {
        let val = it
            .next()
            .unwrap_or_else(|| usage(format!("valeur manquante pour {k}")));
        let num = || -> f64 {
            val.parse()
                .ok()
                .filter(|x: &f64| x.is_finite() && *x >= 0.0)
                .unwrap_or_else(|| usage(format!("{k} : nombre positif attendu")))
        };
        match k.as_str() {
            "--problem" => path = Some(val.clone()),
            "--time" => time = Some(num()),
            "--iters" => iters = Some(num() as u64),
            "--anneal-iters" => anneal_iters = Some(num() as u64),
            "--seed" => seed = num() as u64,
            "--candidates" => candidates = (num() as usize).clamp(1, 4),
            _ => usage(format!("option inconnue : {k}")),
        }
    }
    let path = path.unwrap_or_else(|| usage("--problem requis".into()));
    // Budget de la boucle principale : chaque boucle en plus coûte une demi-part.
    let main_s = time.unwrap_or(20.0) / (1.0 + 0.5 * (candidates - 1) as f64);
    let deadline = time.map(|s| Instant::now() + Duration::from_secs_f64(s));

    let t = Instant::now();
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| fail(Msg::error(Code::InvalidProblem, format!("{path}: {e}"))));
    let p = Problem::from_json(&text).unwrap_or_else(|e| fail(Msg::error(Code::InvalidProblem, e)));
    let load_s = t.elapsed().as_secs_f64();
    let (lo, hi) = ANNEAL_MIN_MAX;
    let anneal_default =
        (ANNEAL_WORK / p.n_edges().max(1) as f64).clamp(lo, hi) * (main_s / 5.0).min(1.0);
    let budget = Budget {
        iters: iters.unwrap_or((FACES_PER_S * main_s) as u64),
        anneal_iters: anneal_iters.unwrap_or(anneal_default as u64),
        seed,
        candidates,
        deadline,
    };
    let t = Instant::now();
    let out = optimize(&p, &budget).unwrap_or_else(|m| fail(m));
    let solve_s = t.elapsed().as_secs_f64();
    let alts: Vec<_> = out
        .alternatives
        .iter()
        .map(|a| {
            let (l, d) = p.stats(a);
            serde_json::json!({"edges": a, "length": l, "dplus": d})
        })
        .collect();
    let res = serde_json::json!({
        "edges": out.ids,
        "length": out.length,
        "dplus": out.dplus,
        "feasible": out.feasible,
        "method": out.method,
        "depart": out.depart,
        "alternatives": alts,
        "iterations": out.face_iterations,
        "anneal_iterations": out.anneal_iterations,
        "it_per_s": out.face_iterations as f64 / solve_s,
        "load_s": load_s,
        "solve_s": solve_s,
        "warnings": out.warnings,
    });
    println!("{res}");
}
