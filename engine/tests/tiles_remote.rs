//! Source distante des dalles (D10, T39) : dossier de test servant de « bucket ».
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex};

use engine::tiles::{Fetch, TileStore};

const SRC: &str = "tests/data/tiles";
// boîte L93 (m) autour de Massy
const MASSY_BOX: [f64; 4] = [645_000.0, 6_850_000.0, 650_000.0, 6_855_000.0];

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("optrail-t39-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// `fail(nom, n° d'appel)` : Some(octets) remplace la réponse, Err simule une panne.
fn bucket(
    hits: Arc<Mutex<HashMap<String, usize>>>,
    tamper: impl Fn(&str, usize, Vec<u8>) -> Result<Vec<u8>, String> + Send + Sync + 'static,
) -> Fetch {
    Box::new(move |name| {
        let n = {
            let mut h = hits.lock().unwrap();
            let c = h.entry(name.to_string()).or_default();
            *c += 1;
            *c
        };
        let data = std::fs::read(Path::new(SRC).join(name)).map_err(|e| e.to_string())?;
        tamper(name, n, data)
    })
}

fn total(h: &Mutex<HashMap<String, usize>>) -> usize {
    h.lock().unwrap().values().sum()
}

#[test]
fn remote_loads_like_local_and_caches() {
    let hits = Arc::new(Mutex::new(HashMap::new()));
    let cache = scratch("cache");
    let remote = TileStore::open_remote(bucket(hits.clone(), |_, _, d| Ok(d)), &cache).unwrap();
    let local = TileStore::open(Path::new(SRC)).unwrap();
    assert_eq!(remote.pois.len(), local.pois.len());
    assert!(!cache.join("32_342.npz").exists(), "rien avant ensure/load");
    let a = remote.load(MASSY_BOX).unwrap();
    let b = local.load(MASSY_BOX).unwrap();
    assert!(!a.is_empty() && a.id == b.id && a.z_dm == b.z_dm && a.gx == b.gx);
    let n = total(&hits);
    assert!(n >= 3, "manifest, pois, au moins une dalle : {n}");
    remote.load(MASSY_BOX).unwrap();
    assert_eq!(total(&hits), n, "instance chaude : aucun GET");
    assert!(
        std::fs::read_dir(&cache)
            .unwrap()
            .all(|e| { !e.unwrap().file_name().to_string_lossy().ends_with(".part") })
    );
}

#[test]
fn one_retry_on_failure_or_corruption_then_error() {
    let hits = Arc::new(Mutex::new(HashMap::new()));
    // 1er GET de la dalle : tronquée ; 2e : bon -> succès
    let flaky = bucket(hits.clone(), |n, k, d| {
        if n.ends_with(".npz") && k == 1 {
            Ok(d[..d.len() / 2].to_vec())
        } else {
            Ok(d)
        }
    });
    let s = TileStore::open_remote(flaky, &scratch("retry")).unwrap();
    s.load(MASSY_BOX).unwrap();
    // toujours en panne : erreur claire, pas de fichier laissé
    let cache = scratch("down");
    let down = bucket(Arc::default(), |n, _, d| {
        if n.ends_with(".npz") {
            Err("HTTP 503".into())
        } else {
            Ok(d)
        }
    });
    let s = TileStore::open_remote(down, &cache).unwrap();
    let e = s.load(MASSY_BOX).unwrap_err();
    assert!(e.contains("503"), "{e}");
    assert!(std::fs::read_dir(&cache).unwrap().next().is_none());
    // contenu altéré à taille égale : sha256 du manifeste
    let evil = bucket(Arc::default(), |n, _, mut d| {
        if n.ends_with(".npz") {
            let k = d.len() / 2;
            d[k] ^= 0xff;
        }
        Ok(d)
    });
    let s = TileStore::open_remote(evil, &scratch("sha")).unwrap();
    assert!(s.load(MASSY_BOX).unwrap_err().contains("sha256"));
}

#[test]
fn open_s3_signs_and_reads_from_a_mock_server() {
    let srv = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = srv.local_addr().unwrap().port();
    let paths = Arc::new(Mutex::new(Vec::new()));
    let seen = paths.clone();
    std::thread::spawn(move || {
        for conn in srv.incoming().flatten() {
            let seen = seen.clone();
            std::thread::spawn(move || {
                let mut r = BufReader::new(&conn);
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                let path = line.split_whitespace().nth(1).unwrap().to_string();
                let mut signed = false;
                loop {
                    let mut h = String::new();
                    r.read_line(&mut h).unwrap();
                    if h.trim().is_empty() {
                        break;
                    }
                    signed |= h
                        .to_lowercase()
                        .starts_with("authorization: aws4-hmac-sha256");
                }
                seen.lock().unwrap().push((path.clone(), signed));
                let name = path.rsplit('/').next().unwrap();
                let mut c = &conn;
                match std::fs::read(Path::new(SRC).join(name)) {
                    Ok(b) => {
                        let _ = write!(
                            c,
                            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                            b.len()
                        );
                        let _ = c.write_all(&b);
                    }
                    Err(_) => {
                        let _ = write!(
                            c,
                            "HTTP/1.1 403 Forbidden\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                        );
                    }
                }
                let _ = r.read(&mut [0u8; 1]);
            });
        }
    });
    // seul test de ce binaire à toucher l'environnement
    unsafe {
        std::env::set_var("S3_ENDPOINT", format!("http://127.0.0.1:{port}"));
        std::env::set_var("AWS_ACCESS_KEY_ID", "AKIDEXAMPLE");
        std::env::set_var("AWS_SECRET_ACCESS_KEY", "secret");
        std::env::set_var("AWS_REGION", "eu-north-1");
    }
    let cache = scratch("s3");
    let s = TileStore::open_s3("s3://bkt/tiles/v-test/", &cache).unwrap();
    assert!(s.root.ends_with("v-test"));
    let t = s.load(MASSY_BOX).unwrap();
    assert!(!t.is_empty());
    let p = paths.lock().unwrap();
    assert!(p.iter().all(|(_, signed)| *signed));
    assert!(
        p.iter()
            .any(|(path, _)| path == "/bkt/tiles/v-test/manifest.json")
    );
    assert!(p.iter().any(|(path, _)| path.ends_with(".npz")));
    assert!(TileStore::open_s3("http://x/y", &cache).is_err());
}

#[test]
fn counter_is_threadsafe_smoke() {
    // 5 dalles en parallèle : toutes présentes, une seule fois
    let hits = Arc::new(Mutex::new(HashMap::new()));
    let cache = scratch("par");
    let s = TileStore::open_remote(bucket(hits.clone(), |_, _, d| Ok(d)), &cache).unwrap();
    let keys: Vec<(i64, i64)> = [(32, 342), (46, 321), (46, 322), (47, 321), (47, 322)].into();
    s.ensure(&keys).unwrap();
    let n = AtomicUsize::new(0);
    for e in std::fs::read_dir(&cache).unwrap().flatten() {
        if e.file_name().to_string_lossy().ends_with(".npz") {
            n.fetch_add(1, SeqCst);
        }
    }
    assert_eq!(n.load(SeqCst), 5);
    assert!(
        hits.lock()
            .unwrap()
            .iter()
            .filter(|(k, _)| k.ends_with(".npz"))
            .all(|(_, &v)| v == 1)
    );
}
