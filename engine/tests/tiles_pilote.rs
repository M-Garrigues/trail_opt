//! Lecteur tiles/1 sur les dalles pilotes (ignorées par git) : passe si elles sont absentes.
use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use engine::tiles::{TileStore, Troncons, read_tile};

const DIR: &str = "../scripts/experiments/tiles_pilote";

#[test]
fn pilot_tiles_read_and_invariants() {
    let root = Path::new(DIR);
    if !root.join("manifest.json").exists() {
        eprintln!("dalles pilotes absentes : test sauté");
        return;
    }
    let store = TileStore::open(root).unwrap();
    let mut keys: Vec<_> = store.manifest.tiles.keys().cloned().collect();
    keys.sort();
    let t0 = Instant::now();
    let mut t = Troncons::new();
    for k in &keys {
        let n0 = t.len();
        let (ix, iy) = k.split_once('_').unwrap();
        read_tile(&root.join(format!("{k}.npz")), ix.parse().unwrap(), iy.parse().unwrap(), &mut t).unwrap();
        assert_eq!((t.len() - n0) as u64, store.manifest.tiles[k]["n"].as_u64().unwrap(), "{k}");
    }
    eprintln!("{} tronçons, {} dalles lus en {:.2} s", t.len(), keys.len(), t0.elapsed().as_secs_f64());
    // montée - descente = z(v) - z(u), et altitude unique par nœud
    let mut node_z: HashMap<i64, i32> = HashMap::new();
    let mut deg: HashMap<i64, u32> = HashMap::new();
    for i in 0..t.len() {
        let (a, b) = (t.poff[i], t.poff[i + 1] - 1);
        assert_eq!(t.dplus_dm[i] - t.dminus_dm[i], t.z_dm[b] - t.z_dm[a]);
        let (ku, kv) = t.end_keys(i);
        for (k, z) in [(ku, t.z_dm[a]), (kv, t.z_dm[b])] {
            assert_eq!(*node_z.entry(k).or_insert(z), z, "altitude de nœud non unique");
            *deg.entry(k).or_default() += 1;
        }
    }
    let leaves = deg.values().filter(|&&d| d == 1).count();
    eprintln!("{} nœuds, {} de degré 1 ({:.1} %)", deg.len(), leaves, 100.0 * leaves as f64 / deg.len() as f64);
    // les parallèles désignent des tronçons existants (hors bords de la zone pilote)
    let ids: std::collections::HashSet<i64> = t.id.iter().copied().collect();
    let known = t.par_id.iter().filter(|p| ids.contains(p)).count();
    assert!(known as f64 >= 0.95 * t.par_id.len() as f64);
}
