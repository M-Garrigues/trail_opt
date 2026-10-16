//! E2 (D70, D72, D73, D76, D78) : le moteur calcule de vrais itinéraires vélo. Un test nommé par
//! comportement, sur des tronçons synthétiques (mêmes mondes que `prep_c4.rs`) : exclusions du
//! réseau, coûts par type de voie, pente en deux valeurs, zone et distances.
use engine::Code;
use engine::l93;
use engine::plan::{
    Access, Goal, Request, check_via, class_mask, network, off_parts, plan_with, pref_mask,
};
use engine::prep::{
    CLS_MAJOR, CLS_ROAD, FREE_RADIUS, Grade, LAB_PREF, N_CLS, N_LAB, Net, Region, disk, max_climb,
    max_grade,
};
use engine::profile::{Profile, TRAIL_ANY};
use engine::tiles::{HW_MAJOR, HW_MINOR, HW_PATH, HW_PEDESTRIAN, HW_TRACK, Troncons, node_key};
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
const OK: u8 = 2;

fn natures() -> Vec<String> {
    NATURES.iter().map(|s| s.to_string()).collect()
}

fn bike(surface: &str) -> Profile {
    Profile::of("bike", surface).unwrap()
}

/// Tronçons synthétiques autour du départ (m relatifs au départ, L93) ; `z` : altitude (x, y).
struct World {
    t: Troncons,
    x0: f64,
    y0: f64,
    z: fn(f64, f64) -> f64,
}

fn rolling(x: f64, y: f64) -> f64 {
    40.0 * (x / 250.0).sin() + 30.0 * (y / 180.0).cos() + 100.0
}

impl World {
    fn new(z: fn(f64, f64) -> f64) -> World {
        let (x0, y0) = l93::forward(LON, LAT);
        World {
            t: Troncons::new(),
            x0,
            y0,
            z,
        }
    }

    /// Ajoute un tronçon ; renvoie son indice.
    fn line(&mut self, pts: &[[f64; 2]], nature: &str, importance: u8) -> usize {
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
        let id = self.t.len() as i64 + 1;
        let code = NATURES.iter().position(|&n| n == nature).unwrap() as u8;
        self.t.push(
            id,
            &dm,
            move |x, y| z(x - x0, y - y0),
            code,
            importance,
            OK,
            &[],
        );
        self.t.len() - 1
    }

    /// Carré de côté `side` en bas à gauche de (x, y) ; les 4 côtés.
    fn square(&mut self, x: f64, y: f64, side: f64, nature: &str, importance: u8) -> [usize; 4] {
        let c = [[x, y], [x + side, y], [x + side, y + side], [x, y + side]];
        [0, 1, 2, 3].map(|k| self.line(&[c[k], c[(k + 1) % 4]], nature, importance))
    }

    fn run(self, req: Value) -> Result<Value, engine::Msg> {
        let mut r = json!({"lat": LAT, "lon": LON, "time_s": 5.0, "enforce_limits": false});
        r.as_object_mut()
            .unwrap()
            .extend(req.as_object().unwrap().clone());
        let req: Request = serde_json::from_value(r).unwrap();
        let t = self.t;
        plan_with(move |_| Ok(t), &natures(), &req, false)
    }
}

fn warning_codes(out: &Value) -> Vec<String> {
    out["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["code"].as_str().unwrap().to_string())
        .collect()
}

/// Un tronçon tout seul : est-il dans le réseau / dans la liste d'accès, pour un profil ?
fn net_of(t: &Troncons, p: &Profile) -> (bool, bool) {
    let (sel, shut) = network(t, &natures(), p);
    (!sel.is_empty(), !shut.is_empty())
}

fn one(nature: &str, f: impl FnOnce(&mut Troncons)) -> Troncons {
    let mut w = World::new(rolling);
    w.line(&[[0.0, 0.0], [100.0, 0.0]], nature, 5);
    f(&mut w.t);
    w.t
}

#[test]
fn max_climb_signed() {
    // rampe de 30 % sur 100 m : montée 0,30 dans le sens u→v, 0 dans l'autre
    let s: Vec<f64> = (0..=20).map(|i| 5.0 * i as f64).collect();
    let z: Vec<f64> = s.iter().map(|x| 0.3 * x).collect();
    let c = max_climb(&z, &s);
    assert!((c[0] - 0.3).abs() < 1e-12 && c[1] == 0.0, "{c:?}");
    let zr: Vec<f64> = z.iter().rev().copied().collect();
    let c = max_climb(&zr, &s);
    assert!(c[0] == 0.0 && (c[1] - 0.3).abs() < 1e-12, "{c:?}");
    // 200 profils pseudo-aléatoires : `max_grade` = max des deux sens, au bit près (trail inchangé)
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut rnd = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    for _ in 0..200 {
        let n = 2 + (rnd() * 60.0) as usize;
        let s: Vec<f64> = (0..n).map(|i| 5.0 * i as f64 * (0.5 + rnd())).collect();
        let z: Vec<f64> = (0..n).map(|_| 100.0 * rnd()).collect();
        let c = max_climb(&z, &s);
        assert_eq!(max_grade(&z, &s).to_bits(), c[0].max(c[1]).to_bits());
    }
}

#[test]
fn grade_keeps_symmetric_is_old_filter() {
    // up == down == g : `keeps` est le filtre `grade <= g` d'avant D73
    for g in [0.05, 0.22, 0.6] {
        let gr = Grade { up: g, down: g };
        for a in [0.0, 0.04, 0.05, 0.2, 0.22, 0.25, 0.6, 0.7] {
            for b in [0.0, 0.05, 0.22, 0.23, 0.6, 0.9] {
                assert_eq!(gr.keeps([a, b]), a.max(b) <= g, "{g} {a} {b}");
                assert_eq!(gr.two_way([a, b]), a.max(b) <= g);
                for d in [0, 1] {
                    assert_eq!(gr.allows([a, b], d), a.max(b) <= g);
                }
            }
        }
    }
    // asymétrique (lot 2, 22 % montée / 35 % descente) : bande montable dans un seul sens
    let gr = Grade {
        up: 0.22,
        down: 0.35,
    };
    assert!(gr.keeps([0.10, 0.30]) && !gr.two_way([0.10, 0.30]));
    assert!(gr.allows([0.10, 0.30], 0) && !gr.allows([0.30, 0.10], 0));
    assert!(!gr.keeps([0.30, 0.30]) && !gr.keeps([0.10, 0.40]));
}

#[test]
fn request_grade_is_the_single_source() {
    let r = |v: Value| -> Request {
        let mut base = json!({"lat": LAT, "lon": LON});
        base.as_object_mut()
            .unwrap()
            .extend(v.as_object().unwrap().clone());
        serde_json::from_value(base).unwrap()
    };
    // trail : la valeur de la requête dans les deux sens, aucune = sans limite
    let t = r(json!({"max_grade": 0.4}));
    assert_eq!(t.grade(&t.profile()), Some(Grade { up: 0.4, down: 0.4 }));
    let t = r(json!({}));
    assert_eq!(t.grade(&t.profile()), None);
    // vélo : montée demandée ou du profil, descente = montée en v1 ; Route : sans limite (D78)
    // D81 : VTT 25 %
    let v = r(json!({"sport": "bike", "surface": "mtb"}));
    assert_eq!(
        v.grade(&v.profile()),
        Some(Grade {
            up: 0.25,
            down: 0.25
        })
    );
    let v = r(json!({"sport": "bike", "surface": "gravel", "max_grade": 0.3}));
    assert_eq!(v.grade(&v.profile()), Some(Grade { up: 0.3, down: 0.3 }));
    let v = r(json!({"sport": "bike", "surface": "road"}));
    let g = v.grade(&v.profile()).unwrap();
    assert!(g.keeps([5.0, 5.0]) && g.two_way([5.0, 5.0]));
    let v = r(json!({"sport": "bike", "surface": "road", "max_grade": 0.15}));
    assert_eq!(
        v.grade(&v.profile()),
        Some(Grade {
            up: 0.15,
            down: 0.15
        })
    );
}

#[test]
fn bike_closed_access() {
    // osm_bike 5 (interdit), 4 (implicite : autoroute, trunk, bridleway), 2 (pied à terre) :
    // hors réseau en vélo (gardés pour l'accès), dans le réseau en trail
    for b in [5, 4, 2] {
        let t = one("Chemin", |t| t.osm_bike[0] = b);
        assert_eq!(net_of(&t, &bike("mtb")), (false, true), "osm_bike {b}");
        assert_eq!(net_of(&t, &TRAIL_ANY), (true, false), "osm_bike {b}");
    }
    // 3 (use_sidepath) et 1 (restreint) : gardés (coût), osm_access ignoré en vélo
    for b in [0, 1, 3] {
        let t = one("Chemin", |t| t.osm_bike[0] = b);
        assert_eq!(net_of(&t, &bike("mtb")), (true, false), "osm_bike {b}");
    }
    let t = one("Chemin", |t| t.osm_access[0] = 2);
    assert_eq!(
        net_of(&t, &bike("mtb")),
        (true, false),
        "foot=no sur une piste cyclable"
    );
    assert_eq!(
        net_of(&t, &TRAIL_ANY),
        (false, true),
        "D66 inchangé en trail"
    );
}

#[test]
fn bike_oneway() {
    // D84 (palliatif 0a) : les sens uniques (1, 2) restent dans le réseau, mesurés à part ;
    // seul `osm_bike` 4 (interdit) et 2 (pied à terre) ferment encore, comme les trail
    for (v, flags, nature) in [
        (1, 0u8, "Route à 1 chaussée"),
        (2, 0, "Route à 1 chaussée"),
        (3, 0, "Route à 1 chaussée"),
        (1, 128, "Route à 1 chaussée"),
        (2, 0, "Rond-point"),
        (0, 0, "Route à 1 chaussée"),
    ] {
        let t = one(nature, |t| {
            t.osm_oneway[0] = v;
            t.osm_flags[0] = flags;
        });
        assert_eq!(
            net_of(&t, &bike("road")),
            (true, false),
            "oneway {v} flags {flags} {nature}"
        );
        assert_eq!(net_of(&t, &TRAIL_ANY), (true, false));
    }
    for b in [4, 2] {
        let t = one("Route à 1 chaussée", |t| {
            t.osm_oneway[0] = 1;
            t.osm_bike[0] = b;
        });
        assert_eq!(net_of(&t, &bike("road")), (false, true), "osm_bike {b}");
    }
}

#[test]
fn bike_oneway_is_penalized() {
    // D84 : un sens unique coûte 12 m de confort par m de plus (cost en dixièmes) ; double sens
    // cyclable (3), rond-point (bit 128) et voie à double sens : pas de surcoût
    let cost = |ow: u8, flags: u8| {
        let t = one("Route à 1 chaussée", |t| {
            t.osm_oneway[0] = ow;
            t.osm_flags[0] = flags;
        });
        let cls = class_mask(&t, &natures(), true);
        pref_mask(&t, &natures(), &bike("road"), &cls)[0]
    };
    let base = cost(0, 0);
    assert_eq!((cost(1, 0), cost(2, 0)), (base + 120, base + 120));
    assert_eq!((cost(3, 0), cost(1, 128)), (base, base));
}

#[test]
fn bike_wrong_way_measured() {
    // carré de 400 m dont les côtés 1 (est) et 3 (ouest) sont des sens uniques ; le côté 1 est
    // dessiné dans le sens trigonométrique (u→v), le côté 3 aussi
    let run = |ow: [u8; 2], flags: u8, sport: &str, surface: &str| {
        let mut w = World::new(rolling);
        let s = w.square(-200.0, -200.0, 400.0, "Route à 1 chaussée", 5);
        w.t.osm_oneway[s[1]] = ow[0];
        w.t.osm_oneway[s[3]] = ow[1];
        w.t.osm_flags[s[3]] = flags;
        w.run(json!({"sport": sport, "surface": surface, "distance_km": 1.6}))
            .unwrap()
    };
    let wrong = |o: &Value| o["debug"]["wrong_way_m"][0].as_f64();
    // un seul sens unique : le circuit choisit le sens qui le respecte, 0 m, pas d'avertissement
    let o = run([1, 0], 0, "bike", "road");
    assert_eq!(wrong(&o), Some(0.0));
    assert!(!warning_codes(&o).contains(&"oneway_against".to_string()));
    // deux sens uniques opposés l'un à l'autre : un côté (400 m) à contresens dans les deux sens
    let o = run([1, 2], 0, "bike", "road");
    let m = wrong(&o).unwrap();
    assert!((399.0..=401.0).contains(&m), "{m}");
    let w = o["warnings"].as_array().unwrap();
    let w = w
        .iter()
        .find(|x| x["code"] == json!("oneway_against"))
        .unwrap();
    assert!((m - w["params"]["m"].as_f64().unwrap()).abs() <= 0.5, "{w}");
    // double sens cyclable (3) et rond-point (bit 128) ne comptent pas
    assert_eq!(wrong(&run([1, 3], 0, "bike", "road")), Some(0.0));
    assert_eq!(wrong(&run([1, 2], 128, "bike", "road")), Some(0.0));
    // trail : ni mesure ni avertissement, quel que soit `osm_oneway`
    let o = run([1, 2], 0, "trail", "any");
    assert!(o["debug"].get("wrong_way_m").is_none());
    assert!(!warning_codes(&o).contains(&"oneway_against".to_string()));
    // sens réel du tracé RENDU (lat/lon) : le côté est est numérisé vers le nord (u→v) ; 1 impose
    // le sens trigonométrique (aire signée > 0), 2 le sens horaire (l'aller-retour d'accès est nul)
    let area = |o: &Value| {
        let c = &o["candidates"][0];
        let (la, lo) = (c["lat"].as_array().unwrap(), c["lon"].as_array().unwrap());
        let f = |v: &Value| v.as_f64().unwrap();
        (1..la.len())
            .map(|i| f(&lo[i - 1]) * f(&la[i]) - f(&lo[i]) * f(&la[i - 1]))
            .sum::<f64>()
    };
    for (ow, ccw) in [(1, true), (2, false)] {
        for surface in ["road", "gravel", "mtb"] {
            let o = run([ow, 0], 0, "bike", surface);
            assert_eq!(area(&o) > 0.0, ccw, "oneway {ow} {surface}");
            assert_eq!(wrong(&o), Some(0.0));
        }
    }
}

#[test]
fn bike_start_on_oneway_uses_access_both_ways() {
    // départ sur un sens unique (100 m) relié au réseau : l'aller-retour d'accès le prend (D72).
    // (la zone vélo est un disque de rayon L/4 : « Le plus court » avec Lcap = 2 km)
    let build = |oneway: u8| {
        let mut w = World::new(rolling);
        let c = w.line(&[[0.0, 0.0], [100.0, 0.0]], "Route à 1 chaussée", 5);
        w.t.osm_oneway[c] = oneway;
        w.square(100.0, -130.0, 260.0, "Route à 1 chaussée", 5);
        w
    };
    let req = json!({"sport": "bike", "surface": "road", "mode": "min_distance",
        "target_dplus": 50.0, "max_distance_km": 2.0});
    let out = build(1).run(req.clone()).unwrap();
    assert!(
        warning_codes(&out).contains(&"access_round_trip".to_string()),
        "{:?}",
        out["warnings"]
    );
    assert_eq!(out["effective_start"]["kind"], "access");
    // D84 : l'aller ou le retour longe le sens unique à contresens (100 m), compté dans la mesure
    let m = out["debug"]["wrong_way_m"][0].as_f64().unwrap();
    assert!((99.0..=101.0).contains(&m), "{m}");
    let out = build(0).run(req).unwrap();
    assert_eq!(out["debug"]["wrong_way_m"][0].as_f64(), Some(0.0));
}

#[test]
fn bike_stairs() {
    // Escalier (nature IGN) et bit 16 d'OSM (`steps`) : exclus en VTT sans exception, gardés en trail
    let t = one("Escalier", |_| {});
    assert!(!net_of(&t, &bike("mtb")).0 && net_of(&t, &TRAIL_ANY).0);
    let t = one("Chemin", |t| t.osm_flags[0] = 16);
    assert!(
        !net_of(&t, &bike("mtb")).0 && !net_of(&t, &bike("gravel")).0 && net_of(&t, &TRAIL_ANY).0
    );
}

#[test]
fn bike_footway_pedestrian() {
    // footway : `osm_bike` = 2 → exclu ; pedestrian : gardé (`osm_bike` 1), mais coûteux
    let t = one("Chemin", |t| {
        t.osm_bike[0] = 2;
        t.osm_hw[0] = HW_PATH;
    });
    assert!(!net_of(&t, &bike("gravel")).0);
    let t = one("Route à 1 chaussée", |t| {
        t.osm_bike[0] = 1;
        t.osm_hw[0] = HW_PEDESTRIAN;
    });
    for s in ["mtb", "gravel", "road"] {
        assert!(net_of(&t, &bike(s)).0, "{s}");
    }
    let cost = |s: &str| {
        let p = bike(s);
        let cls = class_mask(&t, &natures(), true);
        pref_mask(&t, &natures(), &p, &cls)[0]
    };
    assert_eq!((cost("mtb"), cost("gravel"), cost("road")), (5, 15, 10));
}

#[test]
fn mtb_gravel_technical() {
    let case = |f: &dyn Fn(&mut Troncons)| {
        let t = one("Chemin", |t| f(t));
        (
            net_of(&t, &bike("mtb")).0,
            net_of(&t, &bike("gravel")).0,
            net_of(&t, &TRAIL_ANY).0,
        )
    };
    // osm_mtb = mtb:scale + 1 : 5 (scale 4) exclu en VTT ; 3 (scale 2) exclu en gravel, gardé en VTT
    assert_eq!(case(&|t| t.osm_mtb[0] = 5), (false, false, true));
    assert_eq!(case(&|t| t.osm_mtb[0] = 4), (true, false, true));
    assert_eq!(case(&|t| t.osm_mtb[0] = 3), (true, false, true));
    assert_eq!(case(&|t| t.osm_mtb[0] = 2), (true, true, true));
    // sac_scale ≥ 4 sans cotation mtb : exclu en VTT ; avec cotation mtb, la cotation fait foi
    assert_eq!(case(&|t| t.osm_sac[0] = 4), (false, false, true));
    assert_eq!(
        case(&|t| {
            t.osm_sac[0] = 4;
            t.osm_mtb[0] = 2
        }),
        (true, false, true)
    );
    assert_eq!(case(&|t| t.osm_sac[0] = 3), (true, false, true));
    // smoothness : impassable (8) exclu en VTT, très mauvais (≥ 6) en gravel
    assert_eq!(case(&|t| t.osm_smooth[0] = 8), (false, false, true));
    assert_eq!(case(&|t| t.osm_smooth[0] = 6), (true, false, true));
    // cœur de parc national : chemins exclus (VTT, gravel), jamais une route ; trail inchangé
    assert_eq!(case(&|t| t.pn_core[0] = 1), (false, false, true));
    let t = one("Route à 1 chaussée", |t| t.pn_core[0] = 1);
    assert!(net_of(&t, &bike("mtb")).0 && net_of(&t, &bike("road")).0);
    // Route : seul le revêtu compte (D53) ; un chemin technique non revêtu n'est pas du tout dedans
    let t = one("Chemin", |t| t.osm_mtb[0] = 5);
    assert!(!net_of(&t, &bike("road")).0);
}

#[test]
fn bike_major_class_and_max_price() {
    // classe de grand axe en vélo : importance 1–2, OSM trunk/primary, Route à 2 chaussées ;
    // l'importance 3 est une route ordinaire (en trail : grand axe, D50)
    let mut w = World::new(rolling);
    for (n, imp) in [
        ("Route à 1 chaussée", 3),
        ("Route à 1 chaussée", 2),
        ("Route à 2 chaussées", 5),
    ] {
        w.line(&[[0.0, 0.0], [100.0, 0.0]], n, imp);
    }
    w.line(&[[0.0, 0.0], [100.0, 0.0]], "Route à 1 chaussée", 5);
    w.t.osm_major[3] = 1;
    let b = class_mask(&w.t, &natures(), true);
    let tr = class_mask(&w.t, &natures(), false);
    let (ro, ma) = (CLS_ROAD as u8, CLS_MAJOR as u8);
    assert_eq!(b, [ro, ma, ma, ma]);
    assert_eq!(tr, [ma, ma, ro, ma]);
    // Max D+ : le grand axe coûte +1 m de confort par mètre en vélo (D72), rien en trail (D64)
    let mut cls = [0.0; N_CLS];
    cls[CLS_MAJOR] = 100.0;
    let lab = [0.0; N_LAB];
    let m = |p: &Profile, g| off_parts(&cls, &lab, p, g).major;
    assert_eq!(m(&bike("mtb"), Goal::Max), 100.0);
    assert_eq!(m(&bike("road"), Goal::Max), 100.0);
    assert_eq!(m(&TRAIL_ANY, Goal::Max), 0.0);
    // le coût par type de voie vélo vient des dalles (LAB_PREF), pas des classes trail
    let mut lab = [0.0; N_LAB];
    lab[LAB_PREF] = 30.0;
    assert_eq!(
        off_parts(&[0.0; N_CLS], &lab, &bike("gravel"), Goal::Target).pref,
        30.0
    );
    assert_eq!(
        off_parts(&[100.0; N_CLS], &lab, &TRAIL_ANY, Goal::Target).pref,
        0.0
    );
}

#[test]
fn bike_cost_table() {
    // valeurs de la table (conception § 2.3 + D78), en dixièmes de mètre de confort par mètre
    let cost = |surface: &str, nature: &str, imp: u8, f: &dyn Fn(&mut Troncons)| {
        let mut w = World::new(rolling);
        w.line(&[[0.0, 0.0], [100.0, 0.0]], nature, imp);
        f(&mut w.t);
        let p = bike(surface);
        let cls = class_mask(&w.t, &natures(), true);
        pref_mask(&w.t, &natures(), &p, &cls)[0]
    };
    let none = |_: &mut Troncons| {};
    // VTT : sentier 0, chemin 0,2, petites routes 0,5 (D78), grand axe 1,5 (D78)
    assert_eq!(cost("mtb", "Sentier", 6, &none), 0);
    assert_eq!(cost("mtb", "Chemin", 6, &none), 2);
    assert_eq!(cost("mtb", "Route à 1 chaussée", 5, &none), 5);
    assert_eq!(
        cost("mtb", "Route à 1 chaussée", 5, &|t| t.osm_hw[0] =
            HW_MINOR),
        5
    );
    assert_eq!(cost("mtb", "Route à 1 chaussée", 2, &none), 15);
    assert_eq!(
        cost("mtb", "Route à 1 chaussée", 4, &|t| t.osm_hw[0] =
            HW_MAJOR),
        15
    );
    // Gravel : piste 0 ; sentier selon la cotation VTT (D78) : scale 0 → 0,3, 1 → 0,5, sinon 0,8 ;
    // sentier en voie verte 0 ; petite route 1 (0 si non revêtue)
    assert_eq!(cost("gravel", "Chemin", 6, &|t| t.osm_hw[0] = HW_TRACK), 0);
    let path = |mtb: u8| {
        move |t: &mut Troncons| {
            t.osm_hw[0] = HW_PATH;
            t.osm_mtb[0] = mtb
        }
    };
    assert_eq!(cost("gravel", "Sentier", 6, &path(1)), 3);
    assert_eq!(cost("gravel", "Sentier", 6, &path(2)), 5);
    assert_eq!(cost("gravel", "Sentier", 6, &path(0)), 8);
    assert_eq!(
        cost("gravel", "Sentier", 6, &|t| {
            t.osm_hw[0] = HW_PATH;
            t.osm_cycleway[0] = 4
        }),
        0
    );
    assert_eq!(cost("gravel", "Route à 1 chaussée", 5, &none), 10);
    // tracktype et état dégradent le gravel (max avec la base)
    assert_eq!(
        cost("gravel", "Chemin", 6, &|t| {
            t.osm_hw[0] = HW_TRACK;
            t.osm_tracktype[0] = 4
        }),
        6
    );
    assert_eq!(
        cost("gravel", "Chemin", 6, &|t| {
            t.osm_hw[0] = HW_TRACK;
            t.osm_smooth[0] = 5
        }),
        15
    );
    // Route : route mineure 0 ; secondaire 0,3 ; grand axe 0,6 (1 si importance 1) ; pavés +0,8
    assert_eq!(cost("road", "Route à 1 chaussée", 5, &none), 0);
    assert_eq!(cost("road", "Route à 1 chaussée", 3, &none), 3);
    assert_eq!(cost("road", "Route à 1 chaussée", 2, &none), 6);
    assert_eq!(cost("road", "Route à 1 chaussée", 1, &none), 10);
    assert_eq!(
        cost("road", "Route à 1 chaussée", 5, &|t| t.surf[0] =
            engine::tiles::SURF_SETT),
        8
    );
    // use_sidepath +1 ; piste cyclable latérale −0,3 (jamais sous 0)
    assert_eq!(
        cost("road", "Route à 1 chaussée", 3, &|t| t.osm_bike[0] = 3),
        13
    );
    assert_eq!(
        cost("road", "Route à 1 chaussée", 3, &|t| t.osm_cycleway[0] =
            3),
        0
    );
}

/// D54 : en Route vélo, la part de grands axes ne bloque plus : avertissement `major_roads_share`.
#[test]
fn bike_road_major_is_warning() {
    for (n_major, warn) in [(1, false), (2, true)] {
        let mut w = World::new(rolling);
        let c = [
            [-200.0, -200.0],
            [200.0, -200.0],
            [200.0, 200.0],
            [-200.0, 200.0],
        ];
        for k in 0..4 {
            let imp = if k < n_major { 2 } else { 5 };
            w.line(&[c[k], c[(k + 1) % 4]], "Route à 1 chaussée", imp);
        }
        let out = w
            .run(json!({"sport": "bike", "surface": "road", "distance_km": 1.6}))
            .unwrap_or_else(|m| panic!("{n_major} : {:?}", m.code));
        let w = out["warnings"].as_array().unwrap();
        let share = w.iter().find(|x| x["code"] == json!("major_roads_share"));
        assert_eq!(share.is_some(), warn, "{n_major}");
        if let Some(s) = share {
            assert_eq!(s["params"]["pct"], json!(50.0));
        }
    }
}

/// Un isthme raide (25 % sur 200 m) entre deux carrés : avec une montée max de 22 %, il n'est ni
/// doublé ni pris par l'accès ; sans contrôle (trail), il l'est (D73 : `Grade::two_way`).
#[test]
fn bike_twins_and_access_respect_up() {
    fn ramp(x: f64, _y: f64) -> f64 {
        100.0 + 0.25 * (x - 500.0).clamp(0.0, 200.0)
    }
    let mut w = World::new(ramp);
    w.square(-80.0, -80.0, 160.0, "Route à 1 chaussée", 5);
    w.line(
        &[[80.0, 80.0], [600.0, 80.0], [1000.0, 80.0]],
        "Route à 1 chaussée",
        5,
    );
    w.square(1000.0, 80.0, 400.0, "Route à 1 chaussée", 5);
    let t = &w.t;
    let frame = engine::l93::Frame::new(LAT, LON);
    let prof = bike("road");
    let (sel, closed) = network(t, &natures(), &prof);
    let region = Region::new(MultiPolygon::new(vec![disk(5000.0, 64)]));
    let grade = Some(Grade {
        up: 0.22,
        down: 0.22,
    });
    let doubled = |two_way: Option<Grade>| {
        let mut net = Net::new(t, frame);
        let mut ids = net.build(&sel, &region);
        let (s, _, pos) = net.insert_start(&mut ids, [0.0, 0.0]).unwrap();
        net.duplicate_unique(&mut ids, pos, s, FREE_RADIUS, two_way)
            .1
    };
    assert!(
        doubled(None) > 0,
        "contrôle : l'isthme est doublé sans contrôle de pente"
    );
    assert_eq!(doubled(grade), 0);
    // accès : le carré lointain n'est joignable depuis le départ que par l'isthme raide
    let key = node_key(
        ((w.x0 + 1000.0) * 10.0).round() as i64,
        ((w.y0 + 80.0) * 10.0).round() as i64,
    );
    let cls = class_mask(t, &natures(), true);
    let entry = |two_way: Option<Grade>| {
        let a = Access::new(t, frame, &sel, &closed, &cls, &[], &region, two_way, &[]).unwrap();
        a.best_entry(&[key])
    };
    assert_eq!(entry(None), Some(key));
    assert_eq!(entry(grade), None);
}

/// Zone et distances (D72, D76) : rayon par défaut min(L/4 ; 50 km), aire de 7 854 km², gravel 200 km.
#[test]
fn bike_radius_and_distances() {
    let far = |sport: &str, surface: &str, km: f64| -> f64 {
        let r: Request = serde_json::from_value(json!({
            "lat": LAT, "lon": LON, "sport": sport, "surface": surface, "distance_km": km,
            // point de passage à 80 km : hors zone, `via_too_far` donne le rayon
            "via": [[LAT + 0.72, LON]],
        }))
        .unwrap();
        let m = check_via(&r).unwrap_err();
        assert_eq!(m.code, Code::ViaTooFar);
        m.params["max_km"].as_f64().unwrap()
    };
    // 200 km : L/4 = 50 km, plafonné par l'aire (marge 1 %) à 49,7 ; 120 km : 30 km
    assert_eq!(far("bike", "road", 200.0), 49.7);
    assert_eq!(far("bike", "gravel", 200.0), 49.7);
    assert_eq!(far("bike", "mtb", 100.0), 25.0);
    assert_eq!(far("trail", "trail", 100.0), 24.9);
    let near = |km: f64| {
        let r: Request = serde_json::from_value(json!({
            "lat": LAT, "lon": LON, "sport": "bike", "surface": "road", "distance_km": km,
            "via": [[LAT + 0.72, LON]],
        }))
        .unwrap();
        check_via(&r).unwrap_err().params["max_km"]
            .as_f64()
            .unwrap()
    };
    assert_eq!(near(120.0), 30.0);
    // la zone de 7 854 km² ne dépasse jamais l'aire max du profil
    let p = bike("road");
    assert!(std::f64::consts::PI * 49.7_f64.powi(2) < p.max_area_km2);
    // D76 : le gravel va jusqu'à 200 km comme la Route, le VTT reste à 100
    assert_eq!(
        (
            bike("gravel").dist_km,
            bike("road").dist_km,
            bike("mtb").dist_km
        ),
        ((10.0, 200.0), (10.0, 200.0), (5.0, 100.0))
    );
    assert_eq!(bike("gravel").md_cap.2, 200.0);
    // pas d'avertissement `long_distance` en vélo, `Le plus court` borné par le profil
    assert_eq!(
        (bike("road").long_km, TRAIL_ANY.long_km),
        (None, Some(25.0))
    );
}

/// Un calcul vélo de bout en bout sur un petit monde : la boucle existe, ferme, et le sens est permis.
/// (zone vélo = disque de rayon L/4 : le carré est centré sur le départ)
#[test]
fn bike_loop_end_to_end() {
    for surface in ["mtb", "gravel", "road"] {
        for mode in ["max", "target", "min_distance"] {
            let mut w = World::new(rolling);
            let nat = if surface == "road" {
                "Route à 1 chaussée"
            } else {
                "Chemin"
            };
            w.square(-200.0, -200.0, 400.0, nat, 5);
            let req = json!({"sport": "bike", "surface": surface, "mode": mode, "distance_km": 1.6,
                "max_distance_km": 1.8, "target_dplus": 50.0, "n_candidates": 2, "node_simple": true});
            let out = w
                .run(req)
                .unwrap_or_else(|m| panic!("{surface} {mode} : {:?} {:?}", m.code, m.detail));
            let c = &out["candidates"][0];
            assert!(c["length_m"].as_f64().unwrap() > 1000.0, "{surface} {mode}");
            let (lat, lon) = (c["lat"].as_array().unwrap(), c["lon"].as_array().unwrap());
            assert_eq!(lat.first(), lat.last());
            assert_eq!(lon.first(), lon.last());
        }
    }
}
