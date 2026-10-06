//! Projections des zones de dalles (`Proj` : Lambert-93 ici, UTM des DOM dans `utm.rs`) et repère
//! local du moteur (`Frame`).
//!
//! Lambert-93 (EPSG:2154, RGF93 ≈ WGS84 à quelques cm) : conique conforme sécante, constantes
//! IGN (notes NTG_71, ALG0003/0004). Sans dépendance (pas de proj).
//!
//! Repère local du moteur : Lambert-93 translaté au départ et divisé par le facteur d'échelle
//! au départ. Conforme, donc les distances y sont vraies à ~1e-5 près sur 25 km ; il est
//! seulement tourné de la convergence des méridiens (jusqu'à ~2° dans les Alpes) par rapport
//! au repère azimutal de Python, ce qui ne change ni longueurs, ni aires, ni ordre angulaire.
use std::f64::consts::FRAC_PI_4;

pub const A: f64 = 6_378_137.0;
pub const E: f64 = 0.081_819_191_042_815_8;
const N: f64 = 0.725_607_765_053_267;
const C: f64 = 11_754_255.426_096;
const XS: f64 = 700_000.0;
const YS: f64 = 12_655_612.049_876;
const LON0: f64 = 3.0;

fn iso_lat(phi: f64) -> f64 {
    let es = E * phi.sin();
    ((FRAC_PI_4 + phi / 2.0).tan() * ((1.0 - es) / (1.0 + es)).powf(E / 2.0)).ln()
}

/// (lon, lat) en degrés → (X, Y) Lambert-93 en m.
pub fn forward(lon: f64, lat: f64) -> (f64, f64) {
    let r = C * (-N * iso_lat(lat.to_radians())).exp();
    let g = N * (lon - LON0).to_radians();
    (XS + r * g.sin(), YS - r * g.cos())
}

/// (X, Y) Lambert-93 → (lon, lat) en degrés.
pub fn inverse(x: f64, y: f64) -> (f64, f64) {
    let (dx, dy) = (x - XS, YS - y);
    let r = dx.hypot(dy);
    let lon = LON0 + (dx.atan2(dy) / N).to_degrees();
    let l = -(r / C).ln() / N;
    (lon, lat_from_iso(l).to_degrees())
}

/// Latitude (rad) depuis la latitude isométrique (GRS80), par point fixe.
pub fn lat_from_iso(l: f64) -> f64 {
    let mut phi = 2.0 * l.exp().atan() - 2.0 * FRAC_PI_4;
    for _ in 0..12 {
        let es = E * phi.sin();
        let nxt =
            2.0 * (((1.0 + es) / (1.0 - es)).powf(E / 2.0) * l.exp()).atan() - 2.0 * FRAC_PI_4;
        let done = (nxt - phi).abs() < 1e-12;
        phi = nxt;
        if done {
            break;
        }
    }
    phi
}

/// Facteur d'échelle de la projection à la latitude `lat` (degrés).
pub fn scale(lat: f64) -> f64 {
    let phi = lat.to_radians();
    let r = C * (-N * iso_lat(phi)).exp();
    let nu = A / (1.0 - (E * phi.sin()).powi(2)).sqrt();
    N * r / (nu * phi.cos())
}

/// Projection d'une zone de dalles (tiles.md § Zones) : Lambert-93 en métropole, UTM dans les DOM.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Proj {
    #[default]
    L93,
    /// Méridien central (degrés) et faux nord (m).
    Utm { lon0: f64, n0: f64 },
}

impl Proj {
    /// CRS du manifeste (`zones.<zone>.crs`) ; inconnu : `None`.
    pub fn from_crs(crs: &str) -> Option<Proj> {
        let (north, south) = (0.0, 10_000_000.0);
        let utm = |lon0, n0| Some(Proj::Utm { lon0, n0 });
        match crs {
            "EPSG:2154" => Some(Proj::L93),
            "EPSG:2975" => utm(57.0, south), // Réunion, RGR92 / UTM 40S
            "EPSG:5490" => utm(-63.0, north), // Guadeloupe, Martinique, RGAF09 / UTM 20N
            "EPSG:2972" => utm(-51.0, north), // Guyane, RGFG95 / UTM 22N
            "EPSG:4471" => utm(45.0, south), // Mayotte, RGM04 / UTM 38S
            _ => None,
        }
    }

    /// (lon, lat) en degrés → (X, Y) en m.
    pub fn forward(self, lon: f64, lat: f64) -> (f64, f64) {
        match self {
            Proj::L93 => forward(lon, lat),
            Proj::Utm { lon0, n0 } => crate::utm::forward(lon0, n0, lon, lat),
        }
    }

    /// (X, Y) en m → (lon, lat) en degrés.
    pub fn inverse(self, x: f64, y: f64) -> (f64, f64) {
        match self {
            Proj::L93 => inverse(x, y),
            Proj::Utm { lon0, n0 } => crate::utm::inverse(lon0, n0, x, y),
        }
    }

    fn scale(self, lon: f64, lat: f64) -> f64 {
        match self {
            Proj::L93 => scale(lat),
            Proj::Utm { lon0, .. } => crate::utm::scale(lon0, lon, lat),
        }
    }
}

/// Repère local (m) centré sur le départ : projection de la zone translatée et mise à l'échelle.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub proj: Proj,
    pub x0: f64,
    pub y0: f64,
    pub k: f64,
}

impl Frame {
    /// Repère Lambert-93 (métropole).
    pub fn new(lat: f64, lon: f64) -> Frame {
        Frame::new_in(Proj::L93, lat, lon)
    }

    pub fn new_in(proj: Proj, lat: f64, lon: f64) -> Frame {
        let (x0, y0) = proj.forward(lon, lat);
        Frame {
            proj,
            x0,
            y0,
            k: proj.scale(lon, lat),
        }
    }

    /// Repère d'essai : Lambert-93 translaté, sans facteur d'échelle.
    pub fn at_l93(x0: f64, y0: f64) -> Frame {
        Frame {
            proj: Proj::L93,
            x0,
            y0,
            k: 1.0,
        }
    }

    pub fn local(&self, x: f64, y: f64) -> [f64; 2] {
        [(x - self.x0) / self.k, (y - self.y0) / self.k]
    }

    /// Coordonnées projetées (m, repère de la zone) d'un point local.
    pub fn l93(&self, p: [f64; 2]) -> (f64, f64) {
        (self.x0 + p[0] * self.k, self.y0 + p[1] * self.k)
    }

    pub fn to_local(&self, lon: f64, lat: f64) -> [f64; 2] {
        let (x, y) = self.proj.forward(lon, lat);
        self.local(x, y)
    }

    /// (lon, lat) d'un point local.
    pub fn to_wgs(&self, p: [f64; 2]) -> (f64, f64) {
        let (x, y) = self.l93(p);
        self.proj.inverse(x, y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lambert93_matches_pyproj() {
        // Valeurs pyproj (EPSG:4326 -> EPSG:2154, always_xy).
        for (lon, lat, x, y) in [
            (2.2713, 48.7309, 646_404.333_891, 6_848_111.158_720),
            (6.07, 45.092, 941_455.977_407, 6_448_331.230_427),
        ] {
            let (px, py) = forward(lon, lat);
            assert!((px - x).abs() < 0.01 && (py - y).abs() < 0.01, "{px} {py}");
            let (qlon, qlat) = inverse(x, y);
            assert!((qlon - lon).abs() < 1e-9 && (qlat - lat).abs() < 1e-9);
        }
        // Le facteur d'échelle vaut la dérivée numérique de la projection.
        let (lat, d) = (45.092, 1e-4);
        let (_, y1) = forward(3.0, lat - d);
        let (_, y2) = forward(3.0, lat + d);
        let meridian = 2.0 * d.to_radians() * A * (1.0 - E * E)
            / (1.0 - (E * lat.to_radians().sin()).powi(2)).powf(1.5);
        assert!(((y2 - y1) / meridian - scale(lat)).abs() < 1e-7);
    }
}
