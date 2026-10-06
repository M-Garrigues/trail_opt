//! Chaîne complète depuis les dalles : requête → zone → graphe → solveur → tracés
//! (port de `pipeline.plan_loop` / `search_loop` / `build_candidate` / `Access`, source IGN).
use std::collections::HashSet;
use std::time::Instant;

use geo::{BooleanOps, Coord, LineString, MultiPolygon, Polygon, Validation};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::codes::{Code, Msg};
use crate::l93::Frame;
use crate::prep::{self, Edge, FREE_RADIUS, Net, REDUCE_K, REDUCE_MIN_EDGES, Region};
use crate::problem::{SURF_TARGET, SURF_TARGET_BAND};
use crate::tiles::{TileStore, Troncons};
use crate::{Budget, Problem, optimize};

pub const DIST_KM: (f64, f64) = (2.0, 100.0);
pub const TIME_S: (f64, f64) = (5.0, 180.0);
/// Disque de 25 km de rayon.
pub const MAX_AREA_KM2: f64 = 1965.0;
/// Plafonds de tronçons dans la zone (comptés avant téléchargement en Python) : IGN mode max,
/// et autres modes. Au-delà, la zone est réduite autour du départ.
pub const MAX_WAYS_MAX: usize = 650_000;
pub const MAX_WAYS: usize = 110_000;
pub const FALLBACK_AREA_KM2: f64 = 150.0;
pub const LONG_KM: f64 = 25.0;
pub const START_BUFFER_M: f64 = 50.0;
pub const BBOX_MARGIN_M: f64 = 200.0;
/// Au-delà, on cherche un accès par la route depuis le point cliqué.
pub const ACCESS_MIN_M: f64 = 30.0;
/// Le point cliqué doit être à moins de ça d'une voie quelconque.
pub const ACCESS_MAX_START_M: f64 = 100.0;
/// L'aller-retour d'accès prend au plus cette part de la distance.
pub const ACCESS_MAX_SHARE: f64 = 0.6;
pub const MIN_ATTEMPT_S: f64 = 3.0;
pub const MAX_ATTEMPTS: usize = 8;
/// Mode min_distance : bornes du D+ visé (m) et distance max par défaut clamp(X/25, 3, 60) km.
pub const MD_DPLUS_M: (f64, f64) = (50.0, 10_000.0);
pub const MD_CAP_PER_KM: f64 = 25.0;
/// I2 : en plaine (relief de la dalle du départ < `MD_FLAT_RELIEF_M`, z max − z min du manifeste),
/// distance max par défaut X/15 : X/25 rendait `dplus_not_reached` (Massy X = 150 m : 6 km < 6,6).
pub const MD_FLAT_RELIEF_M: f64 = 600.0;
pub const MD_CAP_PER_KM_FLAT: f64 = 15.0;
pub const MD_CAP_KM: (f64, f64) = (3.0, 60.0);
/// Points de passage (T34, api.md v1.5) : au plus 5, accrochés à une voie à <= 150 m.
pub const VIA_MAX: usize = 5;
pub const VIA_SNAP_M: f64 = 150.0;
/// Type de voie (api.md v1.7) : natures BD TOPO comptées « chemin » ; les autres voies gardées
/// (`ROAD_NATURES`) sont « route ». La BD TOPO ne donne pas le revêtement : un « Chemin » de parc
/// peut être goudronné, une petite « Route à 1 chaussée » forestière ne pas l'être.
pub const TRAIL_NATURES: [&str; 4] = ["Sentier", "Chemin", "Route empierrée", "Escalier"];
pub const ROAD_NATURES: [&str; 5] = [
    "Piste cyclable",
    "Route à 1 chaussée",
    "Route à 2 chaussées",
    "Rond-point",
    "Bretelle",
];
/// Avertissement `low_surface_share` : part du type voulu sous ce seuil sur la première boucle.
pub const LOW_SURFACE_SHARE: f64 = 0.5;

/// Requête (champs de `pipeline.Params` utiles à la source IGN).
#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub lat: f64,
    pub lon: f64,
    /// Ignorée en mode min_distance.
    #[serde(default = "ten")]
    pub distance_km: f64,
    /// Mode min_distance : distance de sécurité Lcap (km), défaut clamp(X/25, 3, 60).
    #[serde(default)]
    pub max_distance_km: Option<f64>,
    /// Préférence de montées : "short" | "balanced" | "long" (modes.md B).
    #[serde(default = "balanced")]
    pub climbs: String,
    /// [[lon, lat], …] ; absent : disque centré sur le départ.
    #[serde(default)]
    pub polygon: Option<Vec<[f64; 2]>>,
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default)]
    pub target_dplus: Option<f64>,
    /// Fraction (0,35 = 35 %).
    #[serde(default)]
    pub max_grade: Option<f64>,
    #[serde(default)]
    pub time_s: Option<f64>,
    #[serde(default = "default_tol")]
    pub tol: f64,
    /// Type de voie préféré : "trail" (chemins) | "any" | "road" (routes). Jamais un filtre.
    #[serde(default = "default_surface")]
    pub surface: String,
    #[serde(default)]
    pub seed: u64,
    #[serde(default)]
    pub node_simple: bool,
    #[serde(default = "one")]
    pub n_candidates: usize,
    #[serde(default = "yes")]
    pub enforce_limits: bool,
    /// Plafond de calcul en temps réel (s), fixé par le serveur (D8) : au-delà, le solveur rend
    /// sa meilleure boucle et aucune autre tentative n'est lancée. Absent : pas de plafond.
    #[serde(default)]
    pub max_compute_s: Option<f64>,
    /// Points de passage obligatoires [[lat, lon], …] (ordre libre), au plus `VIA_MAX`.
    #[serde(default)]
    pub via: Vec<[f64; 2]>,
    /// Zone de dalles du départ (métropole par défaut) : posée par `resolve_cap`, jamais lue du JSON.
    #[serde(skip)]
    pub zone: crate::tiles::Zone,
}

fn ten() -> f64 {
    10.0
}
fn balanced() -> String {
    "balanced".into()
}

impl Request {
    pub fn min_distance(&self) -> bool {
        self.mode == "min_distance"
    }

    /// Lcap (km) du mode min_distance.
    pub fn cap_km(&self) -> f64 {
        let x = self.target_dplus.unwrap_or(0.0);
        self.max_distance_km
            .unwrap_or((x / MD_CAP_PER_KM).clamp(MD_CAP_KM.0, MD_CAP_KM.1))
    }

    /// Distance qui dimensionne zone et budget : Lcap en mode min_distance.
    pub fn sizing_km(&self) -> f64 {
        if self.min_distance() {
            self.cap_km()
        } else {
            self.distance_km
        }
    }

    /// γ de la préférence de montées, None si inconnue.
    pub fn gamma(&self) -> Option<i8> {
        match self.climbs.as_str() {
            "short" => Some(-1),
            "balanced" => Some(0),
            "long" => Some(1),
            _ => None,
        }
    }
}

fn default_mode() -> String {
    "max".into()
}
fn default_tol() -> f64 {
    0.05
}
fn default_surface() -> String {
    "trail".into()
}
fn one() -> usize {
    1
}
fn yes() -> bool {
    true
}

/// Budget conseillé (s de budget Python) : 20 s jusqu'à 10 km, 60 s à 100 km, ×(1 + 0,5 (n-1)).
pub fn suggested_time(distance_km: f64, n: usize) -> f64 {
    let t = 20.0 + (distance_km - 10.0).max(0.0) * 40.0 / 90.0;
    let t = (5.0 * (t / 5.0).round()).max(20.0) * (1.0 + 0.5 * (n.max(1) - 1) as f64);
    t.min(TIME_S.1)
}

fn err(code: Code, detail: &str) -> Msg {
    Msg::error(code, detail)
}

fn validate(r: &Request) -> Result<(), Msg> {
    let finite = [r.lat, r.lon, r.distance_km, r.tol]
        .iter()
        .all(|x| x.is_finite())
        && r.polygon.iter().flatten().flatten().all(|x| x.is_finite())
        && r.target_dplus.is_none_or(f64::is_finite)
        && r.max_grade.is_none_or(f64::is_finite)
        && r.time_s.is_none_or(f64::is_finite)
        && r.max_distance_km.is_none_or(f64::is_finite)
        && r.max_compute_s.is_none_or(f64::is_finite)
        && r.via.iter().flatten().all(|x| x.is_finite());
    if !finite || !(-90.0..=90.0).contains(&r.lat) || !(-180.0..=180.0).contains(&r.lon) {
        return Err(err(
            Code::InvalidRequest,
            "non-finite number or bad coordinates",
        ));
    }
    if !matches!(r.mode.as_str(), "max" | "target" | "min_distance") {
        return Err(err(Code::ModeUnknown, &r.mode));
    }
    if r.gamma().is_none() {
        return Err(err(Code::ClimbsUnknown, &r.climbs));
    }
    if !matches!(r.surface.as_str(), "trail" | "any" | "road") {
        return Err(err(Code::RoadsUnknown, &r.surface));
    }
    if r.mode != "max" && !r.target_dplus.is_some_and(|d| d > 0.0) {
        return Err(err(Code::TargetDplusRequired, ""));
    }
    if r.min_distance() && !(MD_DPLUS_M.0..=MD_DPLUS_M.1).contains(&r.target_dplus.unwrap()) {
        return Err(Msg::new(
            Code::DplusOutOfRange,
            json!({"min_m": MD_DPLUS_M.0, "max_m": MD_DPLUS_M.1}),
        ));
    }
    if !(r.tol > 0.0 && r.tol <= 0.5) {
        return Err(err(Code::ToleranceOutOfRange, ""));
    }
    if r.max_grade.is_some_and(|g| g <= 0.0) {
        return Err(err(Code::MaxGradeInvalid, ""));
    }
    if r.via.len() > VIA_MAX {
        return Err(err(Code::InvalidRequest, "at most 5 via points"));
    }
    if !(1..=4).contains(&r.n_candidates) {
        return Err(err(Code::InvalidRequest, "n_candidates must be 1..4"));
    }
    if r.polygon.as_ref().is_some_and(|p| p.len() > 1000) {
        return Err(err(Code::ZoneInvalid, "too many vertices"));
    }
    if !r.enforce_limits {
        return Ok(());
    }
    if !(DIST_KM.0..=DIST_KM.1).contains(&r.sizing_km()) {
        return Err(Msg::new(
            Code::DistanceOutOfRange,
            json!({"min_km": DIST_KM.0, "max_km": DIST_KM.1}),
        ));
    }
    let t = r.time_s.unwrap_or(0.0);
    if !(TIME_S.0..=TIME_S.1).contains(&t) {
        return Err(Msg::new(
            Code::TimeOutOfRange,
            json!({"min_s": TIME_S.0, "max_s": TIME_S.1}),
        ));
    }
    Ok(())
}

/// Rayon du disque par défaut : D/2, plafonné pour respecter MAX_AREA_KM2 (marge 1 %).
fn default_radius(distance_m: f64) -> f64 {
    (distance_m / 2.0).min((0.99 * MAX_AREA_KM2 * 1e6 / std::f64::consts::PI).sqrt())
}

/// Polygone local effectif (polygone dessiné réparé, ou disque), coupé au disque Lmax/2.
fn build_region(r: &Request, frame: &Frame, lmax: f64, l: f64) -> Result<Region, Msg> {
    let reach = MultiPolygon::new(vec![prep::disk(lmax / 2.0, 128)]);
    let Some(poly) = &r.polygon else {
        let d = MultiPolygon::new(vec![prep::disk(default_radius(l), 64)]);
        return Ok(Region::new(d.intersection(&reach)));
    };
    if poly.len() < 3 {
        return Err(err(Code::ZoneInvalid, "fewer than 3 vertices"));
    }
    let ring: Vec<Coord<f64>> = poly
        .iter()
        .map(|&[lon, lat]| {
            let p = frame.to_local(lon, lat);
            Coord { x: p[0], y: p[1] }
        })
        .collect();
    // Anneau auto-intersecté (nœud papillon…) : refusé, pas réparé (revue T25).
    let poly = Polygon::new(LineString::new(ring), vec![]);
    if !poly.is_valid() {
        return Err(err(Code::ZoneInvalid, "self-intersecting polygon"));
    }
    let user = MultiPolygon::new(vec![poly]);
    let user_region = Region::new(user.clone());
    if user_region.area() <= 0.0 {
        return Err(err(Code::ZoneInvalid, "degenerate polygon"));
    }
    if !user_region.contains([0.0, 0.0]) && user_region.boundary_dist([0.0, 0.0]) > START_BUFFER_M {
        return Err(err(Code::StartOutsideZone, ""));
    }
    Ok(Region::new(user.intersection(&reach)))
}

/// Point de passage accroché : nœud de l'arène, position demandée (repère local), distance d'accroche.
#[derive(Clone, Copy)]
pub struct ViaPt {
    pub node: usize,
    pub q: [f64; 2],
    pub snap: f64,
}

/// (L, Lmax) en m : distance qui dimensionne la zone et longueur max de boucle.
fn lengths(r: &Request) -> (f64, f64) {
    let l = r.sizing_km() * 1000.0;
    let lmax = match r.mode.as_str() {
        "max" => l * (1.0 + r.tol),
        "min_distance" => l,
        _ => 1.3 * l,
    };
    (l, lmax)
}

/// Une requête ne mélange pas deux zones de dalles (métropole, DOM) : sommet du polygone dans une
/// autre zone = `zone_invalid`, point de passage = `via_too_far` (`TileStore::near`).
/// `r` : requête de `resolve_cap`.
pub fn check_zone(store: &TileStore, r: &Request) -> Result<(), Msg> {
    let other = |lat: f64, lon: f64| !store.near(&r.zone, lat, lon);
    if r.polygon
        .iter()
        .flatten()
        .any(|&[lon, lat]| other(lat, lon))
    {
        return Err(err(
            Code::ZoneInvalid,
            "polygon vertex in another tile zone",
        ));
    }
    if let Some(i) = r.via.iter().position(|&[lat, lon]| other(lat, lon)) {
        let max_km = round1(default_radius(lengths(r).0) / 1000.0);
        return Err(Msg::new(
            Code::ViaTooFar,
            json!({"n": i + 1, "max_km": max_km}),
        ));
    }
    Ok(())
}

/// Points de passage hors de portée (`via_too_far`) ou hors du polygone (`via_outside_zone`) :
/// géométrie seule, avant tout calcul (et avant Turnstile côté API).
pub fn check_via(r: &Request) -> Result<(), Msg> {
    if r.via.is_empty() {
        return Ok(());
    }
    let frame = Frame::new_in(r.zone.proj, r.lat, r.lon);
    let (l, lmax) = lengths(r);
    let region = build_region(r, &frame, lmax, l)?;
    for (i, &[lat, lon]) in r.via.iter().enumerate() {
        let q = frame.to_local(lon, lat);
        if region.contains(q) || region.boundary_dist(q) <= START_BUFFER_M {
            continue;
        }
        let reach = (q[0].hypot(q[1]) > region.radius()).then_some(());
        return Err(match (&r.polygon, reach) {
            (Some(_), None) => Msg::new(Code::ViaOutsideZone, json!({"n": i + 1})),
            _ => Msg::new(
                Code::ViaTooFar,
                json!({"n": i + 1, "max_km": round1(region.radius() / 1000.0)}),
            ),
        });
    }
    Ok(())
}

/// Codes de nature (manifeste) → tronçons gardés : toutes les voies praticables à pied (port de
/// `ign.keep_mask`, type « all »), quel que soit le type de voie préféré. Exclus : non praticables
/// (privé, ayants droit, hors service), « Type autoroutier », « Bac ou liaison maritime », nature inconnue.
pub fn keep_mask(t: &Troncons, natures: &[String]) -> Vec<usize> {
    let kept = |n: u8| {
        natures.get(n as usize).is_some_and(|x| {
            TRAIL_NATURES.contains(&x.as_str()) || ROAD_NATURES.contains(&x.as_str())
        })
    };
    (0..t.len())
        .filter(|&i| t.flags[i] & 2 != 0 && kept(t.nature[i]) && t.n_vertices(i) >= 2)
        .collect()
}

/// Par tronçon : « chemin » (`TRAIL_NATURES`) ; sinon « route ».
pub fn trail_mask(t: &Troncons, natures: &[String]) -> Vec<bool> {
    let trail = |n: u8| {
        natures
            .get(n as usize)
            .is_some_and(|x| TRAIL_NATURES.contains(&x.as_str()))
    };
    t.nature.iter().map(|&n| trail(n)).collect()
}

/// Longueur (m) de l'arête hors du type de voie voulu (`Request::surface`).
fn off_type(ed: &Edge, surface: &str) -> f64 {
    match surface {
        "trail" => (ed.len - ed.trail).max(0.0),
        "road" => ed.trail,
        _ => 0.0,
    }
}

/// Aller-retour d'accès : chemin, sur toutes les voies, du point cliqué vers le réseau.
pub struct AccessPath {
    pub length: f64,
    /// Longueur sur « chemin ».
    pub trail: f64,
    pub xy: Vec<[f64; 2]>,
    pub z: Vec<f64>,
    pub updown: f64,
    pub snap: f64,
}

/// Réseau d'accès (toutes les voies de la zone), construit à la demande (port de `Access`).
pub struct Access<'a> {
    net: Net<'a>,
    ids: Vec<usize>,
    s0: usize,
    snap0: f64,
    dist: Vec<f64>,
    prev: Vec<Option<(usize, usize)>>,
}

impl<'a> Access<'a> {
    pub fn new(
        t: &'a Troncons,
        frame: Frame,
        sel: &[usize],
        trail_t: &[bool],
        region: &Region,
    ) -> Option<Access<'a>> {
        let mut net = Net::new(t, frame);
        net.trail_t = trail_t.to_vec();
        let mut ids = net.build(sel, region);
        let (s0, snap0, _) = net.insert_start(&mut ids, [0.0, 0.0])?;
        if snap0 > ACCESS_MAX_START_M {
            return None;
        }
        let adj = prep::Adj::new(&net, &ids);
        let (dist, prev) = prep::shortest(&adj, |e| net.edges[e].len, s0, None);
        Some(Access {
            net,
            ids,
            s0,
            snap0,
            dist,
            prev,
        })
    }

    fn reached(&self, key: i64) -> Option<usize> {
        let n = *self.net.key_of.get(&key)?;
        (n < self.dist.len() && self.dist[n].is_finite()).then_some(n)
    }

    /// Parmi les nœuds (clés) du réseau bouclable, le plus proche du point cliqué par la route.
    pub fn best_entry(&self, keys: &[i64]) -> Option<i64> {
        keys.iter()
            .filter_map(|&k| {
                self.reached(k)
                    .filter(|&n| n != self.s0)
                    .map(|n| (self.dist[n], k))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|x| x.1)
    }

    /// Chemin d'accès jusqu'au nœud de clé `key` (position `pos`), ou None.
    pub fn path_to(&mut self, key: i64, pos: [f64; 2]) -> Option<AccessPath> {
        let path = if let Some(t) = self.reached(key) {
            if t == self.s0 {
                return None;
            }
            prep::path_from(&self.prev, self.s0, t)
        } else {
            // `pos` n'est pas un nœud du réseau d'accès : on l'insère et on recalcule.
            let mut ids = self.ids.clone();
            let (t, snap_t, _) = self.net.insert_start(&mut ids, pos)?;
            if snap_t > 10.0 {
                return None;
            }
            let adj = prep::Adj::new(&self.net, &ids);
            let (dist, prev) = prep::shortest(&adj, |e| self.net.edges[e].len, self.s0, Some(t));
            if t == self.s0 || !dist[t].is_finite() {
                return None;
            }
            prep::path_from(&prev, self.s0, t)
        };
        let (xy, z) = route_geometry(&self.net, &path);
        let length = path.iter().map(|&(e, _, _)| self.net.edges[e].len).sum();
        let trail = path.iter().map(|&(e, _, _)| self.net.edges[e].trail).sum();
        let updown = z.windows(2).map(|w| (w[1] - w[0]).abs()).sum();
        Some(AccessPath {
            length,
            trail,
            xy,
            z,
            updown,
            snap: self.snap0,
        })
    }
}

/// Polyligne (xy, z) d'un parcours [(arête de l'arène, de, vers)].
pub fn route_geometry(net: &Net, steps: &[(usize, usize, usize)]) -> (Vec<[f64; 2]>, Vec<f64>) {
    let (mut xy, mut z) = (Vec::new(), Vec::new());
    for &(e, a, _) in steps {
        let ed = &net.edges[e];
        let (mut pxy, mut pz) = net.edge_points(e);
        if ed.u != ed.v && a != ed.u {
            pxy.reverse();
            pz.reverse();
        }
        let skip = usize::from(!xy.is_empty());
        xy.extend_from_slice(&pxy[skip..]);
        z.extend_from_slice(&pz[skip..]);
    }
    (xy, z)
}

/// Graphe de boucles prêt pour le solveur.
pub struct Candidate {
    pub ids: Vec<usize>,
    pub s: usize,
    pub access: Option<AccessPath>,
    /// Mode min_distance : borne inférieure de la longueur de boucle (sans l'accès), calculée
    /// avant la réduction aux arêtes pentues (heuristique) : promesse affichable.
    pub lb: Option<f64>,
}

/// Port de `build_candidate` : départ (accès éventuel), copies d'accès près du départ,
/// élagage, pente max, borne inférieure (min_distance), réduction aux arêtes pentues. Err si aucune boucle >= Lmin
/// possible, avec les arêtes du réseau trop court autour de ce départ (vide pour un autre motif).
/// `x` : D+ visé du mode min_distance ; None (statut `dplus_unreachable`) si la borne
/// inférieure de distance dépasse Lmax. `weight` : poids de recherche d'une arête (réduction).
#[allow(clippy::too_many_arguments)]
pub fn build_candidate(
    net: &mut Net,
    sub: &[usize],
    mut lmin: f64,
    mut lmax: f64,
    max_grade: Option<f64>,
    access: Option<&mut Access>,
    reduce: bool,
    x: Option<f64>,
    via: &mut [ViaPt],
    weight: Option<&dyn Fn(&Edge) -> f64>,
    info: &mut serde_json::Map<String, Value>,
) -> Result<Candidate, Vec<usize>> {
    let mut ids = sub.to_vec();
    let nearest = net.nearest_vertex(&ids);
    let use_access = access.is_some() && nearest > ACCESS_MIN_M;
    let mut point = [0.0, 0.0];
    let mut by_road = false;
    if use_access {
        let mut keys: Vec<i64> = ids
            .iter()
            .flat_map(|&e| [net.key[net.edges[e].u], net.key[net.edges[e].v]])
            .collect();
        keys.sort_unstable();
        keys.dedup();
        if let Some(k) = access.as_deref().and_then(|a| a.best_entry(&keys)) {
            point = net.xy[net.key_of[&k]];
            by_road = true;
        }
    }
    let (mut s, _, mut pos) = net.insert_start(&mut ids, point).ok_or(Vec::new())?;
    info.insert("start_snap_m".into(), json!(round1(pos[0].hypot(pos[1]))));
    let mut acc = None;
    if use_access {
        let a = access.and_then(|a| a.path_to(net.key[s], pos));
        match a {
            // (min_distance : Lmin = 0, la part se mesure sur Lcap / 2)
            Some(a) if 2.0 * a.length <= ACCESS_MAX_SHARE * lmin.max(0.5 * lmax) => {
                lmin -= 2.0 * a.length;
                lmax -= 2.0 * a.length;
                info.insert("access_m".into(), json!(a.length.round()));
                acc = Some(a);
            }
            _ if by_road => {
                // accès inutilisable : départ au plus proche à vol d'oiseau
                ids = sub.to_vec();
                let (s2, snap, p2) = net.insert_start(&mut ids, [0.0, 0.0]).ok_or(Vec::new())?;
                (s, pos) = (s2, p2);
                info.insert("start_snap_m".into(), json!(round1(snap)));
            }
            _ => {}
        }
    }
    let doubled = net.duplicate_near_start(&mut ids, pos, s, FREE_RADIUS);
    info.insert("edges_doubled_near_start".into(), json!(doubled));
    let mut ids = prep::prune(net, ids, s, lmax);
    info.insert("edges_pruned".into(), json!(ids.len()));
    let (pl, pw): (Vec<f64>, Vec<f64>) = ids
        .iter()
        .map(|&e| (net.edges[e].len, net.edges[e].w))
        .unzip();
    info.insert(
        "pruned".into(),
        json!({"edges": ids.len(), "sum_len_m": pl.iter().sum::<f64>(), "sum_w_m": pw.iter().sum::<f64>(),
               "dplus_upper_bound_m": crate::problem::knapsack_ub(&pl, &pw, lmax)}),
    );
    let total = net.total_len(&ids);
    info.insert("network_km".into(), json!((total / 10.0).round() / 100.0));
    if total < lmin {
        info.insert("status".into(), json!("network_too_short"));
        return Err(ids);
    }
    if let Some(g) = max_grade {
        ids.retain(|&e| net.edges[e].grade <= g);
        ids = prep::prune(net, ids, s, lmax);
        if net.total_len(&ids) < lmin {
            info.insert("status".into(), json!("too_short_after_grade_filter"));
            return Err(ids);
        }
    }
    // point de passage sur une impasse ou une voie retirée (pente) : ré-accroché au réseau restant
    for (k, v) in via.iter_mut().enumerate() {
        let touched = |ids: &[usize], n: usize| {
            ids.iter()
                .any(|&e| net.edges[e].u == n || net.edges[e].v == n)
        };
        if touched(&ids, v.node) {
            continue;
        }
        match net.insert_start(&mut ids, v.q) {
            Some((n, snap, _)) if snap <= VIA_SNAP_M => (v.node, v.snap) = (n, snap),
            _ => {
                info.insert("status".into(), json!("via_unreachable"));
                info.insert("via_n".into(), json!(k + 1));
                return Err(Vec::new());
            }
        }
    }
    let mut lb = None;
    if let Some(x) = x {
        // Borne prouvée (promesse affichable) : APRÈS le filtre de pente (contrainte utilisateur,
        // reach mesuré sur le graphe filtré) et AVANT la réduction aux arêtes pentues (heuristique).
        let x = (x - acc.as_ref().map_or(0.0, |a| a.updown)).max(1.0);
        let reach = prep::reach(net, &ids, s);
        let pick = |f: &dyn Fn(usize) -> f64| ids.iter().map(|&e| f(e)).collect::<Vec<f64>>();
        let b = crate::problem::length_lower_bound(
            &pick(&|e| net.edges[e].len),
            &pick(&|e| net.edges[e].w),
            &reach,
            x,
        );
        info.insert("lower_bound_m".into(), json!(b.map(f64::round)));
        if b.is_none_or(|b| b > lmax) {
            info.insert("status".into(), json!("dplus_unreachable"));
            return Err(ids);
        }
        lb = b;
    }
    if reduce && ids.len() > REDUCE_MIN_EDGES && net.total_len(&ids) > 1.25 * REDUCE_K * lmax {
        let keep = prep::steep_reduction(
            net,
            &ids,
            s,
            lmax,
            REDUCE_K,
            &via.iter().map(|v| v.node).collect::<Vec<_>>(),
            weight,
        );
        // La réduction n'est qu'une heuristique de vitesse : si elle laisse un réseau plus court que
        // la boucle demandée (départ resté sur une tige), on garde le réseau complet.
        let reduced = prep::prune(net, keep, s, lmax);
        info.insert("edges_reduced".into(), json!(reduced.len()));
        if net.total_len(&reduced) >= lmax.max(lmin) {
            ids = reduced;
        }
    }
    info.insert("edges_after_grade".into(), json!(ids.len()));
    // point de passage sans arête restante (impasse, pente, hors réseau du départ) : infaisable
    if let Some(k) = via.iter().position(|v| {
        !ids.iter()
            .any(|&e| net.edges[e].u == v.node || net.edges[e].v == v.node)
    }) {
        info.insert("status".into(), json!("via_unreachable"));
        info.insert("via_n".into(), json!(k + 1));
        return Err(Vec::new());
    }
    Ok(Candidate {
        ids,
        s,
        access: acc,
        lb,
    })
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// Instance du solveur (contrat problem.md, mêmes champs) depuis un sous-graphe de l'arène.
/// Renvoie aussi, pour chaque nœud du problème, son nœud dans l'arène.
#[allow(clippy::too_many_arguments)]
pub fn to_problem(
    net: &Net,
    ids: &[usize],
    s: usize,
    l: f64,
    mode: &str,
    tol: f64,
    d: Option<f64>,
    node_simple: bool,
    climbs: i8,
    via: &[usize],
) -> (Problem, Vec<usize>) {
    let mut idx = vec![usize::MAX; net.xy.len()];
    let mut nodes = Vec::new();
    for &e in ids {
        for n in [net.edges[e].u, net.edges[e].v] {
            if idx[n] == usize::MAX {
                idx[n] = nodes.len();
                nodes.push(n);
            }
        }
    }
    let (lo, hi) = match mode {
        "max" => (1.0 - tol, 1.0 + tol),
        "min_distance" => (0.0, 1.0), // l = Lcap ; L indicatif fixé par l'appelant
        _ => (0.7, 1.3),
    };
    let c = net.xy[s];
    // « longues » (T24) : coût par montée, mode max seulement (cible et min_distance : sens seul)
    let (turn, inner) = if climbs > 0 && mode == "max" {
        ids.iter().map(|&e| edge_turns(net, e)).unzip()
    } else {
        (Vec::new(), Vec::new())
    };
    let p = Problem {
        version: crate::problem::VERSION,
        mode: mode.into(),
        l,
        lmin: l * lo,
        lmax: l * hi,
        d,
        s: idx[s],
        node_simple,
        xy: nodes.iter().map(|&n| net.xy[n]).collect(),
        far: (0..nodes.len())
            .filter(|&i| {
                let q = net.xy[nodes[i]];
                (q[0] - c[0]).hypot(q[1] - c[1]) > FREE_RADIUS
            })
            .collect(),
        u: ids.iter().map(|&e| idx[net.edges[e].u]).collect(),
        v: ids.iter().map(|&e| idx[net.edges[e].v]).collect(),
        len: ids.iter().map(|&e| net.edges[e].len).collect(),
        w: ids.iter().map(|&e| net.edges[e].w).collect(),
        ang_u: ids
            .iter()
            .map(|&e| prep::edge_angle(net, e, true))
            .collect(),
        ang_v: ids
            .iter()
            .map(|&e| prep::edge_angle(net, e, false))
            .collect(),
        parallel: {
            let mut par = prep::parallel_pairs(net, ids, c);
            if node_simple {
                // D34 : jamais deux fois au même endroit, même à deux niveaux (pont, tunnel)
                par.extend(prep::crossing_pairs(net, ids, c));
                par.sort_unstable();
                par.dedup();
            }
            par
        },
        q: if climbs != 0 {
            ids.iter().map(|&e| edge_q(net, e)).collect()
        } else {
            Vec::new()
        },
        climbs,
        turn,
        inner,
        node_mu: 0.0,
        via: via.iter().map(|&n| idx[n]).collect(),
        off: Vec::new(),
        off_price: 0.0,
    };
    (p, nodes)
}

/// T24 (modes.md v0.3) : (signe de la pente en partant de u et de v, ½ nombre d'extrema
/// intérieurs), même tampon que `climbs::scan`. Symétrique : l'inverse échange les signes.
fn edge_turns(net: &Net, e: usize) -> ([i8; 2], f64) {
    use crate::climbs::{OPEN_M, tol};
    let (_, z) = net.edge_points(e);
    let (mut dir, mut first, mut n) = (0i8, 0i8, 0u32);
    let (mut lo, mut ext) = (z[0], z[0]); // dernier creux, sommet courant (ou extrême du palier)
    for &zi in &z[1..] {
        match dir {
            0 => {
                if zi - lo >= OPEN_M {
                    (dir, first, ext) = (1, 1, zi);
                } else if ext - zi >= OPEN_M {
                    (dir, first, lo) = (-1, -1, zi);
                }
                lo = lo.min(zi);
                ext = ext.max(zi);
            }
            1 if zi > ext => ext = zi,
            1 if ext - zi >= tol(ext - lo) => (dir, lo, n) = (-1, zi, n + 1),
            -1 if zi < lo => lo = zi,
            -1 if zi - lo >= OPEN_M => (dir, ext, n) = (1, zi, n + 1),
            _ => {}
        }
    }
    ([first, -dir], 0.5 * n as f64)
}

/// q d'une arête (modes.md B) : ½ (Σ G² des montées dans un sens + dans l'autre).
fn edge_q(net: &Net, e: usize) -> f64 {
    let (_, mut z) = net.edge_points(e);
    let s = vec![0.0; z.len()]; // longueurs inutiles ici
    let sq = |z: &[f64]| climbs(z, &s).iter().map(|c| c.0 * c.0).sum::<f64>();
    let f = sq(&z);
    z.reverse();
    0.5 * (f + sq(&z))
}

/// Abscisses cumulées d'une polyligne.
fn abscissa(xy: &[[f64; 2]]) -> Vec<f64> {
    let mut s = 0.0;
    std::iter::once(0.0)
        .chain(xy.windows(2).map(|p| {
            s += (p[1][0] - p[0][0]).hypot(p[1][1] - p[0][1]);
            s
        }))
        .collect()
}

/// Ḡ = Σ G² / Σ G des montées du profil (taille de la montée typique).
fn gbar(xy: &[[f64; 2]], z: &[f64]) -> f64 {
    let c = climbs(z, &abscissa(xy));
    let (g, g2) = c
        .iter()
        .fold((0.0, 0.0), |a, x| (a.0 + x.0, a.1 + x.0 * x.0));
    if g > 0.0 { g2 / g } else { 0.0 }
}

/// Une boucle assemblée (accès éventuel compris).
pub struct Track {
    pub xy: Vec<[f64; 2]>,
    pub z: Vec<f64>,
    pub length: f64,
    pub dplus: f64,
    /// Longueur sur « chemin » (accès compris).
    pub trail: f64,
    pub feasible: bool,
}

/// Port de `assemble` : la boucle, précédée et suivie de l'aller-retour d'accès. Préférence de
/// montées γ ≠ 0 : sens de parcours de plus grand Ḡ (longues) ou de plus petit (courtes).
fn assemble(
    net: &Net,
    ids: &[usize],
    p: &Problem,
    loop_ids: &[usize],
    acc: Option<&AccessPath>,
    gamma: i8,
) -> Result<Track, Msg> {
    let circuit = p
        .euler(loop_ids)
        .map_err(|e| err(Code::InvariantViolated, &e))?;
    // nœuds du problème -> nœuds de l'arène : u ou v de l'arête
    let arena_steps: Vec<(usize, usize, usize)> = circuit
        .iter()
        .map(|&(e, a, _)| {
            let ed = &net.edges[ids[e]];
            if p.u[e] == a {
                (ids[e], ed.u, ed.v)
            } else {
                (ids[e], ed.v, ed.u)
            }
        })
        .collect();
    let (mut xy, mut z) = route_geometry(net, &arena_steps);
    if gamma != 0 {
        let a = gbar(&xy, &z);
        xy.reverse();
        z.reverse();
        let b = gbar(&xy, &z);
        if (gamma > 0) == (a >= b) {
            xy.reverse();
            z.reverse();
        }
    }
    let (mut length, mut dplus) = p.stats(loop_ids);
    let mut trail: f64 = loop_ids.iter().map(|&e| net.edges[ids[e]].trail).sum();
    if let Some(a) = acc {
        trail += 2.0 * a.trail;
        let mut az = a.z.clone();
        *az.last_mut().unwrap() = z[0];
        let rev_xy: Vec<[f64; 2]> = a.xy.iter().rev().skip(1).copied().collect();
        let rev_z: Vec<f64> = az.iter().rev().skip(1).copied().collect();
        xy =
            a.xy.iter()
                .copied()
                .chain(xy.into_iter().skip(1))
                .chain(rev_xy)
                .collect();
        z = az
            .iter()
            .copied()
            .chain(z.into_iter().skip(1))
            .chain(rev_z)
            .collect();
        length += 2.0 * a.length;
        dplus += az.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f64>();
    }
    let feasible = p.score(loop_ids).3;
    Ok(Track {
        xy,
        z,
        length,
        dplus,
        trail,
        feasible,
    })
}

/// Mode min_distance sans distance max : défaut clamp(X / k, 3, 60) km, k = 25 m/km en relief,
/// `MD_CAP_PER_KM_FLAT` en plaine (relief de la dalle du départ, I2). Pose aussi la zone de
/// dalles du départ (`Request::zone`). Sinon, requête inchangée.
pub fn resolve_cap(store: &TileStore, req: &Request) -> Request {
    let mut r = req.clone();
    r.zone = store.zone(r.lat, r.lon);
    if r.min_distance() && r.max_distance_km.is_none() && (-90.0..=90.0).contains(&r.lat) {
        let relief = store
            .tile_in(&r.zone, r.lat, r.lon)
            .and_then(|t| Some(t["z_max_m"].as_f64()? - t["z_min_m"].as_f64()?));
        let k = if relief.is_some_and(|z| z < MD_FLAT_RELIEF_M) {
            MD_CAP_PER_KM_FLAT
        } else {
            MD_CAP_PER_KM
        };
        r.max_distance_km =
            Some((r.target_dplus.unwrap_or(0.0) / k).clamp(MD_CAP_KM.0, MD_CAP_KM.1));
    }
    r
}

/// I2 (modes.md v0.2) : min_distance dont la boucle dépasse X de plus de `MD_POLISH_OVER`.
/// La plus courte boucle trouvée est sur un palier du front (D+, L) (Bourg X = 500 : 11,57 km
/// pour 686 m alors que 11,57 km pour ~500 m existe) : le graphe réduit aux arêtes pentues n'a
/// plus de quoi tenir X. Seconde recherche en mode cible (1,03·L*, 1,03·X) sur la même requête
/// (le mode cible vise L au plus près : à L* exact il rend 493 m < X) ; gardée si D+ >= X et
/// L <= L*·(1 + MD_POLISH_LEN). Sinon la boucle reste. Bourg 500 : 11,57/686 → 11,93 km/500 m.
const MD_POLISH_OVER: f64 = 0.10;
const MD_POLISH_LEN: f64 = 0.05;
fn polish_min_distance(
    run: impl FnOnce(&Request) -> Result<Value, Msg>,
    req: &Request,
    out: &mut Value,
) {
    let Some(x) = req.target_dplus.filter(|_| req.min_distance()) else {
        return;
    };
    let c0 = &out["candidates"][0];
    let (Some(l), Some(d)) = (c0["length_m"].as_f64(), c0["dplus_m"].as_f64()) else {
        return;
    };
    let spent = out["compute_s"].as_f64().unwrap_or(0.0);
    if d <= x * (1.0 + MD_POLISH_OVER) || c0["feasible"] != json!(true) {
        return;
    }
    if req.max_compute_s.is_some_and(|c| c - spent < MIN_ATTEMPT_S) {
        return;
    }
    let mut r = req.clone();
    r.mode = "target".into();
    r.distance_km = 1.03 * l / 1000.0;
    r.target_dplus = Some(x * 1.03);
    r.max_distance_km = None;
    r.n_candidates = 1;
    r.time_s = None;
    r.max_compute_s = req.max_compute_s.map(|c| c - spent);
    let Ok(o) = run(&r) else {
        return;
    };
    let c = &o["candidates"][0];
    let (Some(l2), Some(d2)) = (c["length_m"].as_f64(), c["dplus_m"].as_f64()) else {
        return;
    };
    if d2 >= x && l2 <= l * (1.0 + MD_POLISH_LEN) {
        let mut c = c.clone();
        if let Some(lb) = out["lower_bound_m"].as_f64() {
            c["target_gap"] = json!({"distance_m": round1(l2 - lb), "dplus_m": round1(d2 - x)});
        }
        out["candidates"][0] = c;
    }
    out["compute_s"] = json!(spent + o["compute_s"].as_f64().unwrap_or(0.0));
}

/// T34 : min_distance avec distance max PAR DÉFAUT et `dplus_not_reached` : la distance max
/// X/k est trop courte quand le départ est en plaine au pied du relief (Grenoble sud, X = 300 :
/// 12 km par défaut, boucle de 13,9 km). Un second essai avec 1,5 × la distance max (≤ 60 km),
/// gardé s'il atteint X. La requête retenue (cap élargi) sert à la suite (polish).
const MD_WIDEN: f64 = 1.5;
fn widen_cap(run: impl FnOnce(&Request) -> Result<Value, Msg>, req: &mut Request, out: &mut Value) {
    let spent = out["compute_s"].as_f64().unwrap_or(0.0);
    let cap = req.cap_km();
    if !req.min_distance()
        || out["candidates"][0]["feasible"] != json!(false)
        || cap >= MD_CAP_KM.1
        || req.max_compute_s.is_some_and(|c| c - spent < MIN_ATTEMPT_S)
    {
        return;
    }
    let t = Instant::now();
    let mut r = req.clone();
    r.max_distance_km = Some((MD_WIDEN * cap).min(MD_CAP_KM.1));
    r.max_compute_s = req.max_compute_s.map(|c| c - spent);
    if let Ok(o) = run(&r)
        && o["candidates"][0]["feasible"] == json!(true)
    {
        *out = o;
        req.max_distance_km = r.max_distance_km;
    }
    out["compute_s"] = json!(spent + t.elapsed().as_secs_f64());
}

/// D40/D46 « messages importants » (api.md v1.6) : demande non atteinte (`target_not_reached`) ou
/// moins de boucles que demandé (`fewer_loops`). `plan` ne rend que le message (`params`, sans
/// `suggest` ni `checked`, aucun calcul en plus) ; la CONTRAINTE LIMITANTE vient de `diagnose`
/// (`diagnose=1`) : pour chaque contrainte active (pente max, zone, carrefours
/// uniques, points de passage, distance max fournie) un calcul SANS elle seule (n = demandé,
/// budget = ce qu'il reste de `DIAG_TOTAL_S`, ≤ `DIAG_MAX_S`) ; la première qui atteint la cible
/// (ou rend assez de boucles) est proposée en `suggest` (réglages d'API à changer).
const DIAG_TOL: f64 = 0.10;
const DIAG_MAX_S: f64 = 6.0;
const DIAG_TOTAL_S: f64 = 15.0;
fn reached(req: &Request, o: &Value) -> bool {
    let c = &o["candidates"][0];
    let (Some(l), Some(d)) = (c["length_m"].as_f64(), c["dplus_m"].as_f64()) else {
        return false;
    };
    match req.mode.as_str() {
        "min_distance" => c["feasible"] == json!(true),
        "target" => {
            let (dt, lt) = (req.target_dplus.unwrap_or(d), req.distance_km * 1000.0);
            ((d - dt) / dt).abs() <= DIAG_TOL && ((l - lt) / lt).abs() <= DIAG_TOL
        }
        _ => !o["warnings"].as_array().is_some_and(|w| {
            w.iter()
                .any(|x| x["code"] == json!(Code::DistanceOutOfTolerance))
        }),
    }
}

fn important(req: &Request, out: &mut Value) {
    let asked = req.n_candidates;
    let got = out["candidates"].as_array().map_or(0, Vec::len);
    let c = &out["candidates"][0];
    let (l, d) = (c["length_m"].as_f64().unwrap_or(0.0), c["dplus_m"].as_f64());
    let has_x = req.mode != "max";
    let mut msgs = Vec::new();
    if !reached(req, out) {
        msgs.push(Msg::new(
            Code::TargetNotReached,
            json!({"dplus_m": req.target_dplus.filter(|_| has_x).map(f64::round),
                   "best_dplus_m": d.filter(|_| has_x).map(f64::round),
                   "km": round1(req.sizing_km()), "best_km": round1(l / 1000.0)}),
        ));
    }
    if got < asked {
        msgs.push(Msg::new(
            Code::FewerLoops,
            json!({"asked": asked, "got": got}),
        ));
    }
    if let Some(w) = out["warnings"].as_array_mut() {
        w.extend(msgs.iter().map(|m| json!(m)));
    }
}

/// `diagnose=1` (D46) : `{suggest?, fewer_suggest?, checked, params}`. `suggest` : réglage qui fait
/// atteindre la cible (pour `target_not_reached`), `fewer_suggest` : réglage qui rend assez de
/// boucles (pour `fewer_loops`) ; l'appelant n'utilise que celui du message affiché. `checked` :
/// `true` si toutes les contraintes actives ont été essayées (sans `suggest` : aucune ne suffit seule),
/// `false` si le budget est épuisé avant. `params` : `{dplus_m, km, asked}` demandés.
/// `req` : requête brute (distance max non résolue), comme celle de `plan`.
pub fn diagnose(store: &TileStore, req: &Request) -> Value {
    let auto_cap = req.max_distance_km.is_none();
    let req = resolve_cap(store, req);
    let run = |r: &Request| {
        plan_with(
            |b| store.load_in(&r.zone, b),
            &store.manifest.natures,
            r,
            false,
        )
    };
    diagnose_with(run, &req, auto_cap)
}

fn diagnose_with(
    run: impl Fn(&Request) -> Result<Value, Msg>,
    req: &Request,
    auto_cap: bool,
) -> Value {
    let asked = req.n_candidates;
    let mut relax: Vec<(Request, Value)> = Vec::new();
    let with = |f: &dyn Fn(&mut Request), s: Value| {
        let mut r = req.clone();
        f(&mut r);
        (r, s)
    };
    if req.max_grade.is_some() {
        relax.push(with(&|r| r.max_grade = None, json!({"max_grade_pct": 0})));
    }
    if req.polygon.is_some() {
        relax.push(with(&|r| r.polygon = None, json!({"polygon": null})));
    }
    if req.node_simple {
        relax.push(with(
            &|r| r.node_simple = false,
            json!({"no_repeat_junction": false}),
        ));
    }
    if !req.via.is_empty() {
        relax.push(with(&|r| r.via.clear(), json!({"via": null})));
    }
    let cap = req.cap_km();
    if req.min_distance() && !auto_cap && cap < MD_CAP_KM.1 {
        let v = (2.0 * cap).min(MD_CAP_KM.1);
        relax.push(with(
            &|r| r.max_distance_km = Some(v),
            json!({"max_distance_km": v}),
        ));
    }
    let mut out = json!({"params": {
        "dplus_m": req.target_dplus.filter(|_| req.mode != "max").map(f64::round),
        "km": round1(req.sizing_km()), "asked": asked}});
    let (mut nr, mut fl) = (None, None);
    let (t0, mut checked) = (Instant::now(), true);
    for (mut r, sug) in relax {
        let left = DIAG_TOTAL_S - t0.elapsed().as_secs_f64();
        if left < MIN_ATTEMPT_S {
            checked = false;
            break;
        }
        r.n_candidates = asked;
        r.max_compute_s = Some(left.min(DIAG_MAX_S));
        let Ok(o) = run(&r) else { continue };
        if nr.is_none() && reached(&r, &o) {
            nr = Some(sug.clone());
        }
        if fl.is_none() && o["candidates"].as_array().map_or(0, Vec::len) >= asked {
            fl = Some(sug);
        }
        if nr.is_some() && fl.is_some() {
            break;
        }
    }
    out["checked"] = json!(checked);
    if let Some(v) = nr {
        out["suggest"] = v;
    }
    if let Some(v) = fl {
        out["fewer_suggest"] = v;
    }
    out
}

/// Point d'entrée : requête + dalles → JSON de sortie (ou erreur codée D14).
/// `prep_only` : s'arrête au premier Problem (parité de préparation, sans solveur).
pub fn plan(store: &TileStore, req: &Request, prep_only: bool) -> Result<Value, Msg> {
    let auto_cap = req.max_distance_km.is_none();
    let mut req = resolve_cap(store, req);
    check_zone(store, &req)?;
    let run = |r: &Request, prep_only: bool| {
        plan_with(
            |b| store.load_in(&r.zone, b),
            &store.manifest.natures,
            r,
            prep_only,
        )
    };
    let mut out = run(&req, prep_only)?;
    if !prep_only && auto_cap {
        widen_cap(|r| run(r, false), &mut req, &mut out);
    }
    if !prep_only {
        polish_min_distance(|r| run(r, false), &req, &mut out);
    }
    if !prep_only {
        important(&req, &mut out);
        // type de voie voulu minoritaire sur la première boucle (le réseau n'en offre pas plus)
        let share = out["candidates"][0]["trail_frac"]
            .as_f64()
            .map(|f| if req.surface == "road" { 1.0 - f } else { f })
            .filter(|&s| req.surface != "any" && s < LOW_SURFACE_SHARE);
        if let (Some(s), Some(w)) = (share, out["warnings"].as_array_mut()) {
            w.push(json!(Msg::new(
                Code::LowSurfaceShare,
                json!({"pct": (100.0 * s).round()}),
            )));
        }
    }
    out["data_version"] = json!(store.manifest.data_version);
    if let Some(cands) = out["candidates"].as_array_mut() {
        for c in cands {
            c["landmarks"] = landmarks(&store.pois, &req.zone, c);
        }
    }
    // zone en partie hors des dalles : réseau tronqué au bord de la couverture
    let edge = out["zone"]["geometry"]["coordinates"]
        .as_array()
        .is_some_and(|polys| {
            polys
                .iter()
                .flat_map(|p| p.as_array().into_iter().flatten())
                .flat_map(|r| r.as_array().into_iter().flatten())
                .any(|q| {
                    let (lon, lat) = (
                        q[0].as_f64().unwrap_or(f64::NAN),
                        q[1].as_f64().unwrap_or(f64::NAN),
                    );
                    store.tile_in(&req.zone, lat, lon).is_none()
                })
        });
    if edge && let Some(w) = out["warnings"].as_array_mut() {
        w.push(json!(Msg::new(Code::CoverageEdge, json!({}))));
    }
    Ok(out)
}

/// Repères traversés (api.md v1.5, D34) : col à <= `LANDMARK_COL_M` du tracé, pic/sommet à
/// <= `LANDMARK_SUMMIT_M` (point du toponyme imprécis) ; abscisse du point le plus proche de la
/// boucle, ordre de passage, un repère une fois. `c` : boucle de la réponse (lat, lon, dist) ;
/// `zone` : zone de dalles de la boucle (repères et projection de cette zone seulement).
pub const LANDMARK_COL_M: f64 = 50.0;
pub const LANDMARK_SUMMIT_M: f64 = 100.0;
pub fn landmarks(pois: &[crate::tiles::Poi], zone: &crate::tiles::Zone, c: &Value) -> Value {
    let col = |k: &str| -> Vec<f64> {
        c[k].as_array()
            .map_or(Vec::new(), |a| a.iter().filter_map(Value::as_f64).collect())
    };
    let (lat, lon, dist) = (col("lat"), col("lon"), col("dist"));
    if lat.len() < 2 || lon.len() != lat.len() || dist.len() != lat.len() {
        return json!([]);
    }
    let pts: Vec<(f64, f64)> = lat
        .iter()
        .zip(&lon)
        .map(|(&la, &lo)| zone.proj.forward(lo, la))
        .collect();
    let m = LANDMARK_SUMMIT_M;
    let (x0, x1) = pts
        .iter()
        .fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p.0), a.1.max(p.0)));
    let (y0, y1) = pts
        .iter()
        .fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p.1), a.1.max(p.1)));
    let mut found: Vec<(f64, Value)> = pois
        .iter()
        .filter(|poi| poi.zone.as_deref().unwrap_or("") == zone.name)
        .filter_map(|poi| {
            let (px, py) = (poi.x_dm as f64 / 10.0, poi.y_dm as f64 / 10.0);
            if px < x0 - m || px > x1 + m || py < y0 - m || py > y1 + m {
                return None;
            }
            // (distance au tracé, abscisse du point le plus proche)
            let (d, s) = pts
                .windows(2)
                .zip(dist.windows(2))
                .map(|(w, ds)| {
                    let (dx, dy) = (w[1].0 - w[0].0, w[1].1 - w[0].1);
                    let l2 = dx * dx + dy * dy;
                    let t = if l2 > 0.0 {
                        (((px - w[0].0) * dx + (py - w[0].1) * dy) / l2).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    let d = (px - w[0].0 - t * dx).hypot(py - w[0].1 - t * dy);
                    (d, ds[0] + t * (ds[1] - ds[0]))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0))?;
            let is_col = poi.nature == "Col";
            if d > if is_col {
                LANDMARK_COL_M
            } else {
                LANDMARK_SUMMIT_M
            } {
                return None;
            }
            let (lo, la) = zone.proj.inverse(px, py);
            Some((
                s,
                json!({"kind": if is_col { "col" } else { "summit" }, "name": poi.name,
                       "ele_m": poi.z_dm.map(|z| z as f64 / 10.0), "dist_m": round1(s),
                       "lat": r6(la), "lon": r6(lo)}),
            ))
        })
        .collect();
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    Value::Array(found.into_iter().map(|f| f.1).collect())
}

/// Comme `plan`, avec un chargeur de tronçons (boîte en m dans le repère de `req.zone`, L93 par défaut) : dalles ou données d'essai.
/// `natures` : table des codes de nature (manifeste).
pub fn plan_with(
    load: impl FnOnce([f64; 4]) -> Result<Troncons, String>,
    natures: &[String],
    req: &Request,
    prep_only: bool,
) -> Result<Value, Msg> {
    let t_all = Instant::now();
    let mut r = req.clone();
    if r.time_s.is_none() {
        r.time_s = Some(suggested_time(r.sizing_km(), r.n_candidates));
    }
    validate(&r)?;
    let time_s = r.time_s.unwrap();
    let mut warns: Vec<Msg> = Vec::new();
    let mut dbg = serde_json::Map::new();
    let md = r.min_distance();
    let gamma = r.gamma().unwrap_or(0);
    if !md && r.distance_km > LONG_KM {
        warns.push(Msg::new(Code::LongDistance, json!({"km": LONG_KM})));
    }
    // min_distance : zone et bornes de la distance max Lcap (Lmin = 0).
    let (l, lmax) = lengths(&r);
    let frame = Frame::new_in(r.zone.proj, r.lat, r.lon);
    check_via(&r)?;
    let mut region = build_region(&r, &frame, lmax, l)?;
    let area_km2 = region.area() / 1e6;
    if r.enforce_limits && area_km2 > MAX_AREA_KM2 {
        return Err(Msg::new(
            Code::ZoneTooLarge,
            json!({"area_km2": area_km2.round(), "max_km2": MAX_AREA_KM2}),
        ));
    }
    let l93_box = |reg: &Region| {
        let b = reg.bounds();
        let (x0, y0) = frame.l93([b[0] - BBOX_MARGIN_M, b[1] - BBOX_MARGIN_M]);
        let (x1, y1) = frame.l93([b[2] + BBOX_MARGIN_M, b[3] + BBOX_MARGIN_M]);
        [x0, y0, x1, y1]
    };
    let t0 = Instant::now();
    let t = load(l93_box(&region)).map_err(|e| {
        // source de dalles distante en panne : transitoire (503), pas une erreur de calcul
        let code = if e.starts_with(crate::tiles::REMOTE_ERR) {
            Code::Busy
        } else {
            Code::InvalidProblem
        };
        err(code, &e)
    })?;
    dbg.insert("tiles_load_s".into(), json!(t0.elapsed().as_secs_f64()));
    dbg.insert("troncons_loaded".into(), json!(t.len()));
    if t.is_empty() {
        return Err(err(Code::OutsideCoverage, "no tile around the start"));
    }
    // Grande zone : on compte les tronçons et on réduit si c'est trop dense (MAX_WAYS).
    let mut reduced_radius = None;
    if r.enforce_limits && area_km2 > FALLBACK_AREA_KM2 {
        let cap = if r.mode == "target" {
            MAX_WAYS
        } else {
            MAX_WAYS_MAX
        };
        let radius0 = region.radius();
        let mut radius = radius0;
        for _ in 0..4 {
            let b = l93_box(&region);
            let n = (0..t.len())
                .filter(|&i| {
                    let (x, y) = t.vertex(i, 0);
                    x >= b[0] && x < b[2] && y >= b[1] && y < b[3]
                })
                .count();
            if n <= cap {
                break;
            }
            radius *= 0.95 * (cap as f64 / n as f64).sqrt();
            region = region.with_disk(radius);
        }
        if radius < radius0 - 1.0 {
            let a = region.area() / 1e6;
            warns.push(Msg::new(
                Code::ZoneReduced,
                json!({"radius_km": (radius / 100.0).round() / 10.0, "area_km2": a.round()}),
            ));
            reduced_radius = Some(radius);
        }
    }
    dbg.insert(
        "zone_km2".into(),
        json!((region.area() / 1e4).round() / 100.0),
    );

    let t1 = Instant::now();
    let sel = keep_mask(&t, natures);
    let trail_t = trail_mask(&t, natures);
    let mut net = Net::new(&t, frame);
    net.trail_t = trail_t.clone();
    let mut edges = net.build(&sel, &region);
    dbg.insert("source_ways".into(), json!(sel.len()));
    dbg.insert("edges_simplified".into(), json!(edges.len()));
    if edges.is_empty() {
        return Err(err(Code::NoWayInZone, ""));
    }
    // points de passage : accrochés au réseau comme le départ (nœud inséré), <= VIA_SNAP_M
    let mut via: Vec<ViaPt> = Vec::new();
    for (i, &[lat, lon]) in r.via.iter().enumerate() {
        let q = frame.to_local(lon, lat);
        match net.insert_start(&mut edges, q) {
            Some((node, snap, _)) if snap <= VIA_SNAP_M => via.push(ViaPt { node, q, snap }),
            _ => {
                return Err(Msg::with_detail(
                    Code::ViaUnreachable,
                    json!({"n": i + 1}),
                    "no way near the via point",
                ));
            }
        }
    }
    let mut best_via: Vec<ViaPt> = via.clone();
    // préférence de type de voie (mode max) : prime par mètre sur le bon type, à l'échelle de la
    // densité moyenne de D+ du réseau de la zone
    let off_price = {
        let (pl, pw): (Vec<f64>, Vec<f64>) = edges
            .iter()
            .map(|&e| (net.edges[e].len, net.edges[e].w))
            .unzip();
        crate::problem::SURF_MAX * crate::problem::knapsack_ub(&pl, &pw, lmax) / lmax
    };
    let surface = r.surface.clone();
    let weight = |ed: &Edge| ed.w + off_price * (ed.len - off_type(ed, &surface));
    let weight: Option<&dyn Fn(&Edge) -> f64> = (surface != "any" && !md).then_some(&weight);
    let mut access_net: Option<Option<Access>> = None;

    // search_loop
    let lmin = match r.mode.as_str() {
        "max" => l * (1.0 - r.tol),
        "min_distance" => 0.0,
        _ => 0.7 * l,
    };
    let x_md = r.target_dplus.filter(|_| md);
    let cap_end = r
        .max_compute_s
        .map(|c| t_all + std::time::Duration::from_secs_f64(c.max(0.0)));
    // plus petite borne prouvée parmi les essais refusés (INF : Σw < X partout)
    let mut proven_lb: Option<f64> = None;
    // borne finie prouvée sur le réseau du point cliqué : rendue telle quelle, pas de repli (D34)
    let mut depart_lb = false;
    let mut remaining = time_s;
    let mut attempts: Vec<Value> = Vec::new();
    let mut tried: HashSet<usize> = HashSet::new();
    let mut prep_s = t1.elapsed().as_secs_f64();
    let mut solve_s = 0.0;
    type Found = (
        Problem,
        Vec<usize>,
        crate::Output,
        Option<AccessPath>,
        &'static str,
        f64,
        Option<f64>,
    );
    let mut first: Option<Found> = None;
    // (genre, arêtes, distance du point cliqué) : le réseau du départ, puis les replis du plus proche au plus loin
    let mut sets: Vec<(&str, Vec<usize>, f64)> = vec![("depart", edges.clone(), 0.0)];
    let mind = |net: &Net, c: &[usize]| {
        c.iter()
            .map(|&e| net.min_dist(e))
            .fold(f64::INFINITY, f64::min)
    };
    let mut comps_done = false;
    let mut k = 0;
    loop {
        if k == sets.len() {
            if comps_done {
                break;
            }
            comps_done = true;
            // composantes du réseau sous la pente max : sinon le point le plus proche d'une
            // composante peut être dans une poche fermée par des tronçons trop raides
            let ok: Vec<usize> = edges
                .iter()
                .copied()
                .filter(|&e| r.max_grade.is_none_or(|g| net.edges[e].grade <= g))
                .collect();
            for c in prep::loop_components(&net, &ok) {
                let d = mind(&net, &c);
                sets.push(("repli", c, d));
            }
            sets[k..].sort_by(|a, b| a.2.total_cmp(&b.2));
            continue;
        }
        let (kind, sub) = (sets[k].0, sets[k].1.clone());
        k += 1;
        // D34 : départ déplacé en dernier recours seulement. Dès que le réseau du point cliqué
        // donne une boucle (même hors tolérance ou sous X), on la garde ; en min_distance, une
        // borne prouvée finie (« pas de boucle de ce D+ sous Y km ») est rendue telle quelle
        // plutôt qu'un départ à plusieurs km (Grenoble, X = 500 : départ déplacé de 5,8 km).
        // avec points de passage, déplacer le départ n'a pas de sens
        if kind == "repli" && (first.is_some() || depart_lb || !via.is_empty()) {
            break;
        }
        let solved = attempts
            .iter()
            .filter(|a| a["solved"] == json!(true))
            .count();
        if remaining < MIN_ATTEMPT_S
            || solved >= MAX_ATTEMPTS
            || cap_end.is_some_and(|c| Instant::now() >= c)
        {
            break;
        }
        if kind == "repli" && sub.iter().any(|e| tried.contains(e)) {
            continue;
        }
        let tp = Instant::now();
        let mut cand = Err(Vec::new());
        let mut cur_via = via.clone();
        let mut info = serde_json::Map::new();
        // Avec accès par aller-retour d'abord ; s'il rend la boucle impossible, même
        // sous-réseau avec le départ déplacé.
        for with_access in [true, false] {
            info = serde_json::Map::new();
            info.insert("kind".into(), json!(kind));
            info.insert("solved".into(), json!(false));
            let acc = if with_access {
                let nearest = net.nearest_vertex(&sub);
                if nearest > ACCESS_MIN_M && access_net.is_none() {
                    access_net = Some(Access::new(&t, frame, &sel, &trail_t, &region));
                }
                access_net.as_mut().and_then(|a| a.as_mut())
            } else {
                None
            };
            let had_access = acc.is_some();
            let mut vp = via.clone();
            let c = build_candidate(
                &mut net,
                &sub,
                lmin,
                lmax,
                r.max_grade,
                acc,
                r.mode != "target",
                x_md,
                &mut vp,
                weight,
                &mut info,
            );
            if c.is_ok() || !had_access || !info.contains_key("access_m") {
                (cand, cur_via) = (c, vp);
                break;
            }
            info.insert(
                "status".into(),
                json!(format!(
                    "{}_with_access",
                    info["status"].as_str().unwrap_or("")
                )),
            );
            attempts.push(Value::Object(info.clone()));
        }
        prep_s += tp.elapsed().as_secs_f64();
        let c = match cand {
            Ok(c) => c,
            Err(short) => {
                if info.get("status").and_then(Value::as_str) == Some("dplus_unreachable") {
                    let b = info["lower_bound_m"].as_f64().unwrap_or(f64::INFINITY);
                    proven_lb = Some(proven_lb.map_or(b, |p| p.min(b)));
                    depart_lb |= kind == "depart" && b.is_finite();
                }
                attempts.push(Value::Object(info));
                // Repli : le réseau à portée de ce départ est trop court, mais la composante
                // continue plus loin (reliée par un long détour) : on écarte ce réseau et on
                // reprend sur le reste, au point le plus proche.
                if kind == "repli" && !short.is_empty() {
                    let mut near = vec![false; net.xy.len()];
                    for &e in &short {
                        near[net.edges[e].u] = true;
                        near[net.edges[e].v] = true;
                    }
                    let rest: Vec<usize> = sub
                        .iter()
                        .copied()
                        .filter(|&e| !(near[net.edges[e].u] && near[net.edges[e].v]))
                        .collect();
                    if rest.len() < sub.len() {
                        for c in prep::loop_components(&net, &rest) {
                            let d = mind(&net, &c);
                            sets.push(("repli", c, d));
                        }
                        sets[k..].sort_by(|a, b| a.2.total_cmp(&b.2));
                    }
                }
                continue;
            }
        };
        tried.extend(c.ids.iter().copied());
        let acc_len = c.access.as_ref().map_or(0.0, |a| a.length);
        let l_eff = l - 2.0 * acc_len;
        let d_eff = r.target_dplus.map(|d| match &c.access {
            Some(a) => (d - a.updown).max(1.0),
            None => d,
        });
        let (mut p, _nodes) = to_problem(
            &net,
            &c.ids,
            c.s,
            l_eff,
            &r.mode,
            r.tol * l / l_eff,
            d_eff,
            r.node_simple,
            gamma,
            &cur_via.iter().map(|v| v.node).collect::<Vec<_>>(),
        );
        if let Some(lb) = c.lb {
            p.l = p.lmax.min(2.0 * lb); // indicatif : couloirs et waypoints
        }
        // (min_distance : la sortie la plus courte prime, pas de préférence dans la recherche)
        if r.surface != "any" && !md {
            p.off = c
                .ids
                .iter()
                .map(|&e| off_type(&net.edges[e], &r.surface))
                .collect();
            p.off_price = off_price;
        }
        if prep_only {
            dbg.insert("prep_s".into(), json!(prep_s));
            dbg.insert("attempt".into(), Value::Object(info));
            return Ok(
                json!({"problem_stats": problem_stats(&p), "warnings": warns, "debug": dbg}),
            );
        }
        let ts = Instant::now();
        // itérations fixes par essai (pas `remaining`, qui dépend de l'horloge) : même graine,
        // même boucle, y compris après un essai infructueux (« Autres boucles », api.md)
        let mut budget = Budget::from_time(time_s, r.n_candidates, p.n_edges(), r.seed);
        let end = ts + std::time::Duration::from_secs_f64(remaining);
        budget.deadline = Some(cap_end.map_or(end, |c| c.min(end)));
        let res = optimize(&p, &budget);
        let dt = ts.elapsed().as_secs_f64();
        solve_s += dt;
        remaining -= dt;
        let out = match res {
            Ok(out) => out,
            Err(m) => {
                info.insert("status".into(), json!(m.code));
                attempts.push(Value::Object(info));
                continue;
            }
        };
        info.insert("solved".into(), json!(true));
        info.insert("feasible".into(), json!(out.feasible));
        info.insert("method".into(), json!(out.method));
        attempts.push(Value::Object(info));
        let feasible = out.feasible;
        if first.is_none() || feasible {
            let snap = attempts.last().unwrap()["start_snap_m"]
                .as_f64()
                .unwrap_or(0.0);
            best_via = cur_via.clone();
            first = Some((
                p,
                c.ids,
                out,
                c.access,
                if kind == "depart" { "depart" } else { "repli" },
                snap,
                c.lb,
            ));
        }
        if feasible {
            break;
        }
    }
    let Some((p, ids, out, acc, kind, snap, lb)) = first else {
        let detail = Value::Array(attempts.clone()).to_string();
        if let (Some(x), Some(b)) = (x_md, proven_lb) {
            let min_km = b.is_finite().then(|| (b / 100.0).round() / 10.0);
            return Err(Msg::with_detail(
                Code::DplusUnreachableProven,
                json!({"dplus_m": x.round(), "min_km": min_km}),
                detail,
            ));
        }
        if let Some(n) = attempts
            .iter()
            .find(|a| a["status"] == json!("via_unreachable"))
            .and_then(|a| a["via_n"].as_u64())
        {
            return Err(Msg::with_detail(
                Code::ViaUnreachable,
                json!({"n": n}),
                detail,
            ));
        }
        return Err(err(Code::NoLoopOfDistance, &detail));
    };
    let acc_len = acc.as_ref().map_or(0.0, |a| a.length);
    // borne affichée : boucle + aller-retour d'accès
    let lower_bound = lb.map(|b| b + 2.0 * acc_len);
    dbg.insert("attempts".into(), Value::Array(attempts));
    if let Some(a) = &acc {
        warns.push(Msg::new(
            Code::AccessRoundTrip,
            json!({"access_m": a.length.round()}),
        ));
    } else if kind == "repli" {
        warns.push(Msg::new(
            Code::StartMoved,
            json!({"distance_m": snap.round()}),
        ));
    } else if snap > 150.0 {
        warns.push(Msg::new(
            Code::StartFarFromNetwork,
            json!({"distance_m": snap.round()}),
        ));
    }
    let ub = p.dplus_upper_bound();
    // dplus_not_reached est recalculé ci-dessous avec l'accès compris
    warns.extend(
        out.warnings
            .into_iter()
            .filter(|w| w.code != Code::DplusNotReached),
    );
    let missed = p.via_missed(&out.ids);
    for (j, x) in p.via.iter().enumerate() {
        if missed.contains(x) {
            warns.push(Msg::new(Code::ViaMissed, json!({"n": j + 1})));
        }
    }
    let main = assemble(&net, &ids, &p, &out.ids, acc.as_ref(), gamma)?;
    let prof: f64 = main.z.windows(2).map(|w| (w[1] - w[0]).max(0.0)).sum();
    if (prof - main.dplus).abs() > 1e-6 * main.dplus.max(1.0) {
        warns.push(Msg::new(
            Code::ProfileMismatch,
            json!({"profile_m": prof, "sum_w_m": main.dplus}),
        ));
    }
    if r.mode == "target" {
        let d = r.target_dplus.unwrap();
        if d > ub {
            warns.push(Msg::new(
                Code::TargetDplusAboveBound,
                json!({"max_dplus_m": ub.round(), "max_km": (lmax / 100.0).round() / 10.0}),
            ));
        } else if ((main.dplus - d) / d).abs() > 0.10 {
            warns.push(Msg::new(Code::TargetDplusProbablyUnreachable, json!({})));
        }
    }
    if let Some(x) = x_md.filter(|_| !main.feasible) {
        warns.push(Msg::new(
            Code::DplusNotReached,
            json!({"dplus_m": x.round(), "best_dplus_m": main.dplus.round(), "max_km": (l / 100.0).round() / 10.0}),
        ));
    }
    let mut loops = vec![main];
    for a in &out.alternatives {
        loops.push(assemble(&net, &ids, &p, a, acc.as_ref(), gamma)?);
    }
    match r.mode.as_str() {
        // D+ décroissant ; avec un type de voie préféré, D+ plus sa prime (ce que la recherche maximise)
        "max" => {
            let val = |t: &Track| match r.surface.as_str() {
                "trail" => t.dplus + off_price * t.trail,
                "road" => t.dplus + off_price * (t.length - t.trail),
                _ => t.dplus,
            };
            loops.sort_by(|a, b| val(b).total_cmp(&val(a)))
        }
        // réalisables d'abord, puis par longueur croissante
        "min_distance" => loops.sort_by(|a, b| {
            b.feasible
                .cmp(&a.feasible)
                .then(a.length.total_cmp(&b.length))
        }),
        // cible : la plus proche de la demande d'abord ; si elle la tient (à `DIAG_TOL`), celles
        // qui ne la tiennent pas ne sont pas rendues (sinon `target_not_reached`, voir `important`)
        _ => {
            let d = r.target_dplus.unwrap_or(1.0);
            // (type de voie : même ordre que la recherche, `Problem::score`)
            let err = |t: &Track| {
                let e = ((t.length - l) / l).abs() + ((t.dplus - d) / d).abs();
                match r.surface.as_str() {
                    "trail" => e.max(SURF_TARGET_BAND) + SURF_TARGET * (1.0 - t.trail / l),
                    "road" => e.max(SURF_TARGET_BAND) + SURF_TARGET * t.trail / l,
                    _ => e,
                }
            };
            let tol = |t: &Track| {
                ((t.length - l) / l).abs() <= DIAG_TOL && ((t.dplus - d) / d).abs() <= DIAG_TOL
            };
            loops.sort_by(|a, b| err(a).total_cmp(&err(b)));
            if tol(&loops[0]) {
                loops.retain(tol);
            }
        }
    }
    let via_pts: Vec<(usize, [f64; 2], f64)> = best_via
        .iter()
        .enumerate()
        .map(|(i, v)| (i + 1, net.xy[v.node], v.snap))
        .collect();
    let start_xy = loops[0].xy[0];
    let (slon, slat) = frame.to_wgs(start_xy);
    dbg.insert("problem".into(), problem_stats(&p));
    dbg.insert("method".into(), json!(out.method));
    dbg.insert("iterations".into(), json!(out.face_iterations));
    dbg.insert("prep_s".into(), json!(prep_s));
    dbg.insert("solve_s".into(), json!(solve_s));
    let polys: Vec<Vec<Vec<[f64; 2]>>> = region
        .rings()
        .into_iter()
        .map(|ring| {
            vec![
                ring.into_iter()
                    .map(|q| {
                        let (lo, la) = frame.to_wgs(q);
                        [r6(lo), r6(la)]
                    })
                    .collect(),
            ]
        })
        .collect();
    // écarts : mode cible (L, D) ; min_distance (borne inférieure de distance, X)
    let target = match r.mode.as_str() {
        "target" => r.target_dplus.map(|d| (l, d)),
        "min_distance" => r.target_dplus.map(|d| (lower_bound.unwrap_or(0.0), d)),
        _ => None,
    };
    let candidates: Vec<Value> = loops
        .iter()
        .map(|tr| track_json(&frame, tr, target, &via_pts))
        .collect();
    Ok(json!({
        "solver_version": crate::solver_version(),
        "compute_s": t_all.elapsed().as_secs_f64(),
        "candidates": candidates,
        "lower_bound_m": lower_bound.map(f64::round),
        "warnings": warns,
        "effective_start": {
            "lat": r6(slat), "lon": r6(slon),
            "kind": if acc.is_some() { "access" } else if kind == "repli" { "moved" } else { "clicked" },
            "moved_m": round1(acc.as_ref().map_or(snap, |a| a.snap)),
            "access_m": acc.as_ref().map(|a| a.length.round()),
        },
        "zone": {
            "geometry": {"type": "MultiPolygon", "coordinates": polys},
            "area_km2": (region.area() / 1e4).round() / 100.0,
            "reduced_radius_km": reduced_radius.map(|x| (x / 100.0).round() / 10.0),
        },
        "debug": dbg,
    }))
}

fn r6(x: f64) -> f64 {
    (x * 1e6).round() / 1e6
}

pub use crate::climbs::scan as climbs;

pub use crate::prep::{GRADE_WINDOW_M, max_grade};

/// Points de passage d'un tracé (api.md v1.5) : `via` (passés, ordre de passage) et `legs`
/// (départ → points → arrivée, D+/D− du profil). `pts` : (n, position du nœud, accroche).
fn via_legs(
    frame: &Frame,
    tr: &Track,
    dist: &[f64],
    pts: &[(usize, [f64; 2], f64)],
) -> (Value, Value) {
    let mut hit: Vec<(usize, usize, f64)> = pts
        .iter()
        .filter_map(|&(n, q, snap)| {
            let (i, d) = tr
                .xy
                .iter()
                .enumerate()
                .map(|(i, p)| (i, (p[0] - q[0]).hypot(p[1] - q[1])))
                .min_by(|a, b| a.1.total_cmp(&b.1))?;
            (d < 1.0).then_some((i, n, snap))
        })
        .collect();
    hit.sort_unstable_by_key(|h| h.0);
    let via: Vec<Value> = hit
        .iter()
        .map(|&(i, n, snap)| {
            let (lo, la) = frame.to_wgs(tr.xy[i]);
            json!({"n": n, "lat": r6(la), "lon": r6(lo), "snap_m": round1(snap), "dist_m": round1(dist[i])})
        })
        .collect();
    let last = tr.xy.len() - 1;
    let cuts: Vec<(usize, usize)> = std::iter::once((0, 0))
        .chain(hit.iter().map(|h| (h.0, h.1)))
        .chain([(last, 0)])
        .collect();
    let legs: Vec<Value> = cuts
        .windows(2)
        .map(|w| {
            let z = &tr.z[w[0].0..=w[1].0];
            let up: f64 = z.windows(2).map(|d| (d[1] - d[0]).max(0.0)).sum();
            let down: f64 = z.windows(2).map(|d| (d[0] - d[1]).max(0.0)).sum();
            json!({"from": w[0].1, "to": w[1].1, "length_m": round1(dist[w[1].0] - dist[w[0].0]),
                   "dplus_m": round1(up), "dminus_m": round1(down)})
        })
        .collect();
    (json!(via), json!(legs))
}

fn track_json(
    frame: &Frame,
    tr: &Track,
    target: Option<(f64, f64)>,
    via_pts: &[(usize, [f64; 2], f64)],
) -> Value {
    let (mut lat, mut lon, mut dist) = (Vec::new(), Vec::new(), Vec::new());
    let mut s = 0.0;
    for (i, &q) in tr.xy.iter().enumerate() {
        if i > 0 {
            let p = tr.xy[i - 1];
            s += (q[0] - p[0]).hypot(q[1] - p[1]);
        }
        let (lo, la) = frame.to_wgs(q);
        lat.push(r6(la));
        lon.push(r6(lo));
        dist.push(s);
    }
    let c = climbs(&tr.z, &dist);
    let (sg, sg2, sl) = c.iter().fold((0.0, 0.0, 0.0), |a, &(g, l)| {
        (a.0 + g, a.1 + g * g, a.2 + l)
    });
    let longest = c
        .iter()
        .copied()
        .fold((0.0, 0.0), |a, x| if x.0 > a.0 { x } else { a });
    let (via, legs) = via_legs(frame, tr, &dist, via_pts);
    let zmin = tr.z.iter().copied().fold(f64::INFINITY, f64::min);
    let zmax = tr.z.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    json!({
        "length_m": round1(tr.length),
        "dplus_m": round1(tr.dplus),
        "feasible": tr.feasible,
        "alt_min_m": round1(zmin),
        "alt_max_m": round1(zmax),
        "max_grade_pct": round1(100.0 * max_grade(&tr.z, &dist)),
        "trail_frac": ((1000.0 * tr.trail / tr.length.max(1.0)).round() / 1000.0).clamp(0.0, 1.0),
        "target_gap": target.map(|(l, d)| json!({"distance_m": round1(tr.length - l), "dplus_m": round1(tr.dplus - d)})),
        "climbs": {
            "count": c.iter().filter(|x| x.0 >= 20.0).count(),
            "longest_gain_m": round1(longest.0),
            "longest_len_m": round1(longest.1),
            "gbar_m": round1(if sg > 0.0 { sg2 / sg } else { 0.0 }),
            "mean_grade_pct": round1(if sl > 0.0 { 100.0 * sg / sl } else { 0.0 }),
        },
        "via": via,
        "legs": legs,
        "landmarks": [],
        "lat": lat, "lon": lon,
        "ele": tr.z.iter().map(|&z| round1(z)).collect::<Vec<_>>(),
        "dist": dist.iter().map(|&d| round1(d)).collect::<Vec<_>>(),
    })
}

/// Statistiques de parité de la préparation (axe données).
pub fn problem_stats(p: &Problem) -> Value {
    json!({
        "edges": p.n_edges(),
        "nodes": p.n_nodes(),
        "sum_len_m": p.len.iter().sum::<f64>(),
        "sum_w_m": p.w.iter().sum::<f64>(),
        "dplus_upper_bound_m": p.dplus_upper_bound(),
        "parallel_pairs": p.parallel.len(),
        "far": p.far.len(),
        "Lmin": p.lmin, "Lmax": p.lmax,
    })
}
