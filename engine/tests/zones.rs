//! Zones de dalles (tiles.md § Zones) : choix de la zone, clés `<zone>/<ix>_<iy>`, cache en
//! sous-dossiers, repères par zone, refus des requêtes à cheval ; essai réel sur une dalle de la
//! Réunion si elle est là (`ENGINE_TILES_DOM_DIR`, sinon `scripts/experiments/tiles_dom`, non versionné).
use std::path::{Path, PathBuf};

use engine::l93::Proj;
use engine::tiles::{Fetch, TileStore, Zone};
use serde_json::{Value, json};

const SRC: &str = "tests/data/tiles";
const CILAOS: (f64, f64) = (-21.134, 55.471);
const RE_BBOX: [f64; 4] = [55.15, -21.45, 55.90, -20.80];

fn zones() -> Value {
    json!({
        "re": {"crs": "EPSG:2975", "bbox": RE_BBOX},
        "gp": {"crs": "EPSG:5490", "bbox": [-61.90, 15.75, -60.95, 16.60]},
        "mq": {"crs": "EPSG:5490", "bbox": [-61.30, 14.30, -60.75, 14.95]},
        "gf": {"crs": "EPSG:2972", "bbox": [-54.70, 2.00, -51.50, 5.90]},
        "yt": {"crs": "EPSG:4471", "bbox": [44.95, -13.10, 45.35, -12.55]},
    })
}

/// « Bucket » d'essai : la dalle de test 46_322 en métropole et, recopiée, sous deux clés DOM
/// (Cilaos `re/17_383`, Fort-de-France `mq/35_80`) ; un repère par zone au même point projeté.
fn bucket(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("optrail-zones-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let tile = std::fs::read(Path::new(SRC).join("46_322.npz")).unwrap();
    let entry = json!({"n": 2977, "bytes": tile.len()});
    for k in ["46_322", "re/17_383", "mq/35_80"] {
        let p = d.join(format!("{k}.npz"));
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, &tile).unwrap();
    }
    let poi = |name: &str, zone: Option<&str>| {
        let mut p = json!({"id": 1, "nature": "Col", "name": name, "importance": 3,
                           "x_dm": 3_500_000, "y_dm": 76_650_000, "z_dm": 12_000});
        if let Some(z) = zone {
            p["zone"] = json!(z);
        }
        p
    };
    let pois = json!({"format": "pois/1", "pois": [poi("Col L93", None), poi("Col RE", Some("re")),
                                                    poi("Col MQ", Some("mq"))]});
    std::fs::write(d.join("pois.json"), pois.to_string()).unwrap();
    let manifest = json!({
        "format": "tiles/1", "data_version": "essai-zones", "natures": [], "zones": zones(),
        "pois": {"file": "pois.json"},
        "tiles": {"46_322": entry, "re/17_383": entry, "mq/35_80": entry},
    });
    std::fs::write(d.join("manifest.json"), manifest.to_string()).unwrap();
    d
}

fn remote(src: &Path, cache: &Path) -> TileStore {
    let src = src.to_path_buf();
    let fetch: Fetch = Box::new(move |n| std::fs::read(src.join(n)).map_err(|e| e.to_string()));
    TileStore::open_remote(fetch, cache).unwrap()
}

#[test]
fn zone_of_a_point_and_coverage() {
    let s = TileStore::open(&bucket("choix")).unwrap();
    let name = |lat: f64, lon: f64| s.zone(lat, lon).name;
    assert_eq!(name(CILAOS.0, CILAOS.1), "re");
    // Guadeloupe et Martinique : même EPSG, zones distinctes
    assert_eq!(name(16.0446, -61.6637), "gp");
    assert_eq!(name(14.6040, -61.0730), "mq");
    assert_eq!(s.zone(16.0, -61.6).proj, s.zone(14.6, -61.0).proj);
    assert_eq!(name(5.4980, -54.0320), "gf");
    assert_eq!(name(-12.7810, 45.2280), "yt");
    // partout ailleurs : métropole (Lambert-93), couverte ou non
    for (lat, lon) in [
        (45.092, 6.07),
        (48.7309, 2.2713),
        (40.7, -74.0),
        (15.3, -61.3),
    ] {
        assert_eq!(s.zone(lat, lon), Zone::default());
    }
    assert_eq!(s.zone(45.0, 6.0).proj, Proj::L93);
    let covered = |p: (f64, f64)| engine::api::covered(&s, p.0, p.1);
    assert!(covered(CILAOS) && covered((14.6040, -61.0730)) && covered((45.0555, 6.031)));
    // Massy (dalle absente de ce manifeste), Saint-Denis (zone re, autre dalle), Guadeloupe
    // (aucune dalle : la dalle `mq/…` de même CRS ne compte pas), New York
    for p in [
        (48.7309, 2.2713),
        (-20.88, 55.45),
        (16.0446, -61.6637),
        (40.7, -74.0),
    ] {
        assert!(!covered(p), "{p:?}");
    }
    // à cheval : seule la mer proche d'un DOM est admise hors de sa zone
    let re = s.zone(CILAOS.0, CILAOS.1);
    assert!(s.near(&re, -21.6, 55.0) && !s.near(&re, 48.7, 2.3) && !s.near(&re, -12.78, 45.2));
    assert!(s.near(&Zone::default(), 40.7, -74.0) && !s.near(&Zone::default(), -21.1, 55.5));
    assert!(!s.near(&s.zone(16.0, -61.6), 14.6, -61.0));
}

#[test]
fn manifest_without_zones_is_metropole_only() {
    let s = TileStore::open(Path::new(SRC)).unwrap();
    assert_eq!(s.zone(CILAOS.0, CILAOS.1), Zone::default());
    assert!(!engine::api::covered(&s, CILAOS.0, CILAOS.1));
    // CRS inconnu : refus à l'ouverture plutôt qu'une projection fausse
    let d = bucket("crs");
    let mut m: Value =
        serde_json::from_str(&std::fs::read_to_string(d.join("manifest.json")).unwrap()).unwrap();
    m["zones"]["re"]["crs"] = json!("EPSG:32740");
    std::fs::write(d.join("manifest.json"), m.to_string()).unwrap();
    assert!(TileStore::open(&d).err().unwrap().contains("CRS inconnu"));
}

#[test]
fn zone_tiles_are_cached_in_subfolders_and_evicted() {
    let (src, cache) = (bucket("src"), bucket("cache").join("c"));
    let mut s = remote(&src, &cache);
    let re = s.zone(CILAOS.0, CILAOS.1);
    let (x, y) = re.proj.forward(CILAOS.1, CILAOS.0);
    let t = s.load_in(&re, [x, y, x, y]).unwrap();
    assert!(cache.join("re/17_383.npz").exists() && !cache.join("46_322.npz").exists());
    // origine de grille de la zone : sommets dans la dalle (17, 383) du repère UTM, à ~2 km près
    assert_eq!(t.len(), 2977);
    assert!((3_380_000..3_620_000).contains(&t.gx[0]), "{}", t.gx[0]);
    assert!((76_580_000..76_820_000).contains(&t.gy[0]), "{}", t.gy[0]);
    // même boîte en métropole : aucune dalle
    assert!(s.load([x, y, x, y]).unwrap().is_empty());
    // budget d'une dalle : charger la Martinique évince la Réunion (sous-dossier compris)
    s.budget_bytes = 300_000;
    let mq = s.zone(14.6040, -61.0730);
    s.ensure_in(&mq, &[(35, 80)]).unwrap();
    assert!(cache.join("mq/35_80.npz").exists() && !cache.join("re/17_383.npz").exists());
    // reprise : `.part` orphelin d'un sous-dossier purgé à l'ouverture
    std::fs::write(cache.join("mq/1_1.npz.part"), b"partiel").unwrap();
    remote(&src, &cache);
    assert!(!cache.join("mq/1_1.npz.part").exists());
}

#[test]
fn landmarks_are_those_of_the_zone() {
    let s = TileStore::open(&bucket("pois")).unwrap();
    assert_eq!(s.pois.len(), 3);
    for (lat, lon, want) in [(CILAOS.0, CILAOS.1, "Col RE"), (14.6, -61.0, "Col MQ")] {
        let z = s.zone(lat, lon);
        // tracé de 1 km qui passe sur le repère, dans le repère de la zone
        let pts: Vec<(f64, f64)> = (0..=10)
            .map(|i| z.proj.inverse(349_500.0 + 100.0 * i as f64, 7_665_000.0))
            .collect();
        let c = json!({"lon": pts.iter().map(|p| p.0).collect::<Vec<_>>(),
                       "lat": pts.iter().map(|p| p.1).collect::<Vec<_>>(),
                       "dist": (0..=10).map(|i| 100.0 * i as f64).collect::<Vec<_>>()});
        let l = engine::plan::landmarks(&s.pois, &z, &c);
        assert_eq!(l.as_array().unwrap().len(), 1, "{l}");
        assert_eq!(l[0]["name"], want);
        assert_eq!(l[0]["dist_m"], 500.0);
        let (lo, la) = (l[0]["lon"].as_f64().unwrap(), l[0]["lat"].as_f64().unwrap());
        assert!((lo - pts[5].0).abs() < 2e-6 && (la - pts[5].1).abs() < 2e-6);
    }
}

fn query(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
}

#[test]
fn a_request_never_spans_two_zones() {
    let s = TileStore::open(&bucket("api")).unwrap();
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, b"k");
    let code = |q: &[(&str, &str)]| {
        let r = engine::api::handle(&query(q), None, true, &s, &key, |_| Ok(true));
        (
            r.status,
            r.body["error"]["code"].as_str().unwrap_or("").to_string(),
        )
    };
    let re = [("lat", "-21.134"), ("lon", "55.471"), ("distance_km", "10")];
    let with = |extra: (&'static str, &'static str)| [re[0], re[1], re[2], extra];
    // sommet du polygone en métropole, point de passage en Martinique
    let poly = "55.3,-21.3;55.6,-21.3;2.3,48.7";
    assert_eq!(code(&with(("polygon", poly))), (400, "zone_invalid".into()));
    assert_eq!(
        code(&with(("via", "14.6,-61.0"))),
        (400, "via_too_far".into())
    );
    // départ en métropole, sommet à la Réunion
    let poly = "6.0,45.0;6.1,45.0;55.47,-21.13";
    let q = [
        ("lat", "45.0555"),
        ("lon", "6.031"),
        ("distance_km", "10"),
        ("polygon", poly),
    ];
    assert_eq!(code(&q), (400, "zone_invalid".into()));
    // départ en Guadeloupe : aucune dalle `gp`
    let q = [
        ("lat", "16.0446"),
        ("lon", "-61.6637"),
        ("distance_km", "10"),
    ];
    assert_eq!(code(&q).1, "outside_coverage");
}

/// Dalles DOM réelles, si elles sont là (sinon test sauté, en CI aussi).
fn dom() -> Option<TileStore> {
    let dir = std::env::var("ENGINE_TILES_DOM_DIR")
        .unwrap_or_else(|_| "../scripts/experiments/tiles_dom".into());
    let s = TileStore::open(Path::new(&dir)).ok();
    let s = s.filter(|s| engine::api::covered(s, CILAOS.0, CILAOS.1));
    if s.is_none() {
        eprintln!("pas de dalle de la Réunion dans {dir} : test sauté");
    }
    s
}

/// Distance au sol (m) entre deux points proches (sphère locale, suffisant à ~0,5 %).
fn ground_m(a: (f64, f64), b: (f64, f64)) -> f64 {
    let dx = (b.1 - a.1).to_radians() * a.0.to_radians().cos();
    6_371_000.0 * dx.hypot((b.0 - a.0).to_radians())
}

#[test]
fn reunion_loops_from_cilaos() {
    let Some(s) = dom() else { return };
    for (mode, km, dplus) in [("max", 10.0, None), ("target", 12.0, Some(600.0))] {
        let req: engine::plan::Request = serde_json::from_value(json!({
            "lat": CILAOS.0, "lon": CILAOS.1, "distance_km": km, "mode": mode,
            "target_dplus": dplus, "seed": 7}))
        .unwrap();
        let out = engine::plan::plan(&s, &req, false).unwrap();
        let c = &out["candidates"][0];
        let col = |k: &str| -> Vec<f64> {
            c[k].as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_f64().unwrap())
                .collect()
        };
        let (lat, lon, ele, dist) = (col("lat"), col("lon"), col("ele"), col("dist"));
        let (len, dp) = (
            c["length_m"].as_f64().unwrap(),
            c["dplus_m"].as_f64().unwrap(),
        );
        eprintln!(
            "Cilaos {mode} {km} km : {len:.0} m, D+ {dp:.0} m, {} points",
            lat.len()
        );
        assert!((len / (km * 1000.0) - 1.0).abs() < 0.06, "{len}");
        assert!(dp > 300.0 && dp < 2000.0, "{dp}");
        if let Some(d) = dplus {
            assert!((dp - d).abs() < 0.15 * d, "{dp}");
        }
        // au bon endroit : départ à moins de 100 m du point demandé (ni fuseau, ni hémisphère
        // décalés), boucle fermée, tout le tracé dans le cirque de Cilaos (altitudes comprises)
        let es = &out["effective_start"];
        let start = (es["lat"].as_f64().unwrap(), es["lon"].as_f64().unwrap());
        assert!(ground_m(CILAOS, start) < 100.0, "{start:?}");
        assert!(ground_m((lat[0], lon[0]), (lat[lat.len() - 1], lon[lon.len() - 1])) < 1.0);
        for i in 0..lat.len() {
            assert!(
                ground_m(CILAOS, (lat[i], lon[i])) < km * 500.0,
                "{} {}",
                lat[i],
                lon[i]
            );
            assert!((1_000.0..2_600.0).contains(&ele[i]), "{}", ele[i]);
        }
        // échelle : la longueur au sol du tracé WGS84 vaut l'abscisse finale à 0,5 % près
        let ground: f64 = (1..lat.len())
            .map(|i| ground_m((lat[i - 1], lon[i - 1]), (lat[i], lon[i])))
            .sum();
        assert!(
            (ground / dist[dist.len() - 1] - 1.0).abs() < 5e-3,
            "{ground}"
        );
    }
    // repère nommé au bon endroit : col de Bébour (−21.1312, 55.5754), sur la route forestière
    let req: engine::plan::Request = serde_json::from_value(
        json!({"lat": -21.1315, "lon": 55.5750, "distance_km": 8.0, "mode": "max", "seed": 7}),
    )
    .unwrap();
    let out = engine::plan::plan(&s, &req, false).unwrap();
    let l = &out["candidates"][0]["landmarks"];
    let col = l
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == "Col de Bébour");
    let col = col.unwrap_or_else(|| panic!("{l}"));
    let at = (col["lat"].as_f64().unwrap(), col["lon"].as_f64().unwrap());
    assert!(ground_m((-21.131218, 55.575353), at) < 2.0, "{at:?}");
    assert_eq!(col["kind"], "col");
}
