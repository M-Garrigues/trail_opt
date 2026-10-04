//! Chaîne complète depuis les dalles : requête → zone → graphe → solveur → tracés
//! (port de `pipeline.plan_loop` / `search_loop` / `build_candidate` / `Access`, source IGN).
use std::collections::HashSet;
use std::time::Instant;

use geo::{BooleanOps, Coord, LineString, MultiPolygon, Polygon, Validation};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::codes::{Code, Msg};
use crate::l93::Frame;
use crate::prep::{self, FREE_RADIUS, Net, REDUCE_K, REDUCE_MIN_EDGES, Region};
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
    #[serde(default = "default_roads")]
    pub roads: String,
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
fn default_roads() -> String {
    "minor".into()
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
        && r.max_compute_s.is_none_or(f64::is_finite);
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
    if !matches!(r.roads.as_str(), "unpaved" | "pedestrian" | "minor" | "all") {
        return Err(err(Code::RoadsUnknown, &r.roads));
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

/// Codes de nature (manifeste) → tronçons gardés pour un type de voies (port de `ign.keep_mask`).
pub fn keep_mask(t: &Troncons, natures: &[String], roads: &str) -> Vec<usize> {
    let code = |names: &[&str]| -> Vec<u8> {
        natures
            .iter()
            .enumerate()
            .filter(|(_, n)| names.contains(&n.as_str()))
            .map(|(i, _)| i as u8)
            .collect()
    };
    let unpaved = code(&["Sentier", "Chemin", "Route empierrée"]);
    let ped = code(&[
        "Sentier",
        "Chemin",
        "Route empierrée",
        "Escalier",
        "Piste cyclable",
    ]);
    let road = code(&[
        "Route à 1 chaussée",
        "Route à 2 chaussées",
        "Rond-point",
        "Bretelle",
    ]);
    let excluded = code(&["Type autoroutier", "Bac ou liaison maritime"]);
    (0..t.len())
        .filter(|&i| {
            let (n, imp) = (t.nature[i], t.importance[i]);
            if t.flags[i] & 2 == 0 || excluded.contains(&n) || n == 255 || t.n_vertices(i) < 2 {
                return false;
            }
            match roads {
                "unpaved" => unpaved.contains(&n),
                "pedestrian" => ped.contains(&n),
                "all" => ped.contains(&n) || road.contains(&n),
                _ => ped.contains(&n) || (road.contains(&n) && (4..=6).contains(&imp)),
            }
        })
        .collect()
}

/// Aller-retour d'accès : chemin, sur toutes les voies, du point cliqué vers le réseau.
pub struct AccessPath {
    pub length: f64,
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
        region: &Region,
    ) -> Option<Access<'a>> {
        let mut net = Net::new(t, frame);
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
        let updown = z.windows(2).map(|w| (w[1] - w[0]).abs()).sum();
        Some(AccessPath {
            length,
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
/// élagage, pente max, borne inférieure (min_distance), réduction aux arêtes pentues. None si aucune boucle >= Lmin possible.
/// `x` : D+ visé du mode min_distance ; None (statut `dplus_unreachable`) si la borne
/// inférieure de distance dépasse Lmax.
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
    info: &mut serde_json::Map<String, Value>,
) -> Option<Candidate> {
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
    let (mut s, _, mut pos) = net.insert_start(&mut ids, point)?;
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
                let (s2, snap, p2) = net.insert_start(&mut ids, [0.0, 0.0])?;
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
        return None;
    }
    if let Some(g) = max_grade {
        ids.retain(|&e| net.edges[e].grade <= g);
        ids = prep::prune(net, ids, s, lmax);
        if net.total_len(&ids) < lmin {
            info.insert("status".into(), json!("too_short_after_grade_filter"));
            return None;
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
            return None;
        }
        lb = b;
    }
    if reduce && ids.len() > REDUCE_MIN_EDGES && net.total_len(&ids) > 1.25 * REDUCE_K * lmax {
        let keep = prep::steep_reduction(net, &ids, s, lmax, REDUCE_K);
        ids = prep::prune(net, keep, s, lmax);
        info.insert("edges_reduced".into(), json!(ids.len()));
        if net.total_len(&ids) < lmin {
            info.insert("status".into(), json!("network_too_short_after_reduction"));
            return None;
        }
    }
    info.insert("edges_after_grade".into(), json!(ids.len()));
    Some(Candidate {
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
        parallel: prep::parallel_pairs(net, ids, c),
        q: if climbs < 0 {
            ids.iter().map(|&e| edge_q(net, e)).collect()
        } else {
            Vec::new()
        },
        climbs,
    };
    (p, nodes)
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
    if let Some(a) = acc {
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
        feasible,
    })
}

/// Mode min_distance sans distance max : défaut clamp(X / k, 3, 60) km, k = 25 m/km en relief,
/// `MD_CAP_PER_KM_FLAT` en plaine (relief de la dalle du départ, I2). Sinon, requête inchangée.
pub fn resolve_cap(store: &TileStore, req: &Request) -> Request {
    let mut r = req.clone();
    if r.min_distance() && r.max_distance_km.is_none() && (-90.0..=90.0).contains(&r.lat) {
        let (x, y) = crate::l93::forward(r.lon, r.lat);
        let relief = store
            .tile_l93(x, y)
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

/// Point d'entrée : requête + dalles → JSON de sortie (ou erreur codée D14).
/// `prep_only` : s'arrête au premier Problem (parité de préparation, sans solveur).
pub fn plan(store: &TileStore, req: &Request, prep_only: bool) -> Result<Value, Msg> {
    let req = resolve_cap(store, req);
    let mut out = plan_with(|b| store.load(b), &store.manifest.natures, &req, prep_only)?;
    out["data_version"] = json!(store.manifest.data_version);
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
                    let (x, y) = crate::l93::forward(lon, lat);
                    store.tile_l93(x, y).is_none()
                })
        });
    if edge && let Some(w) = out["warnings"].as_array_mut() {
        w.push(json!(Msg::new(Code::CoverageEdge, json!({}))));
    }
    Ok(out)
}

/// Comme `plan`, avec un chargeur de tronçons (boîte L93 en m) : dalles ou données d'essai.
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
    let l = r.sizing_km() * 1000.0;
    let frame = Frame::new(r.lat, r.lon);
    let lmax = match r.mode.as_str() {
        "max" => l * (1.0 + r.tol),
        "min_distance" => l,
        _ => 1.3 * l,
    };
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
    let t = load(l93_box(&region)).map_err(|e| err(Code::InvalidProblem, &e))?;
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
    let sel = keep_mask(&t, natures, &r.roads);
    let mut net = Net::new(&t, frame);
    let edges = net.build(&sel, &region);
    dbg.insert("source_ways".into(), json!(sel.len()));
    dbg.insert("edges_simplified".into(), json!(edges.len()));
    if edges.is_empty() {
        return Err(err(Code::NoWayInZone, ""));
    }
    let sel_all = keep_mask(&t, natures, "all");
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
    let mut sets: Vec<(&str, Vec<usize>)> = vec![("depart", edges.clone())];
    let mut comps_done = false;
    let mut k = 0;
    loop {
        if k == sets.len() {
            if comps_done {
                break;
            }
            comps_done = true;
            let mut comps = prep::loop_components(&net, &edges);
            let mind: Vec<f64> = comps
                .iter()
                .map(|c| {
                    c.iter()
                        .map(|&e| net.min_dist(e))
                        .fold(f64::INFINITY, f64::min)
                })
                .collect();
            let mut order: Vec<usize> = (0..comps.len()).collect();
            order.sort_by(|&a, &b| mind[a].total_cmp(&mind[b]));
            for i in order {
                sets.push(("repli", std::mem::take(&mut comps[i])));
            }
            continue;
        }
        let (kind, sub) = (sets[k].0, sets[k].1.clone());
        k += 1;
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
        let mut cand = None;
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
                    access_net = Some(Access::new(&t, frame, &sel_all, &region));
                }
                access_net.as_mut().and_then(|a| a.as_mut())
            } else {
                None
            };
            let had_access = acc.is_some();
            let c = build_candidate(
                &mut net,
                &sub,
                lmin,
                lmax,
                r.max_grade,
                acc,
                r.mode != "target",
                x_md,
                &mut info,
            );
            if c.is_some() || !had_access || !info.contains_key("access_m") {
                cand = c;
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
        let Some(c) = cand else {
            if info.get("status").and_then(Value::as_str) == Some("dplus_unreachable") {
                let b = info["lower_bound_m"].as_f64().unwrap_or(f64::INFINITY);
                proven_lb = Some(proven_lb.map_or(b, |p| p.min(b)));
            }
            attempts.push(Value::Object(info));
            continue;
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
        );
        if let Some(lb) = c.lb {
            p.l = p.lmax.min(2.0 * lb); // indicatif : couloirs et waypoints
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
        let detail = Value::Array(attempts).to_string();
        if let (Some(x), Some(b)) = (x_md, proven_lb) {
            let min_km = b.is_finite().then(|| (b / 100.0).round() / 10.0);
            return Err(Msg::with_detail(
                Code::DplusUnreachableProven,
                json!({"dplus_m": x.round(), "min_km": min_km}),
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
        "max" => loops.sort_by(|a, b| b.dplus.total_cmp(&a.dplus)),
        // réalisables d'abord, puis par longueur croissante
        "min_distance" => loops.sort_by(|a, b| {
            b.feasible
                .cmp(&a.feasible)
                .then(a.length.total_cmp(&b.length))
        }),
        _ => {}
    }
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
        .map(|tr| track_json(&frame, tr, target))
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

/// Montées par hystérésis (5 m, contrat modes.md B) : [(gain, longueur)].
pub fn climbs(z: &[f64], s: &[f64]) -> Vec<(f64, f64)> {
    const H: f64 = 5.0;
    let mut out = Vec::new();
    let (mut lo, mut hi): (usize, Option<usize>) = (0, None);
    for i in 1..z.len() {
        match hi {
            None if z[i] < z[lo] => lo = i,
            None if z[i] - z[lo] >= H => hi = Some(i),
            Some(h) if z[i] > z[h] => hi = Some(i),
            Some(h) if z[h] - z[i] >= H => {
                out.push((z[h] - z[lo], s[h] - s[lo]));
                (lo, hi) = (i, None);
            }
            _ => {}
        }
    }
    if let Some(h) = hi {
        out.push((z[h] - z[lo], s[h] - s[lo]));
    }
    out
}

pub use crate::prep::{GRADE_WINDOW_M, max_grade};

fn track_json(frame: &Frame, tr: &Track, target: Option<(f64, f64)>) -> Value {
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
    let zmin = tr.z.iter().copied().fold(f64::INFINITY, f64::min);
    let zmax = tr.z.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    json!({
        "length_m": round1(tr.length),
        "dplus_m": round1(tr.dplus),
        "feasible": tr.feasible,
        "alt_min_m": round1(zmin),
        "alt_max_m": round1(zmax),
        "max_grade_pct": round1(100.0 * max_grade(&tr.z, &dist)),
        "target_gap": target.map(|(l, d)| json!({"distance_m": round1(tr.length - l), "dplus_m": round1(tr.dplus - d)})),
        "climbs": {
            "count": c.iter().filter(|x| x.0 >= 20.0).count(),
            "longest_gain_m": round1(longest.0),
            "longest_len_m": round1(longest.1),
            "gbar_m": round1(if sg > 0.0 { sg2 / sg } else { 0.0 }),
            "mean_grade_pct": round1(if sl > 0.0 { 100.0 * sg / sl } else { 0.0 }),
        },
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
