//! Lambert-93 (EPSG:2154, RGF93 ≈ WGS84 à quelques cm) : conique conforme sécante, constantes
//! IGN (notes NTG_71, ALG0003/0004). Sans dépendance (pas de proj).
//!
//! Repère local du moteur : Lambert-93 translaté au départ et divisé par le facteur d'échelle
//! au départ. Conforme, donc les distances y sont vraies à ~1e-5 près sur 25 km ; il est
//! seulement tourné de la convergence des méridiens (jusqu'à ~2° dans les Alpes) par rapport
//! au repère azimutal de Python, ce qui ne change ni longueurs, ni aires, ni ordre angulaire.
use std::f64::consts::FRAC_PI_4;

const A: f64 = 6_378_137.0;
const E: f64 = 0.081_819_191_042_815_8;
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
    (lon, phi.to_degrees())
}

/// Facteur d'échelle de la projection à la latitude `lat` (degrés).
pub fn scale(lat: f64) -> f64 {
    let phi = lat.to_radians();
    let r = C * (-N * iso_lat(phi)).exp();
    let nu = A / (1.0 - (E * phi.sin()).powi(2)).sqrt();
    N * r / (nu * phi.cos())
}

/// Repère local (m) centré sur le départ.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub x0: f64,
    pub y0: f64,
    pub k: f64,
}

impl Frame {
    pub fn new(lat: f64, lon: f64) -> Frame {
        let (x0, y0) = forward(lon, lat);
        Frame {
            x0,
            y0,
            k: scale(lat),
        }
    }

    /// Repère d'essai : Lambert-93 translaté, sans facteur d'échelle.
    pub fn at_l93(x0: f64, y0: f64) -> Frame {
        Frame { x0, y0, k: 1.0 }
    }

    pub fn local(&self, x: f64, y: f64) -> [f64; 2] {
        [(x - self.x0) / self.k, (y - self.y0) / self.k]
    }

    pub fn l93(&self, p: [f64; 2]) -> (f64, f64) {
        (self.x0 + p[0] * self.k, self.y0 + p[1] * self.k)
    }

    pub fn to_local(&self, lon: f64, lat: f64) -> [f64; 2] {
        let (x, y) = forward(lon, lat);
        self.local(x, y)
    }

    /// (lon, lat) d'un point local.
    pub fn to_wgs(&self, p: [f64; 2]) -> (f64, f64) {
        let (x, y) = self.l93(p);
        inverse(x, y)
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
