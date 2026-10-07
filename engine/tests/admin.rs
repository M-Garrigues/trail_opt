//! Admin (contracts/admin.md) : clé et limitation, période, requêtes Insights simulées → réponse
//! de `/api/admin/stats`, départ arrondi à 500 m.
use std::time::{Duration, Instant};

use engine::Code;
use engine::admin::{self, Admin, Limiter, Views};
use engine::api::grid500;
use serde_json::{Value, json};

const IP: &str = "203.0.113.7";
const KEY: &str = "0123456789abcdef0123456789abcdef";

#[test]
fn cle_admin() {
    // vide : admin désactivé ; courte : refusée au démarrage en prod, acceptée en local
    assert!(Admin::from_key("", false).unwrap().is_none());
    assert!(Admin::from_key("123", false).is_err());
    assert!(Admin::from_key(&KEY[1..], false).is_err());
    let local = Admin::from_key("123", true).unwrap().unwrap();
    let now = Instant::now();
    assert_eq!(local.check(Some("123"), IP, now), Ok(()));
    assert_eq!(local.check(Some("1234"), IP, now), Err(Code::AdminDenied));
    let a = Admin::from_key(KEY, false).unwrap().unwrap();
    assert_eq!(a.check(Some(KEY), IP, now), Ok(()));
    assert_eq!(a.check(None, IP, now), Err(Code::AdminDenied));
    assert_eq!(a.check(Some(&KEY[..31]), IP, now), Err(Code::AdminDenied));
}

#[test]
fn limitation_des_essais() {
    let a = Admin::from_key(KEY, false).unwrap().unwrap();
    let t = Instant::now();
    for i in 0..admin::MAX_FAILS {
        let at = t + Duration::from_secs(u64::from(i) * 60);
        assert_eq!(a.check(Some("x"), IP, at), Err(Code::AdminDenied));
    }
    // 5 échecs dans la fenêtre : verrou, même avec la bonne clé, jusqu'à sa fin
    let late = t + admin::WINDOW - Duration::from_secs(1);
    assert_eq!(a.check(Some(KEY), IP, late), Err(Code::AdminLocked));
    // verrou par adresse (revue F5) : le fondateur, ailleurs, entre toujours
    assert_eq!(a.check(Some(KEY), "198.51.100.1", late), Ok(()));
    assert_eq!(a.check(Some(KEY), IP, t + admin::WINDOW), Ok(()));
    // un succès ne remet pas le compteur à zéro dans la fenêtre, mais 4 échecs ne verrouillent pas
    let b = Admin::from_key(KEY, false).unwrap().unwrap();
    for _ in 0..4 {
        let _ = b.check(Some("x"), IP, t);
    }
    assert_eq!(b.check(Some(KEY), IP, t), Ok(()));
}

#[test]
fn depart_arrondi_500_m() {
    assert_eq!(grid500(48.7309, 2.2713), Some((48.7305, 2.2685)));
    assert_eq!(grid500(46.5, 5.0), Some((46.4985, 4.9985)));
    // la Réunion : hémisphère sud
    assert_eq!(grid500(-21.1151, 55.5364), Some((-21.114, 55.536)));
    // centre de cellule : deux points proches de la même cellule donnent le même centre
    assert_eq!(grid500(45.18451, 5.72649), grid500(45.1863, 5.7286));
    assert_eq!(grid500(45.18451, 5.72649), Some((45.1845, 5.7265)));
    assert_eq!(grid500(f64::NAN, 2.0), None);
    assert_eq!(grid500(91.0, 2.0), None);
    let (la, lo) = grid500(44.123_456, 3.987_654).unwrap();
    assert_eq!(
        ((la * 1e4).round() / 1e4, (lo * 1e4).round() / 1e4),
        (la, lo)
    );
}

fn q(s: &str) -> Vec<(String, String)> {
    s.split('&')
        .filter(|x| !x.is_empty())
        .map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (k.into(), v.into())
        })
        .collect()
}

#[test]
fn periode() {
    let today = admin::parse_day("2026-10-07").unwrap();
    assert_eq!(admin::day_str(today), "2026-10-07");
    assert_eq!(admin::day_str(0), "1970-01-01");
    let r = |s: &str| admin::parse_range(&q(s), today);
    let (f, t) = r("").unwrap();
    assert_eq!((admin::day_str(f).as_str(), t), ("2026-09-08", today));
    let (f, t) = r("from=2026-01-01&to=2026-01-31").unwrap();
    assert_eq!(t - f, 30);
    assert!(r("from=2025-01-01&to=2026-10-07").is_err()); // > 400 j
    assert!(r("from=2026-10-07&to=2026-10-01").is_err()); // inversé
    for bad in [
        "from=2026-02-30",
        "from=26-10-01",
        "to=2026-1-01",
        "from=x",
        "day=2026-10-01",
    ] {
        assert_eq!(r(bad).unwrap_err().code, Code::InvalidRequest, "{bad}");
    }
    assert_eq!(
        admin::parse_day("2024-02-29"),
        Some(admin::parse_day("2024-02-28").unwrap() + 1)
    );
}

/// Résultats Insights figés (format de GetQueryResults), par vue.
fn canned(name: &str) -> Value {
    let rows: Vec<Vec<(&str, &str)>> = match name {
        "visitors" => vec![vec![
            ("day", "2026-10-06 00:00:00.000"),
            ("visitors", "12"),
            ("hits", "30"),
        ]],
        "geo" => vec![
            vec![
                ("country", "FR"),
                ("region", "IDF"),
                ("dev", "mobile"),
                ("visitors", "8"),
                ("hits", "20"),
            ],
            vec![
                ("country", "FR"),
                ("region", "ARA"),
                ("dev", "desktop"),
                ("visitors", "3"),
                ("hits", "5"),
            ],
            vec![("dev", "desktop"), ("visitors", "1"), ("hits", "5")],
        ],
        "refs" => vec![vec![("ref", "google.com"), ("hits", "4")]],
        // v1 (sans code en 200) et v2 mélangés
        "calcs" => vec![
            vec![
                ("day", "2026-10-06 00:00:00.000"),
                ("accepted", "1"),
                ("n", "13"),
            ],
            vec![
                ("day", "2026-10-06 00:00:00.000"),
                ("accepted", "1"),
                ("code", "timeout"),
                ("n", "2"),
            ],
            vec![
                ("day", "2026-10-06 00:00:00.000"),
                ("accepted", "0"),
                ("code", "bot_check_failed"),
                ("n", "9"),
            ],
            vec![
                ("day", "2026-10-07 00:00:00.000"),
                ("accepted", "1"),
                ("code", "ok"),
                ("n", "5"),
            ],
        ],
        "perf" => vec![vec![("p50", "4.1289"), ("p95", "9.6447"), ("n", "18")]],
        "mix" => vec![
            vec![("goal", "max_dplus"), ("climbs", "balanced"), ("n", "6")],
            vec![
                ("goal", "target"),
                ("surface", "trail"),
                ("climbs", "balanced"),
                ("n", "3"),
            ],
        ],
        "hist" => vec![
            vec![
                ("goal", "target"),
                ("surface", "trail"),
                ("n_got", "3"),
                ("km_bin", "10"),
                ("dplus_bin", "250"),
                ("n", "3"),
            ],
            vec![
                ("goal", "max_dplus"),
                ("n_got", "1"),
                ("km_bin", "10"),
                ("dplus_bin", "500"),
                ("n", "1"),
            ],
            vec![
                ("goal", "target"),
                ("surface", "trail"),
                ("n_got", "2"),
                ("km_bin", "5"),
                ("dplus_bin", "250"),
                ("n", "2"),
            ],
        ],
        "events_day" => vec![
            vec![
                ("day", "2026-10-06 00:00:00.000"),
                ("event", "gpx"),
                ("n", "2"),
            ],
            vec![
                ("day", "2026-10-06 00:00:00.000"),
                ("event", "share_click"),
                ("n", "3"),
            ],
            vec![
                ("day", "2026-10-07 00:00:00.000"),
                ("event", "shared_open"),
                ("n", "1"),
            ],
        ],
        "events_mix" => vec![
            vec![
                ("event", "share_created"),
                ("goal", "target"),
                ("surface", "trail"),
                ("rank", "1"),
                ("km_bin", "10"),
                ("dplus_bin", "250"),
                ("n", "2"),
            ],
            vec![
                ("event", "gpx"),
                ("goal", "target"),
                ("surface", "trail"),
                ("rank", "2"),
                ("km_bin", "10"),
                ("dplus_bin", "250"),
                ("n", "1"),
            ],
            vec![
                ("event", "gpx"),
                ("goal", "max_dplus"),
                ("rank", "1"),
                ("km_bin", "10"),
                ("dplus_bin", "500"),
                ("n", "1"),
            ],
        ],
        "starts" => vec![
            vec![
                ("start_lat", "45.1845"),
                ("start_lon", "5.7265"),
                ("code", "ok"),
                ("n", "3"),
            ],
            vec![
                ("start_lat", "45.1845"),
                ("start_lon", "5.7265"),
                ("code", "timeout"),
                ("n", "1"),
            ],
            vec![
                ("start_lat", "43.6"),
                ("start_lon", "1.443"),
                ("code", "outside_coverage"),
                ("n", "2"),
            ],
        ],
        _ => return json!({"status": "Running"}), // perf_day : jamais terminée
    };
    let results: Vec<Value> = rows
        .iter()
        .map(|r| {
            r.iter()
                .map(|(f, v)| json!({"field": f, "value": v}))
                .collect()
        })
        .collect();
    json!({"status": "Complete", "results": results, "statistics": {"bytesScanned": 100_000.0}})
}

/// Client Logs Insights simulé : queryId = nom de la vue retrouvé par le texte de la requête.
fn fake(op: &str, body: &Value) -> Result<Value, String> {
    match op {
        "StartQuery" => {
            assert_eq!(body["logGroupName"], "/aws/lambda/optrail-api");
            assert_eq!(body["limit"], 10_000);
            let qs = body["queryString"].as_str().unwrap();
            let name = admin::queries()
                .into_iter()
                .find(|(_, q)| q == qs)
                .unwrap()
                .0;
            Ok(json!({"queryId": name}))
        }
        "GetQueryResults" => Ok(canned(body["queryId"].as_str().unwrap())),
        _ => unreachable!(),
    }
}

#[test]
fn stats_forme_de_la_reponse() {
    let today = admin::parse_day("2026-10-07").unwrap();
    let a = Admin::from_key(KEY, false).unwrap().unwrap();
    let run = |f, t| {
        admin::run(
            f,
            t,
            "/aws/lambda/optrail-api",
            fake,
            (4, Duration::ZERO),
            Duration::from_millis(50),
        )
    };
    let r = admin::handle(
        &a,
        Some(KEY),
        IP,
        &q("from=2026-10-01&to=2026-10-07"),
        today,
        run,
    );
    assert_eq!(r.status, 200, "{}", r.body);
    let b = &r.body;
    assert_eq!(
        (&b["from"], &b["to"]),
        (&json!("2026-10-01"), &json!("2026-10-07"))
    );
    // perf_day jamais terminée : null et incomplete
    assert_eq!(b["incomplete"], true);
    assert!(b["compute_s"]["by_day"].is_null());
    assert_eq!(
        (b["compute_s"]["p50"].as_f64(), b["compute_s"]["n"].as_f64()),
        (Some(4.1289), Some(18.0))
    );
    assert_eq!(b["scanned_mb"], 1.0); // 10 vues terminées × 0,1 Mo
    assert_eq!(
        b["visitors_by_day"],
        json!([{"day": "2026-10-06", "visitors": 12.0, "hits": 30.0}])
    );
    assert_eq!(
        b["calcs_by_day"],
        json!([{"day": "2026-10-06", "n": 15.0, "ok": 13.0, "rejected": 9.0},
               {"day": "2026-10-07", "n": 5.0, "ok": 5.0, "rejected": 0.0}])
    );
    assert_eq!(
        b["codes"],
        json!([{"code": "timeout", "n": 2.0, "rate": 0.1}])
    );
    assert_eq!(
        b["rejected_codes"],
        json!([{"code": "bot_check_failed", "n": 9.0}])
    );
    assert_eq!(b["goals"][0], json!({"goal": "max_dplus", "n": 6.0}));
    // champ absent de la ligne (v1) : « - »
    assert!(
        b["surfaces"]
            .as_array()
            .unwrap()
            .contains(&json!({"surface": "-", "n": 6.0}))
    );
    assert_eq!(
        b["hist_km"],
        json!({"step": 5.0, "bins": [{"from": 5, "n": 2.0}, {"from": 10, "n": 4.0}]})
    );
    assert_eq!(b["hist_dplus_m"]["bins"][0], json!({"from": 250, "n": 5.0}));
    assert_eq!(
        b["starts"],
        json!([{"lat": 45.1845, "lon": 5.7265, "n": 4.0}])
    );
    assert_eq!(
        b["starts_outside"],
        json!([{"lat": 43.6, "lon": 1.443, "n": 2.0}])
    );
    assert_eq!(
        b["countries"][0],
        json!({"country": "FR", "visitors": 11.0, "hits": 25.0})
    );
    assert!(
        b["countries"]
            .as_array()
            .unwrap()
            .contains(&json!({"country": "-", "visitors": 1.0, "hits": 5.0}))
    );
    assert_eq!(
        b["regions"][0],
        json!({"country": "FR", "region": "IDF", "visitors": 8.0, "hits": 20.0})
    );
    assert_eq!(
        b["devices"],
        json!([{"dev": "mobile", "visitors": 8.0}, {"dev": "desktop", "visitors": 4.0}])
    );
    assert_eq!(b["referrers"], json!([{"ref": "google.com", "hits": 4.0}]));
    // actions (D59)
    assert_eq!(
        b["events_by_day"],
        json!([{"day": "2026-10-06", "gpx": 2.0, "share_click": 3.0}, {"day": "2026-10-07", "shared_open": 1.0}])
    );
    let a = &b["action_rates"];
    assert_eq!(
        a["goal"],
        json!([
            {"key": "target", "calcs": 5.0, "share": 2.0, "gpx": 1.0, "share_rate": 0.4, "gpx_rate": 0.2},
            {"key": "max_dplus", "calcs": 1.0, "share": 0.0, "gpx": 1.0, "share_rate": 0.0, "gpx_rate": 1.0},
        ])
    );
    // rang r : calculs ayant rendu au moins r sorties
    assert_eq!(
        a["rank"],
        json!([
            {"key": "1", "calcs": 6.0, "share": 2.0, "gpx": 1.0, "share_rate": 0.333, "gpx_rate": 0.167},
            {"key": "2", "calcs": 5.0, "share": 0.0, "gpx": 1.0, "share_rate": 0.0, "gpx_rate": 0.2},
        ])
    );
    assert_eq!(a["km"][0]["key"], "5");
    assert_eq!(
        a["km"][1],
        json!({"key": "10", "calcs": 4.0, "share": 2.0, "gpx": 2.0, "share_rate": 0.5, "gpx_rate": 0.5})
    );
    assert_eq!(a["surface"][1]["key"], "-");
    assert_eq!(
        b["top_combos"][0],
        json!({"goal": "target", "surface": "trail", "km_bin": "10", "dplus_bin": "250", "rank": "1", "share": 2.0, "gpx": 0.0})
    );
    assert_eq!(b["top_combos"].as_array().unwrap().len(), 3);
    assert_eq!(
        (&r.log["msg"], &r.log["outcome"]),
        (&json!("admin"), &json!("ok"))
    );
    assert!(!r.log.to_string().contains(KEY));
}

#[test]
fn stats_refus_et_erreurs() {
    let today = admin::parse_day("2026-10-07").unwrap();
    let a = Admin::from_key(KEY, false).unwrap().unwrap();
    let never = |_: i64, _: i64| -> Result<Views, String> { panic!("aucune requête sans clé") };
    let r = admin::handle(&a, Some("mauvaise"), IP, &[], today, never);
    assert_eq!(
        (r.status, &r.body["error"]["code"], &r.log["outcome"]),
        (401, &json!("admin_denied"), &json!("denied"))
    );
    let r = admin::handle(&a, Some(KEY), IP, &q("from=2026-13-01"), today, never);
    assert_eq!(r.status, 400);
    // identifiants AWS expirés : message transmis à la page
    let r = admin::handle(&a, Some(KEY), IP, &[], today, |_, _| {
        Err(admin::EXPIRED.into())
    });
    assert_eq!(
        (r.status, &r.body["error"]["detail"]),
        (503, &json!(admin::EXPIRED))
    );
    assert_eq!(r.log["outcome"], "error");
    // StartQuery en échec : erreur globale
    let fail = |_: &str, _: &Value| -> Result<Value, String> { Err("AccessDenied".into()) };
    assert!(admin::run(0, 1, "g", fail, (4, Duration::ZERO), Duration::ZERO).is_err());
    for _ in 0..4 {
        admin::handle(&a, None, IP, &[], today, never);
    }
    let r = admin::handle(&a, Some(KEY), IP, &[], today, never);
    assert_eq!((r.status, &r.log["outcome"]), (429, &json!("locked")));
}

#[test]
fn plafond_par_ip() {
    let l = Limiter::new(3, Duration::from_secs(3600));
    let t = Instant::now();
    assert!((0..3).all(|_| l.allow("a", t)));
    assert!(!l.allow("a", t) && l.blocked("a", t));
    assert!(l.allow("b", t)); // autre adresse
    let later = t + Duration::from_secs(3600);
    assert!(!l.blocked("a", later) && l.allow("a", later)); // nouvelle fenêtre
}
