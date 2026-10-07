//! Préparation du graphe depuis les tronçons des dalles (port de `trailopt.graph` et de
//! `pipeline.base_edges` / `build_candidate`).
//!
//! Une arête est une suite de morceaux de tronçons (`Piece` : plage de points de profil, sens) :
//! longueur, D+ (`w`) et pente viennent directement des profils des dalles, sans
//! rééchantillonnage ; la géométrie n'est calculée que pour le résultat. Nœuds et arêtes sont
//! dans des arènes (`Net`) ; un sous-graphe est une liste d'identifiants d'arêtes.
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

use geo::{Area, BooleanOps, Coord, LineString, MultiPolygon, Polygon};

use crate::l93::Frame;
use crate::tiles::{Troncons, node_key};

/// Rayon libre autour du départ : aller-retour et carrefours repassés permis.
pub const FREE_RADIUS: f64 = 200.0;
/// Couloirs parallèles : arêtes plus courtes ignorées (comme `graph.PARALLEL_MIN_LEN`).
pub const PARALLEL_MIN_LEN: f64 = 30.0;
/// Réduction aux arêtes pentues : longueur gardée en multiples de Lmax, et seuil d'arêtes.
pub const REDUCE_K: f64 = 8.0;
pub const REDUCE_MIN_EDGES: usize = 5000;
/// Préférence de type de voie : longueur gardée en plus, en multiples de `REDUCE_K` × Lmax.
pub const REDUCE_SURF: f64 = 1.5;

// ---------------------------------------------------------------------------
// Zone
// ---------------------------------------------------------------------------

/// Disque centré sur le départ, `quad` segments par quart de cercle (comme shapely `buffer`).
pub fn disk(r: f64, quad: usize) -> Polygon<f64> {
    let n = 4 * quad;
    let ring: Vec<Coord<f64>> = (0..=n)
        .map(|i| {
            let a = std::f64::consts::TAU * (i % n) as f64 / n as f64;
            Coord {
                x: r * a.cos(),
                y: r * a.sin(),
            }
        })
        .collect();
    Polygon::new(LineString::new(ring), vec![])
}

/// Zone de recherche (repère local) : test d'appartenance rapide.
pub struct Region {
    pub poly: MultiPolygon<f64>,
    segs: Vec<[f64; 4]>,
    r_in: f64,
    r_out: f64,
}

fn seg_dist(p: [f64; 2], s: &[f64; 4]) -> f64 {
    let (ax, ay, bx, by) = (s[0], s[1], s[2], s[3]);
    let (dx, dy) = (bx - ax, by - ay);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 {
        (((p[0] - ax) * dx + (p[1] - ay) * dy) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p[0] - ax - t * dx).hypot(p[1] - ay - t * dy)
}

impl Region {
    pub fn new(poly: MultiPolygon<f64>) -> Region {
        let mut segs = Vec::new();
        for p in &poly {
            for ring in std::iter::once(p.exterior()).chain(p.interiors()) {
                for l in ring.lines() {
                    segs.push([l.start.x, l.start.y, l.end.x, l.end.y]);
                }
            }
        }
        let r_out = segs.iter().map(|s| s[0].hypot(s[1])).fold(0.0, f64::max);
        let mut r = Region {
            poly,
            segs,
            r_in: 0.0,
            r_out,
        };
        if r.pip([0.0, 0.0]) {
            r.r_in = r.boundary_dist([0.0, 0.0]) * (1.0 - 1e-9);
        }
        r
    }

    /// Intersection avec un disque de rayon `radius` (réduction de zone).
    pub fn with_disk(&self, radius: f64) -> Region {
        Region::new(
            self.poly
                .intersection(&MultiPolygon::new(vec![disk(radius, 64)])),
        )
    }

    /// Pair-impair sur tous les anneaux.
    fn pip(&self, p: [f64; 2]) -> bool {
        let mut inside = false;
        for s in &self.segs {
            if (s[1] > p[1]) != (s[3] > p[1])
                && p[0] < (s[2] - s[0]) * (p[1] - s[1]) / (s[3] - s[1]) + s[0]
            {
                inside = !inside;
            }
        }
        inside
    }

    pub fn boundary_dist(&self, p: [f64; 2]) -> f64 {
        self.segs
            .iter()
            .map(|s| seg_dist(p, s))
            .fold(f64::INFINITY, f64::min)
    }

    pub fn contains(&self, p: [f64; 2]) -> bool {
        let r = p[0].hypot(p[1]);
        r < self.r_in || (r <= self.r_out && self.pip(p))
    }

    pub fn area(&self) -> f64 {
        self.poly.unsigned_area()
    }

    /// Rayon : plus grande distance d'un sommet au départ.
    pub fn radius(&self) -> f64 {
        self.r_out
    }

    /// [xmin, ymin, xmax, ymax] (repère local).
    pub fn bounds(&self) -> [f64; 4] {
        let mut b = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for s in &self.segs {
            b = [
                b[0].min(s[0]),
                b[1].min(s[1]),
                b[2].max(s[0]),
                b[3].max(s[1]),
            ];
        }
        b
    }

    /// Anneaux extérieurs.
    pub fn rings(&self) -> Vec<Vec<[f64; 2]>> {
        self.poly
            .iter()
            .map(|p| p.exterior().coords().map(|c| [c.x, c.y]).collect())
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Graphe
// ---------------------------------------------------------------------------

/// Morceau de tronçon : points de profil lo..=hi (lo < hi), parcourus de hi à lo si `rev`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Piece {
    pub t: u32,
    pub lo: u32,
    pub hi: u32,
    pub rev: bool,
}

impl Piece {
    fn flipped(self) -> Piece {
        Piece {
            rev: !self.rev,
            ..self
        }
    }
    /// (premier, dernier) indice de profil dans le sens de parcours.
    fn ends(&self) -> (u32, u32) {
        if self.rev {
            (self.hi, self.lo)
        } else {
            (self.lo, self.hi)
        }
    }
}

/// Fenêtre de la pente max (D30, D31) : 50 m, car la BD TOPO ne place un sommet que tous les
/// 25–55 m en montagne (lacets coupés → pointes de 80–97 % sur 25 m). Sert à la pente affichée
/// et au filtre `max_grade` des arêtes ; n'affecte pas le D+.
pub const GRADE_WINDOW_M: f64 = 50.0;

/// Pente max (fraction) sur une fenêtre glissante de `GRADE_WINDOW_M` ; profil plus court que la
/// fenêtre : pente moyenne |Δz| / L.
pub fn max_grade(z: &[f64], s: &[f64]) -> f64 {
    let l = s.last().copied().unwrap_or(0.0);
    if l < GRADE_WINDOW_M {
        return if l > 0.0 {
            (z[z.len() - 1] - z[0]).abs() / l
        } else {
            0.0
        };
    }
    let (mut g, mut j) = (0.0f64, 0);
    for i in 0..s.len() {
        while j < s.len() && s[j] < s[i] + GRADE_WINDOW_M {
            j += 1;
        }
        if j == s.len() {
            break;
        }
        g = g.max((z[j] - z[i]).abs() / (s[j] - s[i]));
    }
    g
}

/// Classes de voie (demande du fondateur 2026-10-07, D50) : chemin naturel, intermédiaire (voies
/// piétonnes, allées, escaliers), route, grand axe (route d'importance 1 à 3).
pub const CLS_TRAIL: usize = 0;
pub const CLS_MIXED: usize = 1;
pub const CLS_ROAD: usize = 2;
pub const CLS_MAJOR: usize = 3;
pub const N_CLS: usize = 4;
/// Étiquettes des dalles (demande 4, tiles.md § Étiquettes) cumulées par arête.
pub const LAB_NOISY: usize = 0;
pub const LAB_HIKE: usize = 1;
pub const LAB_WATER: usize = 2;
pub const N_LAB: usize = 3;

#[derive(Clone, Debug)]
pub struct Edge {
    pub u: usize,
    pub v: usize,
    pub pieces: Vec<Piece>,
    pub len: f64,
    /// (montée + descente) / 2.
    pub w: f64,
    /// Pente max (fraction) sur 50 m glissants du profil de l'arête (`max_grade`, D31).
    pub grade: f64,
    /// Pont ou tunnel.
    pub flat: bool,
    /// Longueur (m) par classe de voie (`Net::cls_t`, voir `CLS_*`).
    pub cls: [f64; N_CLS],
    /// Étiquettes (m, voir `LAB_*`) : longueur bruyante Σ len·(1 − calm/15), balisée, au bord de
    /// l'eau Σ len·osm_water/15 (colonnes absentes : 0).
    pub lab: [f64; N_LAB],
    /// Original dont cette arête est la copie (aller-retour d'accès près du départ).
    pub twin: Option<usize>,
}

pub struct Net<'a> {
    pub t: &'a Troncons,
    pub frame: Frame,
    /// Par tronçon : classe de voie (`CLS_*`), voir `plan::class_mask`. Vide : tout est route.
    pub cls_t: Vec<u8>,
    pub xy: Vec<[f64; 2]>,
    pub key: Vec<i64>,
    pub key_of: HashMap<i64, usize>,
    pub edges: Vec<Edge>,
}

/// Arête brute avant contraction : (u, v, morceaux, pont/tunnel).
type Raw = (usize, usize, Vec<Piece>, bool);

impl<'a> Net<'a> {
    pub fn new(t: &'a Troncons, frame: Frame) -> Net<'a> {
        Net {
            t,
            frame,
            cls_t: Vec::new(),
            xy: Vec::new(),
            key: Vec::new(),
            key_of: HashMap::new(),
            edges: Vec::new(),
        }
    }

    /// Nœud de clé `key` (créé à la position `pos` s'il n'existe pas).
    pub fn node(&mut self, key: i64, pos: [f64; 2]) -> usize {
        *self.key_of.entry(key).or_insert_with(|| {
            self.xy.push(pos);
            self.key.push(key);
            self.xy.len() - 1
        })
    }

    /// Nœud à une position L93 (m) quelconque : clé arrondie au dm.
    fn node_at_l93(&mut self, x: f64, y: f64) -> usize {
        let pos = self.frame.local(x, y);
        self.node(
            node_key((x * 10.0).round() as i64, (y * 10.0).round() as i64),
            pos,
        )
    }

    /// Points (repère local) et altitudes d'un morceau, dans le sens de parcours.
    pub fn piece_points(&self, p: &Piece) -> (Vec<[f64; 2]>, Vec<f64>) {
        let t = p.t as usize;
        let all = self.t.profile_xy(t);
        let r = p.lo as usize..=p.hi as usize;
        let mut xy: Vec<[f64; 2]> = all[r.clone()]
            .iter()
            .map(|&(x, y)| self.frame.local(x, y))
            .collect();
        let mut z: Vec<f64> = r.map(|i| self.t.z(t, i)).collect();
        if p.rev {
            xy.reverse();
            z.reverse();
        }
        (xy, z)
    }

    /// Géométrie d'une arête de u vers v (points de profil, un tous les 5 m) et altitudes.
    pub fn edge_points(&self, e: usize) -> (Vec<[f64; 2]>, Vec<f64>) {
        let (mut xy, mut z) = (Vec::new(), Vec::new());
        for p in &self.edges[e].pieces {
            let (a, b) = self.piece_points(p);
            let skip = usize::from(!xy.is_empty());
            xy.extend_from_slice(&a[skip..]);
            z.extend_from_slice(&b[skip..]);
        }
        (xy, z)
    }

    fn piece_stats(&self, p: &Piece) -> (f64, f64) {
        let t = p.t as usize;
        let (lo, hi) = (p.lo as usize, p.hi as usize);
        let len = (self.t.abscissa(t, hi) - self.t.abscissa(t, lo)) / self.frame.k;
        let updown = if lo == 0 && hi + 1 == self.t.n_profile(t) {
            (self.t.dplus_dm[t] + self.t.dminus_dm[t]) as f64
        } else {
            let z = &self.t.z_dm[self.t.poff[t] + lo..=self.t.poff[t] + hi];
            z.windows(2).map(|w| (w[1] - w[0]).abs() as f64).sum()
        };
        (len, updown / 20.0)
    }

    /// Ajoute une arête (statistiques calculées) ; renvoie son identifiant.
    pub fn add_edge(&mut self, u: usize, v: usize, pieces: Vec<Piece>, flat: bool) -> usize {
        let (mut len, mut w, mut cls, mut lab) = (0.0, 0.0, [0.0; N_CLS], [0.0; N_LAB]);
        for p in &pieces {
            let (l, x) = self.piece_stats(p);
            len += l;
            w += x;
            let t = p.t as usize;
            cls[self.cls_t.get(t).map_or(CLS_ROAD, |&c| c as usize)] += l;
            lab[LAB_NOISY] += l * (1.0 - f64::from(self.t.calm[t].min(15)) / 15.0);
            lab[LAB_HIKE] += if self.t.hike[t] > 0 { l } else { 0.0 };
            lab[LAB_WATER] += l * f64::from(self.t.water[t].min(15)) / 15.0;
        }
        self.edges.push(Edge {
            u,
            v,
            pieces,
            len,
            w,
            grade: 0.0,
            flat,
            cls,
            lab,
            twin: None,
        });
        let e = self.edges.len() - 1;
        let (xy, z) = self.edge_points(e);
        let mut s = vec![0.0; xy.len()];
        for i in 1..xy.len() {
            s[i] = s[i - 1] + (xy[i][0] - xy[i - 1][0]).hypot(xy[i][1] - xy[i - 1][1]);
        }
        self.edges[e].grade = max_grade(&z, &s);
        e
    }

    /// Arêtes de la zone pour les tronçons `sel` : découpe au bord de zone (aux points de
    /// profil, 5 m), puis contraction des nœuds de degré 2 (port de `base_edges`).
    pub fn build(&mut self, sel: &[usize], region: &Region) -> Vec<usize> {
        let (t, frame) = (self.t, self.frame);
        let rb = region.bounds();
        let mut raw: Vec<Raw> = Vec::new();
        for &i in sel {
            let b = t.bbox_dm(i);
            let lo = self.frame.local(b[0] as f64 / 10.0, b[1] as f64 / 10.0);
            let hi = self.frame.local(b[2] as f64 / 10.0, b[3] as f64 / 10.0);
            // boîte en L93 vs boîte locale : même orientation (repère translaté et mis à l'échelle)
            if hi[0] < rb[0] || lo[0] > rb[2] || hi[1] < rb[1] || lo[1] > rb[3] {
                continue;
            }
            let flat = t.flags[i] & 1 != 0;
            let pn = t.n_profile(i);
            let (ku, kv) = t.end_keys(i);
            let vert = |j: usize| {
                let (x, y) = t.vertex(i, j);
                frame.local(x, y)
            };
            let all_in = (0..t.n_vertices(i)).all(|j| region.contains(vert(j)));
            if all_in {
                let (pu, pv) = (vert(0), vert(t.n_vertices(i) - 1));
                let u = self.node(ku, pu);
                let v = self.node(kv, pv);
                raw.push((
                    u,
                    v,
                    vec![Piece {
                        t: i as u32,
                        lo: 0,
                        hi: pn as u32 - 1,
                        rev: false,
                    }],
                    flat,
                ));
                continue;
            }
            let pts = t.profile_xy(i);
            let inside: Vec<bool> = pts
                .iter()
                .map(|&(x, y)| region.contains(self.frame.local(x, y)))
                .collect();
            let mut a = 0;
            while a < pn {
                if !inside[a] {
                    a += 1;
                    continue;
                }
                let mut b = a;
                while b < pn && inside[b] {
                    b += 1;
                }
                if b - a >= 2 {
                    let u = if a == 0 {
                        self.node(ku, vert(0))
                    } else {
                        self.node_at_l93(pts[a].0, pts[a].1)
                    };
                    let v = if b == pn {
                        self.node(kv, vert(t.n_vertices(i) - 1))
                    } else {
                        self.node_at_l93(pts[b - 1].0, pts[b - 1].1)
                    };
                    let p = Piece {
                        t: i as u32,
                        lo: a as u32,
                        hi: b as u32 - 1,
                        rev: false,
                    };
                    raw.push((u, v, vec![p], flat));
                }
                a = b;
            }
        }
        self.contract(raw)
    }

    /// Fusionne les arêtes aux nœuds de degré 2, sauf pont/tunnel avec tronçon normal
    /// (port de `graph.contract_degree2`, même ordre de parcours des nœuds).
    fn contract(&mut self, raw: Vec<Raw>) -> Vec<usize> {
        let mut inc: Vec<Vec<usize>> = vec![Vec::new(); self.xy.len()];
        let mut order = Vec::new();
        for (i, (u, v, _, _)) in raw.iter().enumerate() {
            for n in [*u, *v] {
                if inc[n].is_empty() {
                    order.push(n);
                }
                inc[n].push(i);
            }
        }
        let mut e: Vec<Option<Raw>> = raw.into_iter().map(Some).collect();
        for n in order {
            if inc[n].len() != 2 || inc[n][0] == inc[n][1] {
                continue;
            }
            let (i, j) = (inc[n][0], inc[n][1]);
            if e[i].as_ref().unwrap().3 != e[j].as_ref().unwrap().3 {
                continue;
            }
            let (mut ui, mut vi, mut pi, fi) = e[i].take().unwrap();
            let (mut uj, mut vj, mut pj, _) = e[j].take().unwrap();
            if vi != n {
                (ui, vi) = (vi, ui);
                pi = pi.into_iter().rev().map(Piece::flipped).collect();
            }
            if uj != n {
                (uj, vj) = (vj, uj);
                pj = pj.into_iter().rev().map(Piece::flipped).collect();
            }
            debug_assert!(vi == n && uj == n);
            pi.extend(pj);
            e[i] = Some((ui, vj, pi, fi));
            inc[n].clear();
            if let Some(k) = inc[vj].iter().position(|&x| x == j) {
                inc[vj][k] = i;
            }
        }
        let mut ids = Vec::new();
        for (u, v, p, f) in e.into_iter().flatten() {
            let k = self.add_edge(u, v, p, f);
            if self.edges[k].len > 1e-9 {
                ids.push(k);
            }
        }
        ids
    }

    /// Distance max d'une arête à `c` (points de profil), arrêt dès qu'elle dépasse `limit`.
    fn max_dist_over(&self, e: usize, c: [f64; 2], limit: f64) -> f64 {
        let ed = &self.edges[e];
        let far = |p: [f64; 2]| (p[0] - c[0]).hypot(p[1] - c[1]);
        let m = far(self.xy[ed.u]).max(far(self.xy[ed.v]));
        if m > limit {
            return m;
        }
        let (xy, _) = self.edge_points(e);
        xy.into_iter().map(far).fold(m, f64::max)
    }

    /// Distance min d'une arête à l'origine (points de profil).
    pub fn min_dist(&self, e: usize) -> f64 {
        let (xy, _) = self.edge_points(e);
        xy.iter()
            .map(|p| p[0].hypot(p[1]))
            .fold(f64::INFINITY, f64::min)
    }

    /// Distance de l'origine au sommet le plus proche de `ids` (sommets bruts des tronçons
    /// entiers, points de profil des morceaux coupés) : comme `build_candidate` en Python,
    /// qui mesure aux sommets et non aux segments (un point à 28 m d'un long segment droit
    /// peut être à plus de 30 m de ses sommets et déclencher la recherche d'accès).
    pub fn nearest_vertex(&self, ids: &[usize]) -> f64 {
        let mut best = f64::INFINITY;
        for &e in ids {
            for p in &self.edges[e].pieces {
                let t = p.t as usize;
                if p.lo == 0 && p.hi as usize + 1 == self.t.n_profile(t) {
                    for j in 0..self.t.n_vertices(t) {
                        let (x, y) = self.t.vertex(t, j);
                        let q = self.frame.local(x, y);
                        best = best.min(q[0].hypot(q[1]));
                    }
                } else {
                    let (xy, _) = self.piece_points(p);
                    best = xy.iter().map(|q| q[0].hypot(q[1])).fold(best, f64::min);
                }
            }
        }
        best
    }

    /// Point de `ids` le plus proche de `point` : (distance à la corde, rang dans `ids`,
    /// morceau, indice du point de profil le plus proche sur cette corde).
    pub fn locate(&self, ids: &[usize], point: [f64; 2]) -> Option<(f64, usize, usize, u32)> {
        let t = self.t;
        let mut best: Option<(f64, usize, usize, u32)> = None; // (d, rang dans ids, morceau, indice)
        for (r, &e) in ids.iter().enumerate() {
            for (pi, p) in self.edges[e].pieces.iter().enumerate() {
                let i = p.t as usize;
                let b = t.bbox_dm(i);
                let lo = self.frame.local(b[0] as f64 / 10.0, b[1] as f64 / 10.0);
                let hi = self.frame.local(b[2] as f64 / 10.0, b[3] as f64 / 10.0);
                let dx = (lo[0] - point[0]).max(point[0] - hi[0]).max(0.0);
                let dy = (lo[1] - point[1]).max(point[1] - hi[1]).max(0.0);
                if best.is_some_and(|bb| dx.hypot(dy) > bb.0 + 5.0) {
                    continue;
                }
                let pts: Vec<[f64; 2]> = t.profile_xy(i)[p.lo as usize..=p.hi as usize]
                    .iter()
                    .map(|&(x, y)| self.frame.local(x, y))
                    .collect();
                for j in 0..pts.len() - 1 {
                    let s = [pts[j][0], pts[j][1], pts[j + 1][0], pts[j + 1][1]];
                    let (ax, ay) = (s[2] - s[0], s[3] - s[1]);
                    let l2 = ax * ax + ay * ay;
                    let f = if l2 > 0.0 {
                        (((point[0] - s[0]) * ax + (point[1] - s[1]) * ay) / l2).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    let d = seg_dist(point, &s);
                    if best.is_none_or(|bb| d < bb.0) {
                        let k = p.lo + j as u32 + u32::from(f >= 0.5);
                        best = Some((d, r, pi, k));
                    }
                }
            }
        }
        best
    }

    /// Insère un nœud au point de `ids` le plus proche de `point` (projection sur les cordes
    /// du profil, coupe au point de profil le plus proche, 2,5 m au plus) ; `ids` est mis à
    /// jour (l'arête coupée est remplacée par ses deux moitiés, l'original reste dans l'arène).
    /// Renvoie (nœud, distance de `point` au nœud, position du nœud).
    pub fn insert_start(
        &mut self,
        ids: &mut Vec<usize>,
        point: [f64; 2],
    ) -> Option<(usize, f64, [f64; 2])> {
        let (_, r, pi, k) = self.locate(ids, point)?;
        let e = ids[r];
        let ed = self.edges[e].clone();
        let p = ed.pieces[pi];
        let (first, last) = p.ends();
        let n_p = ed.pieces.len();
        let pos_of = |net: &Net, k: u32| {
            let (x, y) = net.t.profile_xy(p.t as usize)[k as usize];
            (x, y)
        };
        let (x, y) = pos_of(self, k);
        let pos = self.frame.local(x, y);
        let snap = (pos[0] - point[0]).hypot(pos[1] - point[1]);
        if pi == 0 && k == first {
            return Some((ed.u, snap, self.xy[ed.u]));
        }
        if pi == n_p - 1 && k == last {
            return Some((ed.v, snap, self.xy[ed.v]));
        }
        let node = self.node_at_l93(x, y);
        let (mut a, mut b): (Vec<Piece>, Vec<Piece>);
        if k == first {
            a = ed.pieces[..pi].to_vec();
            b = ed.pieces[pi..].to_vec();
        } else if k == last {
            a = ed.pieces[..=pi].to_vec();
            b = ed.pieces[pi + 1..].to_vec();
        } else {
            a = ed.pieces[..pi].to_vec();
            b = ed.pieces[pi + 1..].to_vec();
            let (p1, p2) = if p.rev {
                (Piece { lo: k, ..p }, Piece { hi: k, ..p })
            } else {
                (Piece { hi: k, ..p }, Piece { lo: k, ..p })
            };
            a.push(p1);
            b.insert(0, p2);
        }
        let e1 = self.add_edge(ed.u, node, a, ed.flat);
        let e2 = self.add_edge(node, ed.v, b, ed.flat);
        ids[r] = e1;
        ids.push(e2);
        Some((node, snap, self.xy[node]))
    }

    /// Passages uniques (demande du fondateur, 2026-10-07) : double, pour l'aller-retour, les
    /// isthmes du réseau (ponts au sens des graphes, après effeuillage des culs-de-sac, `s`
    /// protégé : seul accès à une partie bouclable, pont sur une rivière, vallée à accès unique,
    /// col à sentier unique) et les ponts et tunnels des dalles (`flat`) entre deux carrefours. Le rayon libre du départ
    /// en était le cas particulier (port de `duplicate_near_start`). Renvoie (copies à moins de
    /// `radius` de `center`, copies en tout, longueur copiée en m).
    pub fn duplicate_unique(
        &mut self,
        ids: &mut Vec<usize>,
        center: [f64; 2],
        s: usize,
        radius: f64,
    ) -> (usize, usize, f64) {
        let mut forced = access_bridges(self, ids, s);
        let mut deg = vec![0u32; self.xy.len()];
        for &e in ids.iter() {
            deg[self.edges[e].u] += 1;
            deg[self.edges[e].v] += 1;
        }
        // déjà doublées (2e passage après le filtre de pente) : ni la copie ni l'original
        let done: HashSet<usize> = ids.iter().filter_map(|&e| self.edges[e].twin).collect();
        let free = |e: usize| self.edges[e].twin.is_none() && !done.contains(&e);
        // pont ou tunnel entre deux carrefours. Doubler aussi ses voies d'approche (bout de degré 2)
        // a été essayé (2026-10-07) : 700 copies à Massy, recherche dégradée (D+ max 366 → 113–346 m)
        for &e in ids.iter() {
            let ed = &self.edges[e];
            if ed.flat && free(e) && deg[ed.u] > 2 && deg[ed.v] > 2 {
                forced.insert(e);
            }
        }
        let mut pick: Vec<usize> = ids
            .iter()
            .copied()
            .filter(|&e| free(e) && forced.contains(&e))
            .collect();
        pick.sort_unstable();
        let (mut near, mut km) = (0, 0.0);
        for &e in &pick {
            if self.max_dist_over(e, center, radius) <= radius {
                near += 1;
            }
            km += self.edges[e].len;
            let mut c = self.edges[e].clone();
            c.twin = Some(e);
            self.edges.push(c);
            ids.push(self.edges.len() - 1);
        }
        (near, pick.len(), km)
    }

    pub fn total_len(&self, ids: &[usize]) -> f64 {
        ids.iter().map(|&e| self.edges[e].len).sum()
    }
}

// ---------------------------------------------------------------------------
// Algorithmes de graphe sur un sous-ensemble d'arêtes
// ---------------------------------------------------------------------------

/// Listes d'adjacence (CSR) des nœuds de l'arène, pour les arêtes `ids`.
pub struct Adj {
    start: Vec<u32>,
    list: Vec<(u32, u32)>,
}

impl Adj {
    pub fn new(net: &Net, ids: &[usize]) -> Adj {
        let n = net.xy.len();
        let mut start = vec![0u32; n + 1];
        for &e in ids {
            let ed = &net.edges[e];
            start[ed.u + 1] += 1;
            if ed.u != ed.v {
                start[ed.v + 1] += 1;
            }
        }
        for i in 0..n {
            start[i + 1] += start[i];
        }
        let mut fill = start.clone();
        let mut list = vec![(0, 0); start[n] as usize];
        for &e in ids {
            let ed = &net.edges[e];
            list[fill[ed.u] as usize] = (e as u32, ed.v as u32);
            fill[ed.u] += 1;
            if ed.u != ed.v {
                list[fill[ed.v] as usize] = (e as u32, ed.u as u32);
                fill[ed.v] += 1;
            }
        }
        Adj { start, list }
    }

    pub fn of(&self, n: usize) -> &[(u32, u32)] {
        &self.list[self.start[n] as usize..self.start[n + 1] as usize]
    }

    pub fn n(&self) -> usize {
        self.start.len() - 1
    }

    /// Nœuds présents, dans l'ordre de l'arène.
    pub fn nodes(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.n()).filter(|&i| self.start[i + 1] > self.start[i])
    }
}

/// Dijkstra depuis `src` : (distances, prédécesseur (arête, nœud)) ; arrêt à `target`.
pub fn shortest(
    adj: &Adj,
    cost: impl Fn(usize) -> f64,
    src: usize,
    target: Option<usize>,
) -> (Vec<f64>, Vec<Option<(usize, usize)>>) {
    let n = adj.n();
    let mut dist = vec![f64::INFINITY; n];
    let mut prev = vec![None; n];
    let mut pq = BinaryHeap::new();
    dist[src] = 0.0;
    pq.push(Reverse((0u64, src)));
    while let Some(Reverse((d, x))) = pq.pop() {
        let d = f64::from_bits(d);
        if d > dist[x] {
            continue;
        }
        if Some(x) == target {
            break;
        }
        for &(e, nb) in adj.of(x) {
            let (e, nb) = (e as usize, nb as usize);
            // garde : coût négatif ou NaN compté 0 (voir `faces::shortest`)
            let c = cost(e);
            let nd = d + if c > 0.0 { c } else { 0.0 };
            if nd < dist[nb] {
                dist[nb] = nd;
                prev[nb] = Some((e, x));
                pq.push(Reverse((nd.to_bits(), nb)));
            }
        }
    }
    (dist, prev)
}

/// Chemin [(arête, de, vers)] de `src` à `tgt`.
pub fn path_from(
    prev: &[Option<(usize, usize)>],
    src: usize,
    tgt: usize,
) -> Vec<(usize, usize, usize)> {
    let (mut out, mut n) = (Vec::new(), tgt);
    while n != src {
        let (e, p) = prev[n].expect("chemin");
        out.push((e, p, n));
        n = p;
    }
    out.reverse();
    out
}

/// Ponts (Tarjan itératif ; gère multi-arêtes et boucles).
pub fn find_bridges(adj: &Adj) -> HashSet<usize> {
    const NONE: u32 = u32::MAX;
    let n = adj.n();
    let (mut disc, mut low) = (vec![NONE; n], vec![0u32; n]);
    let mut bridges = HashSet::new();
    let mut t = 0u32;
    for root in adj.nodes() {
        if disc[root] != NONE {
            continue;
        }
        disc[root] = t;
        low[root] = t;
        t += 1;
        // (nœud, arête d'arrivée, position dans la liste)
        let mut stack: Vec<(usize, usize, usize)> = vec![(root, usize::MAX, 0)];
        while let Some(&mut (node, pe, ref mut it)) = stack.last_mut() {
            let lst = adj.of(node);
            let mut pushed = None;
            while *it < lst.len() {
                let (e, nb) = (lst[*it].0 as usize, lst[*it].1 as usize);
                *it += 1;
                if e == pe || nb == node {
                    continue;
                }
                if disc[nb] != NONE {
                    low[node] = low[node].min(disc[nb]);
                } else {
                    disc[nb] = t;
                    low[nb] = t;
                    t += 1;
                    pushed = Some((nb, e));
                    break;
                }
            }
            if let Some((nb, e)) = pushed {
                stack.push((nb, e, 0));
                continue;
            }
            stack.pop();
            if let Some(&(parent, _, _)) = stack.last() {
                low[parent] = low[parent].min(low[node]);
                if low[node] > disc[parent] {
                    bridges.insert(pe);
                }
            }
        }
    }
    bridges
}

/// Ponts d'accès : ceux qui restent après effeuillage des culs-de-sac, `s` protégé.
pub fn access_bridges(net: &Net, ids: &[usize], s: usize) -> HashSet<usize> {
    let adj = Adj::new(net, ids);
    let bridges = find_bridges(&adj);
    let mut deg: Vec<usize> = (0..adj.n()).map(|n| adj.of(n).len()).collect();
    let mut alive: HashSet<usize> = ids.iter().copied().collect();
    let mut leaves: Vec<usize> = adj.nodes().filter(|&n| deg[n] == 1 && n != s).collect();
    while let Some(n) = leaves.pop() {
        if deg[n] != 1 || n == s {
            continue;
        }
        for &(e, nb) in adj.of(n) {
            let (e, nb) = (e as usize, nb as usize);
            if alive.remove(&e) {
                deg[n] -= 1;
                deg[nb] -= 1;
                if deg[nb] == 1 && nb != s {
                    leaves.push(nb);
                }
                break;
            }
        }
    }
    bridges.into_iter().filter(|e| alive.contains(e)).collect()
}

/// Jusqu'à stabilité : retrait des ponts, composante de `s`, filtre de portée
/// d(s,u) + l + d(v,s) <= Lmax.
pub fn prune(net: &Net, mut ids: Vec<usize>, s: usize, lmax: f64) -> Vec<usize> {
    loop {
        let m0 = ids.len();
        let br = find_bridges(&Adj::new(net, &ids));
        ids.retain(|e| !br.contains(e));
        let adj = Adj::new(net, &ids);
        if s >= adj.n() || adj.of(s).is_empty() {
            return Vec::new();
        }
        let mut seen = vec![false; adj.n()];
        let mut st = vec![s];
        seen[s] = true;
        while let Some(x) = st.pop() {
            for &(_, nb) in adj.of(x) {
                if !seen[nb as usize] {
                    seen[nb as usize] = true;
                    st.push(nb as usize);
                }
            }
        }
        ids.retain(|&e| seen[net.edges[e].u]);
        let adj = Adj::new(net, &ids);
        let (dist, _) = shortest(&adj, |e| net.edges[e].len, s, None);
        ids.retain(|&e| {
            let ed = &net.edges[e];
            dist[ed.u] + ed.len + dist[ed.v] <= lmax
        });
        if ids.len() == m0 {
            return ids;
        }
    }
}

/// reach (aligné sur `ids`) : d(s,u) + len + d(v,s), longueur minimale d'une boucle qui
/// emprunte l'arête.
pub fn reach(net: &Net, ids: &[usize], s: usize) -> Vec<f64> {
    let adj = Adj::new(net, ids);
    let (d, _) = shortest(&adj, |e| net.edges[e].len, s, None);
    ids.iter()
        .map(|&e| {
            let ed = &net.edges[e];
            d[ed.u] + ed.len + d[ed.v]
        })
        .collect()
}

/// Composantes 2-arête-connexes (après retrait des ponts).
pub fn loop_components(net: &Net, ids: &[usize]) -> Vec<Vec<usize>> {
    let br = find_bridges(&Adj::new(net, ids));
    let h: Vec<usize> = ids.iter().copied().filter(|e| !br.contains(e)).collect();
    let adj = Adj::new(net, &h);
    let mut label = vec![usize::MAX; adj.n()];
    for root in adj.nodes() {
        if label[root] != usize::MAX {
            continue;
        }
        label[root] = root;
        let mut st = vec![root];
        while let Some(x) = st.pop() {
            for &(_, nb) in adj.of(x) {
                if label[nb as usize] == usize::MAX {
                    label[nb as usize] = root;
                    st.push(nb as usize);
                }
            }
        }
    }
    let mut comps: Vec<Vec<usize>> = Vec::new();
    let mut slot: HashMap<usize, usize> = HashMap::new();
    for e in h {
        let l = label[net.edges[e].u];
        let k = *slot.entry(l).or_insert_with(|| {
            comps.push(Vec::new());
            comps.len() - 1
        });
        comps[k].push(e);
    }
    comps
}

/// HEURISTIQUE (port de `steep_reduction`) : garde les arêtes les plus pentues (w/l
/// décroissant) jusqu'à k × Lmax de longueur, plus leurs chemins vers `s` dans deux arbres de
/// plus courts chemins (le 2e pénalise les arêtes du 1er) ; une arête doublée (passage unique)
/// est gardée avec sa copie. `w` exact (profils des dalles) : pas de criblage grossier.
/// `via` : nœuds à garder (points de passage, T34) : leurs arêtes et leurs chemins vers `s`.
/// `weight` (préférence de type de voie) : poids de recherche d'une arête (D+ plus prime) ; aux
/// k × Lmax des plus pentues s'ajoutent alors les meilleures en `weight`/l, jusqu'à `REDUCE_SURF`
/// × k × Lmax en tout. Un seul classement retirerait soit les chemins plats, soit les routes qui
/// portent le D+ (Massy).
pub fn steep_reduction(
    net: &Net,
    ids: &[usize],
    s: usize,
    lmax: f64,
    k: f64,
    via: &[usize],
    weight: Option<&dyn Fn(&Edge) -> f64>,
) -> Vec<usize> {
    let e = |i: usize| &net.edges[ids[i]];
    let mut keep = vec![false; ids.len()];
    let mut cum = 0.0;
    let steep: &dyn Fn(&Edge) -> f64 = &|ed| ed.w;
    for (rank, share) in std::iter::once((steep, 1.0)).chain(weight.map(|w| (w, REDUCE_SURF))) {
        let mut order: Vec<usize> = (0..ids.len()).collect();
        order.sort_by(|&a, &b| {
            let ra = rank(e(a)) / e(a).len.max(1.0);
            let rb = rank(e(b)) / e(b).len.max(1.0);
            rb.total_cmp(&ra)
        });
        for i in order {
            if cum >= share * k * lmax {
                break;
            }
            // une copie (passage unique) suit son original plus bas : ne compte pas deux fois
            if !keep[i] && e(i).twin.is_none() {
                keep[i] = true;
                cum += e(i).len;
            }
        }
    }
    for (i, k) in keep.iter_mut().enumerate() {
        *k |= via.contains(&e(i).u) || via.contains(&e(i).v);
    }
    let pos: HashMap<usize, usize> = ids.iter().enumerate().map(|(i, &x)| (x, i)).collect();
    let adj = Adj::new(net, ids);
    let mut targets: Vec<usize> = (0..ids.len())
        .filter(|&i| keep[i])
        .flat_map(|i| [e(i).u, e(i).v])
        .collect();
    targets.sort_unstable();
    targets.dedup();
    let mut cost: Vec<f64> = (0..ids.len()).map(|i| e(i).len).collect();
    for _ in 0..2 {
        let (_, prev) = shortest(&adj, |x| cost[pos[&x]].max(1e-9), s, None);
        let mut mark = vec![false; adj.n()];
        mark[s] = true;
        let mut tree = Vec::new();
        for &t in &targets {
            let mut n = t;
            while !mark[n] {
                let Some((x, p)) = prev[n] else { break };
                mark[n] = true;
                tree.push(pos[&x]);
                n = p;
            }
        }
        for i in tree {
            keep[i] = true;
            cost[i] *= 3.0;
            // le 2e arbre doit quitter `s` par un autre côté : sinon le départ reste sur une tige
            // (un pont), retirée par l'élagage, et le réseau réduit est vide
            if e(i).u == s || e(i).v == s {
                cost[i] += 1e9;
            }
        }
    }
    // points de passage : deux chemins sans arête commune s -> via (sinon la tige serait un pont,
    // retiré par l'élagage et le point perdu)
    for &v in via {
        let mut cost: Vec<f64> = (0..ids.len()).map(|i| e(i).len).collect();
        for _ in 0..2 {
            let (_, prev) = shortest(&adj, |x| cost[pos[&x]].max(1e-9), s, None);
            let mut n = v;
            let mut seen = HashSet::new();
            while n != s && seen.insert(n) {
                let Some((x, p)) = prev[n] else { break };
                keep[pos[&x]] = true;
                cost[pos[&x]] += 1e9;
                n = p;
            }
        }
    }
    // passages uniques : la copie suit l'original et inversement (les arbres depuis `s` gardent
    // ceux du chemin d'accès)
    for i in 0..ids.len() {
        if let Some(&j) = e(i).twin.and_then(|o| pos.get(&o)) {
            let k = keep[i] || keep[j];
            (keep[i], keep[j]) = (k, k);
        }
    }
    (0..ids.len())
        .filter(|&i| keep[i])
        .map(|i| ids[i])
        .collect()
}

/// Angle (rad) de la première direction de l'arête depuis u (`from_u`) ou depuis v.
pub fn edge_angle(net: &Net, e: usize, from_u: bool) -> f64 {
    let ed = &net.edges[e];
    let p = if from_u {
        ed.pieces[0]
    } else {
        ed.pieces[ed.pieces.len() - 1].flipped()
    };
    let t = p.t as usize;
    let (k, _) = p.ends();
    let (k, nv) = (k as usize, net.t.n_vertices(t));
    let pt = |j: usize| net.t.vertex(t, j);
    // extrémité du tronçon : premier sommet distinct (comme le premier segment en Python)
    let (a, b) = if k == 0 {
        let a = pt(0);
        (a, (1..nv).map(pt).find(|&b| b != a).unwrap_or(pt(nv - 1)))
    } else if k + 1 == net.t.n_profile(t) {
        let a = pt(nv - 1);
        (
            a,
            (0..nv - 1).rev().map(pt).find(|&b| b != a).unwrap_or(pt(0)),
        )
    } else {
        let all = net.t.profile_xy(t);
        let nxt = if p.rev { k - 1 } else { k + 1 };
        (all[k], all[nxt])
    };
    (b.1 - a.1).atan2(b.0 - a.0)
}

/// Couloirs parallèles : union des parallèles des tronçons (contrat tiles/1), entre arêtes
/// non copiées, d'au moins PARALLEL_MIN_LEN, pas entièrement dans le rayon libre de `c`.
pub fn parallel_pairs(net: &Net, ids: &[usize], c: [f64; 2]) -> Vec<[usize; 2]> {
    let ok: Vec<usize> = (0..ids.len())
        .filter(|&i| {
            let e = &net.edges[ids[i]];
            e.twin.is_none()
                && e.len >= PARALLEL_MIN_LEN
                && net.max_dist_over(ids[i], c, FREE_RADIUS) > FREE_RADIUS
        })
        .collect();
    let mut by_t: HashMap<u32, Vec<usize>> = HashMap::new();
    for &i in &ok {
        for p in &net.edges[ids[i]].pieces {
            by_t.entry(p.t).or_default().push(i);
        }
    }
    let id_t: HashMap<i64, u32> = by_t.keys().map(|&t| (net.t.id[t as usize], t)).collect();
    let mut out = HashSet::new();
    for &a in &ok {
        for p in &net.edges[ids[a]].pieces {
            let t = p.t as usize;
            for pid in &net.t.par_id[net.t.par_off[t]..net.t.par_off[t + 1]] {
                let Some(t2) = id_t.get(pid) else { continue };
                for &b in &by_t[t2] {
                    if a != b {
                        out.insert([a.min(b), a.max(b)]);
                    }
                }
            }
        }
    }
    let mut v: Vec<[usize; 2]> = out.into_iter().collect();
    v.sort_unstable();
    v
}

/// D34 (Croix-Rousse) : paires d'arêtes qui se croisent en plan SANS nœud commun (pont, tunnel,
/// passage dessous : Grande-Côte sous la rue Burdeau). Une boucle qui prend les deux passe deux
/// fois au même endroit : en carrefours uniques, on les traite comme des couloirs parallèles
/// (jamais ensemble). Croisement à moins de FREE_RADIUS de `c` ignoré (comme les carrefours).
pub fn crossing_pairs(net: &Net, ids: &[usize], c: [f64; 2]) -> Vec<[usize; 2]> {
    const CELL: f64 = 25.0;
    let segs: Vec<(usize, [f64; 2], [f64; 2])> = ids
        .iter()
        .enumerate()
        .filter(|&(_, &e)| net.edges[e].twin.is_none())
        .flat_map(|(i, &e)| {
            let (xy, _) = net.edge_points(e);
            xy.windows(2).map(|w| (i, w[0], w[1])).collect::<Vec<_>>()
        })
        .collect();
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (k, &(_, a, b)) in segs.iter().enumerate() {
        let cell = |x: f64| (x / CELL).floor() as i64;
        for gx in cell(a[0].min(b[0]))..=cell(a[0].max(b[0])) {
            for gy in cell(a[1].min(b[1]))..=cell(a[1].max(b[1])) {
                grid.entry((gx, gy)).or_default().push(k);
            }
        }
    }
    // intersection propre (hors extrémités) de [p, q] et [r, s]
    let cross = |p: [f64; 2], q: [f64; 2], r: [f64; 2], s: [f64; 2]| {
        let d = (q[0] - p[0]) * (s[1] - r[1]) - (q[1] - p[1]) * (s[0] - r[0]);
        if d.abs() < 1e-9 {
            return None;
        }
        let t = ((r[0] - p[0]) * (s[1] - r[1]) - (r[1] - p[1]) * (s[0] - r[0])) / d;
        let u = ((r[0] - p[0]) * (q[1] - p[1]) - (r[1] - p[1]) * (q[0] - p[0])) / d;
        const E: f64 = 1e-6;
        (E < t && t < 1.0 - E && E < u && u < 1.0 - E)
            .then(|| [p[0] + t * (q[0] - p[0]), p[1] + t * (q[1] - p[1])])
    };
    let mut out = HashSet::new();
    for cell in grid.values() {
        for (x, &k1) in cell.iter().enumerate() {
            for &k2 in &cell[x + 1..] {
                let ((i, p, q), (j, r, s)) = (segs[k1], segs[k2]);
                if i == j || out.contains(&[i.min(j), i.max(j)]) {
                    continue;
                }
                if let Some(m) = cross(p, q, r, s)
                    && (m[0] - c[0]).hypot(m[1] - c[1]) > FREE_RADIUS
                {
                    out.insert([i.min(j), i.max(j)]);
                }
            }
        }
    }
    let mut v: Vec<[usize; 2]> = out.into_iter().collect();
    v.sort_unstable();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_contains_and_area() {
        let r = Region::new(MultiPolygon::new(vec![disk(1000.0, 64)]));
        assert!(r.contains([0.0, 0.0]) && r.contains([999.0, 0.0]) && !r.contains([1001.0, 0.0]));
        assert!((r.area() - std::f64::consts::PI * 1e6).abs() < 1e3);
        let sq = Polygon::new(
            LineString::from(vec![
                (-10.0, -10.0),
                (500.0, -10.0),
                (500.0, 500.0),
                (-10.0, 500.0),
                (-10.0, -10.0),
            ]),
            vec![],
        );
        let r2 = Region::new(sq.intersection(&disk(300.0, 64)));
        assert!(
            r2.contains([100.0, 100.0])
                && !r2.contains([-50.0, 0.0])
                && !r2.contains([290.0, 290.0])
        );
    }

    #[test]
    fn bridges_and_access_bridges() {
        // s=0 -1- 1 -2- 2, triangle 2-3-4, impasse 1-9, impasse 4-8 (test_access_bridges_ignore_dead_ends)
        let mut t = Troncons::new();
        let pos = |n: i64| [n * 100, (n % 3) * 50];
        let pairs = [(0, 1), (1, 2), (2, 3), (3, 4), (4, 2), (1, 9), (4, 8)];
        for (i, &(a, b)) in pairs.iter().enumerate() {
            t.push(i as i64, &[pos(a), pos(b)], |_, _| 0.0, 9, 0, 2, &[]);
        }
        let mut net = Net::new(&t, Frame::at_l93(0.0, 0.0));
        let all: Vec<usize> = (0..t.len()).collect();
        let big = Region::new(MultiPolygon::new(vec![disk(1e5, 16)]));
        // pas de contraction ici : les nœuds 3 (degré 2) fusionneraient 2-3 et 3-4
        let raw: Vec<usize> = all
            .iter()
            .map(|&i| {
                let (ku, kv) = t.end_keys(i);
                let (x0, y0) = t.vertex(i, 0);
                let (x1, y1) = t.vertex(i, 1);
                let u = net.node(ku, [x0, y0]);
                let v = net.node(kv, [x1, y1]);
                net.add_edge(
                    u,
                    v,
                    vec![Piece {
                        t: i as u32,
                        lo: 0,
                        hi: t.n_profile(i) as u32 - 1,
                        rev: false,
                    }],
                    false,
                )
            })
            .collect();
        let s = net.key_of[&node_key(0, 0)];
        let ab: HashSet<usize> = access_bridges(&net, &raw, s);
        assert_eq!(ab, HashSet::from([raw[0], raw[1]]));
        let br = find_bridges(&Adj::new(&net, &raw));
        assert_eq!(br, HashSet::from([raw[0], raw[1], raw[5], raw[6]]));
        // contraction : 3 disparaît (2-3-4 -> une arête), 9 et 8 restent des feuilles
        let ids = net.build(&all, &big);
        assert_eq!(ids.len(), 6);
    }
}
