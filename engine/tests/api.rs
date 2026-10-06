//! Routeur `GET /api/plan` (api.md v1) : validation stricte, statuts HTTP, Turnstile, appel
//! interne, simplification. La partie bout en bout utilise les dalles de test (`common::tiles`).
use engine::Code;
use engine::api::{self, handle, http_status, parse, simplify};
use ring::hmac;
use serde_json::{Value, json};

mod common;

fn key() -> hmac::Key {
    hmac::Key::new(hmac::HMAC_SHA256, b"cle-de-test-de-32-octets-au-moins!")
}

fn q(s: &str) -> Vec<(String, String)> {
    s.split('&')
        .filter(|x| !x.is_empty())
        .map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (k.to_string(), v.to_string())
        })
        .collect()
}

fn code(s: &str) -> Code {
    parse(&q(s)).expect_err("erreur attendue").code
}

#[test]
fn validation_stricte() {
    let ok = "lat=48.73&lon=2.27";
    assert_eq!(code(&format!("{ok}&foo=1")), Code::InvalidRequest);
    assert_eq!(code(&format!("{ok}&seed=1&seed=2")), Code::InvalidRequest);
    assert_eq!(code("lat=48.73"), Code::InvalidRequest);
    assert_eq!(code(&format!("{ok}&distance_km=NaN")), Code::InvalidRequest);
    assert_eq!(code(&format!("{ok}&distance_km=inf")), Code::InvalidRequest);
    assert_eq!(code(&format!("{ok}&goal=fast")), Code::ModeUnknown);
    assert_eq!(code(&format!("{ok}&roads=pedestrian")), Code::RoadsUnknown);
    assert_eq!(code(&format!("{ok}&n_candidates=5")), Code::InvalidRequest);
    assert_eq!(code(&format!("{ok}&seed=1000")), Code::InvalidRequest);
    assert_eq!(code(&format!("{ok}&seed=-1")), Code::InvalidRequest);
    assert_eq!(
        code(&format!("{ok}&max_grade_pct=70")),
        Code::MaxGradeInvalid
    );
    assert_eq!(
        code(&format!("{ok}&no_repeat_junction=yes")),
        Code::InvalidRequest
    );
    assert_eq!(
        code(&format!("{ok}&goal=target&dplus_m=6000")),
        Code::DplusOutOfRange
    );
    let poly = |n: usize| {
        (0..n)
            .map(|i| {
                let a = std::f64::consts::TAU * i as f64 / n as f64;
                format!("{},{}", 2.27 + 0.001 * a.cos(), 48.73 + 0.001 * a.sin())
            })
            .collect::<Vec<_>>()
            .join(";")
    };
    assert_eq!(
        code(&format!("{ok}&polygon={}", poly(51))),
        Code::ZoneInvalid
    );
    assert_eq!(
        code(&format!("{ok}&polygon=2.2,48.7;x")),
        Code::InvalidRequest
    );
    assert_eq!(
        code(&format!(
            "{ok}&polygon=2.26,48.72;2.28,48.74;2.28,48.72;2.26,48.74"
        )),
        Code::ZoneInvalid
    );
    let (r, debug) = parse(&q(&format!(
        "{ok}&polygon={}&climbs=short&debug=1",
        poly(50)
    )))
    .unwrap();
    assert!(debug);
    assert_eq!(r.polygon.unwrap().len(), 50);
    assert_eq!(
        (r.n_candidates, r.node_simple, r.climbs.as_str()),
        (3, true, "short")
    );
    assert_eq!(
        (r.mode.as_str(), r.distance_km, r.tol, r.enforce_limits),
        ("max", 10.0, 0.05, true)
    );
    assert_eq!(r.max_compute_s, Some(api::MAX_COMPUTE_S));
    assert_eq!(r.max_grade, Some(0.6)); // D31 : 60 % par défaut
    assert_eq!(
        parse(&q(&format!("{ok}&max_grade_pct=0")))
            .unwrap()
            .0
            .max_grade,
        None
    );
    assert_eq!(
        code(&format!("{ok}&max_grade_pct=3")),
        Code::MaxGradeInvalid
    );
    let (r, _) = parse(&q(&format!(
        "{ok}&goal=min_distance&dplus_m=500&max_grade_pct=30"
    )))
    .unwrap();
    assert_eq!(
        (r.mode.as_str(), r.target_dplus, r.max_grade),
        ("min_distance", Some(500.0), Some(0.3))
    );
}

#[test]
fn statuts_http() {
    assert_eq!(http_status(Code::InvalidRequest), 400);
    assert_eq!(http_status(Code::ClimbsUnknown), 400);
    assert_eq!(http_status(Code::BotCheckFailed), 403);
    assert_eq!(http_status(Code::OutsideCoverage), 422);
    assert_eq!(http_status(Code::DplusUnreachableProven), 422);
    assert_eq!(http_status(Code::Timeout), 504);
    assert_eq!(http_status(Code::InvariantViolated), 500);
}

#[test]
fn douglas_peucker() {
    // ligne droite de 100 points + un détour de 5 m au milieu
    let n = 101;
    let lat: Vec<f64> = (0..n).map(|i| 48.7 + 1e-5 * i as f64).collect();
    let lon: Vec<f64> = (0..n)
        .map(|i| if i == 50 { 2.27 + 7e-5 } else { 2.27 })
        .collect();
    let mut c =
        json!({"lat": lat, "lon": lon, "ele": vec![1.0; n], "dist": (0..n).collect::<Vec<_>>()});
    simplify(&mut c, 1.0);
    assert_eq!(c["lat"].as_array().unwrap().len(), 5); // extrémités, sommet, ses deux voisins
    for k in ["lon", "ele", "dist"] {
        assert_eq!(c[k].as_array().unwrap().len(), 5);
    }
    assert_eq!(c["dist"][4], json!(100));
}

#[test]
fn turnstile_local_et_detail() {
    let never =
        |_: &str| -> Result<bool, String> { panic!("siteverify ne doit pas être appelé") };
    // E3 : jeton vide ou trop long refusé sans appel réseau ; appel interne exempté
    assert!(api::check_bot(None, true, never).is_ok());
    assert_eq!(
        api::check_bot(Some(""), false, never).unwrap_err().code,
        Code::BotCheckFailed
    );
    let long = "x".repeat(api::MAX_TOKEN_LEN + 1);
    assert_eq!(
        api::check_bot(Some(&long), false, never).unwrap_err().code,
        Code::BotCheckFailed
    );
    assert!(api::check_bot(Some(&long[1..]), false, |_| Ok(true)).is_ok());
    // `detail` seulement en 400
    let m = engine::Msg::error(Code::BotCheckFailed, "siteverify: timeout");
    assert!(api::error_body(&m, 403)["error"].get("detail").is_none());
    let m = engine::Msg::error(Code::InvalidRequest, "unknown parameter x");
    assert_eq!(
        api::error_body(&m, 400)["error"]["detail"],
        json!("unknown parameter x")
    );
}

#[test]
fn bout_en_bout_dalles_pilotes() {
    let Some((_, store)) = common::tiles(&[common::MASSY, common::BOURG]) else {
        return;
    };
    let key = key();
    let massy = q("lat=48.7309&lon=2.2713&distance_km=10&n_candidates=1");
    let never = |_: &str| -> Result<bool, String> { panic!("Turnstile ne doit pas être appelé") };
    // validation et couverture avant Turnstile (le jeton n'est pas consommé)
    assert_eq!(
        handle(&q("lat=48.7&lon=2.2&x=1"), None, false, &store, &key, never).status,
        400
    );
    let r = handle(&q("lat=43.3&lon=5.4"), None, false, &store, &key, never);
    assert_eq!(
        (r.status, r.body["error"]["code"].clone()),
        (422, json!("outside_coverage"))
    );
    assert!(r.body["error"].get("detail").is_none());
    // Turnstile : jeton absent, refusé, ou erreur réseau -> 403, rien de calculé
    assert_eq!(handle(&massy, None, false, &store, &key, never).status, 403);
    assert_eq!(
        handle(&massy, Some("t"), false, &store, &key, |_| Ok(false)).status,
        403
    );
    let r = handle(&massy, Some("t"), false, &store, &key, |_| {
        Err("down".into())
    });
    assert_eq!((r.status, r.log["detail"].clone()), (403, json!("down")));
    assert!(r.body["error"].get("detail").is_none(), "{}", r.body);
    assert_eq!(r.log["accepted"], json!(false));
    // diagnose=1 (D46) : Turnstile requis, valeur validée, réponse sans géométrie
    let dg = q(
        "lat=48.7309&lon=2.2713&goal=target&distance_km=10&dplus_m=300&roads=unpaved&n_candidates=1&diagnose=1",
    );
    assert_eq!(handle(&dg, None, false, &store, &key, never).status, 403);
    assert_eq!(
        handle(
            &q("lat=48.7309&lon=2.2713&diagnose=2"),
            None,
            true,
            &store,
            &key,
            never
        )
        .status,
        400
    );
    let r = handle(&dg, Some("t"), false, &store, &key, |_| Ok(true));
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(r.body.get("candidates").is_none() && r.body["params"]["asked"] == 1);
    assert_eq!(r.body["suggest"], json!({"roads": "minor"}));
    // appel interne (smoke test deploy.yml) : pas de Turnstile
    let r = handle(&massy, None, true, &store, &key, never);
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.log["accepted"], json!(true));
    assert!(r.log["compute_s"].as_f64().is_some_and(|s| s > 0.0));
    let c = &r.body["candidates"][0];
    let (len, dplus) = (
        c["length_m"].as_f64().unwrap(),
        c["dplus_m"].as_f64().unwrap(),
    );
    assert!(
        dplus >= 300.0 && (9000.0..=11000.0).contains(&len),
        "{len} {dplus}"
    );
    let pts = c["lat"].as_array().unwrap().len();
    assert!(
        pts < (len / 5.0) as usize / 2,
        "simplification : {pts} points"
    );
    assert!(r.body.get("debug").is_none());
    assert!(r.body["solver_version"].is_string() && r.body["data_version"].is_string());
    // log : une ligne, coordonnées au km seulement
    let log = r.log.to_string();
    assert!(!log.contains("48.73") && !log.contains("2.27"), "{log}");
    assert_eq!(r.log["status"], json!(200));
    // avec jeton valide
    let r = handle(&massy, Some("t"), false, &store, &key, |t| Ok(t == "t"));
    assert_eq!(r.status, 200);
    // la réponse se partage telle quelle (corps de POST /api/loops construit comme le front)
    let b = &r.body;
    let body = json!({
        "request": {"lat": "48.7309", "lon": "2.2713", "distance_km": "10", "n_candidates": "1"},
        "data_version": b["data_version"], "solver_version": b["solver_version"],
        "effective_start": b["effective_start"], "zone": b["zone"],
        "candidate": b["candidates"][0], "warnings": b["warnings"],
    });
    // D31 : filtre 60 % par défaut sur 50 m ; boucle du Bourg (lacets coupés) affichée ≤ 60 %
    let r = handle(
        &q("lat=45.0555&lon=6.0310&distance_km=15&seed=0"),
        None,
        true,
        &store,
        &key,
        never,
    );
    assert_eq!(r.status, 200, "{}", r.body);
    for c in r.body["candidates"].as_array().unwrap() {
        assert!(
            c["max_grade_pct"].as_f64().unwrap() <= 60.0,
            "{}",
            c["max_grade_pct"]
        );
    }
    let stored = share::validate(body.to_string().as_bytes(), &key)
        .expect("réponse de /api/plan partageable");
    assert!(stored.len() < 100_000, "{} octets", stored.len());
    // M3 : boucle modifiée, avertissement retiré, ou autre clé -> refus
    let mut t = body.clone();
    t["candidate"]["lat"][1] = json!(t["candidate"]["lat"][1].as_f64().unwrap() + 1e-6);
    assert!(share::validate(t.to_string().as_bytes(), &key).is_err());
    let mut t = body.clone();
    t["warnings"] = json!([]);
    assert!(
        b["warnings"].as_array().unwrap().is_empty()
            || share::validate(t.to_string().as_bytes(), &key).is_err()
    );
    let other = hmac::Key::new(hmac::HMAC_SHA256, b"une-autre-cle-de-32-octets-au-moins");
    assert!(share::validate(body.to_string().as_bytes(), &other).is_err());
    // nombres réécrits par le navigateur (10494.0 -> 10494) : même signature
    let js: Value =
        serde_json::from_str(&body.to_string().replace(".0,", ",").replace(".0]", "]")).unwrap();
    assert!(share::validate(js.to_string().as_bytes(), &key).is_ok());
}

/// I1 : au-delà de 40 km, au plus 2 boucles, avec avertissement. Avec la seule dalle du départ
/// (20 km de côté), le disque de 21 km de rayon sort de la couverture : `coverage_edge`.
#[test]
fn deux_boucles_au_dela_de_40_km_et_bord_de_couverture() {
    let Some((_, mut store)) = common::tiles(&[common::MASSY]) else {
        return;
    };
    let start = store
        .tile_at(common::MASSY.0, common::MASSY.1)
        .unwrap()
        .clone();
    store.manifest.tiles.retain(|_, t| *t == start);
    let never = |_: &str| -> Result<bool, String> { panic!() };
    let r = handle(
        &q("lat=48.7309&lon=2.2713&distance_km=42&n_candidates=4"),
        None,
        true,
        &store,
        &key(),
        never,
    );
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(r.body["candidates"].as_array().unwrap().len() <= 2);
    let warn = |c: &str| {
        r.body["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|w| w["code"] == c)
            .cloned()
            .collect::<Vec<Value>>()
    };
    assert_eq!(
        warn("candidates_reduced"),
        vec![json!({"code": "candidates_reduced", "params": {"max_n": 2, "km": 40.0}})]
    );
    assert_eq!(warn("coverage_edge").len(), 1);
}

// ---- Partage : POST /api/loops, GET /api/loops/<id> (api.md v1.1) ----
use engine::share::{self, Store};

/// Corps de partage signé avec `key()` (comme le ferait /api/plan).
fn shared(c: Value) -> Value {
    let mut v = unsigned(c);
    let s: share::SharedLoop = serde_json::from_value(v.clone()).unwrap();
    v["candidate"]["sig"] = json!(share::signature(&key(), &s));
    v
}

fn unsigned(c: Value) -> Value {
    json!({
        "request": {"lat": "48.7309", "lon": 2.2713, "distance_km": 10, "no_repeat_junction": true, "seed": "7"},
        "data_version": "bdtopo-wfs-2026-10", "solver_version": "0.1.0+abc1234",
        "effective_start": {"lat": 48.73, "lon": 2.27, "kind": "clicked", "moved_m": 3.2, "access_m": null},
        "zone": {"geometry": {"type": "MultiPolygon", "coordinates": [[[[2.2, 48.7], [2.3, 48.7], [2.3, 48.8], [2.2, 48.7]]]]},
                 "area_km2": 78.5, "reduced_radius_km": null},
        "candidate": c,
        "warnings": [{"code": "access_round_trip", "params": {"access_m": 54.0}}],
    })
}

fn cand() -> Value {
    json!({"length_m": 10494.0, "dplus_m": 433.1, "feasible": true, "alt_min_m": 55.6, "alt_max_m": 159.1,
           "max_grade_pct": 48.8, "target_gap": null,
           "climbs": {"count": 8, "longest_gain_m": 67.2, "longest_len_m": 626.0, "gbar_m": 44.0, "mean_grade_pct": 8.2},
           "via": [{"n": 1, "lat": 48.74, "lon": 2.28, "snap_m": 3.0, "dist_m": 1000.0}],
           "legs": [{"from": 0, "to": 1, "length_m": 1000.0, "dplus_m": 20.0, "dminus_m": 0.0},
                    {"from": 1, "to": 0, "length_m": 1000.0, "dplus_m": 0.0, "dminus_m": 20.0}],
           "landmarks": [{"kind": "col", "name": "Col X", "ele_m": null, "dist_m": 1000.0, "lat": 48.74, "lon": 2.28}],
           "lat": [48.73, 48.74, 48.73], "lon": [2.27, 2.28, 2.27], "ele": [60.0, 80.0, 60.0], "dist": [0.0, 1000.0, 2000.0]})
}

fn invalid(v: &Value) -> bool {
    share::validate(v.to_string().as_bytes(), &key()).is_err_and(|m| m.code == Code::InvalidRequest)
}

#[test]
fn partage_schema_strict() {
    let ok = shared(cand());
    let stored: Value =
        serde_json::from_slice(&share::validate(ok.to_string().as_bytes(), &key()).unwrap())
            .unwrap();
    assert_eq!(stored["candidate"]["lat"], ok["candidate"]["lat"]);
    assert!(
        stored["candidate"].get("sig").is_none(),
        "signature non stockée"
    );
    let with = |f: &dyn Fn(&mut Value)| {
        let mut v = shared(cand());
        f(&mut v);
        v
    };
    // M3 : signature absente, mal formée (non ASCII) ou ne couvrant pas la géométrie
    assert!(invalid(&unsigned(cand())));
    assert!(invalid(&with(
        &|v| v["candidate"]["sig"] = json!("é".repeat(32))
    )));
    assert!(invalid(&with(&|v| v["candidate"]["ele"][1] = json!(81.0))));
    assert!(invalid(&with(
        &|v| v["effective_start"]["lat"] = json!(48.0)
    )));
    assert!(invalid(&with(&|v| v["extra"] = json!(1))));
    assert!(invalid(&with(
        &|v| v["candidate"]["html"] = json!("<script>")
    )));
    assert!(invalid(&with(
        &|v| v["candidate"]["ele"] = json!([1.0, 2.0])
    )));
    assert!(invalid(&with(&|v| v["candidate"]["lat"][0] = json!(91.0))));
    assert!(invalid(&with(&|v| v["candidate"]["lat"][0] = json!("48"))));
    assert!(invalid(&with(&|v| v["request"]["foo"] = json!("1"))));
    assert!(invalid(&with(&|v| v["request"]["n_candidates"] = json!(9))));
    assert!(invalid(&with(&|v| v["request"]["polygon"] = json!([1, 2]))));
    assert!(invalid(&with(
        &|v| v["effective_start"]["kind"] = json!("teleported")
    )));
    assert!(invalid(&with(
        &|v| v["warnings"][0]["code"] = json!("timeout")
    )));
    assert!(invalid(&with(
        &|v| v["warnings"][0]["code"] = json!("made_up")
    )));
    assert!(invalid(&with(
        &|v| v["warnings"][0]["params"] = json!({"access_m": "<b>"})
    )));
    assert!(invalid(&with(&|v| v["warnings"][0]["params"] = json!({}))));
    assert!(invalid(&with(&|v| v["solver_version"] = json!("a b"))));
    assert!(invalid(&with(
        &|v| v["zone"]["geometry"]["type"] = json!("Point")
    )));
    assert!(share::validate(&vec![b' '; share::MAX_BODY + 1], &key()).is_err());
    assert!(share::validate(b"{", &key()).is_err());
    // facultatifs absents
    let mut v = unsigned(cand());
    v.as_object_mut().unwrap().remove("zone");
    v["candidate"].as_object_mut().unwrap().remove("target_gap");
    let s: share::SharedLoop = serde_json::from_value(v.clone()).unwrap();
    v["candidate"]["sig"] = json!(share::signature(&key(), &s));
    assert!(!invalid(&v));
}

#[test]
fn partage_ids() {
    let ids: std::collections::HashSet<String> = (0..2000).map(|_| share::new_id()).collect();
    assert_eq!(ids.len(), 2000);
    assert!(
        ids.iter()
            .all(|i| share::valid_id(i) && i.len() == share::ID_LEN)
    );
    for bad in [
        "",
        "abc",
        "abcdefghijk/",
        "../../etc/pa",
        "abcdefghijklm",
        "abcdefghij.k",
    ] {
        assert!(!share::valid_id(bad), "{bad}");
    }
}

#[test]
fn partage_post_get() {
    let body = shared(cand()).to_string();
    let k = key();
    let never = |_: &str| -> Result<bool, String> { panic!("Turnstile ne doit pas être appelé") };
    let no_put = |_: &str, _: &[u8]| -> Result<(), String> { panic!("rien ne doit être écrit") };
    // corps invalide : 400 sans consommer le jeton ni écrire
    assert_eq!(
        share::post(b"{}", Some("t"), false, &k, never, no_put).status,
        400
    );
    // Turnstile : absent ou refusé -> 403, rien d'écrit
    assert_eq!(
        share::post(body.as_bytes(), None, false, &k, never, no_put).status,
        403
    );
    assert_eq!(
        share::post(body.as_bytes(), Some("t"), false, &k, |_| Ok(false), no_put).status,
        403
    );
    // stockage en échec -> 503 busy
    let r = share::post(
        body.as_bytes(),
        Some("t"),
        false,
        &k,
        |_| Ok(true),
        |_, _| Err("s3 down".into()),
    );
    assert_eq!(
        (r.status, r.body["error"]["code"].clone()),
        (503, json!("busy"))
    );
    // aller-retour par le dossier local
    let dir = std::env::temp_dir().join(format!("optrail-share-test-{}", std::process::id()));
    let store = Store::Dir(dir.clone());
    let r = share::post(
        body.as_bytes(),
        Some("t"),
        false,
        &k,
        |t| Ok(t == "t"),
        |id, d| store.put(id, d),
    );
    assert_eq!(r.status, 201, "{}", r.body);
    let id = r.body["id"].as_str().unwrap().to_string();
    assert!(share::valid_id(&id));
    let g = share::get(&id, |i| store.get(i));
    assert_eq!(g.status, 200);
    assert_eq!(g.body["candidate"], cand());
    // inconnu, ou mal formé (pas de lecture) -> 404 loop_not_found
    let other = if id.starts_with('0') { "1" } else { "0" }.to_string() + &id[1..];
    let g = share::get(&other, |i| store.get(i));
    assert_eq!(
        (g.status, g.body["error"]["code"].clone()),
        (404, json!("loop_not_found"))
    );
    assert_eq!(
        share::get("../../x", |_| panic!("pas de lecture")).status,
        404
    );
    // appel interne : sans Turnstile
    assert_eq!(
        share::post(body.as_bytes(), None, true, &k, never, |id, d| store
            .put(id, d))
        .status,
        201
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// Vecteur de la documentation AWS (SigV4, « GET Object » avec Range).
#[test]
fn sigv4_vecteur_aws() {
    let h = |k: &'static str, v: &str| (k, v.to_string());
    let headers = [
        h("host", "examplebucket.s3.amazonaws.com"),
        h("range", "bytes=0-9"),
        h(
            "x-amz-content-sha256",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
        h("x-amz-date", "20130524T000000Z"),
    ];
    let auth = share::sigv4(
        "GET",
        "/test.txt",
        &headers,
        "20130524T000000Z",
        "us-east-1",
        "s3",
        "AKIAIOSFODNN7EXAMPLE",
        "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
    );
    assert_eq!(
        auth,
        "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, \
         SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, \
         Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
    );
    let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_369_353_600); // 2013-05-24
    assert_eq!(share::amz_date(t), "20130524T000000Z");
    let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(951_782_400 + 3661); // 2000-02-29
    assert_eq!(share::amz_date(t), "20000229T010101Z");
}
