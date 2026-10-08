//! Recuit « par faces » pour les grands graphes (port de `trailopt/solvers/faces.py`).
//!
//! Idée (Gemsa, Pajor, Wagner, Zündorf, « Efficient Computation of Jogging Routes », SEA 2013) :
//! une boucle est un élément de l'espace des cycles. On la modifie par différence symétrique avec
//! une face du graphe (un « pâté de maisons ») : les degrés restent pairs par construction, sans
//! aucun plus court chemin. Un mouvement coûte O(taille de la face).
//!
//! Différences avec Python : budget en itérations (déterministe) au lieu de secondes ; chaque
//! sonde a son propre générateur, ce qui permet de lancer les sondes en parallèle (rayon) ;
//! pas de repli « boucle par waypoints » (recuit classique, non porté) ni de `alternates`.
use std::cmp::{Ordering, Reverse};
use std::collections::{BinaryHeap, HashMap};

use rayon::prelude::*;

use std::time::Instant;

use crate::{Annealer, Problem, Rng};

const INF: f64 = f64::INFINITY;
/// Mode cible : erreur relative -> unités comparables à des mètres de D+.
pub(crate) const TARGET_SCALE: f64 = 1000.0;
/// Pénalité par mètre au-delà de Lmax (mode max).
const PEN: f64 = 0.5;
/// Mode min_distance (modes.md A) : pénalité du D+ manquant et du dépassement de Lcap, et
/// consigne de l'asservissement de λ (un peu au-dessus de X).
const MD_PEN: f64 = 3.0;
const MD_MARGIN: f64 = 0.03;
/// λ initial : mode min_distance, autres modes.
const LAM0_MD: f64 = 0.05;
const LAM0: f64 = 0.02;
/// D40 : prix du mètre au-delà de la distance demandée, en multiples de λ (mode max). Plaine : la
/// boucle revient à la distance demandée (+0 % au lieu de +4,9 %) pour −3 à −6 % de D+.
const LEN_PRICE: f64 = 4.0;
/// D52 : nombre de boucles « pétales » proposées comme départs (`FaceSearch::petals`).
const PETALS: usize = 3;
/// D52 : poids de la direction dans l'ordre des départs d'`alternates` (180° d'écart avec les sorties
/// gardées valent DIR_W de recouvrement en moins).
const DIR_W: f64 = 0.3;
/// Points de passage (T34) : pénalité de score par point manqué pendant le recuit.
const VIA_PEN: f64 = 2000.0;

/// Pas d'une face : (arête, de, vers).
pub type Step = (usize, usize, usize);

/// Faces du plongement donné par l'ordre circulaire des arêtes autour de chaque nœud.
/// Chaque face est une marche fermée ; on garde les cycles simples de longueur <= max_len.
/// Renvoie (faces, faces de chaque arête).
pub fn build_faces(
    u: &[usize],
    v: &[usize],
    ang_u: &[f64],
    ang_v: &[f64],
    len: &[f64],
    max_len: f64,
) -> (Vec<Vec<Step>>, Vec<Vec<usize>>) {
    let m = u.len();
    // Demi-arête h : arête h >> 1, de u vers v si h est pair, de v vers u sinon.
    let tail: Vec<usize> = (0..2 * m)
        .map(|h| if h & 1 == 0 { u[h >> 1] } else { v[h >> 1] })
        .collect();
    let ang: Vec<f64> = (0..2 * m)
        .map(|h| {
            if h & 1 == 0 {
                ang_u[h >> 1]
            } else {
                ang_v[h >> 1]
            }
        })
        .collect();
    let mut order: Vec<usize> = (0..2 * m).collect();
    order.sort_by(|&a, &b| tail[a].cmp(&tail[b]).then(ang[a].total_cmp(&ang[b])));
    // nxt_out[h] : demi-arête sortante suivante autour de tail[h], par angle croissant (cyclique).
    let mut nxt_out = vec![0; 2 * m];
    let mut i = 0;
    while i < 2 * m {
        let mut j = i;
        while j < 2 * m && tail[order[j]] == tail[order[i]] {
            j += 1;
        }
        for k in i..j {
            nxt_out[order[k]] = order[if k + 1 < j { k + 1 } else { i }];
        }
        i = j;
    }
    let mut seen = vec![false; 2 * m];
    let (mut faces, mut foe) = (Vec::new(), vec![Vec::new(); m]);
    let mut walk = Vec::new();
    for h0 in 0..2 * m {
        if seen[h0] {
            continue;
        }
        walk.clear();
        let (mut h, mut total) = (h0, 0.0);
        while !seen[h] {
            seen[h] = true;
            walk.push(h);
            total += len[h >> 1];
            h = nxt_out[h ^ 1]; // demi-arête suivante sur la face
        }
        if total > max_len {
            continue;
        }
        let distinct = |mut x: Vec<usize>| {
            x.sort_unstable();
            x.windows(2).all(|p| p[0] != p[1])
        };
        if !distinct(walk.iter().map(|&h| tail[h]).collect())
            || !distinct(walk.iter().map(|&h| h >> 1).collect())
        {
            continue; // marche non simple : ignorée
        }
        let fid = faces.len();
        faces.push(
            walk.iter()
                .map(|&h| (h >> 1, tail[h], tail[h ^ 1]))
                .collect(),
        );
        for &h in &walk {
            foe[h >> 1].push(fid);
        }
    }
    (faces, foe)
}

/// Dijkstra depuis `src` (arrêt sur `target`). `banned` : arêtes interdites ; `banned_nodes` :
/// nœuds interdits comme étapes intermédiaires (la cible reste permise).
/// Renvoie (distances, prédécesseurs (arête, nœud)).
pub(crate) fn shortest(
    adj: &[Vec<(usize, usize)>],
    cost: impl Fn(usize) -> f64,
    src: usize,
    target: Option<usize>,
    banned: Option<&[bool]>,
    banned_nodes: Option<&[bool]>,
) -> (Vec<f64>, Vec<(usize, usize)>) {
    let n = adj.len();
    let mut dist = vec![INF; n];
    let mut prev = vec![(usize::MAX, usize::MAX); n];
    dist[src] = 0.0;
    // Pour des réels positifs, l'ordre des bits est celui des valeurs.
    let mut pq = BinaryHeap::from([Reverse((0f64.to_bits(), src))]);
    while let Some(Reverse((db, x))) = pq.pop() {
        let d = f64::from_bits(db);
        if d > dist[x] {
            continue;
        }
        if Some(x) == target {
            break;
        }
        for &(e, nb) in &adj[x] {
            if banned.is_some_and(|b| b[e])
                || (banned_nodes.is_some_and(|b| b[nb]) && Some(nb) != target)
            {
                continue;
            }
            // garde : un coût négatif (ou NaN) bouclerait sans fin sur l'arête (cycle négatif) et
            // ferait grossir la file sans borne (gel du poste, 2026-10-07) ; compté 0
            let c = cost(e);
            let nd = d + if c > 0.0 { c } else { 0.0 };
            if nd < dist[nb] {
                dist[nb] = nd;
                prev[nb] = (e, x);
                pq.push(Reverse((nd.to_bits(), nb)));
            }
        }
    }
    (dist, prev)
}

/// Chemin src -> tgt reconstruit depuis les prédécesseurs : [(arête, de, vers)].
pub(crate) fn path_from(prev: &[(usize, usize)], src: usize, tgt: usize) -> Vec<Step> {
    let (mut out, mut x) = (Vec::new(), tgt);
    while x != src {
        let (e, p) = prev[x];
        out.push((e, p, x));
        x = p;
    }
    out.reverse();
    out
}

/// (réalisable, score) : ordre de préférence entre boucles.
pub(crate) fn key_cmp(a: (bool, f64), b: (bool, f64)) -> Ordering {
    a.0.cmp(&b.0).then(a.1.total_cmp(&b.1))
}

pub struct Solution {
    pub ids: Vec<usize>,
    pub length: f64,
    pub dplus: f64,
    pub feasible: bool,
    pub iterations: u64,
    pub depart: String,
    pub departs: Vec<String>,
}

/// Boucle courante pendant le recuit : ensemble X d'arêtes (tableau + positions) et degrés.
struct Loop {
    inx: Vec<bool>,
    pos: Vec<usize>,
    x: Vec<usize>,
    deg: Vec<u32>,
    /// T24 : arêtes de la boucle à chaque nœud (hors boucles sur soi), tenu si `track`.
    inc: Vec<Vec<usize>>,
    track: bool,
}

impl Loop {
    /// Tient `inc` à jour quand l'arête e (a–b) entre (`add`) ou sort de la boucle.
    fn link(&mut self, e: usize, a: usize, b: usize, add: bool) {
        if !self.track || a == b {
            return;
        }
        for n in [a, b] {
            let l = &mut self.inc[n];
            if add {
                l.push(e);
            } else if let Some(i) = l.iter().position(|&x| x == e) {
                l.swap_remove(i);
            }
        }
    }

    /// Différence symétrique avec la face f (involutive).
    fn toggle(&mut self, f: &[Step]) {
        for &(e, a, b) in f {
            if self.inx[e] {
                self.inx[e] = false;
                self.link(e, a, b, false);
                let p = self.pos[e];
                let last = self.x.pop().unwrap();
                if last != e {
                    self.x[p] = last;
                    self.pos[last] = p;
                }
                self.deg[a] -= 1;
                self.deg[b] -= 1;
            } else {
                self.inx[e] = true;
                self.link(e, a, b, true);
                self.pos[e] = self.x.len();
                self.x.push(e);
                self.deg[a] += 1;
                self.deg[b] += 1;
            }
        }
    }
}

/// Connexité de X juste après la bascule d'une face. X était connexe, donc toute composante
/// contient un nœud de la face : il suffit que les nœuds actifs de la face soient reliés.
/// Recherches entrelacées depuis chacun d'eux (fusionnées quand elles se rencontrent) : la
/// première qui s'épuise seule prouve la coupure, en un temps proportionnel à la plus petite
/// composante, au lieu d'un parcours complet de la boucle (85 à 90 % des cas à 100 km).
struct Connectivity {
    seen: Vec<u64>,
    owner: Vec<usize>,
    stamp: u64,
    stacks: Vec<Vec<usize>>,
    uf: Vec<usize>, // union-find sur les recherches
}

impl Connectivity {
    fn new(n: usize) -> Connectivity {
        Connectivity {
            seen: vec![0; n],
            owner: vec![0; n],
            stamp: 0,
            stacks: Vec::new(),
            uf: Vec::new(),
        }
    }

    fn find(&mut self, mut i: usize) -> usize {
        while self.uf[i] != i {
            self.uf[i] = self.uf[self.uf[i]];
            i = self.uf[i];
        }
        i
    }

    fn check(&mut self, f: &[Step], st: &Loop, adj: &[Vec<(usize, usize)>]) -> bool {
        self.stamp += 1;
        let stamp = self.stamp;
        self.uf.clear();
        for &(_, a, _) in f {
            if st.deg[a] > 0 && self.seen[a] != stamp {
                let k = self.uf.len();
                self.seen[a] = stamp;
                self.owner[a] = k;
                self.uf.push(k);
                if self.stacks.len() <= k {
                    self.stacks.push(Vec::new());
                }
                self.stacks[k].clear();
                self.stacks[k].push(a);
            }
        }
        let k = self.uf.len();
        let mut groups = k;
        while groups > 1 {
            for i in 0..k {
                if self.uf[i] != i {
                    continue; // recherche absorbée par une autre
                }
                let Some(x) = self.stacks[i].pop() else {
                    return false; // composante épuisée sans rejoindre les autres
                };
                for &(e, nb) in &adj[x] {
                    if !st.inx[e] {
                        continue;
                    }
                    if self.seen[nb] != stamp {
                        self.seen[nb] = stamp;
                        self.owner[nb] = i;
                        self.stacks[i].push(nb);
                    } else {
                        let j = self.find(self.owner[nb]);
                        if j != i {
                            self.uf[j] = i;
                            let s = std::mem::take(&mut self.stacks[j]);
                            self.stacks[i].extend(s);
                            groups -= 1;
                            if groups == 1 {
                                return true;
                            }
                        }
                    }
                }
            }
        }
        true
    }
}

pub struct FaceSearch<'a> {
    p: &'a Problem,
    adj: Vec<Vec<(usize, usize)>>,
    pub faces: Vec<Vec<Step>>,
    foe: Vec<Vec<usize>>,
    par: Vec<Vec<usize>>,
    far: Option<Vec<bool>>,
    /// Bonus B par arête (`Problem::search_bonus` : montées courtes, type de voie).
    wb: Option<Vec<f64>>,
    lam0: f64,
    /// Plafond de temps (sécurité) : le recuit s'arrête et rend sa meilleure boucle.
    pub deadline: Option<Instant>,
}

impl<'a> FaceSearch<'a> {
    pub fn new(p: &'a Problem) -> FaceSearch<'a> {
        let mut adj = vec![Vec::new(); p.n_nodes()];
        for e in 0..p.n_edges() {
            adj[p.u[e]].push((e, p.v[e]));
            if p.u[e] != p.v[e] {
                adj[p.v[e]].push((e, p.u[e]));
            }
        }
        let (faces, foe) = build_faces(&p.u, &p.v, &p.ang_u, &p.ang_v, &p.len, p.lmax);
        let far = p.node_simple.then(|| {
            let mut f = vec![false; p.n_nodes()];
            p.far.iter().for_each(|&x| f[x] = true);
            f
        });
        FaceSearch {
            p,
            adj,
            faces,
            foe,
            par: p.par_lists(),
            far,
            wb: p.search_bonus(),
            lam0: if p.min_distance() { LAM0_MD } else { LAM0 },
            deadline: None,
        }
    }

    // ------------------------------------------------------------ boucles initiales
    /// Plus petite face passant par le départ (boucle initiale minimale).
    pub fn start_face(&self) -> Option<Vec<usize>> {
        let mut best: Option<(f64, usize)> = None;
        for &(e, _) in &self.adj[self.p.s] {
            for &f in &self.foe[e] {
                let ids: Vec<usize> = self.faces[f].iter().map(|s| s.0).collect();
                if ids
                    .iter()
                    .any(|x| self.par[*x].iter().any(|q| ids.contains(q)))
                {
                    continue; // face qui longe son propre couloir
                }
                let ln: f64 = ids.iter().map(|&x| self.p.len[x]).sum();
                if best.is_none_or(|b| ln < b.0) {
                    best = Some((ln, f));
                }
            }
        }
        best.map(|(_, f)| self.faces[f].iter().map(|s| s.0).collect())
    }

    /// Nœuds cibles : cellules de 1 km les plus denses en D+ (moyenne 3×3), atteignables
    /// (aller-retour <= 60 % de L), espacées d'au moins 3 km.
    pub fn relief_targets(&self, k: usize) -> Vec<usize> {
        const CELL: f64 = 1000.0;
        const MIN_SEP: f64 = 3000.0;
        let p = self.p;
        let (d, _) = shortest(&self.adj, |e| p.len[e].max(1e-9), p.s, None, None, None);
        let c = p.xy[p.s];
        let rel = |n: usize| (p.xy[n][0] - c[0], p.xy[n][1] - c[1]);
        let ci: Vec<i64> = (0..p.n_nodes())
            .map(|n| (rel(n).0 / CELL).floor() as i64)
            .collect();
        let cj: Vec<i64> = (0..p.n_nodes())
            .map(|n| (rel(n).1 / CELL).floor() as i64)
            .collect();
        let (i0, j0) = (*ci.iter().min().unwrap(), *cj.iter().min().unwrap());
        let nx = (*ci.iter().max().unwrap() - i0 + 1) as usize;
        let ny = (*cj.iter().max().unwrap() - j0 + 1) as usize;
        let cell = |n: usize| ((cj[n] - j0) as usize, (ci[n] - i0) as usize);
        let (mut w, mut ln) = (vec![0.0; nx * ny], vec![0.0; nx * ny]);
        for e in 0..p.n_edges() {
            let (j, i) = cell(p.u[e]);
            w[j * nx + i] += p.w[e];
            ln[j * nx + i] += p.len[e];
        }
        let smooth = |z: &[f64], j: usize, i: usize| {
            let mut t = 0.0;
            for jj in j.saturating_sub(1)..(j + 2).min(ny) {
                for ii in i.saturating_sub(1)..(i + 2).min(nx) {
                    t += z[jj * nx + ii];
                }
            }
            t
        };
        let dens = |(j, i): (usize, usize)| {
            let ls = smooth(&ln, j, i);
            if ls > 5.0 * CELL {
                smooth(&w, j, i) / ls.max(1.0)
            } else {
                0.0
            }
        };
        // Représentant d'une cellule : le nœud le plus proche du départ.
        let mut by_d: Vec<usize> = (0..p.n_nodes()).filter(|&n| d[n].is_finite()).collect();
        by_d.sort_by(|&a, &b| d[a].total_cmp(&d[b]));
        let mut rep = HashMap::new();
        for n in by_d {
            rep.entry(cell(n)).or_insert(n);
        }
        let mut cands: Vec<(f64, usize)> = rep
            .iter()
            .filter(|&(&cl, &n)| 2.0 * d[n] <= 0.6 * p.l && dens(cl) > 0.0)
            .map(|(&cl, &n)| ((p.l - 2.0 * d[n]) * dens(cl), n))
            .collect();
        cands.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.cmp(&a.1)));
        let dist = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]);
        let mut out: Vec<usize> = Vec::new();
        for (_, n) in cands {
            if dist(p.xy[n], c) >= MIN_SEP && out.iter().all(|&q| dist(p.xy[n], p.xy[q]) >= MIN_SEP)
            {
                out.push(n);
                if out.len() == k {
                    break;
                }
            }
        }
        out
    }

    /// Aller-retour s -> t -> s par deux chemins sans arête commune ni couloir parallèle
    /// (ni carrefour commun en mode carrefours uniques).
    pub fn corridor(&self, t: usize) -> Option<Vec<usize>> {
        let (p, s) = (self.p, self.p.s);
        let (d1, p1) = shortest(&self.adj, |e| p.len[e], s, Some(t), None, None);
        if d1[t] == INF {
            return None;
        }
        let a = path_from(&p1, s, t);
        let mut banned = vec![false; p.n_edges()];
        for &(e, _, _) in &a {
            banned[e] = true;
            self.par[e].iter().for_each(|&q| banned[q] = true);
        }
        let bn = self.far.as_ref().map(|far| {
            let mut b = vec![false; p.n_nodes()];
            a.iter()
                .filter(|st| far[st.2] && st.2 != t)
                .for_each(|st| b[st.2] = true);
            b
        });
        let (d2, p2) = shortest(
            &self.adj,
            |e| p.len[e],
            t,
            Some(s),
            Some(&banned),
            bn.as_deref(),
        );
        if d2[s] == INF {
            return None;
        }
        let ids: Vec<usize> = a
            .iter()
            .chain(&path_from(&p2, t, s))
            .map(|st| st.0)
            .collect();
        p.parallel_ok(&ids).then_some(ids)
    }

    /// D52 : boucles « pétales » départ → A → B → départ, A et B deux cibles de relief
    /// (`relief_targets` : cellules denses en D+ à >= 3 km l'une de l'autre, donc deux reliefs),
    /// écartées de 40 à 120° vues du départ, de longueur routée dans [0,85 ; 1,10] × L (le recuit
    /// par faces est local : une graine hors tolérance n'est pas rattrapée). Paires présélectionnées
    /// sur une longueur estimée (d(s,A) + 1,3·|AB| + d(B,s)), au plus `PETAL_TRIES` routées ; à défaut de
    /// seconde cible, B = le nœud de 40 à 120° de A qui donne la meilleure longueur estimée. Triées :
    /// par score du mode décroissant (`Problem::score`) ; au plus `k`.
    /// Montagne (banc du 08/10, Chartreuse et Bourg 35–50 km) : aucun pétale dans la tolérance. Les
    /// trois tronçons ne partageant ni arête ni carrefour (carrefours uniques), les pétales routés font
    /// 1,4 à 2,6 L (même estimés sur les distances réseau), ou n'ont pas de route ; leur D+ y est 2 fois
    /// la cible. Les pétales n'agissent donc qu'en plaine et en ville.
    pub fn petals(&self, k: usize) -> Vec<Vec<usize>> {
        const ANG: (f64, f64) = (40.0, 120.0);
        const LEN: (f64, f64) = (0.85, 1.10);
        const PETAL_TRIES: usize = 8;
        let p = self.p;
        let ts = self.relief_targets(8);
        if ts.len() < 2 {
            return Vec::new();
        }
        // distances réelles (estimation de longueur), bornées > 0
        let (d, _) = shortest(&self.adj, |e| p.len[e].max(1.0), p.s, None, None, None);
        let c = p.xy[p.s];
        let bear = |n: usize| (p.xy[n][1] - c[1]).atan2(p.xy[n][0] - c[0]).to_degrees();
        let gap = |a: usize, b: usize| (p.xy[a][0] - p.xy[b][0]).hypot(p.xy[a][1] - p.xy[b][1]);
        let mut pairs: Vec<(f64, usize, usize)> = Vec::new();
        for (i, &a) in ts.iter().enumerate() {
            for &b in &ts[i + 1..] {
                let ang = ((bear(a) - bear(b)).rem_euclid(360.0) + 180.0).rem_euclid(360.0) - 180.0;
                if (ANG.0..=ANG.1).contains(&ang.abs()) {
                    let est = d[a] + 1.3 * gap(a, b) + d[b];
                    pairs.push(((est - p.l).abs(), a, b));
                }
            }
        }
        // et, pour chaque cible A, le nœud B de part et d'autre (40 à 120° de A) dont la longueur
        // estimée colle le mieux à L : un pétale existe même sans seconde cible à la bonne distance
        for &a in ts.iter().take(4) {
            for side in [-1.0, 1.0] {
                let best = (0..p.n_nodes())
                    .filter(|&b| d[b].is_finite() && b != p.s)
                    .filter(|&b| {
                        let ang = side
                            * (((bear(b) - bear(a)).rem_euclid(360.0) + 180.0).rem_euclid(360.0)
                                - 180.0);
                        (ANG.0..=ANG.1).contains(&ang)
                    })
                    .map(|b| ((d[a] + 1.3 * gap(a, b) + d[b] - p.l).abs(), b))
                    .min_by(|x, y| x.0.total_cmp(&y.0));
                if let Some((err, b)) = best {
                    pairs.push((err, a, b));
                }
            }
        }
        pairs.sort_by(|x, y| x.0.total_cmp(&y.0));
        let mut out: Vec<(f64, Vec<usize>)> = Vec::new();
        for &(_, a, b) in pairs.iter().take(PETAL_TRIES) {
            let Some(ids) = self.route_through(&[a, b]) else {
                continue;
            };
            let l = p.stats(&ids).0;
            if l < LEN.0 * p.l || l > LEN.1 * p.l {
                continue;
            }
            // classées par le score du mode (cible : erreur, confort et virages ; max : D+ et confort) :
            // un pétale qui tient la cible sur des routes ne passe plus devant un pétale sur chemins
            out.push((p.score(&ids).0, ids));
        }
        out.sort_by(|x, y| y.0.total_cmp(&x.0));
        out.into_iter().take(k).map(|x| x.1).collect()
    }

    /// D52 : direction (radians) d'une boucle vue du départ : barycentre des milieux d'arêtes
    /// pondéré par la longueur.
    pub fn bearing(&self, ids: &[usize]) -> f64 {
        let (p, c) = (self.p, self.p.xy[self.p.s]);
        let (mut x, mut y) = (0.0, 0.0);
        for &e in ids {
            let (a, b) = (p.xy[p.u[e]], p.xy[p.v[e]]);
            x += p.len[e] * ((a[0] + b[0]) / 2.0 - c[0]);
            y += p.len[e] * ((a[1] + b[1]) / 2.0 - c[1]);
        }
        y.atan2(x)
    }

    /// Boucle départ → wps… → départ par plus courts chemins (`search_len`, toujours > 0), sans
    /// arête répétée ni couloir parallèle repris, ni (carrefours uniques) nœud lointain repassé.
    fn route_through(&self, wps: &[usize]) -> Option<Vec<usize>> {
        let p = self.p;
        let mut used = vec![false; p.n_edges()];
        let mut visited = vec![false; p.n_nodes()];
        let (mut cur, mut ids) = (p.s, Vec::new());
        for &t in wps.iter().chain([&p.s]) {
            let bn = self.far.as_ref().map(|_| visited.as_slice());
            let (dist, prev) = shortest(
                &self.adj,
                |e| p.search_len(e),
                cur,
                Some(t),
                Some(&used),
                bn,
            );
            if dist[t] == INF {
                return None;
            }
            for (e, _, b) in path_from(&prev, cur, t) {
                used[e] = true;
                self.par[e].iter().for_each(|&q| used[q] = true);
                if self.far.as_ref().is_some_and(|f| f[b]) {
                    visited[b] = true;
                }
                ids.push(e);
            }
            cur = t;
        }
        p.parallel_ok(&ids).then_some(ids)
    }

    // ------------------------------------------------------------ recherche locale
    /// Recuit depuis la boucle `init` (ids d'arêtes), `iters` itérations. Renvoie (ids de la
    /// meilleure boucle dans les bornes, ou de la dernière ; λ final ; itérations faites).
    /// Mode max : score lagrangien D+ - λ·L, λ asservi pour que L reste dans [Lmin, Lmax] :
    /// la boucle ne grossit que par des faces assez denses en D+. Mode cible : -erreur.
    /// Mode min_distance (modes.md A) : score W + B - λL - pénalités, λ asservi pour garder W
    /// juste au-dessus de X ; on garde la plus courte boucle de W >= X et L <= Lmax (avec
    /// bonus `wb` : la meilleure B - λL), sinon celle de plus fort W sous Lmax.
    #[allow(clippy::too_many_arguments)] // mêmes paramètres que `FaceSearch.run` en Python
    pub fn run(
        &self,
        init: &[usize],
        iters: u64,
        t0: f64,
        t1: f64,
        lam0: f64,
        w: &[f64],
        wb: Option<&[f64]>,
        rng: &mut Rng,
    ) -> (Vec<usize>, f64, u64) {
        self.run_until(init, iters, t0, t1, lam0, w, wb, rng, self.deadline)
    }

    /// `run` avec une échéance explicite (`None` : toutes les itérations, quel que soit le temps).
    #[allow(clippy::too_many_arguments)]
    fn run_until(
        &self,
        init: &[usize],
        iters: u64,
        t0: f64,
        t1: f64,
        lam0: f64,
        w: &[f64],
        wb: Option<&[f64]>,
        rng: &mut Rng,
        deadline: Option<Instant>,
    ) -> (Vec<usize>, f64, u64) {
        // `w` : poids de recherche (le D+ réel, ou réduit sur les arêtes déjà prises par les
        // boucles précédentes pour s'en écarter, voir `alternates`).
        let p = self.p;
        let (ln, s, target, md) = (&p.len, p.s, p.target(), p.min_distance());
        let bonus = |e: usize| wb.map_or(0.0, |b| b[e]);
        let (lmin, lmax, lt, dt) = (p.lmin, p.lmax, p.l, p.d.unwrap_or(1.0));
        let (x_md, wt) = (dt, dt * (1.0 + MD_MARGIN));
        // type de voie en mode cible : sous cette erreur la cible est tenue, B départage
        let band = if p.off.is_empty() {
            0.0
        } else {
            crate::problem::SURF_TARGET_BAND
        };
        let (m, n) = (p.n_edges(), p.n_nodes());
        // coût aux nœuds de degré 2 de la boucle : T24 « longues » (max) et D52 virages (max, cible)
        let nodes_on = p.node_mu != 0.0 || p.turn_mu != 0.0;
        let mut st = Loop {
            inx: vec![false; m],
            pos: vec![0; m],
            x: Vec::new(),
            deg: vec![0; n],
            inc: vec![Vec::new(); if nodes_on { n } else { 0 }],
            track: nodes_on,
        };
        let (mut cur_l, mut cur_w, mut cur_b) = (0.0, 0.0, 0.0);
        for &e in init {
            cur_b += bonus(e);
            st.inx[e] = true;
            st.pos[e] = st.x.len();
            st.x.push(e);
            st.deg[p.u[e]] += 1;
            st.deg[p.v[e]] += 1;
            st.link(e, p.u[e], p.v[e], true);
            cur_l += ln[e];
            cur_w += w[e];
        }
        // T24 « longues » (extrema entre deux arêtes consécutives, max seulement : search_problem le
        // pose) et D52 virages aux carrefours (max et cible) : compté dans B ; jamais en min_distance
        let nterm = nodes_on && !md;
        let junction: Vec<bool> = if nterm {
            p.degrees().iter().map(|&d| d >= 3).collect()
        } else {
            Vec::new()
        };
        if nterm {
            cur_b -= p.node_costs(&st.x);
        }
        // coût courant de chaque nœud de la boucle (évite de recalculer « avant » à chaque mouvement)
        // (extremum, virage) non pondérés, sommés à part : sans virage, calculs bit à bit d'avant D52
        let mut ncost = vec![[0.0; 2]; if nterm { n } else { 0 }];
        if nterm {
            for (a, es) in st.inc.iter().enumerate() {
                if es.len() == 2 {
                    ncost[a] = p.node_terms(a, es[0], es[1], junction[a]);
                }
            }
        }
        let mut aft: Vec<(usize, [f64; 2])> = Vec::new();
        let (mut mark_t, mut stamp_t) = (vec![0u64; if nterm { m } else { 0 }], 0u64);
        // points de passage : nœuds marqués, nombre de points que la boucle ne touche pas
        let mut is_via = vec![false; if p.via.is_empty() { 0 } else { n }];
        p.via.iter().for_each(|&x| is_via[x] = true);
        let mut cur_m = p.via.iter().filter(|&&x| st.deg[x] == 0).count() as i64;
        let pen = |k: i64| VIA_PEN * k as f64;
        let score = |l: f64, wv: f64, b: f64, lam: f64| {
            if md {
                wv + b
                    - lam * l
                    - if wv < x_md { MD_PEN * (x_md - wv) } else { 0.0 }
                    - if l > lmax {
                        MD_PEN * lam * (l - lmax)
                    } else {
                        0.0
                    }
            } else if target {
                // erreur, plus une forte pénalité hors des bornes de distance
                let out = if l < lmin {
                    lmin - l
                } else if l > lmax {
                    l - lmax
                } else {
                    0.0
                };
                // b : type de voie, et prix des arêtes déjà prises (`alternates`)
                let err = ((l - lt).abs() / lt + (wv - dt).abs() / dt).max(band);
                b - TARGET_SCALE * (err + 3.0 * out / lt)
            } else {
                wv + b - lam * l - if l > lmax { PEN * (l - lmax) } else { 0.0 }
            }
        };
        // ce qu'on cherche à maximiser parmi les boucles dans les bornes
        // mode max (D40) : au-delà de la distance demandée, chaque mètre doit rapporter au moins
        // le prix λ (D+ par mètre) : pas de détour sans D+ pour boucher la tolérance
        let lmid = 0.5 * (lmin + lmax);
        let value = |l: f64, wv: f64, b: f64, lam: f64| {
            if target {
                score(l, wv, b, 0.0)
            } else {
                wv + b - LEN_PRICE * lam * (l - lmid).max(0.0)
            }
        };
        let in_bounds = |l: f64| lmin <= l && l <= lmax;

        let mut lam = lam0;
        let mut cur = score(cur_l, cur_w, cur_b, lam) - pen(cur_m);
        let mut best = if in_bounds(cur_l) && !md && cur_m == 0 {
            value(cur_l, cur_w, cur_b, lam)
        } else {
            -INF
        };
        let mut best_x = st.x.clone();
        // min_distance : meilleure boucle réalisable (L, B), et repli (plus fort W sous Lmax)
        let md_ok = |l: f64, wv: f64, k: i64| wv >= x_md && l <= lmax && k == 0;
        let (mut best_l, mut best_b) = (INF, 0.0);
        let (mut fb_w, mut fb_x) = (-INF, Vec::new());
        if md && md_ok(cur_l, cur_w, cur_m) {
            (best_l, best_b) = (cur_l, cur_b);
        }
        // Marques datées (évitent de remettre des tableaux à zéro) : couloirs et connexité.
        let (mut mark_f, mut mark_a, mut stamp) = (vec![0u64; m], vec![0u64; m], 0u64);
        let mut conn = Connectivity::new(n);
        let has_par = !p.parallel.is_empty();
        let (mut it, mut temp) = (0u64, t0);
        while !st.x.is_empty() {
            if it & 255 == 0 {
                if it >= iters || deadline.is_some_and(|d| Instant::now() >= d) {
                    break;
                }
                temp = t0 * (t1 / t0).powf(it as f64 / iters as f64);
                if md {
                    // trop de D+ : la distance coûte plus cher, et inversement
                    lam *= if cur_w > wt { 1.01 } else { 0.99 };
                    cur = score(cur_l, cur_w, cur_b, lam) - pen(cur_m);
                } else if !target {
                    lam *= if cur_l > lmid { 1.01 } else { 0.99 };
                    cur = score(cur_l, cur_w, cur_b, lam) - pen(cur_m);
                }
            }
            it += 1;
            let e = st.x[rng.below(st.x.len())];
            let an = &self.adj[if rng.random() < 0.5 { p.u[e] } else { p.v[e] }];
            let fl = &self.foe[an[rng.below(an.len())].0];
            if fl.is_empty() {
                continue;
            }
            let f = &self.faces[fl[rng.below(fl.len())]];
            let (mut dl, mut dw, mut db, mut k) = (0.0, 0.0, 0.0, 0);
            for &(x, _, _) in f {
                if st.inx[x] {
                    k += 1;
                    dl -= ln[x];
                    dw -= w[x];
                    db -= bonus(x);
                } else {
                    dl += ln[x];
                    dw += w[x];
                    db += bonus(x);
                }
            }
            if nterm {
                stamp_t += 1;
                for &(x, _, _) in f {
                    mark_t[x] = stamp_t;
                }
                // extremum en a avant / après bascule (après = arêtes gardées + arêtes de la face
                // en a qui entrent) ; ignoré si le degré n'est pas 2
                // coût du nœud a APRÈS bascule (avant : `ncost`, tenu à jour à chaque mouvement accepté)
                let ext = |a: usize, fa: [usize; 2], after: bool| {
                    if p.node_mu == 0.0 && !junction[a] {
                        return [0.0; 2]; // virage seul : rien hors carrefour
                    }
                    let (mut es, mut k) = ([usize::MAX; 2], 0);
                    let kept = st.inc[a]
                        .iter()
                        .copied()
                        .filter(|&x| !(after && mark_t[x] == stamp_t));
                    let added = fa.into_iter().filter(|&x| after && !st.inx[x]);
                    for x in kept.chain(added) {
                        if k < 2 {
                            es[k] = x;
                        }
                        k += 1;
                    }
                    if k == 2 {
                        p.node_terms(a, es[0], es[1], junction[a])
                    } else {
                        [0.0; 2]
                    }
                };
                let nf = f.len();
                aft.clear();
                let (mut dk, mut dt) = (0.0, 0.0);
                for i in 0..nf {
                    let a = f[i].1;
                    let c = ext(a, [f[(i + nf - 1) % nf].0, f[i].0], true);
                    dk += c[0] - ncost[a][0];
                    dt += c[1] - ncost[a][1];
                    aft.push((a, c));
                }
                db -= p.node_mu * dk + p.turn_mu * dt;
            }
            let mut dm = 0i64;
            if !is_via.is_empty() {
                let nf = f.len();
                let sg = |x: usize| if st.inx[x] { -1i64 } else { 1 };
                for i in 0..nf {
                    let a = f[i].1;
                    if is_via[a] {
                        let d = st.deg[a] as i64 + sg(f[(i + nf - 1) % nf].0) + sg(f[i].0);
                        dm += i64::from(d == 0) - i64::from(st.deg[a] == 0);
                    }
                }
            }
            let new = score(cur_l + dl, cur_w + dw, cur_b + db, lam) - pen(cur_m + dm);
            if new < cur && rng.random() >= ((new - cur) / temp).exp() {
                continue;
            }
            let nf = f.len();
            if has_par {
                // une arête ajoutée ne doit pas longer une arête qui reste ou qui arrive
                stamp += 1;
                for &(x, _, _) in f {
                    mark_f[x] = stamp;
                    if !st.inx[x] {
                        mark_a[x] = stamp;
                    }
                }
                let bad = f.iter().any(|&(x, _, _)| {
                    mark_a[x] == stamp
                        && self.par[x]
                            .iter()
                            .any(|&q| mark_a[q] == stamp || (st.inx[q] && mark_f[q] != stamp))
                });
                if bad {
                    continue;
                }
            }
            // connexité : cas rapide (une seule plage commune, nœuds intérieurs de degré 2)
            let mut need_full = k == nf;
            if k > 0 && !need_full {
                let sh = |i: usize| st.inx[f[i].0];
                let prev = |i: usize| if i == 0 { nf - 1 } else { i - 1 };
                let runs = (0..nf).filter(|&i| sh(i) && !sh(prev(i))).count();
                need_full =
                    runs != 1 || (0..nf).any(|i| sh(i) && sh(prev(i)) && st.deg[f[i].1] != 2);
            }
            st.toggle(f);
            let mut ok = st.deg[s] > 0 && !st.x.is_empty();
            if ok && let Some(far) = &self.far {
                // carrefours uniques : degré <= 2 hors du rayon libre
                ok = !f.iter().any(|&(_, a, _)| st.deg[a] > 2 && far[a]);
            }
            if ok && need_full {
                ok = conn.check(f, &st, &self.adj);
            }
            if !ok {
                st.toggle(f); // la différence symétrique est involutive
                continue;
            }
            cur_l += dl;
            cur_w += dw;
            cur_b += db;
            cur_m += dm;
            cur = new;
            if nterm {
                aft.iter().for_each(|&(a, c)| ncost[a] = c);
            }
            if md {
                if md_ok(cur_l, cur_w, cur_m) {
                    let better = best_l == INF
                        || if wb.is_some() {
                            cur_b - lam * cur_l > best_b - lam * best_l
                        } else {
                            cur_l < best_l
                        };
                    if better {
                        (best_l, best_b) = (cur_l, cur_b);
                        best_x.clone_from(&st.x);
                    }
                } else if best_l == INF && cur_l <= lmax && cur_w > fb_w {
                    fb_w = cur_w;
                    fb_x.clone_from(&st.x);
                }
            } else if in_bounds(cur_l) && cur_m == 0 {
                let val = value(cur_l, cur_w, cur_b, lam);
                if val > best {
                    best = val;
                    best_x.clone_from(&st.x);
                }
            }
        }
        let out = if md {
            if best_l < INF {
                best_x
            } else if fb_w > -INF {
                fb_x
            } else {
                st.x
            }
        } else if best > -INF {
            best_x
        } else {
            st.x
        };
        (out, lam, it)
    }

    /// Sondes courtes depuis plusieurs boucles initiales (en parallèle), puis le reste du
    /// budget sur les deux meilleures. `warm` : boucle supplémentaire à essayer (ids d'arêtes).
    /// `None` si aucune boucle initiale n'est trouvée.
    pub fn solve(&self, iters: u64, seed: u64, warm: Option<&[usize]>) -> Option<Solution> {
        const K: usize = 6;
        const PROBE: f64 = 0.07;
        let p = self.p;
        let mut inits: Vec<(String, Vec<usize>)> = Vec::new();
        if let Some(x) = warm {
            inits.push(("recuit".into(), x.to_vec()));
        }
        if !p.via.is_empty() {
            // points de passage : boucle par waypoints qui les contient tous (si possible)
            let mut a = Annealer::new(p, seed ^ 0x5A5A);
            a.deadline = self.deadline;
            if let Some(r) = (0..50).find_map(|_| a.construct()) {
                inits.push(("points".into(), r.iter().map(|s| s.0).collect()));
            }
        }
        if let Some(f0) = self.start_face() {
            inits.push(("face".into(), f0));
        }
        for t in self.relief_targets(K) {
            if let Some(c) = self.corridor(t) {
                let d = (p.xy[t][0] - p.xy[p.s][0]).hypot(p.xy[t][1] - p.xy[p.s][1]);
                inits.push((format!("couloir {:.1} km", d / 1000.0), c));
            }
        }
        // D52 : pétales sur deux reliefs voisins (les sondes les départagent par `p.score`, qui
        // compte les virages : un pétale lisse l'emporte à erreur égale)
        if p.smooth {
            for (i, x) in self.petals(PETALS).into_iter().enumerate() {
                inits.push((format!("pétale {}", i + 1), x));
            }
        }
        if inits.is_empty() {
            // réseau sans face simple au départ : boucle par waypoints
            let mut a = Annealer::new(p, seed ^ 0xA5A5);
            a.deadline = self.deadline;
            let r = (0..200).find_map(|_| a.construct())?;
            inits.push(("waypoints".into(), r.iter().map(|s| s.0).collect()));
        }
        // Un générateur par sonde : résultat indépendant du nombre de fils.
        let rng = |i: usize| Rng::new(seed.wrapping_mul(1000).wrapping_add(i as u64));
        let probe = if inits.len() > 1 {
            (PROBE * iters as f64) as u64
        } else {
            0
        };
        let mut res: Vec<_> = inits
            .par_iter()
            .enumerate()
            .map(|(i, (name, x0))| {
                let (ids, lam, it) = self.run(
                    x0,
                    probe,
                    10.0,
                    1.0,
                    self.lam0,
                    &p.w,
                    self.wb.as_deref(),
                    &mut rng(i),
                );
                let sc = p.score(&ids);
                ((sc.3, sc.0), name.clone(), ids, lam, it)
            })
            .collect();
        let mut total: u64 = res.iter().map(|r| r.4).sum();
        res.sort_by(|a, b| key_cmp(b.0, a.0)); // tri stable : à égalité, ordre des départs
        // Les sondes sont courtes, donc bruitées : on affine les deux meilleures, moitié du
        // reste chacune (une seule peut rester coincée dans un optimum local médiocre).
        let top = &res[..res.len().min(2)];
        let left = iters
            .saturating_sub(probe * inits.len() as u64)
            .max(iters / 4)
            / top.len() as u64;
        let fin: Vec<_> = top
            .par_iter()
            .enumerate()
            .map(|(i, (_, name, x0, lam, _))| {
                let (ids, _, it) = self.run(
                    x0,
                    left,
                    3.0,
                    0.2,
                    *lam,
                    &p.w,
                    self.wb.as_deref(),
                    &mut rng(100 + i),
                );
                let sc = p.score(&ids);
                ((sc.3, sc.0), name.clone(), ids, it)
            })
            .collect();
        total += fin.iter().map(|r| r.3).sum::<u64>();
        // premier maximum, comme max() en Python
        let best = fin.iter().fold(&fin[0], |b, r| {
            if key_cmp(r.0, b.0) == Ordering::Greater {
                r
            } else {
                b
            }
        });
        let (length, dplus) = p.stats(&best.2);
        Some(Solution {
            ids: best.2.clone(),
            length,
            dplus,
            feasible: best.0.0,
            iterations: total,
            depart: best.1.clone(),
            departs: res.iter().map(|r| r.1.clone()).collect(),
        })
    }

    /// Part de la plus courte des deux boucles qui est commune aux deux (en longueur).
    pub fn overlap(&self, a: &[usize], b: &[usize]) -> f64 {
        let ln = &self.p.len;
        let mut inb = vec![false; ln.len()];
        b.iter().for_each(|&e| inb[e] = true);
        let common: f64 = a.iter().filter(|&&e| inb[e]).map(|&e| ln[e]).sum();
        let la: f64 = a.iter().map(|&e| ln[e]).sum();
        let lb: f64 = b.iter().map(|&e| ln[e]).sum();
        common / la.min(lb).max(1e-9)
    }

    /// Jusqu'à `n` boucles différentes de `existing` et entre elles (D44 : on rend le nombre
    /// demandé tant que le réseau le permet). Chaque tour part du secteur le moins recouvrant, le
    /// D+ des arêtes déjà prises compté avec une remise (mode min_distance : dans B, jamais dans W,
    /// sinon la contrainte X ment). Toute boucle produite va dans une réserve ; on y prend la
    /// meilleure qui passe le palier courant de `LADDER` (recouvrement max avec chacune des autres,
    /// qualité min : D+ rapporté à la meilleure, ou longueur en min_distance ; mode cible : tenir
    /// la cible, `Problem::on_target`, ou une erreur comparable si la meilleure ne la tient pas). Un tour sans prise
    /// monte d'un palier. S'il manque encore des boucles (départs, budget ou temps épuisés) :
    /// paliers restants sur la réserve, puis recherches courtes HORS échéance (`FORCE_ITERS`,
    /// le nombre de boucles ne dépend pas de la charge de la machine) où chaque mètre déjà pris
    /// coûte le D+ moyen de la meilleure boucle. Réseau trop petit pour les bornes (la meilleure
    /// boucle est elle-même hors bornes) : les autres doivent seulement s'en approcher.
    /// Budget `iters` (+ au plus 6·n·`FORCE_ITERS`). Renvoie (boucles, itérations faites).
    pub fn alternates(
        &self,
        existing: &[Vec<usize>],
        n: usize,
        iters: u64,
        seed: u64,
    ) -> (Vec<Vec<usize>>, u64) {
        /// (recouvrement max, qualité min) : on relâche d'abord le recouvrement, puis la qualité.
        const LADDER: [(f64, f64); 6] = [
            (0.5, 0.75),
            (0.65, 0.75),
            (0.8, 0.75),
            (0.9, 0.75),
            (0.9, 0.6),
            (0.9, 0.5),
        ];
        /// Remise sur le D+ des arêtes déjà prises, par palier de la recherche.
        const DISCOUNT: [f64; 4] = [0.6, 0.75, 0.9, 0.9];
        const FORCE_ITERS: u64 = 100_000;
        /// Mode cible, recherches forcées : prix d'une boucle entièrement déjà prise, en unités
        /// de score (`TARGET_SCALE` = 100 % d'erreur) : 10 % d'erreur.
        const TARGET_FEE: f64 = 100.0;
        let p = self.p;
        let mut inits: Vec<Vec<usize>> = Vec::new();
        inits.extend(self.start_face());
        if p.smooth {
            inits.extend(self.petals(PETALS));
        }
        inits.extend(
            self.relief_targets(8)
                .into_iter()
                .filter_map(|t| self.corridor(t)),
        );
        // Réseau clairsemé (peu de faces, couloirs introuvables) : boucles par waypoints.
        let mut a = Annealer::new(p, seed ^ 0x5A5A);
        a.deadline = self.deadline;
        for _ in 0..50 {
            if inits.len() >= 2 * n {
                break;
            }
            inits.extend(
                a.construct()
                    .map(|r| r.iter().map(|s| s.0).collect::<Vec<_>>()),
            );
        }
        let all_inits = inits.clone();
        let (mut kept, mut out) = (existing.to_vec(), Vec::new());
        let (mut left, mut round, mut level) = (iters, 0u64, 0usize);
        // référence : la plus courte longueur et le meilleur D+ des boucles déjà là
        let (ref_l, ref_d) = existing
            .iter()
            .map(|k| p.stats(k))
            .fold((INF, 0.0), |a: (f64, f64), s| (a.0.min(s.0), a.1.max(s.1)));
        let ref_ok = existing.iter().any(|k| p.score(k).3);
        // admise dans la réserve : boucle valide dans les bornes, ou qui s'en approche autant que
        // la référence quand celle-ci est hors bornes
        let ok = |ids: &[usize]| {
            let sc = p.score(ids);
            let near = !ref_ok && sc.1 <= p.lmax && (p.min_distance() || sc.1 >= 0.75 * ref_l);
            (sc.3 || near) && p.check(ids).is_ok()
        };
        // mode cible : si la meilleure tient la cible, les autres aussi ; sinon erreur comparable
        let ref_on = existing.iter().any(|k| p.on_target(k));
        let ref_sc = existing.iter().map(|k| p.score(k).0).fold(-INF, f64::max);
        let good = |ids: &[usize], q: f64| {
            let s = p.stats(ids);
            if p.target() {
                p.on_target(ids) || (!ref_on && p.score(ids).0 >= ref_sc / q)
            } else if p.min_distance() && ref_ok {
                s.0 <= ref_l / q
            } else {
                s.1 >= q * ref_d
            }
        };
        // meilleure boucle de la réserve qui passe le palier
        let pick = |pool: &[Vec<usize>], kept: &[Vec<usize>], (max_ov, q): (f64, f64)| {
            (0..pool.len())
                .filter(|&i| {
                    good(&pool[i], q) && kept.iter().all(|k| self.overlap(&pool[i], k) <= max_ov)
                })
                .max_by(|&a, &b| p.score(&pool[a]).0.total_cmp(&p.score(&pool[b]).0))
        };
        // poids de recherche : les arêtes déjà prises perdent `cut`·w, et `fee` D+ par mètre par
        // rapport aux autres (mode max : prime aux arêtes neuves, qui allonge aussi les boucles
        // d'un réseau trop petit ; min_distance : dans B ; mode cible : poids inchangés, et avec
        // `fee` > 0 un prix dans B : `TARGET_FEE`·`cut` pour une boucle entièrement déjà prise)
        let weights = |kept: &[Vec<usize>], cut: f64, fee: f64| {
            let mut used = vec![false; p.n_edges()];
            kept.iter().flatten().for_each(|&e| used[e] = true);
            let w_eff: Vec<f64> = if p.target() || p.min_distance() {
                p.w.clone()
            } else {
                (0..p.n_edges())
                    .map(|e| {
                        if used[e] {
                            (1.0 - cut) * p.w[e]
                        } else {
                            p.w[e] + fee * p.len[e]
                        }
                    })
                    .collect()
            };
            let base = |e: usize| self.wb.as_ref().map_or(0.0, |b| b[e]);
            let wb_eff: Option<Vec<f64>> = if p.min_distance() {
                Some(
                    (0..p.n_edges())
                        .map(|e| {
                            let pen = if used[e] {
                                cut * p.w[e] + fee * p.len[e]
                            } else {
                                0.0
                            };
                            base(e) - pen
                        })
                        .collect(),
                )
            } else if p.target() && fee > 0.0 {
                let c = TARGET_FEE * cut / p.l;
                Some(
                    (0..p.n_edges())
                        .map(|e| base(e) - if used[e] { c * p.len[e] } else { 0.0 })
                        .collect(),
                )
            } else {
                // mode max : la prime de type de voie des arêtes déjà prises est remisée comme leur D+
                self.wb.as_ref().map(|b| {
                    (0..p.n_edges())
                        .map(|e| if used[e] { (1.0 - cut) * b[e] } else { b[e] })
                        .collect()
                })
            };
            (w_eff, wb_eff)
        };
        let mut pool: Vec<Vec<usize>> = Vec::new();
        while out.len() < n {
            if inits.is_empty() {
                if level >= 3 {
                    break;
                }
                level += 1;
                // départs d'appoint : les boucles déjà là (réalisables), que la remise éloigne
                inits = all_inits
                    .iter()
                    .cloned()
                    .chain(kept.iter().cloned())
                    .collect();
            }
            let per = left / (n - out.len()) as u64;
            if per < 10_000 || self.deadline.is_some_and(|d| Instant::now() >= d) {
                break;
            }
            let (w_eff, wb_eff) = weights(&kept, 1.0 - DISCOUNT[level], 0.0);
            let ov = |x: &[usize]| kept.iter().map(|k| self.overlap(x, k)).fold(0.0, f64::max);
            // D52 : à recouvrement égal, partir de la direction la plus éloignée des sorties gardées
            // (écart angulaire min, 0 à 1 pour 0 à 180° ; 180° valent DIR_W de recouvrement)
            let kb: Vec<f64> = kept.iter().map(|k| self.bearing(k)).collect();
            let sep = |x: &[usize]| {
                let b = self.bearing(x);
                kb.iter()
                    .map(|&k| (b - k).sin().atan2((b - k).cos()).abs() / std::f64::consts::PI)
                    .fold(1.0, f64::min)
            };
            if p.smooth {
                let key = |x: &[usize]| ov(x) - DIR_W * sep(x);
                inits.sort_by(|x, y| key(x).total_cmp(&key(y)));
            } else {
                inits.sort_by(|x, y| ov(x).total_cmp(&ov(y)));
            }
            // Deux départs par tour : un seul donne un résultat très variable.
            let tries: Vec<Vec<usize>> = inits.drain(..inits.len().min(2)).collect();
            // on garde du budget pour les tours suivants (palier relâché) tant qu'il en reste
            let share = if inits.is_empty() && level >= 3 {
                1.0
            } else {
                0.6
            };
            let budget = (per as f64 / tries.len() as f64 * share) as u64;
            let res: Vec<_> = tries
                .par_iter()
                .enumerate()
                .map(|(i, x0)| {
                    let mut rng = Rng::new(
                        seed.wrapping_mul(1000)
                            .wrapping_add(200 + 2 * round + i as u64),
                    );
                    self.run(
                        x0,
                        budget,
                        10.0,
                        0.2,
                        self.lam0,
                        &w_eff,
                        wb_eff.as_deref(),
                        &mut rng,
                    )
                })
                .collect();
            round += 1;
            for (ids, _, it) in res {
                left = left.saturating_sub(it);
                if ok(&ids) {
                    pool.push(ids);
                }
            }
            if let Some(i) = pick(&pool, &kept, LADDER[level]) {
                let ids = pool.swap_remove(i);
                kept.push(ids.clone());
                out.push(ids);
            } else {
                level = (level + 1).min(3);
            }
        }
        // D44 : il manque des boucles. Réserve à tous les paliers, sinon recherches forcées.
        let fee = (ref_d / ref_l.max(1.0)).max(0.01);
        // `forced` : recherches forcées depuis la dernière prise ; `drawn` : au total (tirages)
        let (mut forced, mut drawn, mut extra) = (0u32, 0usize, 0u64);
        while out.len() < n {
            if let Some(i) = LADDER.iter().find_map(|&l| pick(&pool, &kept, l)) {
                let ids = pool.swap_remove(i);
                kept.push(ids.clone());
                out.push(ids);
                forced = 0;
                continue;
            }
            if forced == 3 || kept.is_empty() {
                break;
            }
            // de plus en plus loin des boucles gardées : 40, 70 puis 100 % de leur D+ retiré
            let cut = (0.4 + 0.3 * forced as f64).min(1.0);
            let (w_eff, mut wb_eff) = weights(&kept, cut, cut * fee);
            // mode max : sans la prime de type de voie ni le coût des grands axes (D50), qui
            // enferment la recherche dans les mêmes vallées (Bourg 12 km : 2 à 3 boucles sur 4) ;
            // le choix parmi la réserve reste au score complet
            if !p.target() && !p.min_distance() {
                wb_eff = None;
            }
            let res: Vec<_> = (0..2usize)
                .into_par_iter()
                .map(|i| {
                    let k = 2 * drawn + i;
                    let mut rng = Rng::new(seed.wrapping_mul(1000).wrapping_add(500 + k as u64));
                    self.run_until(
                        &kept[k % kept.len()],
                        FORCE_ITERS,
                        3.0,
                        0.2,
                        self.lam0,
                        &w_eff,
                        wb_eff.as_deref(),
                        &mut rng,
                        None,
                    )
                })
                .collect();
            forced += 1;
            drawn += 1;
            for (ids, _, it) in res {
                extra += it;
                if ok(&ids) {
                    pool.push(ids);
                }
            }
        }
        (out, iters - left + extra)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// Gel du poste (2026-10-07) : des coûts négatifs faisaient relâcher Dijkstra sans fin (file
    /// sans borne). Coût négatif, nul ou NaN compté 0 : la recherche termine. Le compteur arrête
    /// le test bien avant toute explosion de mémoire si la garde disparaît.
    #[test]
    fn shortest_terminates_with_negative_costs() {
        // triangle 0-1-2 et arête 2-3
        let adj = vec![
            vec![(0, 1), (2, 2)],
            vec![(0, 0), (1, 2)],
            vec![(1, 1), (2, 0), (3, 3)],
            vec![(3, 2)],
        ];
        for c in [-5.0, f64::NAN, 0.0] {
            let calls = Cell::new(0u32);
            let cost = |_| {
                calls.set(calls.get() + 1);
                assert!(calls.get() < 1000, "Dijkstra ne termine pas");
                c
            };
            let (d, _) = shortest(&adj, cost, 0, None, None, None);
            assert!(d.iter().all(|&x| x == 0.0), "{c} {d:?}");
        }
    }
}
