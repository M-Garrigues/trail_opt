//! Profil de sport (D70, T82) : tout ce qui change selon (sport, type de voie), construit par
//! `Profile::of` sans trait ni configuration. E1 : seulement ce que la validation de l'API lit
//! (bornes de distance et de pente), plus (E2) les limites de zone, de budget et de distance
//! max de « Le plus court ». Les coûts par type de voie et l'exclusion des tronçons ne sont pas des
//! champs : ce sont les bras de `plan::off_parts`, `plan::pref_mask` et `plan::network`.
use crate::codes::{Code, Msg};
use crate::plan::DIST_KM;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Profile {
    pub sport: &'static str,
    pub surface: &'static str,
    /// Bornes de `distance_km` (km).
    pub dist_km: (f64, f64),
    /// Bornes de `max_grade_pct` ; `grade_unlimited` : 0 = sans limite (trail seulement).
    pub grade_pct: (f64, f64),
    pub grade_unlimited: bool,
    /// D73 : limites de pente (fractions, sur 50 m) en montée et en descente. Égales en v1 : celle
    /// du profil par défaut ; en trail, la valeur de la requête s'applique aux deux (E2).
    pub grade_up_max: f64,
    pub grade_down_max: f64,
    /// Aire maximale de la zone (km²) : disque de 25 km en trail, de 50 km en vélo (D72).
    pub max_area_km2: f64,
    /// Rayon par défaut de la zone = `radius_frac` × distance (D/2 en trail, L/4 en vélo).
    pub radius_frac: f64,
    /// Avertissement `long_distance` au-delà de ce nombre de km (trail seulement).
    pub long_km: Option<f64>,
    /// Distance max par défaut de « Le plus court » : (m de D+ par km en relief, en plaine, km max).
    pub md_cap: (f64, f64, f64),
}

/// Profil trail « Tout » : réseau sans préférence (`keep_mask`, tests).
pub const TRAIL_ANY: Profile = Profile {
    sport: "trail",
    surface: "any",
    dist_km: DIST_KM,
    grade_pct: (5.0, 60.0),
    grade_unlimited: true,
    grade_up_max: 0.60,
    grade_down_max: 0.60,
    max_area_km2: 1965.0,
    radius_frac: 0.5,
    long_km: Some(25.0),
    md_cap: (25.0, 15.0, 60.0),
};

impl Profile {
    /// Profil d'un couple (sport, type de voie) : `sport` inconnu → `invalid_request`, type de voie
    /// hors de la liste du sport → `roads_unknown` (trail : `trail|any|road`, bike : `mtb|gravel|road`).
    pub fn of(sport: &str, surface: &str) -> Result<Profile, Msg> {
        // (type de voie, distance min–max km, pente max, md_cap : m/km relief, m/km plaine, km max)
        let (surface, dist_km, grade, md_cap) = match (sport, surface) {
            ("trail", "trail") => ("trail", DIST_KM, 0.60, TRAIL_ANY.md_cap),
            ("trail", "any") => ("any", DIST_KM, 0.60, TRAIL_ANY.md_cap),
            ("trail", "road") => ("road", DIST_KM, 0.60, TRAIL_ANY.md_cap),
            // [E] md_cap vélo : à régler au banc B1 ; D76 : gravel comme la Route (200 km) ;
            // D81 : pente max VTT 25 %, gravel 22 % (symétriques)
            ("bike", "mtb") => ("mtb", (5.0, 100.0), 0.25, (20.0, 12.0, 100.0)),
            ("bike", "gravel") => ("gravel", (10.0, 200.0), 0.22, (15.0, 10.0, 200.0)),
            ("bike", "road") => ("road", (10.0, 200.0), 0.20, (12.0, 8.0, 200.0)),
            ("trail" | "bike", s) => return Err(Msg::error(Code::RoadsUnknown, s)),
            _ => {
                return Err(Msg::error(
                    Code::InvalidRequest,
                    "sport: trail or bike expected",
                ));
            }
        };
        let trail = sport == "trail";
        let bike_road = !trail && surface == "road";
        Ok(Profile {
            sport: if trail { "trail" } else { "bike" },
            surface,
            dist_km,
            // vélo : limite de montée 5–40 %, pas de « sans limite »
            grade_pct: (5.0, if trail { 60.0 } else { 40.0 }),
            // trail et Route vélo : 0 = sans limite ; Route vélo : sans limite PAR DÉFAUT (D78,
            // `grade_up_max` infini : le filtre revêtu suffit)
            grade_unlimited: trail || surface == "road",
            grade_up_max: if bike_road { f64::INFINITY } else { grade },
            grade_down_max: if bike_road { f64::INFINITY } else { grade },
            max_area_km2: if trail { 1965.0 } else { 7854.0 },
            radius_frac: if trail { 0.5 } else { 0.25 },
            long_km: trail.then_some(25.0),
            md_cap,
        })
    }

    pub fn bike(&self) -> bool {
        self.sport == "bike"
    }

    /// Préférence de voie active (coût par type) : tout en vélo, sauf trail « Tout ».
    pub fn pref_on(&self) -> bool {
        self.bike() || self.surface != "any"
    }
}
