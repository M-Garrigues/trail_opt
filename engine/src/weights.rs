//! `engine weights` : export des pondérations par arête (contrat `.team/contracts/weights.md`, D55).
//! Aucun coût n'est calculé ici : tout vient des fonctions de la recherche (`plan::off_parts`,
//! `plan::off_price`, `plan::search_weight`, `plan::off_on`, `Problem::search_len`,
//! `Problem::search_bonus`, `anneal::cost`). Ce fichier ne fait que l'orchestration et l'écriture.
use std::path::Path;

use geo::{LineString, MultiPolygon, Polygon, Simplify};
use serde_json::{Value, json};

use crate::anneal::{ALPHA_HI, Annealer, cost};
use crate::l93::Frame;
use crate::plan::{
    self, BBOX_MARGIN_M, ROAD_NATURES, Request, TRAIL_NATURES, class_mask, keep_mask, paved_mask,
};
use crate::prep::{self, N_CLS, Net, Region};
use crate::tiles::TileStore;

/// Tolérance de simplification de la géométrie exportée (m) : affichage seul.
const SIMPLIFY_M: f64 = 2.0;
/// Pas de la moyenne sur α du facteur de pente du recuit.
const ALPHA_STEPS: usize = 21;
/// Options à venir (D48/D49) : colonnes à 0 tant que le moteur ne les a pas.
const OPTIONS: [&str; 4] = ["forest", "summit", "rough", "cross"];

/// Zone d'audit : disque (lat, lon, rayon m) ou rectangle WGS84 [lon0, lat0, lon1, lat1].
#[derive(Clone, Copy)]
pub enum Area {
    Disk(f64, f64, f64),
    Bbox([f64; 4]),
}

fn r(x: f64, k: f64) -> Value {
    if x.is_finite() {
        json!((x * k).round() / k)
    } else {
        Value::Null
    }
}

/// Point d'entrée : `engine weights --tiles … (--center lat,lon --radius-km R | --bbox …)
/// --goal … --surface … [--distance-km 10] [--dplus-m D] [--max-grade-pct 60] --out <dossier>`.
#[allow(clippy::too_many_arguments)]
pub fn run(
    store: &TileStore,
    area: Area,
    goal: &str,
    surface: &str,
    distance_km: f64,
    dplus_m: Option<f64>,
    max_grade_pct: f64,
    out: &Path,
) -> Result<(), String> {
    let (geom, table, _) = compute(
        store,
        area,
        goal,
        surface,
        distance_km,
        dplus_m,
        max_grade_pct,
    )?;
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let write = |name: &str, v: &Value| {
        std::fs::write(out.join(name), serde_json::to_vec(v).unwrap()).map_err(|e| e.to_string())
    };
    write(table["geom"].as_str().unwrap(), &geom)?;
    write(&format!("cost_{goal}_{surface}.json"), &table)
}

/// (géométrie GeoJSON, table de coûts, problème du solveur sur toutes les arêtes : ligne k = arête k).
#[allow(clippy::too_many_arguments)]
pub fn compute(
    store: &TileStore,
    area: Area,
    goal: &str,
    surface: &str,
    distance_km: f64,
    dplus_m: Option<f64>,
    max_grade_pct: f64,
) -> Result<(Value, Value, crate::Problem), String> {
    let (lat, lon) = match area {
        Area::Disk(lat, lon, _) => (lat, lon),
        Area::Bbox(b) => ((b[1] + b[3]) / 2.0, (b[0] + b[2]) / 2.0),
    };
    let dplus = dplus_m.or(match goal {
        "target" => Some(40.0 * distance_km),
        "min_distance" => Some(500.0),
        _ => None,
    });
    let max_grade = (max_grade_pct > 0.0).then_some(max_grade_pct / 100.0);
    // même requête que `plan` (validation serde comprise), puis zone et Lcap comme `plan`
    let req: Request = serde_json::from_value(json!({
        "lat": lat, "lon": lon, "distance_km": distance_km, "mode": goal, "surface": surface,
        "target_dplus": dplus, "max_grade": max_grade, "enforce_limits": false,
    }))
    .map_err(|e| e.to_string())?;
    let req = plan::resolve_cap(store, &req);
    let md = req.min_distance();
    let gk = plan::Goal::of(&req.mode);
    let (l, lmax) = plan::lengths(&req);
    let frame = Frame::new_in(req.zone.proj, lat, lon);
    let poly = match area {
        Area::Disk(_, _, rad) => prep::disk(rad, 64),
        Area::Bbox(b) => {
            let (p, q) = (frame.to_local(b[0], b[1]), frame.to_local(b[2], b[3]));
            let ring = vec![
                (p[0], p[1]),
                (q[0], p[1]),
                (q[0], q[1]),
                (p[0], q[1]),
                (p[0], p[1]),
            ];
            Polygon::new(LineString::from(ring), vec![])
        }
    };
    let region = Region::new(MultiPolygon::new(vec![poly]));
    let b = region.bounds();
    let (x0, y0) = frame.l93([b[0] - BBOX_MARGIN_M, b[1] - BBOX_MARGIN_M]);
    let (x1, y1) = frame.l93([b[2] + BBOX_MARGIN_M, b[3] + BBOX_MARGIN_M]);
    let t = store.load_in(&req.zone, [x0, y0, x1, y1])?;
    let natures = &store.manifest.natures;

    // --- réseau : exactement les masques de `plan_with`
    let keep = keep_mask(&t, natures);
    let pv = paved_mask(&t, natures);
    let road = surface == "road";
    let mut sel = keep.clone();
    if road {
        sel.retain(|&i| pv[i]);
    }
    let cls_t = class_mask(&t, natures);
    let mut net = Net::new(&t, frame);
    net.cls_t = cls_t.clone();
    let mut ids = net.build(&sel, &region);
    if ids.is_empty() {
        return Err("aucune voie dans la zone".into());
    }
    // départ au centre, coupé une fois ici : `build_candidate` le retrouve sur un nœud (pas de
    // seconde coupe), et la géométrie reste la même pour tous les modes
    let (s, _, _) = net
        .insert_start(&mut ids, [0.0, 0.0])
        .ok_or("départ non accroché")?;
    let edges = ids.clone();
    let off_price = plan::off_price(&net, &edges, lmax);
    let off_on = plan::off_on(&net, &edges, surface, gk);
    let weight = |ed: &prep::Edge| plan::search_weight(ed, off_price, surface, gk);
    let weight: Option<&dyn Fn(&prep::Edge) -> f64> = (surface != "any" && !md).then_some(&weight);

    // --- graphe de recherche (élagage, pente max, passages uniques, réduction) : `srch`, `dbl`
    let n_arena = net.edges.len();
    let lmin = match goal {
        "max" => l * (1.0 - req.tol),
        "min_distance" => 0.0,
        _ => 0.7 * l,
    };
    let mut info = serde_json::Map::new();
    let x_md = req.target_dplus.filter(|_| md);
    let cand = plan::build_candidate(
        &mut net,
        &edges,
        lmin,
        lmax,
        max_grade,
        None,
        goal != "target",
        x_md,
        &mut [],
        weight,
        &mut info,
    );
    let in_search: std::collections::HashSet<usize> = match &cand {
        Ok(c) => c
            .ids
            .iter()
            .map(|&e| net.edges[e].twin.unwrap_or(e))
            .collect(),
        Err(_) => Default::default(),
    };
    let doubled: std::collections::HashSet<usize> = net.edges[n_arena..]
        .iter()
        .filter_map(|ed| ed.twin)
        .collect();

    // --- coûts : Problem sur TOUTES les arêtes de la zone (une arête hors du graphe réduit garde
    // le coût qu'elle aurait dedans), `off` et `off_price` posés comme dans `plan_with`
    let (mut p, _) = plan::to_problem(&net, &edges, s, l, goal, req.tol, dplus, false, 0, &[]);
    if let Ok(c) = &cand
        && let Some(lb) = c.lb
    {
        p.l = p.lmax.min(2.0 * lb); // comme `plan_with` (min_distance : D/L du bonus)
    }
    let parts: Vec<plan::OffParts> = edges
        .iter()
        .map(|&e| plan::off_parts(&net.edges[e].cls, &net.edges[e].lab, surface, gk))
        .collect();
    if off_on {
        p.off = parts.iter().map(plan::OffParts::total).collect();
        p.off_price = off_price;
    }
    let bonus = p.search_bonus();
    let ann = Annealer::new(&p, 0);
    let alphas: Vec<f64> = (0..ALPHA_STEPS)
        .map(|k| ann.alpha_lo + (ALPHA_HI - ann.alpha_lo) * k as f64 / (ALPHA_STEPS - 1) as f64)
        .collect();
    let costs: Vec<Box<dyn Fn(usize) -> f64 + '_>> = alphas
        .iter()
        .map(|&a| Box::new(cost(&p, &ann.gn, a, 0.0, 0)) as Box<dyn Fn(usize) -> f64>)
        .collect();

    // --- géométrie et colonnes
    let fer_col = store.manifest.columns.contains_key("osm_flags");
    let fer = |i: usize| fer_col && t.osm_flags[i] & 1 != 0;
    let wgs = |pts: &[[f64; 2]]| -> Value {
        let ls: LineString<f64> = pts.iter().map(|q| (q[0], q[1])).collect();
        let c: Vec<Value> = ls
            .simplify(SIMPLIFY_M)
            .points()
            .map(|q| {
                let (lo, la) = frame.to_wgs([q.x(), q.y()]);
                json!([r(lo, 1e5), r(la, 1e5)])
            })
            .collect();
        json!({"type": "LineString", "coordinates": c})
    };
    let updown = |z: &[f64]| {
        z.windows(2).fold((0.0, 0.0), |(u, d), w| {
            let dz = w[1] - w[0];
            (u + dz.max(0.0), d + (-dz).max(0.0))
        })
    };
    let mut feats = Vec::new();
    let mut cols: std::collections::BTreeMap<&str, Vec<Value>> = Default::default();
    let mut push = |k: &'static str, v: Value| cols.entry(k).or_default().push(v);
    for (k, &e) in edges.iter().enumerate() {
        let ed = &net.edges[e];
        let (xy, z) = net.edge_points(e);
        let (dp, dm) = updown(&z);
        let longest = ed.pieces.iter().max_by_key(|q| q.hi - q.lo).unwrap();
        let ti = longest.t as usize;
        let mut tid: Vec<i64> = ed.pieces.iter().map(|q| t.id[q.t as usize]).collect();
        tid.dedup();
        let cls = (0..N_CLS)
            .max_by(|&a, &b| ed.cls[a].total_cmp(&ed.cls[b]))
            .unwrap();
        let excl = max_grade
            .is_some_and(|g| ed.grade > g)
            .then_some("pente_max");
        feats.push(
            json!({"type": "Feature", "id": k, "geometry": wgs(&xy), "properties": {
                "i": k, "e": e, "t": tid, "len": r(ed.len, 10.0),
                "nat": natures.get(t.nature[ti] as usize), "imp": t.importance[ti], "cls": cls,
                "cls_m": ed.cls.map(|x| r(x, 10.0)), "dp": r(dp, 10.0), "dm": r(dm, 10.0),
                "w": r(ed.w, 10.0), "grade": r(100.0 * ed.grade, 10.0),
                "lab": ed.lab.map(|x| r(x, 10.0)), "flat": u8::from(ed.flat),
                "pav": u8::from(ed.pieces.iter().all(|q| pv[q.t as usize])),
                "fer": fer_col.then(|| u8::from(ed.pieces.iter().any(|q| fer(q.t as usize)))),
                "excl": excl,
            }}),
        );
        // coûts : `search_len` (hors type borné) puis facteur de pente moyen du recuit
        let sl = p.search_len(k);
        let c = costs.iter().map(|f| f(k)).sum::<f64>() / costs.len() as f64;
        let q = if off_on {
            parts[k]
        } else {
            plan::OffParts::default()
        };
        let raw = ed.len + q.pref + q.major + q.noisy + q.hike + q.water;
        for (name, v) in [
            ("cost", c),
            ("cpm", c / ed.len.max(1e-9)),
            ("base", ed.len),
            ("pref", q.pref),
            ("major", q.major),
            ("calm", q.noisy),
            ("hike", q.hike),
            ("water", q.water),
            ("search_len", sl),
            ("floor", sl - raw),
            ("slope", c - sl),
            ("gn", ann.gn[k]),
            ("bonus", bonus.as_ref().map_or(0.0, |b| b[k])),
        ] {
            push(
                name,
                r(
                    v,
                    if name == "cpm" || name == "gn" {
                        1e3
                    } else {
                        1e2
                    },
                ),
            );
        }
        for o in OPTIONS {
            push(o, json!(0.0)); // BRANCHEMENT : contribution du moteur quand l'option existe
        }
        push("srch", json!(u8::from(in_search.contains(&e))));
        push("dbl", json!(u8::from(doubled.contains(&e))));
    }
    // tronçons exclus avant le réseau (premier sommet dans la zone), sans coût
    let in_sel: std::collections::HashSet<usize> = sel.iter().copied().collect();
    let kept: std::collections::HashSet<usize> = keep.iter().copied().collect();
    let mut n_excl = 0;
    for i in 0..t.len() {
        let (x, y) = t.vertex(i, 0);
        if in_sel.contains(&i) || !region.contains(frame.local(x, y)) {
            continue;
        }
        let nat = natures.get(t.nature[i] as usize).map_or("", String::as_str);
        // raison : étiquette seulement (l'exclusion elle-même vient de keep_mask / paved_mask)
        let why = if kept.contains(&i) {
            if road && !pv[i] {
                "non_revetu"
            } else {
                "autre"
            }
        } else if t.flags[i] & 2 == 0 {
            "non_praticable"
        } else if fer(i) {
            "via_ferrata"
        } else if t.osm_access[i] == plan::ACCESS_CLOSED {
            "acces_interdit"
        } else if !TRAIL_NATURES.contains(&nat) && !ROAD_NATURES.contains(&nat) {
            "nature_exclue"
        } else {
            "autre"
        };
        let pts: Vec<[f64; 2]> = (0..t.n_vertices(i))
            .map(|j| {
                let (x, y) = t.vertex(i, j);
                frame.local(x, y)
            })
            .collect();
        let k = feats.len();
        feats.push(json!({"type": "Feature", "id": k, "geometry": wgs(&pts), "properties": {
            "i": k, "e": null, "t": [t.id[i]], "len": r(t.len_dm[i] as f64 / 10.0, 10.0), "nat": nat,
            "imp": t.importance[i], "cls": cls_t[i], "dp": r(t.dplus_dm[i] as f64 / 10.0, 10.0),
            "dm": r(t.dminus_dm[i] as f64 / 10.0, 10.0), "pav": u8::from(pv[i]),
            "fer": fer_col.then(|| u8::from(fer(i))), "excl": why,
        }}));
        cols.values_mut().for_each(|c| c.push(Value::Null));
        n_excl += 1;
    }

    // --- écriture ; poids par classe lus dans le moteur (arête d'un mètre de chaque classe)
    let pref_w: Vec<Value> = (0..N_CLS)
        .map(|c| {
            let mut u = [0.0; N_CLS];
            u[c] = 1.0;
            json!(plan::off_parts(&u, &[0.0; prep::N_LAB], surface, gk).pref)
        })
        .collect();
    let net_name = if road { "paved" } else { "all" };
    let geom_file = format!("geom_{net_name}.geojson");
    let area_json = match area {
        Area::Disk(_, _, rad) => json!({"radius_km": rad / 1000.0}),
        Area::Bbox(b) => json!({"bbox": b}),
    };
    let mut meta = json!({"format": "weights/1", "net": net_name,
        "data_version": store.manifest.data_version, "center": [lat, lon],
        "max_grade_pct": max_grade_pct, "n_edges": edges.len(), "n_excluded": n_excl,
        "columns": store.manifest.columns.keys().collect::<Vec<_>>()});
    meta.as_object_mut()
        .unwrap()
        .extend(area_json.as_object().unwrap().clone());
    let geom = json!({"type": "FeatureCollection", "weights": meta, "features": feats});
    let table = json!({"format": "weights/1", "goal": goal, "surface": surface, "geom": geom_file,
        "n": edges.len() + n_excl,
        "params": {"distance_km": distance_km, "dplus_m": dplus, "L_m": l, "Lmax_m": lmax,
            "off_on": off_on, "off_price": off_price, "major_k": crate::problem::MAJOR_K,
            "pref_w": pref_w, "noisy_w": plan::NOISY_W, "hike_w": plan::HIKE_W,
            "water_w": plan::WATER_W, "min_search_len": crate::problem::MIN_SEARCH_LEN,
            "alpha": [ann.alpha_lo, ALPHA_HI],
            "bonus_unit": if goal == "target" { "target_pts" } else { "m_dplus" },
            "options": Vec::<&str>::new(), "search": info},
        "cols": cols});
    drop(costs);
    drop(ann);
    Ok((geom, table, p))
}

/// Arguments de `engine weights` (main.rs : `Some("weights") => { engine::weights::cli(&args[1..])
/// .unwrap_or_else(|e| usage(e)); return; }`).
pub fn cli(args: &[String]) -> Result<(), String> {
    let mut kv = std::collections::HashMap::new();
    let mut it = args.iter();
    while let Some(k) = it.next() {
        let v = it.next().ok_or(format!("valeur manquante pour {k}"))?;
        kv.insert(k.as_str(), v.as_str());
    }
    let nums = |k: &str| -> Result<Vec<f64>, String> {
        kv.get(k).map_or(Ok(Vec::new()), |v| {
            v.split(',')
                .map(|x| x.trim().parse::<f64>().ok().filter(|x| x.is_finite()))
                .collect::<Option<Vec<_>>>()
                .ok_or(format!("{k} : nombres attendus"))
        })
    };
    let one = |k: &str, d: Option<f64>| -> Result<Option<f64>, String> {
        Ok(nums(k)?.first().copied().or(d))
    };
    for k in kv.keys() {
        if ![
            "--tiles",
            "--center",
            "--radius-km",
            "--bbox",
            "--goal",
            "--surface",
            "--distance-km",
            "--dplus-m",
            "--max-grade-pct",
            "--out",
        ]
        .contains(k)
        {
            return Err(format!("option inconnue : {k}"));
        }
    }
    let area = match (nums("--center")?.as_slice(), nums("--bbox")?.as_slice()) {
        ([lat, lon], []) => {
            let rad = one("--radius-km", None)?.ok_or("--radius-km requis avec --center")?;
            if !(0.5..=25.0).contains(&rad) {
                return Err("--radius-km : 0,5 à 25".into());
            }
            Area::Disk(*lat, *lon, rad * 1000.0)
        }
        ([], [a, b, c, d]) if a < c && b < d => Area::Bbox([*a, *b, *c, *d]),
        _ => return Err("--center lat,lon --radius-km R, ou --bbox lon0,lat0,lon1,lat1".into()),
    };
    let goal = *kv.get("--goal").ok_or("--goal requis")?;
    let surface = *kv.get("--surface").ok_or("--surface requis")?;
    if !["max", "target", "min_distance"].contains(&goal)
        || !["trail", "any", "road"].contains(&surface)
    {
        return Err("--goal max|target|min_distance, --surface trail|any|road".into());
    }
    let tiles = kv.get("--tiles").ok_or("--tiles requis")?;
    let out = kv.get("--out").ok_or("--out requis")?;
    let store = TileStore::open(Path::new(tiles))?;
    run(
        &store,
        area,
        goal,
        surface,
        one("--distance-km", Some(10.0))?.unwrap(),
        one("--dplus-m", None)?,
        one("--max-grade-pct", Some(60.0))?.unwrap(),
        Path::new(out),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sur la dalle de test versionnée : le coût exporté est exactement celui de la recherche
    /// (`Problem::search_len` puis `anneal::cost` moyenné sur α), additif, toujours > 0.
    #[test]
    fn export_cost_is_search_cost() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/tiles");
        let store = TileStore::open(&dir).unwrap();
        for (goal, surface) in [
            ("max", "trail"),
            ("target", "any"),
            ("min_distance", "road"),
        ] {
            let area = Area::Disk(48.7309, 2.2713, 2000.0);
            let (geom, table, p) = compute(&store, area, goal, surface, 8.0, None, 60.0).unwrap();
            let feats = geom["features"].as_array().unwrap();
            let col = |k: &str, i: usize| table["cols"][k][i].as_f64().unwrap();
            let ann = Annealer::new(&p, 0);
            let n = p.n_edges();
            assert!(n > 100, "{goal} {surface} : {n} arêtes");
            assert_eq!(table["n"].as_u64().unwrap() as usize, feats.len());
            for (k, f) in feats.iter().enumerate().take(n) {
                assert_eq!(f["properties"]["i"], json!(k));
                let sl = p.search_len(k);
                let c = (0..ALPHA_STEPS)
                    .map(|j| {
                        let a = ann.alpha_lo
                            + (ALPHA_HI - ann.alpha_lo) * j as f64 / (ALPHA_STEPS - 1) as f64;
                        cost(&p, &ann.gn, a, 0.0, 0)(k)
                    })
                    .sum::<f64>()
                    / ALPHA_STEPS as f64;
                assert_eq!(col("search_len", k), (sl * 100.0).round() / 100.0);
                assert_eq!(col("cost", k), (c * 100.0).round() / 100.0);
                assert!(col("cost", k) > 0.0);
                let sum: f64 = [
                    "base", "pref", "major", "calm", "hike", "water", "forest", "summit", "rough",
                    "cross", "floor", "slope",
                ]
                .iter()
                .map(|x| col(x, k))
                .sum();
                assert!(
                    (sum - col("cost", k)).abs() < 0.08,
                    "{k} : {sum} ≠ {}",
                    col("cost", k)
                );
            }
            // en Route, le réseau est revêtu ; ailleurs, « Tout » n'a pas de préférence
            for (k, f) in feats.iter().enumerate().take(n) {
                if surface == "road" {
                    assert_eq!(f["properties"]["pav"], json!(1));
                }
                if surface == "any" {
                    assert_eq!(col("pref", k), 0.0);
                }
            }
        }
    }
}
