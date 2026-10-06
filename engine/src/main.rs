//! CLI du moteur.
//!   engine solve --problem f.json [--time S] [--iters N] [--anneal-iters M] [--seed S] [--candidates K]
//!   engine plan --tiles DIR --request req.json [--prep-only]   boucles depuis les dalles tiles/1
//!   engine codes          liste des codes d'erreur et d'avertissement (JSON)
//! `solve` écrit le résultat en JSON sur la sortie standard ; en cas d'échec
//! {"error": {"code", "params", "detail"}} et code de sortie 1.
//! Budget : --time S fixe le plafond de temps et, par défaut, les itérations (S en secondes
//! de budget Python : le moteur vise ~1/10 de ce temps) ; --iters / --anneal-iters les forcent.
use std::process::exit;
use std::time::{Duration, Instant};

use engine::{Budget, Code, Msg, Problem, codes, optimize};

const USAGE: &str = "usage : engine solve --problem f.json [--time S] [--iters N] [--anneal-iters M] [--seed S] [--candidates K] | engine plan --tiles DIR --request req.json [--prep-only] | engine codes";

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
        Some("plan") => plan(&args[1..]),
        _ => usage("commande attendue : solve, plan ou codes".into()),
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
    let deadline = time.map(|s| Instant::now() + Duration::from_secs_f64(s));

    let t = Instant::now();
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| fail(Msg::error(Code::InvalidProblem, format!("{path}: {e}"))));
    let p = Problem::from_json(&text).unwrap_or_else(|e| fail(Msg::error(Code::InvalidProblem, e)));
    let load_s = t.elapsed().as_secs_f64();
    let mut budget = Budget::from_time(time.unwrap_or(20.0), candidates, p.n_edges(), seed);
    budget.iters = iters.unwrap_or(budget.iters);
    budget.anneal_iters = anneal_iters.unwrap_or(budget.anneal_iters);
    budget.deadline = deadline;
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

/// `engine plan` : JSON de sortie sur stdout ; erreur {"error": …} et code 1.
fn plan(args: &[String]) -> ! {
    let (mut tiles, mut request, mut prep_only) = (None, None, false);
    let mut it = args.iter();
    while let Some(k) = it.next() {
        match k.as_str() {
            "--prep-only" => prep_only = true,
            "--tiles" | "--request" => {
                let v = it
                    .next()
                    .cloned()
                    .unwrap_or_else(|| usage(format!("valeur manquante pour {k}")));
                if k == "--tiles" {
                    tiles = Some(v)
                } else {
                    request = Some(v)
                }
            }
            _ => usage(format!("option inconnue : {k}")),
        }
    }
    let tiles = tiles.unwrap_or_else(|| usage("--tiles requis".into()));
    let request = request.unwrap_or_else(|| usage("--request requis".into()));
    let text = std::fs::read_to_string(&request)
        .unwrap_or_else(|e| fail(Msg::error(Code::InvalidRequest, format!("{request}: {e}"))));
    let req: engine::plan::Request = serde_json::from_str(&text)
        .unwrap_or_else(|e| fail(Msg::error(Code::InvalidRequest, e.to_string())));
    // `--tiles s3://bucket/tiles/<version>/` : même lecture à la demande que la Lambda (cache
    // TILES_CACHE, défaut <tmp>/optrail-tiles) ; sinon dossier local.
    let store = if tiles.starts_with("s3://") {
        let cache = std::env::var_os("TILES_CACHE")
            .map_or(std::env::temp_dir().join("optrail-tiles"), Into::into);
        engine::tiles::TileStore::open_s3(&tiles, &cache)
    } else {
        engine::tiles::TileStore::open(std::path::Path::new(&tiles))
    }
    .unwrap_or_else(|e| fail(Msg::error(Code::InvalidProblem, e)));
    let out = engine::plan::plan(&store, &req, prep_only).unwrap_or_else(|m| fail(m));
    println!("{out}");
    exit(0)
}
