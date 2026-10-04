//! Comportements de la préparation portés de Python (revue C4, décision D9) : un test nommé
//! par comportement, sur des tronçons synthétiques (mêmes scénarios que tests/test_fallback.py,
//! test_revisit.py, test_ign.py, test_large.py).
use engine::Code;
use engine::l93::{self, Frame};
use engine::plan::{Request, keep_mask, plan_with};
use engine::prep::{Net, Region, disk};
use engine::tiles::Troncons;
use geo::MultiPolygon;
use serde_json::{Value, json};

const LAT: f64 = 48.7;
const LON: f64 = 2.27;
const NATURES: [&str; 11] = [
    "Bac ou liaison maritime",
    "Bretelle",
    "Chemin",
    "Escalier",
    "Piste cyclable",
    "Rond-point",
    "Route empierrée",
    "Route à 1 chaussée",
    "Route à 2 chaussées",
    "Sentier",
    "Type autoroutier",
];
const OK: u8 = 2; // praticable
const FLAT: u8 = 1; // pont / tunnel

fn natures() -> Vec<String> {
    NATURES.iter().map(|s| s.to_string()).collect()
}

fn code(name: &str) -> u8 {
    NATURES.iter().position(|&n| n == name).unwrap() as u8
}

fn relief(x: f64, y: f64) -> f64 {
    40.0 * (x / 250.0).sin() + 30.0 * (y / 180.0).cos() + 0.05 * x + 100.0
}

/// Tronçons synthétiques autour du départ (coordonnées en m relatives au départ, en L93).
struct World {
    t: Troncons,
    x0: f64,
    y0: f64,
    next: i64,
    z: fn(f64, f64) -> f64,
}

impl World {
    fn new() -> World {
        let (x0, y0) = l93::forward(LON, LAT);
        World {
            t: Troncons::new(),
            x0,
            y0,
            next: 1,
            z: relief,
        }
    }

    fn line_full(
        &mut self,
        pts: &[[f64; 2]],
        nature: &str,
        importance: u8,
        flags: u8,
        par: &[i64],
    ) -> i64 {
        let dm: Vec<[i64; 2]> = pts
            .iter()
            .map(|p| {
                [
                    ((self.x0 + p[0]) * 10.0).round() as i64,
                    ((self.y0 + p[1]) * 10.0).round() as i64,
                ]
            })
            .collect();
        let (x0, y0, z) = (self.x0, self.y0, self.z);
        let id = self.next;
        self.next += 1;
        self.t.push(
            id,
            &dm,
            move |x, y| z(x - x0, y - y0),
            code(nature),
            importance,
            flags,
            par,
        );
        id
    }

    fn trail(&mut self, pts: &[[f64; 2]]) -> i64 {
        self.line_full(pts, "Sentier", 6, OK, &[])
    }

    /// Carré (comme `_square` en Python) : 4 tronçons de 3 sommets.
    fn square(&mut self, x: f64, y: f64, side: f64) {
        let c = [[x, y], [x + side, y], [x + side, y + side], [x, y + side]];
        for k in 0..4 {
            let (a, b) = (c[k], c[(k + 1) % 4]);
            self.trail(&[a, [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0], b]);
        }
    }

    fn run(self, req: Value, prep_only: bool) -> Result<Value, engine::Msg> {
        // bornes de distance levées par défaut (les scénarios Python font moins de 2 km)
        let mut r = json!({"lat": LAT, "lon": LON, "time_s": 5.0, "enforce_limits": false});
        r.as_object_mut()
            .unwrap()
            .extend(req.as_object().unwrap().clone());
        let req: Request = serde_json::from_value(r).unwrap();
        let t = self.t;
        plan_with(move |_| Ok(t), &natures(), &req, prep_only)
    }
}

fn codes(out: &Value) -> Vec<String> {
    out["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["code"].as_str().unwrap().to_string())
        .collect()
}

fn lonlat_dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    let p = l93::forward(a.0, a.1);
    let q = l93::forward(b.0, b.1);
    (p.0 - q.0).hypot(p.1 - q.1)
}

/// Invariants d'un tracé : fermé, sans saut, longueur = abscisse finale, D+ profil = D+ annoncé.
fn check_track(lp: &Value) {
    let (lat, lon) = (lp["lat"].as_array().unwrap(), lp["lon"].as_array().unwrap());
    let ele = lp["ele"].as_array().unwrap();
    let n = lat.len();
    let pt = |i: usize| (lon[i].as_f64().unwrap(), lat[i].as_f64().unwrap());
    assert!(lonlat_dist(pt(0), pt(n - 1)) < 0.5, "tracé non fermé");
    for i in 1..n {
        assert!(lonlat_dist(pt(i - 1), pt(i)) < 6.0, "saut dans le tracé");
    }
    let dist = lp["dist"].as_array().unwrap();
    assert!((dist[n - 1].as_f64().unwrap() - lp["length_m"].as_f64().unwrap()).abs() < 2.0);
    let prof: f64 = ele
        .windows(2)
        .map(|w| (w[1].as_f64().unwrap() - w[0].as_f64().unwrap()).max(0.0))
        .sum();
    // altitudes arrondies au dm dans le JSON
    assert!((prof - lp["dplus_m"].as_f64().unwrap()).abs() < 0.05 * n as f64 + 1.0);
}

// ---------------------------------------------------------------------------
// Départ, rayon libre, ponts d'accès
// ---------------------------------------------------------------------------

/// Impasse de 170 m depuis le départ vers un carré : seul le tronçon d'accès est doublé
/// (pas le bout d'impasse derrière le départ) et la boucle fait l'aller-retour. (Le bout
/// d'impasse fait 20 m et non 50 m comme en Python : à plus de ACCESS_MIN_M du sommet le plus
/// proche, plan_loop chercherait d'abord un accès par la route.)
#[test]
fn free_radius_dead_end_start_allows_out_and_back() {
    for node_simple in [false, true] {
        let mut w = World::new();
        let (a, b) = ([0.0, -20.0], [0.0, 150.0]);
        let (c, d, e) = ([400.0, 150.0], [400.0, 550.0], [0.0, 550.0]);
        w.trail(&[a, b]);
        w.trail(&[b, c]);
        w.trail(&[c, d]);
        w.trail(&[d, e]);
        w.trail(&[e, b]);
        w.trail(&[c, [230.0, 350.0], e]);
        let out = w
            .run(
                json!({"distance_km": 1.9, "node_simple": node_simple}),
                false,
            )
            .unwrap();
        assert_eq!(out["debug"]["attempts"][0]["edges_doubled_near_start"], 1);
        assert_eq!(out["candidates"][0]["feasible"], true, "{node_simple}");
        check_track(&out["candidates"][0]);
        let l = out["candidates"][0]["length_m"].as_f64().unwrap();
        assert!((1805.0..=1995.0).contains(&l), "{l}");
    }
}

/// Sur une grille (aucun accès obligé), rien n'est doublé près du départ.
#[test]
fn no_out_and_back_farming_near_start() {
    let mut w = World::new();
    for i in -3..=3 {
        for j in -3..3 {
            let (x, y) = (i as f64 * 100.0, j as f64 * 100.0);
            w.trail(&[[x, y], [x, y + 100.0]]);
            w.trail(&[[y, x], [y + 100.0, x]]);
        }
    }
    let out = w.run(json!({"distance_km": 2.0}), false).unwrap();
    assert_eq!(out["debug"]["attempts"][0]["edges_doubled_near_start"], 0);
    assert_eq!(out["candidates"][0]["feasible"], true);
}

/// Réseau bouclable à 600 m, relié au point cliqué par une route hors type de voies : la boucle
/// part du point cliqué avec un aller-retour sur cette route (toutes classes de voies, D9).
#[test]
fn access_out_and_back_keeps_clicked_start() {
    let mut w = World::new();
    w.square(600.0, 0.0, 375.0);
    w.line_full(
        &[[0.0, 0.0], [300.0, 0.0], [600.0, 0.0]],
        "Route à 1 chaussée",
        2,
        OK,
        &[],
    );
    let out = w.run(json!({"distance_km": 2.7}), false).unwrap();
    assert_eq!(out["effective_start"]["kind"], "access");
    assert_eq!(out["effective_start"]["access_m"], 600.0);
    assert!(codes(&out).contains(&"access_round_trip".to_string()));
    let lp = &out["candidates"][0];
    let p0 = (
        lp["lon"][0].as_f64().unwrap(),
        lp["lat"][0].as_f64().unwrap(),
    );
    assert!(
        lonlat_dist(p0, (LON, LAT)) < 1.0,
        "la boucle part du point cliqué"
    );
    let l = lp["length_m"].as_f64().unwrap();
    assert!((2565.0..=2835.0).contains(&l), "{l}");
    check_track(lp);
    assert!(!codes(&out).contains(&"profile_mismatch".to_string()));
}

/// Aller-retour trop long (> ACCESS_MAX_SHARE de la distance) : refusé, départ déplacé.
#[test]
fn access_longer_than_max_share_is_refused() {
    let mut w = World::new();
    w.square(300.0, -237.5, 475.0);
    // route en lacets de 700 m jusqu'au carré : 2 × 700 m > 0,6 × 1 805 m
    let road = [
        [0.0, 0.0],
        [0.0, -200.0],
        [150.0, -200.0],
        [150.0, 0.0],
        [300.0, 0.0],
    ];
    w.line_full(&road, "Route à 1 chaussée", 2, OK, &[]);
    let out = w.run(json!({"distance_km": 1.9}), false).unwrap();
    assert!(out["effective_start"]["access_m"].is_null());
    let snap = out["effective_start"]["moved_m"].as_f64().unwrap();
    assert!((295.0..=305.0).contains(&snap), "{snap}");
    assert!(codes(&out).contains(&"start_far_from_network".to_string()));
    assert_eq!(out["candidates"][0]["feasible"], true);
}

/// Point cliqué à moins de ACCESS_MIN_M d'un sommet du réseau : pas de recherche d'accès.
#[test]
fn access_not_searched_within_min_distance() {
    let mut w = World::new();
    w.square(-200.0, 20.0, 400.0); // sommet (0, 20) à 20 m
    w.line_full(
        &[[0.0, 0.0], [0.0, -500.0]],
        "Route à 1 chaussée",
        2,
        OK,
        &[],
    );
    let out = w.run(json!({"distance_km": 1.6}), false).unwrap();
    assert!(out["effective_start"]["access_m"].is_null());
    assert!((out["effective_start"]["moved_m"].as_f64().unwrap() - 20.0).abs() < 1.0);
    assert!(codes(&out).is_empty(), "{:?}", codes(&out));
}

/// Aucune voie à moins de ACCESS_MAX_START_M du point cliqué : pas d'accès, départ au plus
/// proche (avertissement au-delà de 150 m). Le carré est à 250 m (et non 600 m comme dans le
/// test Python, qui contourne la zone) : la zone reste le disque Lmax/2 autour du point cliqué.
#[test]
fn no_access_falls_back_to_moved_start() {
    let mut w = World::new();
    w.square(250.0, -187.5, 375.0);
    let out = w.run(json!({"distance_km": 1.5}), false).unwrap();
    assert!(out["effective_start"]["access_m"].is_null());
    assert_eq!(out["effective_start"]["kind"], "clicked");
    assert!(out["effective_start"]["moved_m"].as_f64().unwrap() > 240.0);
    assert!(codes(&out).contains(&"start_far_from_network".to_string()));
    assert_eq!(out["candidates"][0]["feasible"], true);
}

/// Réseau du départ trop court : sous-réseau bouclable le plus proche (candidate_sets),
/// départ déplacé avec avertissement.
#[test]
fn fallback_to_nearest_loopable_network() {
    let mut w = World::new();
    w.square(-50.0, -50.0, 100.0); // 400 m de tour autour du départ
    w.square(250.0, -187.5, 375.0); // 1,5 km à 250 m (dans le disque Lmax/2)
    w.square(-3000.0, 0.0, 1000.0); // 4 km, hors zone
    let out = w.run(json!({"distance_km": 1.5}), false).unwrap();
    assert_eq!(out["effective_start"]["kind"], "moved");
    assert_eq!(out["candidates"][0]["feasible"], true);
    let snap = out["effective_start"]["moved_m"].as_f64().unwrap();
    assert!((245.0..=260.0).contains(&snap), "{snap}");
    assert!(codes(&out).contains(&"start_moved".to_string()));
    check_track(&out["candidates"][0]);
}

#[test]
fn start_network_used_when_possible() {
    let mut w = World::new();
    w.square(-200.0, -200.0, 400.0);
    w.square(600.0, 0.0, 375.0);
    let out = w.run(json!({"distance_km": 1.6}), false).unwrap();
    assert_eq!(out["effective_start"]["kind"], "clicked");
    assert_eq!(out["debug"]["attempts"].as_array().unwrap().len(), 1);
    assert_eq!(out["candidates"][0]["feasible"], true);
}

#[test]
fn no_loop_anywhere() {
    let mut w = World::new();
    w.square(-50.0, -50.0, 100.0);
    let e = w.run(json!({"distance_km": 2.0}), false).unwrap_err();
    assert_eq!(e.code, Code::NoLoopOfDistance);
}

// ---------------------------------------------------------------------------
// Types de voies, contraction, découpe, parallèles, pente
// ---------------------------------------------------------------------------

/// Table de `tests/test_ign.py::test_keep_mask`.
#[test]
fn keep_mask_by_road_type() {
    let cases: [(&str, u8, u8, &str, bool); 13] = [
        ("Sentier", 6, OK, "unpaved", true),
        ("Route empierrée", 5, OK, "unpaved", true),
        ("Escalier", 6, OK, "unpaved", false),
        ("Escalier", 6, OK, "pedestrian", true),
        ("Route à 1 chaussée", 5, OK, "pedestrian", false),
        ("Route à 1 chaussée", 5, OK, "minor", true),
        ("Route à 1 chaussée", 3, OK, "minor", false),
        ("Route à 1 chaussée", 3, OK, "all", true),
        ("Route à 2 chaussées", 2, OK, "all", true),
        ("Type autoroutier", 1, OK, "all", false),
        ("?", 5, OK, "all", false),
        ("Sentier", 6, 0, "all", false), // privé, ayants droit ou hors service
        ("Sentier", 6, OK | FLAT, "all", true),
    ];
    for (nature, imp, flags, roads, expected) in cases {
        let mut t = Troncons::new();
        let n = if nature == "?" { 255 } else { code(nature) };
        t.push(1, &[[0, 0], [100, 0]], |_, _| 0.0, n, imp, flags, &[]);
        assert_eq!(
            !keep_mask(&t, &natures(), roads).is_empty(),
            expected,
            "{nature} {imp} {roads}"
        );
    }
}

/// Contraction des nœuds de degré 2, sauf entre pont/tunnel et tronçon normal.
#[test]
fn contraction_merges_degree2_except_bridges() {
    for (flat, expected) in [(false, 2), (true, 3)] {
        let mut w = World::new();
        // carré de 8 tronçons, le départ (origine) sur le 1er
        let c = [[0.0, -200.0], [0.0, 200.0], [400.0, 200.0], [400.0, -200.0]];
        for k in 0..4 {
            let (a, b) = (c[k], c[(k + 1) % 4]);
            let m = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
            w.trail(&[a, m]);
            let f = if flat && k == 2 { OK | FLAT } else { OK };
            w.line_full(&[m, b], "Sentier", 6, f, &[]);
        }
        let out = w.run(json!({"distance_km": 1.6}), true).unwrap();
        assert_eq!(out["problem_stats"]["edges"], expected, "pont : {flat}");
    }
}

/// Découpe au bord de zone : un tronçon qui traverse le bord est coupé (à 5 m près).
#[test]
fn clip_cuts_edges_at_zone_border() {
    let mut w = World::new();
    w.trail(&[[-1000.0, 0.0], [1000.0, 0.0]]);
    w.trail(&[[0.0, -300.0], [0.0, 300.0]]); // entièrement dedans
    let frame = Frame::at_l93(w.x0, w.y0);
    let region = Region::new(MultiPolygon::new(vec![disk(500.0, 64)]));
    let mut net = Net::new(&w.t, frame);
    let ids = net.build(&[0, 1], &region);
    let mut lens: Vec<f64> = ids.iter().map(|&e| net.edges[e].len).collect();
    lens.sort_by(f64::total_cmp);
    assert_eq!(lens.len(), 2);
    assert!((lens[0] - 600.0).abs() < 0.5);
    assert!((lens[1] - 1000.0).abs() <= 10.0, "{}", lens[1]);
}

/// Couloirs parallèles : union des parallèles des tronçons, hors rayon libre du départ.
#[test]
fn parallel_pairs_from_tiles_outside_free_radius() {
    let mut w = World::new();
    let (a, p, q, b) = ([0.0, -300.0], [0.0, -50.0], [0.0, 50.0], [0.0, 300.0]);
    let (c, d) = ([600.0, 300.0], [600.0, -300.0]);
    w.trail(&[a, p]);
    // paire près du départ (ids 2, 3) : ignorée ; paire loin (ids 7, 8) : gardée
    w.line_full(&[p, q], "Sentier", 6, OK, &[3]);
    w.line_full(&[p, [10.0, 0.0], q], "Sentier", 6, OK, &[2]);
    w.trail(&[q, b]);
    w.trail(&[b, c]);
    w.trail(&[d, a]);
    w.line_full(&[c, d], "Sentier", 6, OK, &[8]);
    w.line_full(&[c, [610.0, 0.0], d], "Sentier", 6, OK, &[7]);
    let out = w.run(json!({"distance_km": 2.4}), true).unwrap();
    assert_eq!(out["problem_stats"]["parallel_pairs"], 1);
}

/// Pente max : les arêtes plus raides sont retirées avant le solveur.
#[test]
fn max_grade_removes_steep_edges() {
    fn steep(x: f64, y: f64) -> f64 {
        if x > 350.0 { 0.5 * y } else { 0.0 }
    }
    for (grade, ok) in [(None, true), (Some(0.3), false)] {
        let mut w = World::new();
        w.z = steep;
        w.square(0.0, -200.0, 400.0);
        let mut req = json!({"distance_km": 1.6});
        if let Some(g) = grade {
            req["max_grade"] = json!(g);
        }
        let r = w.run(req, false);
        assert_eq!(r.is_ok(), ok, "{grade:?}");
    }
}

/// Grand graphe : réduction aux arêtes pentues (w exact des profils), boucle toujours possible.
#[test]
fn steep_reduction_keeps_a_loopable_graph() {
    let mut w = World::new();
    let n = 26;
    for i in -n..n {
        for j in -n..=n {
            let (x, y) = (i as f64 * 100.0, j as f64 * 100.0);
            w.trail(&[[x, y], [x + 100.0, y]]);
            w.trail(&[[y, x], [y, x + 100.0]]);
        }
    }
    let out = w.run(json!({"distance_km": 10.0}), false).unwrap();
    let a = &out["debug"]["attempts"][0];
    eprintln!("{a}");
    assert!(a["edges_pruned"].as_u64().unwrap() > 5000);
    assert!(a["edges_reduced"].as_u64().unwrap() < a["edges_pruned"].as_u64().unwrap());
    assert_eq!(out["candidates"][0]["feasible"], true);
}

// ---------------------------------------------------------------------------
// Zone et bornes
// ---------------------------------------------------------------------------

/// Trop de tronçons dans la zone : rayon réduit autour du départ (MAX_WAYS), avertissement.
#[test]
fn zone_reduced_when_too_many_ways() {
    let mut w = World::new();
    // 120 000 tronçons hors type de voies (autoroute), sur ±10 km
    for i in 0..400 {
        for j in 0..300 {
            let (x, y) = (-10_000.0 + i as f64 * 50.0, -10_000.0 + j as f64 * 66.6);
            w.line_full(&[[x, y], [x + 10.0, y]], "Type autoroutier", 1, OK, &[]);
        }
    }
    w.square(0.0, -2000.0, 4000.0);
    let out = w.run(json!({"distance_km": 20.0, "mode": "target", "target_dplus": 100.0, "enforce_limits": true}), true).unwrap();
    assert!(
        codes(&out).contains(&"zone_reduced".to_string()),
        "{:?}",
        codes(&out)
    );
    assert!(out["debug"]["zone_km2"].as_f64().unwrap() < 314.0);
}

#[test]
fn request_limits() {
    let err = |mut req: Value| {
        req["enforce_limits"] = json!(true);
        World::new().run(req, true).unwrap_err().code
    };
    assert_eq!(err(json!({"distance_km": 1.0})), Code::DistanceOutOfRange);
    assert_eq!(err(json!({"distance_km": 101.0})), Code::DistanceOutOfRange);
    assert_eq!(
        err(json!({"distance_km": 10.0, "time_s": 200.0})),
        Code::TimeOutOfRange
    );
    assert_eq!(
        err(json!({"distance_km": 10.0, "mode": "x"})),
        Code::ModeUnknown
    );
    assert_eq!(
        err(json!({"distance_km": 10.0, "roads": "x"})),
        Code::RoadsUnknown
    );
    assert_eq!(
        err(json!({"distance_km": 10.0, "mode": "target"})),
        Code::TargetDplusRequired
    );
    assert_eq!(
        err(json!({"distance_km": 10.0, "tol": 0.9})),
        Code::ToleranceOutOfRange
    );
    assert_eq!(
        err(json!({"distance_km": 10.0, "max_grade": 0.0})),
        Code::MaxGradeInvalid
    );
    assert_eq!(
        err(json!({"distance_km": 10.0, "n_candidates": 5})),
        Code::InvalidRequest
    );
    let poly = |d: f64| {
        json!([
            [LON - d, LAT - d],
            [LON + d, LAT - d],
            [LON + d, LAT + d],
            [LON - d, LAT + d]
        ])
    };
    let off = json!([[LON + 0.1, LAT], [LON + 0.2, LAT], [LON + 0.2, LAT + 0.1]]);
    assert_eq!(
        err(json!({"distance_km": 10.0, "polygon": off})),
        Code::StartOutsideZone
    );
    let flat = json!([[LON, LAT], [LON + 0.1, LAT], [LON, LAT]]);
    assert_eq!(
        err(json!({"distance_km": 10.0, "polygon": flat})),
        Code::ZoneInvalid
    );
    assert_eq!(
        err(json!({"distance_km": 100.0, "polygon": poly(0.6)})),
        Code::ZoneTooLarge
    );
    // champ inconnu refusé (validation stricte)
    assert!(
        serde_json::from_value::<Request>(
            json!({"lat": 1.0, "lon": 1.0, "distance_km": 5.0, "x": 1})
        )
        .is_err()
    );
    // disque par défaut plafonné à 25 km de rayon : jamais trop grand
    let out = World::new().run(json!({"distance_km": 100.0, "enforce_limits": true}), true);
    assert!(out.is_err_and(|e| e.code != Code::ZoneTooLarge));
}

/// Polygone dessiné : zone = polygone ∩ disque de portée ; le départ peut être à 50 m du bord.
#[test]
fn drawn_polygon_limits_the_zone() {
    let mut w = World::new();
    w.square(-200.0, -200.0, 400.0);
    w.square(1000.0, -200.0, 400.0); // hors du polygone
    let d = 0.004; // ~300 m en latitude
    let poly = json!([
        [LON - d, LAT - d],
        [LON + d, LAT - d],
        [LON + d, LAT + d],
        [LON - d, LAT + d]
    ]);
    let out = w
        .run(json!({"distance_km": 1.6, "polygon": poly}), true)
        .unwrap();
    assert_eq!(out["problem_stats"]["edges"], 2);
    // départ à 40 m du bord du polygone : accepté
    let e = 0.0005;
    let near = json!([
        [LON + e, LAT - d],
        [LON + 2.0 * d, LAT - d],
        [LON + 2.0 * d, LAT + d],
        [LON + e, LAT + d]
    ]);
    let r = World::new().run(json!({"distance_km": 1.6, "polygon": near}), true);
    assert!(r.is_err_and(|e| e.code != Code::StartOutsideZone));
    // nœud papillon (anneau auto-intersecté) : zone_invalid, pas de réparation
    let bowtie = json!([
        [LON - d, LAT - d],
        [LON + d, LAT + d],
        [LON + d, LAT - d],
        [LON - d, LAT + d]
    ]);
    let r = World::new().run(json!({"distance_km": 1.6, "polygon": bowtie}), true);
    assert!(r.is_err_and(|e| e.code == Code::ZoneInvalid));
}

/// Montées par hystérésis de 5 m (contrat modes.md B) : petites bosses ignorées.
#[test]
fn climbs_by_hysteresis() {
    let z = [
        100.0, 103.0, 101.0, 130.0, 126.0, 140.0, 120.0, 124.0, 121.0, 150.0,
    ];
    let s: Vec<f64> = (0..z.len()).map(|i| 10.0 * i as f64).collect();
    // 100 -> 140 (le creux de 4 m ne coupe pas), puis 120 -> 150
    assert_eq!(
        engine::plan::climbs(&z, &s),
        vec![(40.0, 50.0), (30.0, 30.0)]
    );
}

// ---------------------------------------------------------------------------
// Modes D19 (contracts/modes.md)
// ---------------------------------------------------------------------------

#[test]
fn min_distance_request_limits() {
    let err = |mut req: Value| {
        req["enforce_limits"] = json!(true);
        World::new().run(req, true).unwrap_err()
    };
    let md = |x: f64| json!({"mode": "min_distance", "target_dplus": x});
    assert_eq!(
        err(json!({"mode": "min_distance"})).code,
        Code::TargetDplusRequired
    );
    let e = err(md(40.0));
    assert_eq!(
        (e.code, e.params["max_m"].as_f64()),
        (Code::DplusOutOfRange, Some(10_000.0))
    );
    assert_eq!(err(md(20_000.0)).code, Code::DplusOutOfRange);
    let mut r = md(500.0);
    r["max_distance_km"] = json!(150.0);
    assert_eq!(err(r).code, Code::DistanceOutOfRange);
    let mut r = md(500.0);
    r["climbs"] = json!("steep");
    assert_eq!(err(r).code, Code::ClimbsUnknown);
    // défaut Lcap = clamp(X / 25, 3, 60) km
    let req = |x: f64| {
        serde_json::from_value::<Request>(
            json!({"lat": 1.0, "lon": 1.0, "mode": "min_distance", "target_dplus": x}),
        )
        .unwrap()
    };
    assert_eq!(
        (
            req(50.0).cap_km(),
            req(500.0).cap_km(),
            req(5000.0).cap_km()
        ),
        (3.0, 20.0, 60.0)
    );
}

/// Boucle la plus courte de D+ >= X : réalisable, au-dessus de la borne affichée, sans
/// tolérance sous X ; X au-delà de tout le réseau : refus prouvé sans calcul.
#[test]
fn min_distance_plan() {
    let world = || {
        let mut w = World::new();
        for i in 0..4 {
            for j in 0..4 {
                w.square(-800.0 + 400.0 * i as f64, -800.0 + 400.0 * j as f64, 400.0);
            }
        }
        w
    };
    let out = world()
        .run(json!({"mode": "min_distance", "target_dplus": 60.0, "max_distance_km": 6.0, "n_candidates": 2}), false)
        .unwrap();
    let lb = out["lower_bound_m"].as_f64().unwrap();
    for c in out["candidates"].as_array().unwrap() {
        check_track(c);
        assert!(c["feasible"].as_bool().unwrap());
        assert!(c["dplus_m"].as_f64().unwrap() >= 60.0 - 0.05);
        assert!(c["length_m"].as_f64().unwrap() >= lb - 1.0);
        assert!(c["target_gap"]["dplus_m"].as_f64().unwrap() >= -0.05);
    }
    let e = world()
        .run(
            json!({"mode": "min_distance", "target_dplus": 9000.0, "max_distance_km": 6.0}),
            false,
        )
        .unwrap_err();
    assert_eq!(e.code, Code::DplusUnreachableProven);
}

/// Préférence de montées : même boucle (même D+, même longueur), seul le sens peut changer.
#[test]
fn climbs_choose_direction() {
    let world = || {
        let mut w = World::new();
        w.square(-300.0, -300.0, 600.0);
        w
    };
    let run = |c: &str| {
        world()
            .run(json!({"distance_km": 2.4, "climbs": c}), false)
            .unwrap()
    };
    let (s, l) = (run("short"), run("long"));
    let g = |o: &Value| o["candidates"][0]["climbs"]["gbar_m"].as_f64().unwrap();
    let d = |o: &Value| o["candidates"][0]["dplus_m"].as_f64().unwrap();
    assert!((d(&s) - d(&l)).abs() < 0.1);
    assert!(g(&s) <= g(&l), "{} > {}", g(&s), g(&l));
    check_track(&s["candidates"][0]);
}

/// D23 (sans dalles) : « longues » ne change pas la recherche, seulement le sens : même D+ et
/// même longueur qu'« équilibré » à graine égale, Ḡ au moins égal.
#[test]
fn long_climbs_is_direction_only() {
    let world = || {
        let mut w = World::new();
        for i in 0..4 {
            for j in 0..4 {
                w.square(-800.0 + 400.0 * i as f64, -800.0 + 400.0 * j as f64, 400.0);
            }
        }
        w
    };
    let run = |c: &str| {
        world()
            .run(json!({"distance_km": 4.0, "climbs": c, "seed": 2}), false)
            .unwrap()
    };
    let (b, l) = (run("balanced"), run("long"));
    let f = |o: &Value, k: &str| o["candidates"][0][k].as_f64().unwrap();
    assert_eq!(
        (f(&b, "dplus_m"), f(&b, "length_m")),
        (f(&l, "dplus_m"), f(&l, "length_m"))
    );
    let g = |o: &Value| o["candidates"][0]["climbs"]["gbar_m"].as_f64().unwrap();
    assert!(g(&l) >= g(&b));
}
