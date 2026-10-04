//! Lecture des dalles `tiles/1` (contrat `.team/contracts/tiles.md`, producteur
//! `pipeline/build.py`, oracle Python `pipeline/load.py`).
//!
//! `.npz` = zip deflate de `.npy` : lecteur `.npy` maison (en-tête texte + données petit-boutistes,
//! entiers seulement), décompression par la crate `zip`. Tous les tronçons des dalles chargées
//! sont concaténés dans `Troncons` (colonnes, coordonnées L93 absolues en dm).
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Deserialize;

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

/// Dossier de dalles (manifest.json + <ix>_<iy>.npz).
pub struct TileStore {
    pub root: PathBuf,
    pub manifest: Manifest,
}

impl TileStore {
    pub fn open(root: &Path) -> Result<TileStore, String> {
        let text = std::fs::read_to_string(root.join("manifest.json"))
            .map_err(|e| format!("{}: {e}", root.display()))?;
        let manifest: Manifest =
            serde_json::from_str(&text).map_err(|e| format!("manifest.json : {e}"))?;
        if manifest.format != FORMAT {
            return Err(format!(
                "format de dalles inconnu : {} (attendu {FORMAT})",
                manifest.format
            ));
        }
        Ok(TileStore {
            root: root.to_path_buf(),
            manifest,
        })
    }

    /// Entrée du manifeste de la dalle qui contient le point L93 (m), si elle existe.
    pub fn tile_l93(&self, x: f64, y: f64) -> Option<&serde_json::Value> {
        let f = |v: f64| (v / TILE_M).floor() as i64;
        self.manifest.tiles.get(&format!("{}_{}", f(x), f(y)))
    }

    /// Dalles du manifeste qui touchent la boîte L93 (m) [x0, y0, x1, y1], marge comprise.
    pub fn keys(&self, b: [f64; 4]) -> Vec<(i64, i64)> {
        let f = |v: f64| (v / TILE_M).floor() as i64;
        let mut out = Vec::new();
        for ix in f(b[0] - MARGIN_M)..=f(b[2] + MARGIN_M) {
            for iy in f(b[1] - MARGIN_M)..=f(b[3] + MARGIN_M) {
                if self.manifest.tiles.contains_key(&format!("{ix}_{iy}")) {
                    out.push((ix, iy));
                }
            }
        }
        out
    }

    /// Charge les dalles qui touchent la boîte L93 (m).
    pub fn load(&self, b: [f64; 4]) -> Result<Troncons, String> {
        let mut t = Troncons::new();
        for (ix, iy) in self.keys(b) {
            read_tile(&self.root.join(format!("{ix}_{iy}.npz")), ix, iy, &mut t)?;
        }
        Ok(t)
    }
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
