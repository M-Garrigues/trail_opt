//! Lecture des dalles `tiles/1` (contrat `.team/contracts/tiles.md`, producteur
//! `pipeline/build.py`, oracle Python `pipeline/load.py`).
//!
//! `.npz` = zip deflate de `.npy` : lecteur `.npy` maison (en-tête texte + données petit-boutistes,
//! entiers seulement), décompression par la crate `zip`. Tous les tronçons des dalles chargées
//! sont concaténés dans `Troncons` (colonnes, coordonnées absolues en dm dans le repère de la zone :
//! Lambert-93 en métropole, UTM dans les DOM ; une requête ne charge qu'une zone).
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use ring::digest;
use serde::Deserialize;

use crate::l93::Proj;

pub const FORMAT: &str = "tiles/1";
pub const TILE_M: f64 = 20_000.0;
/// Un tronçon peut déborder de sa dalle : marge de chargement.
pub const MARGIN_M: f64 = 1_000.0;
/// Profil : un point tous les 5 m (50 dm).
pub const STEP_DM: i64 = 50;

/// Nombre de points de profil : ceil(len_dm / 50) + 1 (calcul entier, comme `load.profile_counts`).
pub fn profile_count(len_dm: i32) -> usize {
    ((len_dm as i64 + STEP_DM - 1) / STEP_DM + 1) as usize
}

/// Clé de nœud du contrat : (x_dm << 32) | y_dm.
pub fn node_key(x_dm: i64, y_dm: i64) -> i64 {
    (x_dm << 32) | y_dm
}

#[derive(Deserialize)]
pub struct Manifest {
    pub format: String,
    #[serde(default)]
    pub data_version: String,
    #[serde(default)]
    pub natures: Vec<String>,
    pub tiles: HashMap<String, serde_json::Value>,
    /// `pois.json` (format pois/1, tiles.md § Repères) ; absent : pas de repères.
    #[serde(default)]
    pub pois: Option<serde_json::Value>,
    /// Zones DOM (tiles.md § Zones) ; absent : métropole seule.
    #[serde(default)]
    pub zones: HashMap<String, ZoneDef>,
    /// Colonnes ajoutées par `pipeline enrich` (tiles.md § Étiquettes) ; absent : aucune.
    #[serde(default)]
    pub columns: HashMap<String, serde_json::Value>,
}

/// Revêtement d'un tronçon (`Troncons::surf` après `load_in`) : inconnu (repli sur la nature
/// IGN), revêtu, non revêtu, bois (revêtu sur un pont seulement).
pub const SURF_UNKNOWN: u8 = 0;
pub const SURF_PAVED: u8 = 1;
pub const SURF_UNPAVED: u8 = 2;
pub const SURF_WOOD: u8 = 3;
/// Bits de `Troncons::has` : étiquettes lues dans au moins une dalle.
pub const HAS_CALM: u8 = 1;
pub const HAS_HIKE: u8 = 2;
pub const HAS_WATER: u8 = 4;

impl Manifest {
    /// Code brut `osm_highway` → grand axe OSM (trunk, primary et leurs bretelles, D54).
    fn major_highways(&self) -> Vec<bool> {
        let codes = self
            .columns
            .get("osm_highway")
            .and_then(|c| c["codes"].as_array());
        let mut k = vec![false; 256];
        for (i, c) in codes.into_iter().flatten().enumerate().take(255) {
            k[i] = matches!(
                c.as_str(),
                Some("trunk" | "trunk_link" | "primary" | "primary_link")
            );
        }
        k
    }

    /// Code brut `osm_surface` → `SURF_*` (table `columns.osm_surface.codes` ; 0 et hors table :
    /// inconnu ; toute autre valeur connue hors `columns.osm_surface.paved` : non revêtu).
    fn surface_kinds(&self) -> Vec<u8> {
        // D53, D57 : codes revêtus = `columns.osm_surface.paved` (table du lead data : pavés, briques
        // compris) ; bois revêtu sur un pont seulement ; sans cette liste, tout est inconnu (IGN)
        let col = self.columns.get("osm_surface");
        let paved: Vec<u64> = col
            .and_then(|c| c["paved"].as_array())
            .map(|a| a.iter().filter_map(serde_json::Value::as_u64).collect())
            .unwrap_or_default();
        let mut k = vec![SURF_UNKNOWN; 256];
        if paved.is_empty() {
            return k;
        }
        let codes = col.and_then(|c| c["codes"].as_array());
        for (i, c) in codes.into_iter().flatten().enumerate().take(255).skip(1) {
            k[i] = match c.as_str().unwrap_or("") {
                "" => SURF_UNKNOWN,
                "wood" => SURF_WOOD,
                _ if paved.contains(&(i as u64)) => SURF_PAVED,
                _ => SURF_UNPAVED,
            };
        }
        k
    }
}

#[derive(Deserialize)]
pub struct ZoneDef {
    pub crs: String,
    /// WGS84 : ouest, sud, est, nord.
    pub bbox: [f64; 4],
}

/// Zone de dalles : nom (`""` = métropole, sinon préfixe des clés `<nom>/<ix>_<iy>`) et projection.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Zone {
    pub name: String,
    pub proj: Proj,
}

impl Zone {
    /// Clé de manifeste (et chemin sans `.npz`) de la dalle (ix, iy).
    pub fn key(&self, ix: i64, iy: i64) -> String {
        if self.name.is_empty() {
            format!("{ix}_{iy}")
        } else {
            format!("{}/{ix}_{iy}", self.name)
        }
    }
}

/// Repère col/pic/sommet (pois/1) : position au dm dans le repère de sa zone (L93 sans `zone`),
/// altitude approchée en dm.
#[derive(Deserialize, Clone, Debug)]
pub struct Poi {
    pub nature: String,
    pub name: String,
    pub x_dm: i64,
    pub y_dm: i64,
    pub z_dm: Option<i64>,
    /// Zone DOM du repère ; absent : métropole.
    #[serde(default)]
    pub zone: Option<String>,
}

#[derive(Deserialize)]
struct PoiFile {
    format: String,
    pois: Vec<Poi>,
}

/// Tronçons concaténés de plusieurs dalles.
#[derive(Default, Debug)]
pub struct Troncons {
    pub id: Vec<i64>,
    pub len_dm: Vec<i32>,
    pub dplus_dm: Vec<i32>,
    pub dminus_dm: Vec<i32>,
    pub grade_pm: Vec<u16>,
    pub nature: Vec<u8>,
    pub importance: Vec<u8>,
    pub flags: Vec<u8>,
    /// Sommets du tronçon i : gx/gy[goff[i]..goff[i+1]] (dm L93 absolus).
    pub goff: Vec<usize>,
    pub gx: Vec<i32>,
    pub gy: Vec<i32>,
    /// Profil du tronçon i : z_dm[poff[i]..poff[i+1]] (dm), points à s = 0, 5, …, puis la fin.
    pub poff: Vec<usize>,
    pub z_dm: Vec<i32>,
    /// Parallèles du tronçon i : par_id[par_off[i]..par_off[i+1]] (identifiants globaux).
    pub par_off: Vec<usize>,
    pub par_id: Vec<i64>,
    /// Étiquettes facultatives (tiles.md § Étiquettes), colonne absente = valeur neutre :
    /// classe OSM (0 chemin naturel, 1 intermédiaire, 255 inconnue), revêtement (`SURF_*`),
    /// calme (0–15, 15 = calme), balisage (0–2), bord de l'eau (0–15).
    pub osm_class: Vec<u8>,
    pub surf: Vec<u8>,
    /// `osm_highway` brut, puis au chargement 1 si OSM trunk/primary (grand axe, D54), sinon 0.
    pub osm_major: Vec<u8>,
    /// `osm_flags` brut (bit 1 via ferrata, 2 éclairé, 4 eau potable, 8 sommet/vue), 0 si absent.
    pub osm_flags: Vec<u8>,
    pub calm: Vec<u8>,
    pub hike: Vec<u8>,
    pub water: Vec<u8>,
    /// `HAS_*` : colonnes calm / osm_hike / osm_water présentes dans au moins une dalle.
    pub has: u8,
}

impl Troncons {
    pub fn new() -> Troncons {
        Troncons {
            goff: vec![0],
            poff: vec![0],
            par_off: vec![0],
            ..Default::default()
        }
    }

    pub fn len(&self) -> usize {
        self.id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.id.is_empty()
    }

    pub fn n_profile(&self, t: usize) -> usize {
        self.poff[t + 1] - self.poff[t]
    }

    /// Sommet j du tronçon t, L93 en m.
    pub fn vertex(&self, t: usize, j: usize) -> (f64, f64) {
        let k = self.goff[t] + j;
        (self.gx[k] as f64 / 10.0, self.gy[k] as f64 / 10.0)
    }

    pub fn n_vertices(&self, t: usize) -> usize {
        self.goff[t + 1] - self.goff[t]
    }

    /// Clés des nœuds u (premier sommet) et v (dernier sommet).
    pub fn end_keys(&self, t: usize) -> (i64, i64) {
        let (a, b) = (self.goff[t], self.goff[t + 1] - 1);
        (
            node_key(self.gx[a] as i64, self.gy[a] as i64),
            node_key(self.gx[b] as i64, self.gy[b] as i64),
        )
    }

    /// Abscisse (m) du point de profil i : 5 i, et la longueur pour le dernier.
    pub fn abscissa(&self, t: usize, i: usize) -> f64 {
        let l = self.len_dm[t] as f64 / 10.0;
        if i + 1 == self.n_profile(t) {
            l
        } else {
            (5.0 * i as f64).min(l)
        }
    }

    pub fn z(&self, t: usize, i: usize) -> f64 {
        self.z_dm[self.poff[t] + i] as f64 / 10.0
    }

    /// Positions L93 (m) des points du profil (comme `load.profile_points`) : interpolées sur
    /// la polyligne aux abscisses 5 i ; le dernier point est le dernier sommet.
    pub fn profile_xy(&self, t: usize) -> Vec<(f64, f64)> {
        let (nv, pn) = (self.n_vertices(t), self.n_profile(t));
        let mut out = Vec::with_capacity(pn);
        let mut j = 0;
        let (mut s0, mut a) = (0.0, self.vertex(t, 0));
        let mut b = self.vertex(t, 1.min(nv - 1));
        let mut seg = (b.0 - a.0).hypot(b.1 - a.1);
        for i in 0..pn - 1 {
            let s = 5.0 * i as f64;
            while s > s0 + seg && j + 2 < nv {
                s0 += seg;
                j += 1;
                a = b;
                b = self.vertex(t, j + 1);
                seg = (b.0 - a.0).hypot(b.1 - a.1);
            }
            let f = if seg > 0.0 {
                ((s - s0) / seg).clamp(0.0, 1.0)
            } else {
                0.0
            };
            out.push((a.0 + f * (b.0 - a.0), a.1 + f * (b.1 - a.1)));
        }
        out.push(self.vertex(t, nv - 1));
        out
    }

    /// Boîte englobante des sommets (dm).
    pub fn bbox_dm(&self, t: usize) -> [i32; 4] {
        let r = self.goff[t]..self.goff[t + 1];
        let (xs, ys) = (&self.gx[r.clone()], &self.gy[r]);
        [
            *xs.iter().min().unwrap(),
            *ys.iter().min().unwrap(),
            *xs.iter().max().unwrap(),
            *ys.iter().max().unwrap(),
        ]
    }

    /// Ajoute un tronçon (tests, données synthétiques) : sommets en dm L93, altitude z(x, y)
    /// en m. Profil, montée/descente et pente calculés selon le contrat.
    #[allow(clippy::too_many_arguments)]
    pub fn push(
        &mut self,
        id: i64,
        xy_dm: &[[i64; 2]],
        z: impl Fn(f64, f64) -> f64,
        nature: u8,
        importance: u8,
        flags: u8,
        par: &[i64],
    ) {
        let len: f64 = xy_dm
            .windows(2)
            .map(|p| ((p[1][0] - p[0][0]) as f64).hypot((p[1][1] - p[0][1]) as f64))
            .sum();
        self.id.push(id);
        self.len_dm.push((len.round() as i32).max(1));
        for p in xy_dm {
            self.gx.push(p[0] as i32);
            self.gy.push(p[1] as i32);
        }
        self.goff.push(self.gx.len());
        let t = self.len() - 1;
        let pts = self.profile_xy_from(t);
        let zs: Vec<i32> = pts
            .iter()
            .map(|&(x, y)| (z(x, y) * 10.0).round() as i32)
            .collect();
        let (mut up, mut down) = (0, 0);
        for d in zs.windows(2).map(|w| w[1] - w[0]) {
            if d > 0 { up += d } else { down -= d }
        }
        // pente max sur 25 m (comme graph.compute_profile)
        let s: Vec<f64> = (0..pts.len())
            .map(|i| self.abscissa_from(t, i, pts.len()))
            .collect();
        let mut g: f64 = 0.0;
        let l = *s.last().unwrap();
        if l <= 25.0 {
            g = ((zs[zs.len() - 1] - zs[0]) as f64 / 10.0).abs() / l.max(1e-6);
        } else {
            for i in 0..s.len() {
                if let Some(j) = (i..s.len()).find(|&j| s[j] >= s[i] + 25.0) {
                    g = g.max(((zs[j] - zs[i]) as f64 / 10.0).abs() / (s[j] - s[i]));
                }
            }
        }
        self.z_dm.extend(&zs);
        self.poff.push(self.z_dm.len());
        self.dplus_dm.push(up);
        self.dminus_dm.push(down);
        self.grade_pm.push((g * 1000.0).round().min(65535.0) as u16);
        self.nature.push(nature);
        self.importance.push(importance);
        self.flags.push(flags);
        self.par_id.extend(par);
        self.par_off.push(self.par_id.len());
        self.push_labels();
    }

    /// Étiquettes neutres du dernier tronçon ajouté.
    fn push_labels(&mut self) {
        self.osm_class.push(255);
        self.surf.push(SURF_UNKNOWN);
        self.osm_major.push(0);
        self.osm_flags.push(0);
        self.calm.push(15);
        self.hike.push(0);
        self.water.push(0);
    }

    // `push` : profil avant que poff soit complété.
    fn profile_xy_from(&mut self, t: usize) -> Vec<(f64, f64)> {
        self.poff.push(self.poff[t] + profile_count(self.len_dm[t]));
        let pts = self.profile_xy(t);
        self.poff.pop();
        pts
    }

    fn abscissa_from(&self, t: usize, i: usize, pn: usize) -> f64 {
        let l = self.len_dm[t] as f64 / 10.0;
        if i + 1 == pn {
            l
        } else {
            (5.0 * i as f64).min(l)
        }
    }
}

/// Lecture d'un objet de la source distante (chemin relatif au préfixe : `manifest.json`,
/// `<ix>_<iy>.npz`) : octets, ou erreur (absent compris).
pub type Fetch = Box<dyn Fn(&str) -> Result<Vec<u8>, String> + Send + Sync>;

/// Téléchargements simultanés au plus.
const FETCH_THREADS: usize = 16;
const ENOSPC: i32 = 28;
/// Délai d'un GET S3 (s), réponse entière (mesuré : 21 Mo en 9 dalles en 1,5 s depuis un poste
/// en France). 2 essais ≤ 10 s : une panne rend `busy` avant le plafond de calcul de 15 s
/// (compté depuis avant le chargement), loin du timeout Lambda de 30 s. `TILES_GET_TIMEOUT_S`.
const GET_TIMEOUT_S: u64 = 5;
/// Taille du cache `/tmp` par défaut (Mo) : éphémère Lambda 2 048 Mo moins la marge de /tmp.
const DEFAULT_CACHE_MB: u64 = 1_500;
/// Préfixe des erreurs de la source distante (S3 en panne, dalle illisible) : l'API les rend en
/// `busy` (503, réessayer) et non en erreur de calcul.
pub const REMOTE_ERR: &str = "source distante : ";

/// Dossier de dalles (manifest.json + <ix>_<iy>.npz, DOM : <zone>/<ix>_<iy>.npz). Source distante : `root` est le cache
/// local (/tmp), rempli par `ensure` avant chaque `load`.
pub struct TileStore {
    pub root: PathBuf,
    pub manifest: Manifest,
    /// Repères cols/sommets (vide sans `pois.json`).
    pub pois: Vec<Poi>,
    /// Taille maximale du cache (octets) : au-delà, éviction LRU avant téléchargement.
    pub budget_bytes: u64,
    /// Zones DOM du manifeste et leur emprise WGS84 (ouest, sud, est, nord), rangées par nom.
    zones: Vec<(Zone, [f64; 4])>,
    remote: Option<Fetch>,
}

/// Zones du manifeste ; CRS inconnu : refus (mieux qu'une projection fausse).
fn parse_zones(m: &Manifest) -> Result<Vec<(Zone, [f64; 4])>, String> {
    let mut zones = Vec::new();
    for (name, z) in &m.zones {
        let proj = Proj::from_crs(&z.crs)
            .ok_or_else(|| format!("manifest.json : zone {name} : CRS inconnu {}", z.crs))?;
        let name = name.clone();
        zones.push((Zone { name, proj }, z.bbox));
    }
    zones.sort_by(|a, b| a.0.name.cmp(&b.0.name));
    Ok(zones)
}

fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let manifest: Manifest =
        serde_json::from_str(text).map_err(|e| format!("manifest.json : {e}"))?;
    if manifest.format != FORMAT {
        return Err(format!(
            "format de dalles inconnu : {} (attendu {FORMAT})",
            manifest.format
        ));
    }
    Ok(manifest)
}

fn parse_pois(f: &str, text: &str) -> Result<Vec<Poi>, String> {
    let pf: PoiFile = serde_json::from_str(text).map_err(|e| format!("{f} : {e}"))?;
    if pf.format != "pois/1" {
        return Err(format!("{f} : format {} (attendu pois/1)", pf.format));
    }
    Ok(pf.pois)
}

impl TileStore {
    pub fn open(root: &Path) -> Result<TileStore, String> {
        let read = |f: &str| std::fs::read_to_string(root.join(f)).map_err(|e| format!("{f}: {e}"));
        let manifest = parse_manifest(&read("manifest.json")?)?;
        let pois = match manifest.pois.as_ref().and_then(|p| p["file"].as_str()) {
            None => Vec::new(),
            Some(f) => parse_pois(f, &read(f)?)?,
        };
        Ok(TileStore {
            root: root.to_path_buf(),
            zones: parse_zones(&manifest)?,
            manifest,
            pois,
            budget_bytes: u64::MAX,
            remote: None,
        })
    }

    /// Source distante (D10 : `TILES_S3`) : manifeste et repères lus au démarrage, dalles
    /// téléchargées à la demande dans `cache` (le dossier est créé).
    pub fn open_remote(fetch: Fetch, cache: &Path) -> Result<TileStore, String> {
        std::fs::create_dir_all(cache).map_err(|e| format!("{}: {e}", cache.display()))?;
        // reprise après coupure : fichiers partiels d'un environnement précédent
        for (_, _, path, name) in cache_files(cache) {
            if name.ends_with(".part") {
                let _ = std::fs::remove_file(path);
            }
        }
        // un seul retry : S3 peut hoqueter au démarrage à froid
        let text = |f: &str| {
            fetch(f)
                .or_else(|_| fetch(f))
                .and_then(|b| String::from_utf8(b).map_err(|e| format!("{f} : {e}")))
                .map_err(|e| format!("{REMOTE_ERR}{e}"))
        };
        let manifest = parse_manifest(&text("manifest.json")?)?;
        let pois = match manifest.pois.as_ref().and_then(|p| p["file"].as_str()) {
            None => Vec::new(),
            Some(f) => {
                let t = text(f)?;
                if let Some(h) = manifest.pois.as_ref().and_then(|p| p["sha256"].as_str())
                    && crate::share::hex(digest::digest(&digest::SHA256, t.as_bytes()).as_ref())
                        != h
                {
                    return Err(format!("{REMOTE_ERR}{f} : sha256 ≠ manifeste"));
                }
                parse_pois(f, &t)?
            }
        };
        Ok(TileStore {
            root: cache.to_path_buf(),
            zones: parse_zones(&manifest)?,
            manifest,
            pois,
            budget_bytes: u64::MAX,
            remote: Some(fetch),
        })
    }

    /// `s3://bucket/tiles/<version>/` : cache `<cache_root>/<version>` (défaut /tmp/tiles).
    /// Région `AWS_REGION` (défaut eu-north-1), identifiants du rôle ; `S3_ENDPOINT` =
    /// serveur compatible S3 (MinIO, tests locaux).
    pub fn open_s3(url: &str, cache_root: &Path) -> Result<TileStore, String> {
        use crate::share::{Creds, S3Target, s3_call};
        let rest = url
            .strip_prefix("s3://")
            .ok_or("TILES_S3 : s3://bucket/préfixe/")?;
        let (bucket, prefix) = rest.split_once('/').unwrap_or((rest, ""));
        let prefix = prefix.trim_matches('/');
        let version = prefix.rsplit('/').next().unwrap_or_default();
        if bucket.is_empty() || version.is_empty() {
            return Err("TILES_S3 : s3://bucket/tiles/<version>/ attendu".into());
        }
        let target = S3Target {
            bucket: bucket.into(),
            region: std::env::var("AWS_REGION").unwrap_or_else(|_| "eu-north-1".into()),
            endpoint: std::env::var("S3_ENDPOINT").ok().filter(|e| !e.is_empty()),
        };
        Creds::from_env()?;
        let timeout = std::env::var("TILES_GET_TIMEOUT_S")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(GET_TIMEOUT_S);
        let prefix = prefix.to_string();
        let fetch: Fetch = Box::new(move |name| {
            let key = format!("{prefix}/{name}");
            // identifiants temporaires du rôle (AWS_SESSION_TOKEN compris) : Lambda les fixe pour
            // la vie de l'environnement d'exécution, recyclé avant leur expiration
            let creds = Creds::from_env()?;
            match s3_call(&target, &creds, "GET", &key, &[], None, (timeout, 64 << 20))? {
                (200, b) => Ok(b),
                (s, _) => Err(format!("s3 GET {key} : HTTP {s}")),
            }
        });
        let mut store = TileStore::open_remote(fetch, &cache_root.join(version))?;
        // TILES_CACHE_MB : plafond du cache (défaut 1 500 Mo pour un éphémère de 2 048 Mo)
        let mb = std::env::var("TILES_CACHE_MB")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(DEFAULT_CACHE_MB);
        store.budget_bytes = mb << 20;
        Ok(store)
    }

    /// Source distante : télécharge dans le cache les dalles absentes (16 GET en parallèle ;
    /// `.part` puis rename ; taille et sha256 du manifeste vérifiés ; un retry). Les dalles déjà
    /// là sont « touchées » (LRU) ; si le cache dépasse `budget_bytes`, les plus anciennes hors
    /// requête sont évincées AVANT le téléchargement. Sans effet en local.
    pub fn ensure(&self, keys: &[(i64, i64)]) -> Result<(), String> {
        self.ensure_in(&Zone::default(), keys)
    }

    /// `ensure` pour les dalles d'une zone (cache : sous-dossier de la zone).
    pub fn ensure_in(&self, zone: &Zone, keys: &[(i64, i64)]) -> Result<(), String> {
        let Some(fetch) = &self.remote else {
            return Ok(());
        };
        let keep: Vec<String> = keys
            .iter()
            .map(|&(ix, iy)| zone.key(ix, iy) + ".npz")
            .collect();
        let (mut missing, mut need) = (Vec::new(), 0u64);
        for n in &keep {
            let Some(e) = self.manifest.tiles.get(n.trim_end_matches(".npz")) else {
                return Err(format!("dalle {n} hors du manifeste"));
            };
            let want = e["bytes"].as_u64();
            let path = self.root.join(n);
            match std::fs::metadata(&path) {
                Ok(m) if want.is_none_or(|w| w == m.len()) => {
                    let _ = File::open(&path)
                        .and_then(|f| f.set_modified(std::time::SystemTime::now()));
                }
                _ => {
                    need += want.unwrap_or(0);
                    missing.push(n);
                }
            }
        }
        if !missing.is_empty() && self.budget_bytes != u64::MAX {
            let used = cache_bytes(&self.root);
            if used + need > self.budget_bytes {
                evict(&self.root, &keep, used + need - self.budget_bytes);
            }
        }
        let next = std::sync::atomic::AtomicUsize::new(0);
        let failure = std::sync::Mutex::new(None);
        std::thread::scope(|sc| {
            for _ in 0..FETCH_THREADS.min(missing.len()) {
                sc.spawn(|| {
                    while failure.lock().unwrap().is_none() {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(n) = missing.get(i) else { break };
                        if let Err(e) = self.fetch_tile(fetch, n, &keep) {
                            failure.lock().unwrap().get_or_insert(e);
                        }
                    }
                });
            }
        });
        failure
            .into_inner()
            .unwrap()
            .map_or(Ok(()), |e| Err(format!("{REMOTE_ERR}{e}")))
    }

    fn fetch_tile(&self, fetch: &Fetch, name: &str, keep: &[String]) -> Result<(), String> {
        let entry = &self.manifest.tiles[name.trim_end_matches(".npz")]; // vérifié par `ensure`
        let once = || -> Result<(), String> {
            let data = fetch(name)?;
            if entry["bytes"]
                .as_u64()
                .is_some_and(|b| b != data.len() as u64)
            {
                return Err(format!("taille {} ≠ manifeste", data.len()));
            }
            if let Some(h) = entry["sha256"].as_str()
                && crate::share::hex(digest::digest(&digest::SHA256, &data).as_ref()) != h
            {
                return Err("sha256 ≠ manifeste".into());
            }
            self.write_atomic(name, &data, keep)
        };
        once()
            .or_else(|_| once())
            .map_err(|e| format!("dalle {name} : {e}"))
    }

    /// `.part` puis rename ; disque plein : éviction des plus anciennes dalles hors `keep`.
    fn write_atomic(&self, name: &str, data: &[u8], keep: &[String]) -> Result<(), String> {
        let (part, dest) = (self.root.join(format!("{name}.part")), self.root.join(name));
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let mut res = std::fs::write(&part, data);
        if res
            .as_ref()
            .is_err_and(|e| e.raw_os_error() == Some(ENOSPC))
        {
            evict(&self.root, keep, data.len() as u64);
            res = std::fs::write(&part, data);
        }
        res.and_then(|_| std::fs::rename(&part, &dest))
            .map_err(|e| {
                let _ = std::fs::remove_file(&part);
                e.to_string()
            })
    }

    /// Zone d'un point : celle dont l'emprise le contient (emprises disjointes), sinon métropole.
    pub fn zone(&self, lat: f64, lon: f64) -> Zone {
        self.zones
            .iter()
            .find(|(_, b)| (b[0]..=b[2]).contains(&lon) && (b[1]..=b[3]).contains(&lat))
            .map(|z| z.0.clone())
            .unwrap_or_default()
    }

    /// Le point peut-il figurer dans une requête de `zone` ? Oui dans sa zone ; autour d'un DOM, oui
    /// aussi en mer, jusqu'à 0,5° de l'emprise (polygone dessiné plus large que l'île).
    pub fn near(&self, zone: &Zone, lat: f64, lon: f64) -> bool {
        const PAD: f64 = 0.5;
        let z = self.zone(lat, lon);
        z == *zone
            || z.name.is_empty()
                && self.zones.iter().any(|(q, b)| {
                    q == zone
                        && (b[0] - PAD..=b[2] + PAD).contains(&lon)
                        && (b[1] - PAD..=b[3] + PAD).contains(&lat)
                })
    }

    /// Entrée du manifeste de la dalle de `zone` qui contient le point, si elle existe.
    pub fn tile_in(&self, zone: &Zone, lat: f64, lon: f64) -> Option<&serde_json::Value> {
        let (x, y) = zone.proj.forward(lon, lat);
        let f = |v: f64| (v / TILE_M).floor() as i64;
        self.manifest.tiles.get(&zone.key(f(x), f(y)))
    }

    /// Entrée du manifeste de la dalle qui contient le point (zone du point), si elle existe.
    pub fn tile_at(&self, lat: f64, lon: f64) -> Option<&serde_json::Value> {
        self.tile_in(&self.zone(lat, lon), lat, lon)
    }

    /// Dalles de métropole du manifeste qui touchent la boîte L93 (m) [x0, y0, x1, y1], marge comprise.
    pub fn keys(&self, b: [f64; 4]) -> Vec<(i64, i64)> {
        self.keys_in(&Zone::default(), b)
    }

    /// Dalles de `zone` qui touchent la boîte (m, repère de la zone), marge comprise.
    pub fn keys_in(&self, zone: &Zone, b: [f64; 4]) -> Vec<(i64, i64)> {
        let f = |v: f64| (v / TILE_M).floor() as i64;
        let mut out = Vec::new();
        for ix in f(b[0] - MARGIN_M)..=f(b[2] + MARGIN_M) {
            for iy in f(b[1] - MARGIN_M)..=f(b[3] + MARGIN_M) {
                if self.manifest.tiles.contains_key(&zone.key(ix, iy)) {
                    out.push((ix, iy));
                }
            }
        }
        out
    }

    /// Charge les dalles de métropole qui touchent la boîte L93 (m).
    pub fn load(&self, b: [f64; 4]) -> Result<Troncons, String> {
        self.load_in(&Zone::default(), b)
    }

    /// Charge les dalles de `zone` qui touchent la boîte (m, repère de la zone).
    pub fn load_in(&self, zone: &Zone, b: [f64; 4]) -> Result<Troncons, String> {
        let mut t = Troncons::new();
        let keys = self.keys_in(zone, b);
        self.ensure_in(zone, &keys)?;
        for (ix, iy) in keys {
            let path = self.root.join(zone.key(ix, iy) + ".npz");
            // cache /tmp corrompu : on le jette et on retélécharge une fois
            if let Err(e) = read_tile(&path, ix, iy, &mut t) {
                if self.remote.is_none() {
                    return Err(e);
                }
                let _ = std::fs::remove_file(&path);
                self.ensure_in(zone, &[(ix, iy)])?;
                read_tile(&path, ix, iy, &mut t).map_err(|e| format!("{REMOTE_ERR}{e}"))?;
            }
        }
        let kinds = self.manifest.surface_kinds();
        t.surf.iter_mut().for_each(|c| *c = kinds[*c as usize]);
        let major = self.manifest.major_highways();
        t.osm_major
            .iter_mut()
            .for_each(|c| *c = u8::from(major[*c as usize]));
        Ok(t)
    }
}

/// Fichiers du cache : (date, taille, chemin, nom relatif `<ix>_<iy>.npz` ou `<zone>/<ix>_<iy>.npz`).
fn cache_files(dir: &Path) -> Vec<(Option<std::time::SystemTime>, u64, PathBuf, String)> {
    let ls = |d: &Path| std::fs::read_dir(d).into_iter().flatten().flatten();
    let mut out = Vec::new();
    for e in ls(dir) {
        let name = e.file_name().to_string_lossy().into_owned();
        let Ok(m) = e.metadata() else { continue };
        if !m.is_dir() {
            out.push((m.modified().ok(), m.len(), e.path(), name));
            continue;
        }
        // sous-dossier d'une zone DOM
        for f in ls(&e.path()) {
            if let Ok(m) = f.metadata() {
                let rel = format!("{name}/{}", f.file_name().to_string_lossy());
                out.push((m.modified().ok(), m.len(), f.path(), rel));
            }
        }
    }
    out
}

/// Octets du cache (dalles et fichiers partiels).
fn cache_bytes(dir: &Path) -> u64 {
    cache_files(dir).iter().map(|f| f.1).sum()
}

/// Supprime les dalles les plus anciennes (hors `keep`) jusqu'à libérer `need` octets.
fn evict(dir: &Path, keep: &[String], need: u64) -> u64 {
    let mut files = cache_files(dir);
    files.retain(|f| f.3.ends_with(".npz") && !keep.contains(&f.3));
    files.sort();
    let mut freed = 0;
    for (_, len, p, _) in files {
        if freed >= need {
            break;
        }
        if std::fs::remove_file(p).is_ok() {
            freed += len;
        }
    }
    freed
}

/// Tableau `.npy` d'entiers, converti en i64.
fn read_npy(mut r: impl Read, name: &str) -> Result<Vec<i64>, String> {
    let err = |m: &str| format!("{name} : {m}");
    let mut head = [0u8; 10];
    r.read_exact(&mut head).map_err(|e| err(&e.to_string()))?;
    if &head[..6] != b"\x93NUMPY" {
        return Err(err("pas un fichier .npy"));
    }
    let hlen = if head[6] == 1 {
        u16::from_le_bytes([head[8], head[9]]) as usize
    } else {
        let mut b = [0u8; 2];
        r.read_exact(&mut b).map_err(|e| err(&e.to_string()))?;
        u32::from_le_bytes([head[8], head[9], b[0], b[1]]) as usize
    };
    let mut h = vec![0u8; hlen];
    r.read_exact(&mut h).map_err(|e| err(&e.to_string()))?;
    let h = String::from_utf8_lossy(&h);
    let field = |k: &str| -> Option<String> {
        let i = h.find(&format!("'{k}'"))? + k.len() + 3;
        let rest = h[i..].trim_start_matches([':', ' ']);
        Some(rest.to_string())
    };
    let descr = field("descr").ok_or_else(|| err("descr absent"))?;
    let descr = descr.trim_start_matches('\'');
    let descr = &descr[..descr.find('\'').ok_or_else(|| err("descr"))?];
    if field("fortran_order").is_some_and(|f| f.starts_with("True")) {
        return Err(err("ordre Fortran non géré"));
    }
    let shape = field("shape").ok_or_else(|| err("shape absent"))?;
    let shape = &shape[1..shape.find(')').ok_or_else(|| err("shape"))?];
    let dims: Vec<usize> = shape
        .split(',')
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().parse().map_err(|_| err("shape")))
        .collect::<Result<_, _>>()?;
    let n: usize = dims.iter().product();
    let size = match &descr[1..] {
        "i1" | "u1" | "b1" => 1,
        "i2" | "u2" => 2,
        "i4" | "u4" => 4,
        "i8" | "u8" => 8,
        d => return Err(err(&format!("type non géré : {d}"))),
    };
    if size > 1 && !descr.starts_with('<') {
        return Err(err("ordre des octets non petit-boutiste"));
    }
    let mut buf = vec![0u8; n * size];
    r.read_exact(&mut buf).map_err(|e| err(&e.to_string()))?;
    let signed = descr.as_bytes()[1] == b'i';
    Ok(buf
        .chunks_exact(size)
        .map(|c| match (size, signed) {
            (1, true) => c[0] as i8 as i64,
            (1, false) => c[0] as i64,
            (2, true) => i16::from_le_bytes([c[0], c[1]]) as i64,
            (2, false) => u16::from_le_bytes([c[0], c[1]]) as i64,
            (4, true) => i32::from_le_bytes(c.try_into().unwrap()) as i64,
            (4, false) => u32::from_le_bytes(c.try_into().unwrap()) as i64,
            _ => i64::from_le_bytes(c.try_into().unwrap()),
        })
        .collect())
}

/// Ajoute à `t` les tronçons de la dalle (ix, iy) ; vérifie la cohérence des tailles.
pub fn read_tile(path: &Path, ix: i64, iy: i64, t: &mut Troncons) -> Result<(), String> {
    let f = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut z = zip::ZipArchive::new(f).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut col = |name: &str| -> Result<Vec<i64>, String> {
        let file = z
            .by_name(&format!("{name}.npy"))
            .map_err(|e| format!("{} : {name} : {e}", path.display()))?;
        read_npy(file, name)
    };
    let id_d = col("id_d")?;
    let n = id_d.len();
    let len_dm = col("len_dm")?;
    let geom_n = col("geom_n")?;
    let (gx, gy) = (col("geom_x")?, col("geom_y")?);
    let (prof_d, z0) = (col("prof_d")?, col("prof_z0_dm")?);
    let (par_n, par_id) = (col("par_n")?, col("par_id")?);
    // étiquettes facultatives : colonne absente = neutre (tiles.md § Étiquettes)
    let mut opt = |name: &str| -> Result<Option<Vec<i64>>, String> {
        match z.by_name(&format!("{name}.npy")) {
            Ok(file) => read_npy(file, name).map(Some),
            Err(_) => Ok(None),
        }
    };
    let labels = [
        opt("osm_class")?,
        opt("osm_surface")?,
        opt("calm")?,
        opt("osm_hike")?,
        opt("osm_water")?,
        opt("osm_highway")?,
        opt("osm_flags")?,
    ];
    let mut col = |name: &str| -> Result<Vec<i64>, String> {
        let file = z
            .by_name(&format!("{name}.npy"))
            .map_err(|e| format!("{} : {name} : {e}", path.display()))?;
        read_npy(file, name)
    };
    let cols = [
        col("dplus_dm")?,
        col("dminus_dm")?,
        col("max_grade_pm")?,
        col("nature")?,
        col("importance")?,
        col("flags")?,
    ];
    let bad = |m: &str| Err(format!("{} : {m}", path.display()));
    if [len_dm.len(), geom_n.len(), z0.len(), par_n.len()]
        .into_iter()
        .chain(cols.iter().map(Vec::len))
        .chain(labels.iter().flatten().map(Vec::len))
        .any(|k| k != n)
    {
        return bad("colonnes de tailles différentes");
    }
    if geom_n.iter().any(|&k| k < 2) || geom_n.iter().sum::<i64>() as usize != gx.len() {
        return bad("géométrie incohérente");
    }
    if gy.len() != gx.len() || par_n.iter().sum::<i64>() as usize != par_id.len() {
        return bad("géométrie ou parallèles incohérents");
    }
    if len_dm.iter().any(|&l| l < 1 || l > i32::MAX as i64)
        || len_dm
            .iter()
            .map(|&l| profile_count(l as i32))
            .sum::<usize>()
            != prof_d.len()
    {
        return bad("profil incohérent avec len_dm");
    }
    let (ox, oy) = (ix * TILE_M as i64 * 10, iy * TILE_M as i64 * 10);
    let (mut id, mut gk, mut pk) = (0i64, 0usize, 0usize);
    for i in 0..n {
        id += id_d[i];
        t.id.push(id);
        t.len_dm.push(len_dm[i] as i32);
        t.dplus_dm.push(cols[0][i] as i32);
        t.dminus_dm.push(cols[1][i] as i32);
        t.grade_pm.push(cols[2][i] as u16);
        t.nature.push(cols[3][i] as u8);
        t.importance.push(cols[4][i] as u8);
        t.flags.push(cols[5][i] as u8);
        let lab = |k: usize, d: u8| labels[k].as_ref().map_or(d, |c| c[i] as u8);
        t.osm_class.push(lab(0, 255));
        t.surf.push(lab(1, SURF_UNKNOWN));
        t.calm.push(lab(2, 15));
        t.hike.push(lab(3, 0));
        t.water.push(lab(4, 0));
        t.osm_major.push(lab(5, 0));
        t.osm_flags.push(lab(6, 0));
        let (mut x, mut y) = (ox, oy);
        for j in 0..geom_n[i] as usize {
            x += gx[gk + j];
            y += gy[gk + j];
            t.gx.push(x as i32);
            t.gy.push(y as i32);
        }
        gk += geom_n[i] as usize;
        t.goff.push(t.gx.len());
        let mut zz = z0[i];
        for j in 0..profile_count(len_dm[i] as i32) {
            zz += prof_d[pk + j];
            t.z_dm.push(zz as i32);
        }
        pk += profile_count(len_dm[i] as i32);
        t.poff.push(t.z_dm.len());
    }
    for (k, bit) in [(2, HAS_CALM), (3, HAS_HIKE), (4, HAS_WATER)] {
        if labels[k].is_some() {
            t.has |= bit;
        }
    }
    t.par_id.extend(&par_id);
    let base = t.par_off.last().copied().unwrap_or(0);
    let mut acc = base;
    for k in par_n {
        acc += k as usize;
        t.par_off.push(acc);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// D53 : table `columns.osm_surface.codes` du manifeste → revêtu / non revêtu / bois / inconnu.
    #[test]
    fn surface_kinds_from_manifest() {
        let m: Manifest = serde_json::from_str(
            r#"{"format": "tiles/1", "tiles": {}, "columns": {"osm_surface": {"codes":
                ["", "asphalt", "paved", "cobblestone", "wood", "gravel", "sett"],
                "paved": [1, 2, 3, 4, 6]}}}"#,
        )
        .unwrap();
        let k = m.surface_kinds();
        assert_eq!(
            k[..8],
            [
                SURF_UNKNOWN,
                SURF_PAVED,
                SURF_PAVED,
                SURF_PAVED, // pavés : revêtus (D57)
                SURF_WOOD,
                SURF_UNPAVED,
                SURF_PAVED,
                SURF_UNKNOWN
            ]
        );
        assert_eq!(k[255], SURF_UNKNOWN);
        let none: Manifest = serde_json::from_str(r#"{"format": "tiles/1", "tiles": {}}"#).unwrap();
        assert!(none.surface_kinds().iter().all(|&x| x == SURF_UNKNOWN));
    }

    #[test]
    fn profile_points_follow_contract() {
        // L de 12,3 m puis 7,7 m : 20 m, 5 points (0, 5, 10, 15, fin)
        let mut t = Troncons::new();
        t.push(1, &[[0, 0], [123, 0], [123, 77]], |x, _| x, 0, 0, 0, &[]);
        assert_eq!(t.len_dm[0], 200);
        assert_eq!(t.n_profile(0), 5);
        let p = t.profile_xy(0);
        assert_eq!(p.len(), 5);
        assert!((p[2].0 - 10.0).abs() < 1e-9 && p[2].1 == 0.0);
        assert!((p[3].0 - 12.3).abs() < 1e-9 && (p[3].1 - 2.7).abs() < 1e-9);
        assert_eq!(p[4], (12.3, 7.7));
        // montée - descente = z(v) - z(u)
        assert_eq!(t.dplus_dm[0] - t.dminus_dm[0], t.z_dm[4] - t.z_dm[0]);
    }
}

#[cfg(test)]
mod evict_tests {
    use super::evict;

    #[test]
    fn evicts_oldest_first_and_spares_kept_tiles() {
        let d = std::env::temp_dir().join(format!("optrail-evict-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let t0 = std::time::SystemTime::now();
        for (i, n) in ["a", "b", "c", "keep"].iter().enumerate() {
            let f = std::fs::File::create(d.join(format!("{n}.npz"))).unwrap();
            f.set_len(100).unwrap();
            let age = std::time::Duration::from_secs(100 - 10 * i as u64);
            f.set_modified(t0 - age).unwrap();
        }
        let freed = evict(&d, &["keep.npz".to_string()], 150);
        assert_eq!(freed, 200);
        assert!(!d.join("a.npz").exists() && !d.join("b.npz").exists());
        assert!(d.join("c.npz").exists() && d.join("keep.npz").exists());
    }
}
