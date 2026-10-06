//! Mercator transverse (UTM des DOM : EPSG:2975, 5490, 2972, 4471), ellipsoïde GRS80, série de
//! Krüger à l'ordre n⁴ (Karney 2011) : < 1 mm jusqu'au bord de fuseau. Sans dépendance.
use crate::l93::{A, E, lat_from_iso};

const K0: f64 = 0.9996;
const E0: f64 = 500_000.0;
/// Troisième aplatissement n = f / (2 − f), 1/f = 298,257222101.
const N: f64 = 1.0 / (2.0 * 298.257_222_101 - 1.0);
const N2: f64 = N * N;
const N3: f64 = N2 * N;
const N4: f64 = N2 * N2;
/// Rayon rectifiant × k0.
const KA: f64 = K0 * A / (1.0 + N) * (1.0 + N2 / 4.0 + N4 / 64.0);
const ALPHA: [f64; 4] = [
    N / 2.0 - 2.0 / 3.0 * N2 + 5.0 / 16.0 * N3 + 41.0 / 180.0 * N4,
    13.0 / 48.0 * N2 - 3.0 / 5.0 * N3 + 557.0 / 1440.0 * N4,
    61.0 / 240.0 * N3 - 103.0 / 140.0 * N4,
    49_561.0 / 161_280.0 * N4,
];
const BETA: [f64; 4] = [
    N / 2.0 - 2.0 / 3.0 * N2 + 37.0 / 96.0 * N3 - 1.0 / 360.0 * N4,
    1.0 / 48.0 * N2 + 1.0 / 15.0 * N3 - 437.0 / 1440.0 * N4,
    17.0 / 480.0 * N3 - 37.0 / 840.0 * N4,
    4_397.0 / 161_280.0 * N4,
];

/// Σ cⱼ sin(2jξ) cosh(2jη), Σ cⱼ cos(2jξ) sinh(2jη).
fn series(c: &[f64; 4], xi: f64, eta: f64) -> (f64, f64) {
    c.iter().enumerate().fold((0.0, 0.0), |s, (j, c)| {
        let k = 2.0 * (j + 1) as f64;
        (
            s.0 + c * (k * xi).sin() * (k * eta).cosh(),
            s.1 + c * (k * xi).cos() * (k * eta).sinh(),
        )
    })
}

/// (lon, lat) en degrés → (E, N) en m ; `lon0` méridien central (degrés), `n0` faux nord (m).
pub fn forward(lon0: f64, n0: f64, lon: f64, lat: f64) -> (f64, f64) {
    let (phi, lam) = (lat.to_radians(), (lon - lon0).to_radians());
    // tangente de la latitude conforme
    let t = (phi.sin().atanh() - E * (E * phi.sin()).atanh()).sinh();
    let xi = t.atan2(lam.cos());
    let eta = (lam.sin() / t.hypot(1.0)).atanh();
    let (dxi, deta) = series(&ALPHA, xi, eta);
    (E0 + KA * (eta + deta), n0 + KA * (xi + dxi))
}

/// (E, N) en m → (lon, lat) en degrés.
pub fn inverse(lon0: f64, n0: f64, x: f64, y: f64) -> (f64, f64) {
    let (xi, eta) = ((y - n0) / KA, (x - E0) / KA);
    let (dxi, deta) = series(&BETA, xi, eta);
    let (xi, eta) = (xi - dxi, eta - deta);
    let chi = (xi.sin() / eta.cosh()).asin();
    let lam = eta.sinh().atan2(xi.cos());
    // latitude isométrique = atanh(sin χ)
    (
        lon0 + lam.to_degrees(),
        lat_from_iso(chi.sin().atanh()).to_degrees(),
    )
}

/// Facteur d'échelle au point (développement en x = distance au méridien, à ~1e-9 près dans
/// un fuseau).
pub fn scale(lon0: f64, lon: f64, lat: f64) -> f64 {
    let phi = lat.to_radians();
    let w = 1.0 - (E * phi.sin()).powi(2);
    // x² / (ρ ν), ρ et ν rayons de courbure principaux
    let x = (forward(lon0, 0.0, lon, lat).0 - E0) / K0;
    let q = x * x * w * w / (A * A * (1.0 - E * E));
    K0 * (1.0 + q / 2.0 + q * q / 24.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: f64 = 10_000_000.0;
    /// Points de contrôle pyproj du lead data (inbox CTO 2026-10-06) : zone, méridien, faux nord,
    /// lon, lat, E, N.
    const CONTROL: [(&str, f64, f64, f64, f64, f64, f64); 18] = [
        ("re", 57.0, S, 55.4786, -21.0990, 341_973.349, 7_666_140.307),
        ("re", 57.0, S, 55.4504, -20.8789, 338_807.609, 7_690_477.745),
        ("re", 57.0, S, 55.7140, -21.2440, 366_557.644, 7_650_305.138),
        ("re", 57.0, S, 55.2190, -21.0330, 314_921.220, 7_673_167.523),
        (
            "gp",
            -63.0,
            0.0,
            -61.6637,
            16.0446,
            642_952.134,
            1_774_329.591,
        ),
        (
            "gp",
            -63.0,
            0.0,
            -61.5340,
            16.2410,
            656_674.774,
            1_796_154.422,
        ),
        (
            "gp",
            -63.0,
            0.0,
            -61.0600,
            16.3030,
            707_281.550,
            1_803_437.388,
        ),
        (
            "gp",
            -63.0,
            0.0,
            -61.2700,
            15.9300,
            685_183.786,
            1_761_959.886,
        ),
        (
            "mq",
            -63.0,
            0.0,
            -61.1652,
            14.8093,
            697_457.972,
            1_638_042.225,
        ),
        (
            "mq",
            -63.0,
            0.0,
            -61.0730,
            14.6040,
            707_577.618,
            1_615_407.761,
        ),
        (
            "mq",
            -63.0,
            0.0,
            -60.8700,
            14.4700,
            729_591.777,
            1_600_773.705,
        ),
        ("gf", -51.0, 0.0, -52.3260, 4.9372, 352_980.205, 545_868.922),
        ("gf", -51.0, 0.0, -54.0320, 5.4980, 164_001.108, 608_565.277),
        ("gf", -51.0, 0.0, -52.6500, 5.1600, 317_110.388, 570_587.389),
        ("gf", -51.0, 0.0, -51.8050, 3.8900, 410_625.717, 430_011.103),
        ("yt", 45.0, S, 45.1553, -12.8062, 516_853.934, 8_584_290.668),
        ("yt", 45.0, S, 45.2280, -12.7810, 524_746.189, 8_587_071.568),
        ("yt", 45.0, S, 45.2830, -12.7900, 530_714.617, 8_586_070.408),
    ];

    #[test]
    fn utm_matches_pyproj_control_points() {
        let mut worst: std::collections::BTreeMap<&str, (f64, f64)> = Default::default();
        for (zone, lon0, n0, lon, lat, e, n) in CONTROL {
            let (x, y) = forward(lon0, n0, lon, lat);
            let d = (x - e).hypot(y - n);
            // sens inverse : écart en m au sol (1° ≈ 111 km), depuis les E, N au mm de pyproj
            let (qlon, qlat) = inverse(lon0, n0, e, n);
            let di = ((qlon - lon) * lat.to_radians().cos()).hypot(qlat - lat) * 111_320.0;
            assert!(d < 0.01 && di < 0.01, "{zone} {lon} {lat} : {d} m, {di} m");
            // aller-retour exact (indépendant de l'arrondi au mm des points de contrôle)
            let (rlon, rlat) = inverse(lon0, n0, x, y);
            assert!((rlon - lon).abs() < 1e-10 && (rlat - lat).abs() < 1e-10);
            let w = worst.entry(zone).or_default();
            *w = (w.0.max(d), w.1.max(di));
        }
        for (zone, (d, di)) in worst {
            eprintln!(
                "{zone} : pire écart direct {:.2} mm, inverse {:.2} mm",
                d * 1e3,
                di * 1e3
            );
        }
    }

    #[test]
    fn scale_is_the_derivative_of_the_projection() {
        // Saint-Laurent-du-Maroni (3° du méridien), Cilaos, Mamoudzou
        for (lon0, lon, lat) in [
            (-51.0, -54.032, 5.498),
            (57.0, 55.471, -21.134),
            (45.0, 45.228, -12.781),
        ] {
            let d = 1e-4_f64;
            let (x1, y1) = forward(lon0, 0.0, lon, lat - d);
            let (x2, y2) = forward(lon0, 0.0, lon, lat + d);
            let meridian = 2.0 * d.to_radians() * A * (1.0 - E * E)
                / (1.0 - (E * lat.to_radians().sin()).powi(2)).powf(1.5);
            let k = (x2 - x1).hypot(y2 - y1) / meridian;
            assert!((k - scale(lon0, lon, lat)).abs() < 1e-7, "{lon} {lat}");
        }
    }
}
