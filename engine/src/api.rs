//! `GET /api/plan` (contrat `.team/contracts/api.md` v1.7 ; `diagnose=1` est traité à part, hors `KNOWN`) : paramètres de requête → statut HTTP,
//! corps JSON et ligne de log. Logique pure, sans dépendance Lambda (testée par `cargo test`) ;
//! le binaire `lambda` ne fait que la glue (événement, Turnstile, réponse).
use serde_json::{Map, Value, json};

use crate::codes::{Code, Msg};
use crate::plan::{self, Request};
use crate::tiles::TileStore;

/// Plafond de calcul en temps réel (D8 : ~15 s, timeout Lambda 30 s).
pub const MAX_COMPUTE_S: f64 = 15.0;
pub const MAX_VERTICES: usize = 50;
pub const DEFAULT_CANDIDATES: usize = 3;
/// I1 (D32) : au-delà de `MANY_KM` km, au plus `MANY_MAX_N` boucles (plafond de 15 s).
pub const MANY_KM: f64 = 40.0;
pub const MANY_MAX_N: usize = 2;
/// Jeton Turnstile : au-delà, refus local sans appel à siteverify (E3).
pub const MAX_TOKEN_LEN: usize = 2048;
pub const SEED_MAX: u64 = 999;
pub const GRADE_PCT: (f64, f64) = (5.0, 60.0);
/// Filtre de pente par défaut (D31, sur 50 m glissants) ; `max_grade_pct=0` : sans limite.
pub const DEFAULT_GRADE_PCT: f64 = 60.0;
/// D+ du mode cible (m) ; min_distance : bornes du moteur (`plan::MD_DPLUS_M`).
pub const TARGET_DPLUS_M: (f64, f64) = (10.0, 5000.0);
/// Simplification Douglas–Peucker en plan (m).
pub const SIMPLIFY_M: f64 = 1.0;
const KNOWN: [&str; 16] = [
    "lat",
    "lon",
    "goal",
    "distance_km",
    "dplus_m",
    "max_distance_km",
    "climbs",
    "max_grade_pct",
    "surface",
    "roads",
    "no_repeat_junction",
    "n_candidates",
    "polygon",
    "seed",
    "debug",
    "via",
];

pub struct Reply {
    pub status: u16,
    pub body: Value,
    /// Une ligne de log JSON par calcul (coordonnées arrondies au km).
    pub log: Value,
}

/// Statut HTTP d'un code d'erreur (api.md).
pub fn http_status(code: Code) -> u16 {
    use Code::*;
    match code {
        BotCheckFailed => 403,
        AdminDenied => 401,
        AdminLocked => 429,
        LoopNotFound => 404,
        OutsideCoverage
        | NoWayInZone
        | NoLoopOfDistance
        | DplusUnreachableProven
        | ViaUnreachable => 422,
        Busy | ServicePaused => 503,
        Timeout => 504,
        NoLoopFound | InvariantViolated | InvalidProblem => 500,
        _ => match code.info().0 {
            crate::codes::Kind::Error => 400,
            crate::codes::Kind::Warning => 500,
        },
    }
}

fn bad(detail: impl Into<String>) -> Msg {
    Msg::error(Code::InvalidRequest, detail)
}

/// Paramètres de requête → requête moteur (validation stricte : paramètre inconnu ou répété,
/// nombre non fini ou mal formé → erreur, aucun calcul) ; renvoie aussi le drapeau `debug`.
pub fn parse(query: &[(String, String)]) -> Result<(Request, bool), Msg> {
    let mut q: Map<String, Value> = Map::new();
    for (k, v) in query {
        if !KNOWN.contains(&k.as_str()) {
            return Err(bad(format!("unknown parameter {k}")));
        }
        if q.insert(k.clone(), Value::String(v.clone())).is_some() {
            return Err(bad(format!("repeated parameter {k}")));
        }
    }
    let s = |k: &str| q.get(k).and_then(Value::as_str);
    let num = |k: &str| -> Result<Option<f64>, Msg> {
        s(k).map(|v| {
            v.parse::<f64>()
                .ok()
                .filter(|x| x.is_finite())
                .ok_or_else(|| bad(format!("{k}: number expected")))
        })
        .transpose()
    };
    let int = |k: &str| -> Result<Option<u64>, Msg> {
        s(k).map(|v| {
            v.parse::<u64>()
                .map_err(|_| bad(format!("{k}: integer expected")))
        })
        .transpose()
    };
    let flag = |k: &str, default: bool| -> Result<bool, Msg> {
        match s(k) {
            None => Ok(default),
            Some("1" | "true") => Ok(true),
            Some("0" | "false") => Ok(false),
            Some(_) => Err(bad(format!("{k}: boolean expected"))),
        }
    };
    let (Some(lat), Some(lon)) = (num("lat")?, num("lon")?) else {
        return Err(bad("lat and lon required"));
    };
    let mode = match s("goal").unwrap_or("max_dplus") {
        "max_dplus" => "max",
        "target" => "target",
        "min_distance" => "min_distance",
        g => return Err(Msg::error(Code::ModeUnknown, g)),
    };
    let dplus = num("dplus_m")?;
    if mode == "target"
        && dplus.is_some_and(|d| !(TARGET_DPLUS_M.0..=TARGET_DPLUS_M.1).contains(&d))
    {
        return Err(Msg::new(
            Code::DplusOutOfRange,
            json!({"min_m": TARGET_DPLUS_M.0, "max_m": TARGET_DPLUS_M.1}),
        ));
    }
    // v1.7 : `surface` (préférence) ; l'ancien filtre `roads` (liens partagés, historique) est
    // encore lu et traduit, `surface` l'emporte
    let surface = match (s("surface"), s("roads")) {
        (Some(x @ ("trail" | "any" | "road")), _) => x,
        (None, None) => "trail",
        (None, Some("unpaved" | "pedestrian")) => "trail",
        (None, Some("minor" | "all")) => "any",
        (x, y) => return Err(Msg::error(Code::RoadsUnknown, x.or(y).unwrap_or(""))),
    };
    let grade = match num("max_grade_pct")?.unwrap_or(DEFAULT_GRADE_PCT) {
        0.0 => None,
        g if (GRADE_PCT.0..=GRADE_PCT.1).contains(&g) => Some(g),
        _ => {
            return Err(Msg::error(
                Code::MaxGradeInvalid,
                "max_grade_pct must be 0 or 5..60",
            ));
        }
    };
    let n = int("n_candidates")?.unwrap_or(DEFAULT_CANDIDATES as u64);
    if !(1..=4).contains(&n) {
        return Err(bad("n_candidates must be 1..4"));
    }
    let seed = int("seed")?.unwrap_or(0);
    if seed > SEED_MAX {
        return Err(bad("seed must be 0..999"));
    }
    let polygon = s("polygon").map(parse_polygon).transpose()?;
    let via = s("via").map(parse_via).transpose()?.unwrap_or_default();
    let req = json!({
        "lat": lat, "lon": lon, "mode": mode,
        "distance_km": num("distance_km")?.unwrap_or(10.0),
        "target_dplus": dplus,
        "max_distance_km": num("max_distance_km")?,
        "climbs": s("climbs").unwrap_or("balanced"),
        "max_grade": grade.map(|g| g / 100.0),
        "surface": surface,
        "node_simple": flag("no_repeat_junction", true)?,
        "n_candidates": n,
        "polygon": polygon,
        "seed": seed,
        "tol": 0.05,
        "enforce_limits": true,
        "max_compute_s": MAX_COMPUTE_S,
        "via": via,
    });
    let req: Request = serde_json::from_value(req).map_err(|e| bad(e.to_string()))?;
    Ok((req, flag("debug", false)?))
}

/// `lon,lat;lon,lat;…`, 3 à MAX_VERTICES sommets.
fn parse_polygon(text: &str) -> Result<Vec<[f64; 2]>, Msg> {
    let pts: Option<Vec<[f64; 2]>> = text
        .split(';')
        .map(|p| {
            let (a, b) = p.split_once(',')?;
            let (x, y) = (a.trim().parse::<f64>().ok()?, b.trim().parse::<f64>().ok()?);
            (x.is_finite() && y.is_finite()).then_some([x, y])
        })
        .collect();
    let simple = |p: &[[f64; 2]]| {
        use geo::Validation;
        geo::Polygon::new(
            p.iter().map(|&[x, y]| (x, y)).collect::<Vec<_>>().into(),
            vec![],
        )
        .is_valid()
    };
    match pts {
        Some(p) if (3..=MAX_VERTICES).contains(&p.len()) && simple(&p) => Ok(p),
        Some(p) if (3..=MAX_VERTICES).contains(&p.len()) => {
            Err(Msg::error(Code::ZoneInvalid, "self-intersecting polygon"))
        }
        Some(_) => Err(Msg::error(
            Code::ZoneInvalid,
            "polygon must have 3..50 vertices",
        )),
        None => Err(bad("polygon: lon,lat;lon,lat;… expected")),
    }
}

/// `lat,lon;lat,lon;…`, 1 à `plan::VIA_MAX` points de passage (api.md v1.5).
fn parse_via(text: &str) -> Result<Vec<[f64; 2]>, Msg> {
    let pts: Option<Vec<[f64; 2]>> = text
        .split(';')
        .map(|p| {
            let (a, b) = p.split_once(',')?;
            let (la, lo) = (a.trim().parse::<f64>().ok()?, b.trim().parse::<f64>().ok()?);
            ((-90.0..=90.0).contains(&la) && (-180.0..=180.0).contains(&lo)).then_some([la, lo])
        })
        .collect();
    pts.filter(|p| (1..=plan::VIA_MAX).contains(&p.len()))
        .ok_or_else(|| bad("via: 1..5 points lat,lon;lat,lon expected"))
}

/// Le départ tombe-t-il dans une dalle du manifeste (couverture) ?
pub fn covered(store: &TileStore, lat: f64, lon: f64) -> bool {
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return false;
    }
    store.tile_at(lat, lon).is_some()
}

/// Douglas–Peucker en plan (`eps` m) d'une boucle de la réponse : `lat`, `lon`, `ele`, `dist`
/// gardent les mêmes indices ; longueur, D+ et `climbs` restent ceux du profil complet.
pub fn simplify(cand: &mut Value, eps: f64) {
    let col = |k: &str| -> Vec<f64> {
        cand[k]
            .as_array()
            .map_or(Vec::new(), |a| a.iter().filter_map(Value::as_f64).collect())
    };
    let (lat, lon) = (col("lat"), col("lon"));
    let n = lat.len();
    if n < 3 || lon.len() != n {
        return;
    }
    // repère local équirectangulaire (erreur négligeable à l'échelle d'une boucle)
    let k = lat[0].to_radians().cos();
    let xy: Vec<[f64; 2]> = (0..n)
        .map(|i| {
            [
                (lon[i] - lon[0]) * k * 111_320.0,
                (lat[i] - lat[0]) * 110_574.0,
            ]
        })
        .collect();
    let mut keep = vec![false; n];
    (keep[0], keep[n - 1]) = (true, true);
    let mut stack = vec![(0, n - 1)];
    while let Some((a, b)) = stack.pop() {
        let (p, q) = (xy[a], xy[b]);
        let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
        let l2 = dx * dx + dy * dy;
        let dist = |r: [f64; 2]| {
            let t = if l2 > 0.0 {
                (((r[0] - p[0]) * dx + (r[1] - p[1]) * dy) / l2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            (r[0] - p[0] - t * dx).hypot(r[1] - p[1] - t * dy)
        };
        let far = (a + 1..b)
            .map(|i| (dist(xy[i]), i))
            .max_by(|x, y| x.0.total_cmp(&y.0));
        if let Some((_, i)) = far.filter(|f| f.0 > eps) {
            keep[i] = true;
            stack.push((a, i));
            stack.push((i, b));
        }
    }
    for k in ["lat", "lon", "ele", "dist"] {
        if let Some(a) = cand[k].as_array_mut() {
            let mut i = 0;
            a.retain(|_| {
                i += 1;
                keep.get(i - 1).copied().unwrap_or(false)
            });
        }
    }
}

/// Corps d'erreur envoyé au client : `detail` (anglais, logs) seulement en 400, jamais en
/// 403/404/5xx (revue T26 : pas de détail interne côté client).
pub fn error_body(m: &Msg, status: u16) -> Value {
    let mut v = json!({ "error": m });
    if status != 400
        && let Some(e) = v["error"].as_object_mut()
    {
        e.remove("detail");
    }
    v
}

/// Turnstile (D8, E3) : appel interne exempté ; jeton vide ou de plus de `MAX_TOKEN_LEN`
/// caractères refusé localement (aucun appel réseau) ; sinon `verify(token)`.
pub fn check_bot(
    token: Option<&str>,
    internal: bool,
    verify: impl FnOnce(&str) -> Result<bool, String>,
) -> Result<(), Msg> {
    if internal {
        return Ok(());
    }
    let r = match token {
        None | Some("") => Err("missing token".to_string()),
        Some(t) if t.len() > MAX_TOKEN_LEN => Err("token too long".to_string()),
        Some(t) => verify(t).and_then(|ok| {
            if ok {
                Ok(())
            } else {
                Err("token rejected".into())
            }
        }),
    };
    r.map_err(|e| Msg::error(Code::BotCheckFailed, e))
}

/// Départ arrondi pour les journaux (admin.md § 1) : centre de la cellule d'une grille en degrés
/// (0,0045° N–S ≈ 500 m ; 0,0065° E–O, 455 à 690 m selon la latitude), 4 décimales ; `None` si invalide.
pub fn grid500(lat: f64, lon: f64) -> Option<(f64, f64)> {
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }
    let r = |x: f64, step: f64| ((x / step).round() * step * 1e4).round() / 1e4;
    Some((r(lat, 0.0045), r(lon, 0.0065)))
}

/// Ligne de journal finale : `phase` (admin.md § 1), `code: "ok"` en 200, clés `null` retirées
/// (Insights : `ispresent()` et regroupements fiables).
fn seal(mut log: Value, diagnose: bool) -> Value {
    log["phase"] = json!(if log["internal"] == true {
        "internal"
    } else if log["accepted"] != true {
        "rejected"
    } else if diagnose {
        "diagnose"
    } else {
        "plan"
    });
    if log["status"] == 200 {
        log["code"] = json!("ok");
    }
    if let Some(o) = log.as_object_mut() {
        o.retain(|_, v| !v.is_null());
    }
    log
}

/// Traite une requête. `internal` : appel direct IAM (smoke test), sans Turnstile.
/// `verify(token)` : vérification Turnstile (Ok(true) si valide). `key` : clé HMAC qui signe
/// chaque boucle rendue (champ `sig`, exigé par `POST /api/loops`, M3).
pub fn handle(
    query: &[(String, String)],
    token: Option<&str>,
    internal: bool,
    store: &TileStore,
    key: &ring::hmac::Key,
    verify: impl FnOnce(&str) -> Result<bool, String>,
) -> Reply {
    let t0 = std::time::Instant::now();
    let get = |k: &str| {
        query
            .iter()
            .find(|x| x.0 == k)
            .and_then(|x| x.1.parse::<f64>().ok())
            .unwrap_or(f64::NAN)
    };
    let goal = query
        .iter()
        .find(|x| x.0 == "goal")
        .map_or("max_dplus", |x| x.1.as_str());
    let goal = ["max_dplus", "target", "min_distance"]
        .contains(&goal)
        .then_some(goal);
    let start = grid500(get("lat"), get("lon"));
    let mut log = json!({
        "msg": "plan", "v": 2, "internal": internal, "accepted": false, "goal": goal,
        "req_km": get("distance_km").is_finite().then(|| get("distance_km")),
        "req_dplus_m": get("dplus_m").is_finite().then(|| get("dplus_m")),
        "req_max_km": get("max_distance_km").is_finite().then(|| get("max_distance_km")),
        "start_lat": start.map(|s| s.0), "start_lon": start.map(|s| s.1),
        "zone": store.zone(get("lat"), get("lon")).name,
        "data_version": store.manifest.data_version, "solver_version": crate::solver_version(),
    });
    // D46 : `diagnose=1` = diagnostic seul (mêmes paramètres que la requête initiale)
    let dq: Vec<&str> = query
        .iter()
        .filter(|x| x.0 == "diagnose")
        .map(|x| x.1.as_str())
        .collect();
    let diagnose = matches!(dq.as_slice(), ["1" | "true"]);
    let fail = |m: Msg, mut log: Value| {
        let status = http_status(m.code);
        log["status"] = json!(status);
        log["code"] = json!(m.code);
        // revue F2 : texte venant de la requête (nom de paramètre…) borné dans les journaux
        log["detail"] = json!(
            m.detail
                .as_ref()
                .map(|d| d.chars().take(100).collect::<String>())
        );
        log["compute_s"] = json!(t0.elapsed().as_secs_f64());
        Reply {
            status,
            body: error_body(&m, status),
            log: seal(log, diagnose),
        }
    };
    if !diagnose && !matches!(dq.as_slice(), [] | ["0" | "false"]) {
        return fail(bad("diagnose: boolean expected"), log);
    }
    let query: Vec<(String, String)> = query
        .iter()
        .filter(|x| x.0 != "diagnose")
        .cloned()
        .collect();
    let (req, debug) = match parse(&query) {
        Ok(x) => x,
        Err(m) => return fail(m, log),
    };
    // revue F2 : seulement des valeurs validées dans les journaux (climbs est validé plus loin)
    let climbs = ["short", "balanced", "long"].contains(&req.climbs.as_str());
    log["climbs"] = json!(climbs.then_some(&req.climbs));
    log["surface"] = json!(req.surface);
    log["max_grade_pct"] = json!(req.max_grade.map_or(0.0, |g| (g * 100.0).round()));
    log["via_n"] = json!(req.via.len());
    log["polygon"] = json!(req.polygon.is_some());
    log["n_asked"] = json!(req.n_candidates);
    if !covered(store, req.lat, req.lon) {
        return fail(
            Msg::error(Code::OutsideCoverage, "start outside the tiles"),
            log,
        );
    }
    // points de passage hors de portée ou de la zone : avant Turnstile (jeton non consommé)
    let located = plan::resolve_cap(store, &req);
    if let Err(m) = plan::check_zone(store, &located).and_then(|_| plan::check_via(&located)) {
        return fail(m, log);
    }
    if let Err(m) = check_bot(token, internal, verify) {
        return fail(m, log);
    }
    // cap par défaut résolu dans plan::plan (qui peut l'élargir, T34) ; ici pour compter seulement
    let fewer =
        plan::resolve_cap(store, &req).sizing_km() > MANY_KM && req.n_candidates > MANY_MAX_N;
    let req = plan::Request {
        n_candidates: if fewer { MANY_MAX_N } else { req.n_candidates },
        ..req
    };
    if diagnose {
        log["accepted"] = json!(true);
        let body = plan::diagnose(store, &req);
        log["status"] = json!(200);
        log["compute_s"] = json!(t0.elapsed().as_secs_f64());
        return Reply {
            status: 200,
            body,
            log: seal(log, true),
        };
    }
    // coupe-circuit (B2, T27) : filtre `{ $.msg = "plan" && $.accepted IS TRUE }`, somme de
    // `compute_s` ; `accepted` reste vrai sur toutes les sorties qui suivent (200 comme erreurs)
    log["accepted"] = json!(true);
    let mut out = match plan::plan(store, &req, false) {
        Ok(o) => o,
        Err(m) => {
            // plafond atteint sans boucle : timeout plutôt qu'échec de recherche
            let late = t0.elapsed().as_secs_f64() >= MAX_COMPUTE_S;
            let m = if late && matches!(m.code, Code::NoLoopOfDistance | Code::NoLoopFound) {
                Msg::error(Code::Timeout, m.detail.unwrap_or_default())
            } else {
                m
            };
            return fail(m, log);
        }
    };
    if fewer && let Some(w) = out["warnings"].as_array_mut() {
        w.push(json!(Msg::new(
            Code::CandidatesReduced,
            json!({"max_n": MANY_MAX_N, "km": MANY_KM})
        )));
    }
    if let Some(cands) = out["candidates"].as_array_mut() {
        cands.iter_mut().for_each(|c| simplify(c, SIMPLIFY_M));
    }
    if let Err(m) = crate::share::sign_candidates(&mut out, key) {
        return fail(m, log);
    }
    // surcoût des dalles (téléchargement S3 à froid compris) : suivi dans les logs
    log["tiles_load_s"] = out["debug"]["tiles_load_s"].clone();
    if !debug && let Some(o) = out.as_object_mut() {
        o.remove("debug");
    }
    let c0 = &out["candidates"][0];
    log["status"] = json!(200);
    log["got_km"] = json!(c0["length_m"].as_f64().map(|m| (m / 10.0).round() / 100.0));
    log["got_dplus_m"] = json!(c0["dplus_m"].as_f64().map(|d| d.round() as i64));
    log["n_got"] = json!(out["candidates"].as_array().map_or(0, Vec::len));
    log["warnings"] = json!(
        out["warnings"]
            .as_array()
            .map(|w| w.iter().map(|x| x["code"].clone()).collect::<Vec<_>>())
    );
    log["compute_s"] = json!(t0.elapsed().as_secs_f64());
    Reply {
        status: 200,
        body: out,
        log: seal(log, false),
    }
}
