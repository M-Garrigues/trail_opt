//! Lecteur tiles/1 et moteur sur les dalles de test (`common::tiles` : sautés en local sans
//! dalle, en échec en CI).
use std::collections::HashMap;
use std::time::Instant;

use engine::tiles::{Troncons, read_tile};

mod common;
use common::{ALPES, MASSY};

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
        let (ix, iy) = k.split_once('_').unwrap();
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

/// D23 : « longues » = seul choix du sens de parcours. À graine égale, même boucle donc même D+
/// que « équilibré » (Alpes 12 km, où le poids de recherche faisait perdre ~8 % de D+).
#[test]
fn long_climbs_keep_balanced_dplus() {
    let Some((_, store)) = common::tiles(&[ALPES]) else {
        return;
    };
    let run = |climbs: &str| {
        let req: engine::plan::Request = serde_json::from_value(serde_json::json!({
            "lat": 45.0920, "lon": 6.0700, "distance_km": 12.0, "climbs": climbs, "seed": 3
        }))
        .unwrap();
        let out = engine::plan::plan(&store, &req, false).unwrap();
        let c = &out["candidates"][0];
        (
            c["dplus_m"].as_f64().unwrap(),
            c["length_m"].as_f64().unwrap(),
            c["climbs"]["gbar_m"].as_f64().unwrap(),
        )
    };
    let (b, l) = (run("balanced"), run("long"));
    assert_eq!((b.0, b.1), (l.0, l.1), "D+ et longueur");
    assert!(l.2 >= b.2, "Ḡ longues {} < équilibré {}", l.2, b.2);
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
