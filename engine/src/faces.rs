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
const TARGET_SCALE: f64 = 1000.0;
/// Pénalité par mètre au-delà de Lmax (mode max).
const PEN: f64 = 0.5;
/// Mode min_distance (modes.md A) : pénalité du D+ manquant et du dépassement de Lcap, et
/// consigne de l'asservissement de λ (un peu au-dessus de X).
const MD_PEN: f64 = 3.0;
const MD_MARGIN: f64 = 0.03;
/// λ initial : mode min_distance, autres modes.
const LAM0_MD: f64 = 0.05;
const LAM0: f64 = 0.02;

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
            let nd = d + cost(e);
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
}

impl Loop {
    /// Différence symétrique avec la face f (involutive).
    fn toggle(&mut self, f: &[Step]) {
        for &(e, a, b) in f {
            if self.inx[e] {
                self.inx[e] = false;
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
    /// Mode min_distance avec préférence de montées : bonus B par arête (γβq/H).
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
            wb: p.climb_bonus().filter(|_| p.min_distance()),
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
        // `w` : poids de recherche (le D+ réel, ou réduit sur les arêtes déjà prises par les
        // boucles précédentes pour s'en écarter, voir `alternates`).
        let p = self.p;
        let (ln, s, target, md) = (&p.len, p.s, p.target(), p.min_distance());
        let bonus = |e: usize| wb.map_or(0.0, |b| b[e]);
        let (lmin, lmax, lt, dt) = (p.lmin, p.lmax, p.l, p.d.unwrap_or(1.0));
        let (x_md, wt) = (dt, dt * (1.0 + MD_MARGIN));
        let (m, n) = (p.n_edges(), p.n_nodes());
        let mut st = Loop {
            inx: vec![false; m],
            pos: vec![0; m],
            x: Vec::new(),
            deg: vec![0; n],
        };
        let (mut cur_l, mut cur_w, mut cur_b) = (0.0, 0.0, 0.0);
        for &e in init {
            cur_b += bonus(e);
            st.inx[e] = true;
            st.pos[e] = st.x.len();
            st.x.push(e);
            st.deg[p.u[e]] += 1;
            st.deg[p.v[e]] += 1;
            cur_l += ln[e];
            cur_w += w[e];
        }
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
                -TARGET_SCALE * ((l - lt).abs() / lt + (wv - dt).abs() / dt + 3.0 * out / lt)
            } else {
                wv - lam * l - if l > lmax { PEN * (l - lmax) } else { 0.0 }
            }
        };
        // ce qu'on cherche à maximiser parmi les boucles dans les bornes
        let value = |l: f64, wv: f64| if target { score(l, wv, 0.0, 0.0) } else { wv };
        let in_bounds = |l: f64| lmin <= l && l <= lmax;

        let mut lam = lam0;
        let lmid = 0.5 * (lmin + lmax);
        let mut cur = score(cur_l, cur_w, cur_b, lam);
        let mut best = if in_bounds(cur_l) && !md {
            value(cur_l, cur_w)
        } else {
            -INF
        };
        let mut best_x = st.x.clone();
        // min_distance : meilleure boucle réalisable (L, B), et repli (plus fort W sous Lmax)
        let md_ok = |l: f64, wv: f64| wv >= x_md && l <= lmax;
        let (mut best_l, mut best_b) = (INF, 0.0);
        let (mut fb_w, mut fb_x) = (-INF, Vec::new());
        if md && md_ok(cur_l, cur_w) {
            (best_l, best_b) = (cur_l, cur_b);
        }
        // Marques datées (évitent de remettre des tableaux à zéro) : couloirs et connexité.
        let (mut mark_f, mut mark_a, mut stamp) = (vec![0u64; m], vec![0u64; m], 0u64);
        let mut conn = Connectivity::new(n);
        let has_par = !p.parallel.is_empty();
        let (mut it, mut temp) = (0u64, t0);
        while !st.x.is_empty() {
            if it & 255 == 0 {
                if it >= iters || self.deadline.is_some_and(|d| Instant::now() >= d) {
                    break;
                }
                temp = t0 * (t1 / t0).powf(it as f64 / iters as f64);
                if md {
                    // trop de D+ : la distance coûte plus cher, et inversement
                    lam *= if cur_w > wt { 1.01 } else { 0.99 };
                    cur = score(cur_l, cur_w, cur_b, lam);
                } else if !target {
                    lam *= if cur_l > lmid { 1.01 } else { 0.99 };
                    cur = score(cur_l, cur_w, cur_b, lam);
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
            let new = score(cur_l + dl, cur_w + dw, cur_b + db, lam);
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
            cur = new;
            if md {
                if md_ok(cur_l, cur_w) {
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
            } else if in_bounds(cur_l) {
                let val = value(cur_l, cur_w);
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
        if let Some(f0) = self.start_face() {
            inits.push(("face".into(), f0));
        }
        for t in self.relief_targets(K) {
            if let Some(c) = self.corridor(t) {
                let d = (p.xy[t][0] - p.xy[p.s][0]).hypot(p.xy[t][1] - p.xy[p.s][1]);
                inits.push((format!("couloir {:.1} km", d / 1000.0), c));
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

    /// Jusqu'à `n` boucles réellement différentes de `existing` et entre elles : chacune part
    /// du secteur le moins recouvrant, avec le D+ des arêtes déjà prises compté à 0,6 (mode
    /// min_distance : bonus −0,4·w dans B, jamais dans W, sinon la contrainte X ment). Gardée
    /// seulement si elle est dans les bornes et partage au plus 50 % de sa longueur avec
    /// chacune des autres. Budget total `iters`. Renvoie (boucles, itérations faites).
    pub fn alternates(
        &self,
        existing: &[Vec<usize>],
        n: usize,
        iters: u64,
        seed: u64,
    ) -> (Vec<Vec<usize>>, u64) {
        const MAX_OVERLAP: f64 = 0.5;
        const DISCOUNT: f64 = 0.6;
        let p = self.p;
        let mut inits: Vec<Vec<usize>> = Vec::new();
        inits.extend(self.start_face());
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
        let (mut kept, mut out) = (existing.to_vec(), Vec::new());
        let (mut left, mut round) = (iters, 0u64);
        while out.len() < n && !inits.is_empty() {
            let per = left / (n - out.len()) as u64;
            if per < 10_000 || self.deadline.is_some_and(|d| Instant::now() >= d) {
                break;
            }
            let mut used = vec![false; p.n_edges()];
            kept.iter().flatten().for_each(|&e| used[e] = true);
            let w_eff: Vec<f64> = if p.target() || p.min_distance() {
                p.w.clone()
            } else {
                p.w.iter()
                    .zip(&used)
                    .map(|(&x, &u)| if u { x * DISCOUNT } else { x })
                    .collect()
            };
            let wb_eff: Option<Vec<f64>> = p.min_distance().then(|| {
                (0..p.n_edges())
                    .map(|e| {
                        let d = if used[e] {
                            (1.0 - DISCOUNT) * p.w[e]
                        } else {
                            0.0
                        };
                        self.wb.as_ref().map_or(0.0, |b| b[e]) - d
                    })
                    .collect()
            });
            let ov = |x: &[usize]| kept.iter().map(|k| self.overlap(x, k)).fold(0.0, f64::max);
            inits.sort_by(|x, y| ov(x).total_cmp(&ov(y)));
            // Deux départs par boucle, on garde la meilleure : un seul donne un résultat très
            // variable. Si aucune n'est valable, le tour suivant essaie d'autres départs.
            let tries: Vec<Vec<usize>> = inits.drain(..inits.len().min(2)).collect();
            let share = if inits.is_empty() { 1.0 } else { 0.6 };
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
            let mut best: Option<(f64, Vec<usize>)> = None;
            for (ids, _, it) in res {
                left = left.saturating_sub(it);
                let sc = p.score(&ids);
                if sc.3 && ov(&ids) <= MAX_OVERLAP && best.as_ref().is_none_or(|b| sc.0 > b.0) {
                    best = Some((sc.0, ids));
                }
            }
            if let Some((_, ids)) = best {
                kept.push(ids.clone());
                out.push(ids);
            }
        }
        (out, iters - left)
    }
}
