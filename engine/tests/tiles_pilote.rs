//! Lecteur tiles/1 et moteur sur les dalles de test (`common::tiles` : sautés en local sans
//! dalle, en échec en CI).
use std::collections::HashMap;
use std::time::Instant;

use engine::tiles::{Troncons, read_tile};

mod common;
use common::{ALPES, BOURG, MASSY};

#[test]
fn pilot_tiles_read_and_invariants() {
    let Some((root, store)) = common::tiles(&[]) else {
        return;
    };
    let mut keys: Vec<_> = store.manifest.tiles.keys().cloned().collect();
    keys.sort();
    let t0 = Instant::now();
    let mut t = Troncons::new();
    for k in &keys {
        let n0 = t.len();
        // DOM : `<zone>/<ix>_<iy>`, même grille dans le repère de la zone
        let (ix, iy) = k.rsplit('/').next().unwrap().split_once('_').unwrap();
        read_tile(
            &root.join(format!("{k}.npz")),
            ix.parse().unwrap(),
            iy.parse().unwrap(),
            &mut t,
        )
        .unwrap();
        assert_eq!(
            (t.len() - n0) as u64,
            store.manifest.tiles[k]["n"].as_u64().unwrap(),
            "{k}"
        );
    }
    eprintln!(
        "{} tronçons, {} dalles lus en {:.2} s",
        t.len(),
        keys.len(),
        t0.elapsed().as_secs_f64()
    );
    // montée - descente = z(v) - z(u), et altitude unique par nœud
    let mut node_z: HashMap<i64, i32> = HashMap::new();
    let mut deg: HashMap<i64, u32> = HashMap::new();
    for i in 0..t.len() {
        let (a, b) = (t.poff[i], t.poff[i + 1] - 1);
        assert_eq!(t.dplus_dm[i] - t.dminus_dm[i], t.z_dm[b] - t.z_dm[a]);
        let (ku, kv) = t.end_keys(i);
        for (k, z) in [(ku, t.z_dm[a]), (kv, t.z_dm[b])] {
            assert_eq!(
                *node_z.entry(k).or_insert(z),
                z,
                "altitude de nœud non unique"
            );
            *deg.entry(k).or_default() += 1;
        }
    }
    let leaves = deg.values().filter(|&&d| d == 1).count();
    eprintln!(
        "{} nœuds, {} de degré 1 ({:.1} %)",
        deg.len(),
        leaves,
        100.0 * leaves as f64 / deg.len() as f64
    );
    // les parallèles désignent des tronçons existants (hors bords de la zone pilote)
    let ids: std::collections::HashSet<i64> = t.id.iter().copied().collect();
    let known = t.par_id.iter().filter(|p| ids.contains(p)).count();
    assert!(known as f64 >= 0.95 * t.par_id.len() as f64);
}

fn plan(store: &engine::tiles::TileStore, v: serde_json::Value) -> serde_json::Value {
    let req: engine::plan::Request = serde_json::from_value(v).unwrap();
    engine::plan::plan(store, &req, false).unwrap()
}

/// D33 (T24) : « longues » paie un coût par montée. En montagne (Bourg 15 km), la plus longue
/// montée dépasse celle de « courtes », pour au plus 15 % de D+ perdu face à « équilibré ».
#[test]
fn long_climbs_longer_than_short() {
    let Some((_, store)) = common::tiles(&[BOURG]) else {
        return;
    };
    let run = |climbs: &str| {
        let out = plan(
            &store,
            serde_json::json!({"lat": BOURG.0, "lon": BOURG.1, "distance_km": 15.0,
                "climbs": climbs, "seed": 0}),
        );
        let c = &out["candidates"][0];
        (
            c["dplus_m"].as_f64().unwrap(),
            c["climbs"]["longest_gain_m"].as_f64().unwrap(),
        )
    };
    let (s, b, l) = (run("short"), run("balanced"), run("long"));
    eprintln!("courtes {s:?}, équilibré {b:?}, longues {l:?} (D+, montée max)");
    assert!(l.1 > s.1, "montée max longues {} <= courtes {}", l.1, s.1);
    assert!(
        l.0 >= 0.85 * b.0,
        "D+ longues {} < 0,85 × équilibré {}",
        l.0,
        b.0
    );
}

/// D40 : en plaine, la boucle revient à la distance demandée (avant : toujours +4,9 %, le haut
/// de la tolérance, avec des détours sans D+).
#[test]
fn flat_loop_not_longer_than_requested() {
    let Some((_, store)) = common::tiles(&[MASSY]) else {
        return;
    };
    let out = plan(
        &store,
        serde_json::json!({"lat": MASSY.0, "lon": MASSY.1, "distance_km": 10.0, "seed": 0}),
    );
    let l = out["candidates"][0]["length_m"].as_f64().unwrap();
    assert!(l <= 10_200.0, "boucle de {l} m pour 10 km demandés");
}

/// D33 (I2) : min_distance tient le D+ près de X (seconde passe en mode cible si D+ > 1,1·X).
#[test]
fn min_distance_dplus_near_target() {
    let Some((_, store)) = common::tiles(&[BOURG]) else {
        return;
    };
    let out = plan(
        &store,
        serde_json::json!({"lat": BOURG.0, "lon": BOURG.1, "mode": "min_distance",
            "target_dplus": 500.0, "seed": 0}),
    );
    let c = &out["candidates"][0];
    let (d, l) = (
        c["dplus_m"].as_f64().unwrap(),
        c["length_m"].as_f64().unwrap(),
    );
    eprintln!("Bourg X = 500 : {l:.0} m, D+ {d:.0} m");
    assert!((500.0..=550.0).contains(&d), "D+ {d} hors [X, 1,1·X]");
}

/// « Autres boucles » (ui-spec E7, AC9) : même graine et mêmes paramètres, la boucle de
/// `n_candidates = 1` figure parmi celles de `n_candidates = 4` (pas forcément en tête : les
/// candidats sont triés par D+, ou par longueur en min_distance), hors plafond de 15 s.
#[test]
fn same_seed_loop_among_more_candidates() {
    let Some((_, store)) = common::tiles(&[MASSY, ALPES]) else {
        return;
    };
    let cases = [
        serde_json::json!({"lat": 48.7309, "lon": 2.2713, "distance_km": 10.0, "seed": 7}),
        serde_json::json!({"lat": 45.0920, "lon": 6.0700, "distance_km": 12.0, "seed": 3}),
        serde_json::json!({"lat": 48.7309, "lon": 2.2713, "mode": "min_distance", "target_dplus": 300.0, "seed": 1}),
    ];
    for case in cases {
        let run = |n: usize| {
            let mut v = case.clone();
            v["n_candidates"] = n.into();
            v["max_compute_s"] = 15.0.into();
            let req: engine::plan::Request = serde_json::from_value(v).unwrap();
            let out = engine::plan::plan(&store, &req, false).unwrap();
            let geo = |c: &serde_json::Value| (c["lat"].clone(), c["lon"].clone());
            out["candidates"]
                .as_array()
                .unwrap()
                .iter()
                .map(geo)
                .collect::<Vec<_>>()
        };
        let (one, again, four) = (run(1), run(1), run(4));
        assert_eq!(one, again, "{case}: même graine, même boucle");
        let pos = four.iter().position(|c| *c == one[0]);
        eprintln!("{case}: n°1 de n=1 au rang {pos:?} sur {}", four.len());
        assert!(pos.is_some(), "{case}: boucle n=1 absente de n=4");
    }
}

/// D34 : un point de passage est traversé ; `via`, `legs` cohérents avec la boucle (T34).
#[test]
fn via_point_is_visited_and_legs_add_up() {
    let Some((_, store)) = common::tiles(&[MASSY]) else {
        return;
    };
    let o = plan(
        &store,
        serde_json::json!({"lat": MASSY.0, "lon": MASSY.1, "mode": "max", "distance_km": 10.0,
                           "via": [[48.7405, 2.2900]], "node_simple": true, "enforce_limits": true}),
    );
    let c = &o["candidates"][0];
    assert_eq!(c["via"].as_array().unwrap().len(), 1, "{:?}", o["warnings"]);
    assert!(c["via"][0]["snap_m"].as_f64().unwrap() <= 150.0);
    let legs = c["legs"].as_array().unwrap();
    assert_eq!(legs.len(), 2);
    let sum = |k: &str| legs.iter().map(|l| l[k].as_f64().unwrap()).sum::<f64>();
    // échelle de `dist` (profil planaire), à < 1 % de `length_m`
    let last = c["dist"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()
        .as_f64()
        .unwrap();
    assert!((sum("length_m") - last).abs() < 1.0);
    assert!((sum("length_m") / c["length_m"].as_f64().unwrap() - 1.0).abs() < 0.01);
    assert!((sum("dplus_m") - c["dplus_m"].as_f64().unwrap()).abs() < 1.0);
    // trop loin : refusé avant tout calcul
    let r: engine::plan::Request = serde_json::from_value(
        serde_json::json!({"lat": MASSY.0, "lon": MASSY.1, "distance_km": 10.0, "via": [[48.9, 2.5]]}),
    )
    .unwrap();
    assert_eq!(
        engine::plan::check_via(&r).unwrap_err().code,
        engine::codes::Code::ViaTooFar
    );
}

/// D34 : repères traversés = col à <= 50 m, sommet à <= 100 m, dans l'ordre du tracé.
#[test]
fn landmarks_within_distance() {
    let (x0, y0) = (900_000.0_f64, 6_500_000.0_f64);
    let ll = |x: f64, y: f64| engine::l93::inverse(x, y);
    let poi = |nature: &str, name: &str, x: f64, y: f64| {
        serde_json::from_value::<engine::tiles::Poi>(serde_json::json!({
            "nature": nature, "name": name, "x_dm": (x * 10.0) as i64, "y_dm": (y * 10.0) as i64,
            "z_dm": 12345}))
        .unwrap()
    };
    let pois = [
        poi("Sommet", "Loin", x0 + 500.0, y0 + 90.0), // 90 m : sommet gardé
        poi("Col", "Pres", x0 + 100.0, y0 + 30.0),    // 30 m : gardé
        poi("Col", "Trop loin", x0 + 200.0, y0 + 70.0), // 70 m : col refusé
    ];
    let pts: Vec<(f64, f64)> = (0..=10).map(|i| ll(x0 + 100.0 * i as f64, y0)).collect();
    let c = serde_json::json!({
        "lon": pts.iter().map(|p| p.0).collect::<Vec<_>>(),
        "lat": pts.iter().map(|p| p.1).collect::<Vec<_>>(),
        "dist": (0..=10).map(|i| 100.0 * i as f64).collect::<Vec<_>>()});
    let l = engine::plan::landmarks(&pois, &Default::default(), &c);
    let names: Vec<_> = l
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Pres", "Loin"]);
    assert_eq!(l[0]["kind"], "col");
    assert_eq!(l[0]["ele_m"], 1234.5);
    assert!((l[0]["dist_m"].as_f64().unwrap() - 100.0).abs() < 1.5);
}

/// Type de voie (api.md v1.7) : jamais un filtre. Dans les trois modes de préférence et les trois
/// types de sortie, le nombre demandé de boucles est rendu, sans erreur ni message « demande non
/// atteinte » ; la part de chemin de la première boucle va dans le sens de la préférence ;
/// « chemins » ne coûte pas plus de 15 % de D+ (mode max) ; la cible reste tenue sur la distance
/// et le D+ RÉELS (5 %) ; `low_surface_share` si et seulement si la part de chemin est sous 50 % en
/// « Chemins », jamais en min_distance (la préférence n'y agit pas, revue f54746a). « Route » (D53)
/// est un filtre : voies revêtues seulement (aucun chemin naturel), le nombre de boucles et la cible
/// peuvent alors manquer (réseau réduit) ou la demande être refusée (D54 : plus de 25 % de grands
/// axes), jamais d'avertissement de part.
#[test]
fn surface_preference_never_rejects() {
    // D67 : D+ « montre » (hystérésis 3 m) : 500 m bruts ≈ 460 m ; à 12 km, Bourg a un palier vers
    // 466–480 m (500 n'y est plus atteignable)
    for (site, km, dplus) in [(MASSY, 10.0, 250.0), (BOURG, 12.0, 460.0)] {
        let Some((_, store)) = common::tiles(&[site]) else {
            return;
        };
        for mode in ["max", "target", "min_distance"] {
            let run = |surface: &str| {
                let req: engine::plan::Request = serde_json::from_value(
                    serde_json::json!({"lat": site.0, "lon": site.1, "mode": mode, "distance_km": km,
                        "target_dplus": if mode == "max" { None } else { Some(dplus) },
                        "surface": surface, "n_candidates": 3, "node_simple": true,
                        "max_grade": 0.6, "enforce_limits": true}),
                )
                .unwrap();
                let out = match engine::plan::plan(&store, &req, false) {
                    Ok(o) => o,
                    // D53/D54 : « Route » peut refuser (réseau revêtu trop court ou surtout grands axes)
                    Err(m) if surface == "road" => {
                        assert!(
                            matches!(
                                m.code,
                                engine::Code::PavedNetworkMajorRoads
                                    | engine::Code::PavedNetworkTooShort
                            ),
                            "{site:?} {mode} : {m:?}"
                        );
                        return (0.0, 0.0);
                    }
                    Err(m) => panic!("{site:?} {mode} {surface} : {m:?}"),
                };
                let w: Vec<&str> = out["warnings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|x| x["code"].as_str().unwrap())
                    .collect();
                let c = out["candidates"].as_array().unwrap();
                if surface == "road" {
                    assert!(
                        c.iter().all(|x| x["surface_share"][0] == 0.0),
                        "{site:?} {mode}"
                    );
                    return (
                        c[0]["dplus_m"].as_f64().unwrap(),
                        c[0]["trail_frac"].as_f64().unwrap(),
                    );
                }
                assert_eq!(c.len(), 3, "{site:?} {mode} {surface}");
                assert!(
                    !w.contains(&"target_not_reached") && !w.contains(&"fewer_loops"),
                    "{site:?} {mode} {surface} : {w:?}"
                );
                let (l, d, f) = (
                    c[0]["length_m"].as_f64().unwrap(),
                    c[0]["dplus_m"].as_f64().unwrap(),
                    c[0]["trail_frac"].as_f64().unwrap(),
                );
                eprintln!("{site:?} {mode} {surface} : {l:.0} m, +{d:.0} m, chemin {f}");
                if mode == "target" {
                    assert!(
                        ((l / 1000.0 - km) / km).abs() <= 0.05
                            && ((d - dplus) / dplus).abs() <= 0.05
                    );
                }
                let share = match surface {
                    "trail" if mode != "min_distance" => f,
                    _ => 1.0,
                };
                assert_eq!(w.contains(&"low_surface_share"), share < 0.5, "{w:?}");
                (d, f)
            };
            let (trail, any, road) = (run("trail"), run("any"), run("road"));
            if mode != "min_distance" {
                assert!(trail.1 >= any.1 && any.1 >= road.1, "{site:?} {mode}");
                let gap = if mode == "max" { 0.1 } else { 0.0 };
                assert!(trail.1 > road.1 + gap, "{site:?} {mode}");
            }
            if mode == "max" {
                assert!(trail.0 >= 0.85 * any.0, "{site:?} : {trail:?} {any:?}");
            }
        }
    }
}

/// D44 : « n demandé = n rendu » (requête comme celle de l'API, n = 4, 2 graines) et toutes les
/// boucles sont correctes (D+ >= 50 % de la meilleure, dernier palier de `alternates`). Le nombre
/// ne dépend pas du temps de calcul (voir `invariants::candidats_malgre_echeance_depassee`).
fn four_loops(store: &engine::tiles::TileStore, site: (f64, f64)) {
    for seed in 0..2 {
        let out = plan(
            store,
            serde_json::json!({"lat": site.0, "lon": site.1, "distance_km": 12.0,
                "n_candidates": 4, "seed": seed, "node_simple": true, "max_grade": 0.6,
                "enforce_limits": true}),
        );
        let d: Vec<f64> = out["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["dplus_m"].as_f64().unwrap())
            .collect();
        eprintln!("{site:?} graine {seed} : D+ {d:?}");
        assert_eq!(d.len(), 4, "{site:?} graine {seed} : {d:?}");
        assert!(d.iter().all(|&x| x >= 0.5 * d[0]), "{site:?} : {d:?}");
        let w = out["warnings"].as_array().unwrap();
        assert!(w.iter().all(|x| x["code"] != "fewer_loops"));
    }
}

#[test]
fn n_loops_returned_all_decent() {
    for site in [MASSY, BOURG] {
        let Some((_, store)) = common::tiles(&[site]) else {
            return;
        };
        four_loops(&store, site);
    }
}

/// Idem à Lyon centre (retour fondateur : 1 boucle sur 4) et en Chartreuse (un seul versant),
/// hors de la dalle de test : dalles locales seulement (`ENGINE_TILES_DIR` ou tiles_v1), sinon sauté.
#[test]
fn n_loops_returned_lyon_chartreuse() {
    for site in [(45.7640, 4.8357), (45.33, 5.78)] {
        let dirs = [
            std::env::var("ENGINE_TILES_DIR").ok(),
            Some("../scripts/experiments/tiles_v1".into()),
        ];
        let store = dirs
            .into_iter()
            .flatten()
            .filter_map(|d| engine::tiles::TileStore::open(std::path::Path::new(&d)).ok())
            .find(|s| engine::api::covered(s, site.0, site.1));
        match store {
            Some(s) => four_loops(&s, site),
            None => eprintln!("aucune dalle locale ne couvre {site:?} : test sauté"),
        }
    }
}

/// Retour fondateur (« pentes courtes casse les boucles Cible ») : en mode cible, quelle que soit
/// la préférence de montées, toutes les boucles rendues tiennent distance ET D+ RÉEL à 10 % près,
/// la plus proche de la cible d'abord, sans message `target_not_reached`.
#[test]
fn target_holds_real_dplus_whatever_climbs() {
    // D67 : D+ « montre » (hystérésis 3 m) : 500 m bruts ≈ 460 m ; à 12 km, Bourg a un palier vers
    // 466–480 m (500 n'y est plus atteignable)
    for (site, km, dplus) in [(MASSY, 10.0, 250.0), (BOURG, 12.0, 460.0)] {
        let Some((_, store)) = common::tiles(&[site]) else {
            return;
        };
        for climbs in ["balanced", "short", "long"] {
            let out = plan(
                &store,
                serde_json::json!({"lat": site.0, "lon": site.1, "mode": "target",
                    "distance_km": km, "target_dplus": dplus, "climbs": climbs, "n_candidates": 3,
                    "surface": "any", "node_simple": true, "max_grade": 0.6, "enforce_limits": true}),
            );
            let c: Vec<(f64, f64)> = out["candidates"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| {
                    (
                        c["length_m"].as_f64().unwrap() / 1000.0,
                        c["dplus_m"].as_f64().unwrap(),
                    )
                })
                .collect();
            eprintln!("{site:?} {climbs} : {c:?}");
            let err = |x: &(f64, f64)| ((x.0 - km) / km).abs() + ((x.1 - dplus) / dplus).abs();
            for x in &c {
                assert!(((x.0 - km) / km).abs() <= 0.10, "{site:?} {climbs} : {c:?}");
                assert!(
                    ((x.1 - dplus) / dplus).abs() <= 0.10,
                    "{site:?} {climbs} : {c:?}"
                );
            }
            // ordre de `plan` : erreur bornée à la bande (sous 2 % d'erreur, la cible est tenue et le
            // type de voie puis les virages départagent, D52), plus au plus 3 % de prime de type de voie
            let band = |x: &(f64, f64)| err(x).max(engine::problem::SURF_TARGET_BAND);
            assert!(
                c.windows(2)
                    .all(|w| band(&w[0]) <= band(&w[1]) + engine::problem::SURF_TARGET + 1e-9),
                "{site:?} {climbs} : {c:?}"
            );
            if site == MASSY {
                assert_eq!(c.len(), 3, "{climbs} : {c:?}");
            }
            let w = out["warnings"].as_array().unwrap();
            assert!(w.iter().all(|x| x["code"] != "target_not_reached"));
        }
    }
}

/// Retour fondateur : 20 km en « sentiers seuls » rendait `no_loop_of_distance` (filtre dur).
/// Avec la préférence, une boucle à la distance demandée. Dalles locales, sauté sinon.
#[test]
fn trail_preference_where_trails_alone_had_no_loop() {
    let p = (48.68, 2.35);
    let Some((_, store)) = common::tiles(&[p]) else {
        return;
    };
    let out = plan(
        &store,
        serde_json::json!({"lat": p.0, "lon": p.1, "mode": "max", "distance_km": 20.0,
            "surface": "trail", "node_simple": true, "enforce_limits": true, "max_compute_s": 15.0}),
    );
    let l = out["candidates"][0]["length_m"].as_f64().unwrap();
    assert!((19_000.0..=21_000.0).contains(&l), "{l}");
}

/// Revue pré-déploiement A1 : la réduction aux arêtes pentues vidait le réseau pour certains départs
/// urbains (relief d'un seul côté) : erreur, départ déplacé de plusieurs km, ou panique en
/// « Le plus court ». Le réseau complet est gardé dans ce cas. Dalles locales, sauté sinon.
#[test]
fn reduction_never_empties_network() {
    for site in [(45.19, 5.72), (48.71671, 2.26322)] {
        let dirs = [
            std::env::var("ENGINE_TILES_DIR").ok(),
            Some("../scripts/experiments/tiles_v1".into()),
        ];
        let Some(store) = dirs
            .into_iter()
            .flatten()
            .filter_map(|d| engine::tiles::TileStore::open(std::path::Path::new(&d)).ok())
            .find(|s| engine::api::covered(s, site.0, site.1))
        else {
            eprintln!("aucune dalle locale ne couvre {site:?} : test sauté");
            continue;
        };
        for req in [
            serde_json::json!({"mode": "max", "distance_km": 10.0}),
            serde_json::json!({"mode": "max", "distance_km": 20.0}),
            serde_json::json!({"mode": "min_distance", "target_dplus": 300.0}),
        ] {
            let mut r = req.clone();
            r["lat"] = site.0.into();
            r["lon"] = site.1.into();
            r["node_simple"] = true.into();
            r["max_grade"] = 0.6.into();
            r["enforce_limits"] = true.into();
            let out = plan(&store, r);
            assert!(
                out["candidates"].as_array().is_some_and(|c| !c.is_empty()),
                "{site:?} {req} : {}",
                out["error"]
            );
            assert_ne!(out["effective_start"]["kind"], "moved", "{site:?} {req}");
        }
    }
}

/// D62 : paquet « élégance » coupé (`smooth: false`) = chemin de calcul d'avant D52. Valeurs
/// vérifiées identiques au binaire du commit 290e2c7 (dalle de test versionnée, cible) jusqu'à D63 ;
/// D63 change volontairement les poids de confort de la Cible : valeurs régénérées avec `smooth: false`
/// (garde contre toute dérive du calcul sans élégance) ; D67 (D+ « montre ») : valeurs régénérées,
/// Bourg à 460 m (≈ 500 m bruts). Le défaut de la cible est actif.
#[test]
fn smooth_off_is_stable() {
    let Ok(store) = engine::tiles::TileStore::open(std::path::Path::new("tests/data/tiles")) else {
        return;
    };
    let cases = [
        (
            MASSY,
            10.0,
            250.0,
            [(9966.8, 247.2), (9937.1, 246.7), (10013.5, 245.6)],
        ),
        (
            BOURG,
            12.0,
            460.0,
            [(11814.4, 459.3), (11946.7, 453.7), (12000.0, 460.0)],
        ),
    ];
    for (site, km, dplus, want) in cases {
        let req = |smooth: Option<bool>| {
            serde_json::json!({"lat": site.0, "lon": site.1, "mode": "target", "distance_km": km,
                "target_dplus": dplus, "n_candidates": 3, "node_simple": true, "max_grade": 0.6,
                "enforce_limits": true, "smooth": smooth})
        };
        let got = |out: serde_json::Value| -> Vec<(f64, f64)> {
            out["candidates"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| {
                    (
                        c["length_m"].as_f64().unwrap(),
                        c["dplus_m"].as_f64().unwrap(),
                    )
                })
                .collect()
        };
        assert_eq!(got(plan(&store, req(Some(false)))), want, "{site:?}");
        assert_ne!(
            got(plan(&store, req(None))),
            want,
            "{site:?} : défaut cible = actif"
        );
    }
}
