//! Dalles des tests bout en bout : `ENGINE_TILES_DIR`, sinon la dalle de test versionnée
//! `tests/data/tiles/` (T29), sinon les dalles pilotes locales (non versionnées). Sans dalle
//! couvrant les points du test : saut en local, ÉCHEC en CI (`CI` défini), sauf
//! `ENGINE_TILES_OPTIONAL=1` (transition jusqu'à la livraison de la dalle de test).
#![allow(dead_code)]
use std::path::PathBuf;

use engine::tiles::TileStore;

pub fn tiles(points: &[(f64, f64)]) -> Option<(PathBuf, TileStore)> {
    let dirs = [
        std::env::var("ENGINE_TILES_DIR").ok(),
        Some("tests/data/tiles".into()),
        Some("../scripts/experiments/tiles_pilote".into()),
    ];
    for d in dirs.into_iter().flatten() {
        let p = PathBuf::from(d);
        if let Ok(s) = TileStore::open(&p)
            && points
                .iter()
                .all(|&(lat, lon)| engine::api::covered(&s, lat, lon))
        {
            return Some((p, s));
        }
    }
    let ci =
        std::env::var_os("CI").is_some() && std::env::var_os("ENGINE_TILES_OPTIONAL").is_none();
    assert!(
        !ci,
        "CI : aucune dalle ne couvre {points:?} (attendu : engine/tests/data/tiles/, T29)"
    );
    eprintln!("aucune dalle ne couvre {points:?} : test sauté");
    None
}

pub const MASSY: (f64, f64) = (48.7309, 2.2713);
pub const ALPES: (f64, f64) = (45.0920, 6.0700);
pub const BOURG: (f64, f64) = (45.0555, 6.0310);

/// Comme `tiles`, mais sans échec en CI : None si aucune dalle ne couvre les points.
pub fn tiles_opt(points: &[(f64, f64)]) -> Option<(PathBuf, TileStore)> {
    let dirs = [
        std::env::var("ENGINE_TILES_DIR").ok(),
        Some("tests/data/tiles".into()),
    ];
    dirs.into_iter().flatten().find_map(|d| {
        let p = PathBuf::from(d);
        let s = TileStore::open(&p).ok()?;
        points
            .iter()
            .all(|&(lat, lon)| engine::api::covered(&s, lat, lon))
            .then_some((p, s))
    })
}
