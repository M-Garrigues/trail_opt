//! Construction par waypoints + recuit simulé par remplacement de segment
//! (port de `trailopt/solvers/anneal.py`). Bon en réseau clairsemé et en mode cible, se noie
//! dans les grands graphes. Budget en itérations : une itération = une mutation.
use std::time::Instant;

use crate::faces::{Step, path_from, shortest};
use crate::{Problem, Rng};

const INF: f64 = f64::INFINITY;

pub struct Annealer<'a> {
    p: &'a Problem,
    pub rng: Rng,
    /// Pente normalisée par arête (w / l rapporté au 95e centile).
    gn: Vec<f64>,
    adj: Vec<Vec<(usize, usize)>>,
    dist_s: Vec<f64>,
    /// Mode carrefours uniques : nœuds à ne traverser qu'une fois.
    far: Option<Vec<bool>>,
    par: Vec<Vec<usize>>,
    /// α < 0 pousse vers le plat : utile en mode cible quand le D+ est trop haut.
    alpha_lo: f64,
    pub iterations: u64,
    pub deadline: Option<Instant>,
}

/// Coût de parcours : longueur réduite sur les arêtes pentues (α), bruitée (σ). Le bruit d'une
/// arête est tiré d'un hachage (sel, arête) : calculé seulement pour les arêtes que Dijkstra
/// regarde, et identique à chaque fois pendant un même appel.
fn cost<'b>(
    p: &'b Problem,
    gn: &'b [f64],
    alpha: f64,
    sigma: f64,
    salt: u64,
) -> impl Fn(usize) -> f64 + 'b {
    move |e: usize| {
        let c = p.len[e] * (1.0 - alpha * gn[e]).max(0.05);
        if sigma > 0.0 {
            let mut r = Rng::new(salt ^ (e as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
            c * (sigma * r.normal()).exp()
        } else {
            c
        }
    }
}

/// Quantile à interpolation linéaire (comme numpy.quantile).
fn quantile(xs: &[f64], q: f64) -> f64 {
    if xs.is_empty() {
        return 1.0;
    }
    let mut v = xs.to_vec();
    v.sort_by(f64::total_cmp);
    let x = q * (v.len() - 1) as f64;
    let (i, fr) = (x.floor() as usize, x.fract());
    v[i] + fr * (v[(i + 1).min(v.len() - 1)] - v[i])
}

impl<'a> Annealer<'a> {
    pub fn new(p: &'a Problem, seed: u64) -> Annealer<'a> {
        let g: Vec<f64> = (0..p.n_edges())
            .map(|e| p.w[e] / p.len[e].max(1.0))
            .collect();
        let q = quantile(&g, 0.95).max(1e-6);
        let mut adj = vec![Vec::new(); p.n_nodes()];
        for e in 0..p.n_edges() {
            adj[p.u[e]].push((e, p.v[e]));
            if p.u[e] != p.v[e] {
                adj[p.v[e]].push((e, p.u[e]));
            }
        }
        let (dist_s, _) = shortest(&adj, |e| p.len[e], p.s, None, None, None);
        let far = p.node_simple.then(|| {
            let mut f = vec![false; p.n_nodes()];
            p.far.iter().for_each(|&x| f[x] = true);
            f
        });
        Annealer {
            p,
            rng: Rng::new(seed),
            gn: g.iter().map(|x| x / q).collect(),
            adj,
            dist_s,
            far,
            par: p.par_lists(),
            alpha_lo: if p.target() { -0.95 } else { 0.0 },
            iterations: 0,
            deadline: None,
        }
    }

    fn late(&self) -> bool {
        self.deadline.is_some_and(|d| Instant::now() >= d)
    }

    /// Marque les arêtes et leurs tronçons parallèles (à bannir ensemble).
    fn ban(&self, mask: &mut [bool], steps: &[Step]) {
        for &(e, _, _) in steps {
            mask[e] = true;
            self.par[e].iter().for_each(|&q| mask[q] = true);
        }
    }

    fn ids(route: &[Step]) -> Vec<usize> {
        route.iter().map(|s| s.0).collect()
    }

    pub fn evaluate(&self, route: &[Step]) -> (f64, f64, f64, bool) {
        self.p.score(&Self::ids(route))
    }

    /// Boucle par 1 à 4 waypoints à 12–42 % de L du départ, reliés par plus courts chemins
    /// sous un coût aléatoire, sans réutiliser d'arête (ni de carrefour lointain).
    pub fn construct(&mut self) -> Option<Vec<Step>> {
        let (p, n) = (self.p, self.p.n_nodes());
        let d = &self.dist_s;
        let mut cands: Vec<usize> = (0..n)
            .filter(|&x| 0.12 * p.l <= d[x] && d[x] <= 0.42 * p.l)
            .collect();
        if cands.is_empty() {
            cands = (0..n).filter(|&x| d[x] < INF && x != p.s).collect();
        }
        if cands.is_empty() {
            return None;
        }
        let mut k = [1, 2, 2, 3, 3, 4][self.rng.below(6)].min(cands.len());
        // points de passage (T34) : tous imposés, plus 0 à 2 waypoints libres pour la distance
        let mut wps: Vec<usize> = p.via.iter().copied().filter(|&x| x != p.s).collect();
        wps.dedup();
        if !p.via.is_empty() {
            k = wps.len() + self.rng.below(3).min(cands.len());
        }
        for _ in 0..1000 {
            if wps.len() >= k {
                break;
            }
            let c = cands[self.rng.below(cands.len())];
            if !wps.contains(&c) {
                wps.push(c);
            }
        }
        let ang = |x: usize| p.xy[x][1].atan2(p.xy[x][0]);
        wps.sort_by(|&a, &b| ang(a).total_cmp(&ang(b)));
        if self.rng.random() < 0.5 {
            wps.reverse();
        }
        let a = self.rng.uniform(self.alpha_lo, 0.95);
        let cost = cost(p, &self.gn, a, 0.3, self.rng.next_u64());
        let (mut route, mut used, mut cur) = (Vec::new(), vec![false; p.n_edges()], p.s);
        let mut visited = vec![false; n];
        for t in wps.into_iter().chain([p.s]) {
            if self.far.is_some() && visited[t] {
                continue; // waypoint déjà traversé
            }
            let bn = self.far.as_ref().map(|_| visited.as_slice());
            let (dist, prev) = shortest(&self.adj, &cost, cur, Some(t), Some(&used), bn);
            if dist[t] == INF {
                return None;
            }
            let seg = path_from(&prev, cur, t);
            self.ban(&mut used, &seg);
            if let Some(far) = &self.far {
                seg.iter()
                    .filter(|s| far[s.2])
                    .for_each(|s| visited[s.2] = true);
            }
            route.extend(seg);
            cur = t;
        }
        (!route.is_empty() && p.parallel_ok(&Self::ids(&route))).then_some(route)
    }

    fn random_node_near(&mut self, c: [f64; 2], r: f64) -> usize {
        let xy = &self.p.xy;
        let idx: Vec<usize> = (0..xy.len())
            .filter(|&i| (xy[i][0] - c[0]).hypot(xy[i][1] - c[1]) <= r)
            .collect();
        if idx.is_empty() {
            self.rng.below(xy.len())
        } else {
            idx[self.rng.below(idx.len())]
        }
    }

    /// Remplace route[i..j] par un plus court chemin a->b (direct ou via c) qui évite les
    /// arêtes du reste de la boucle : la boucle reste valide.
    fn mutate(&mut self, route: &[Step], length: f64) -> Option<Vec<Step>> {
        let p = self.p;
        let k = route.len();
        let nodes: Vec<usize> = std::iter::once(route[0].1)
            .chain(route.iter().map(|s| s.2))
            .collect();
        let seg = 1 + self.rng.below(k.min(25));
        let i = self.rng.below(k - seg + 1);
        let j = i + seg;
        let (a, b) = (nodes[i], nodes[j]);
        let mut banned = vec![false; p.n_edges()];
        self.ban(&mut banned, &route[..i]);
        self.ban(&mut banned, &route[j..]);
        // nœuds du reste de la boucle : interdits en transit
        let mut bn = self.far.as_ref().map(|far| {
            let mut b = vec![false; p.n_nodes()];
            nodes[..=i]
                .iter()
                .chain(&nodes[j..])
                .filter(|&&x| far[x])
                .for_each(|&x| b[x] = true);
            b
        });
        let alpha = self.rng.uniform(self.alpha_lo, 0.95);
        let sigma = self.rng.uniform(0.0, 0.5);
        let salt = self.rng.next_u64();
        let (lo, hi) = if p.target() {
            (0.97 * p.l, 1.03 * p.l)
        } else {
            (p.lmin, p.lmax)
        };
        let p_via = if length < lo {
            0.75
        } else if length > hi {
            0.2
        } else {
            0.45
        };
        // étape intermédiaire c près du milieu de a-b (tirée avant d'emprunter le coût)
        let via = (self.rng.random() < p_via).then(|| {
            let mid = [
                (p.xy[a][0] + p.xy[b][0]) / 2.0,
                (p.xy[a][1] + p.xy[b][1]) / 2.0,
            ];
            let r = self.rng.uniform(80.0, (0.2 * p.l).max(100.0));
            self.random_node_near(mid, r)
        });
        let cost = cost(p, &self.gn, alpha, sigma, salt);
        let new = if let Some(c) = via {
            if bn.as_ref().is_some_and(|b| b[c]) {
                return None;
            }
            let (d1, p1) = shortest(&self.adj, &cost, a, Some(c), Some(&banned), bn.as_deref());
            if d1[c] == INF {
                return None;
            }
            let leg1 = path_from(&p1, a, c);
            if let (Some(b), Some(far)) = (bn.as_mut(), &self.far) {
                leg1.iter().filter(|s| far[s.2]).for_each(|s| b[s.2] = true);
                b[a] |= far[a];
            }
            self.ban(&mut banned, &leg1);
            let (d2, p2) = shortest(&self.adj, &cost, c, Some(b), Some(&banned), bn.as_deref());
            if d2[b] == INF {
                return None;
            }
            [leg1, path_from(&p2, c, b)].concat()
        } else {
            let (d, pr) = shortest(&self.adj, &cost, a, Some(b), Some(&banned), bn.as_deref());
            if d[b] == INF {
                return None;
            }
            path_from(&pr, a, b)
        };
        // le nouveau segment ne doit pas longer son propre couloir
        if !p.parallel.is_empty() && !p.parallel_ok(&Self::ids(&new)) {
            return None;
        }
        let out = [&route[..i], &new, &route[j..]].concat();
        (!out.is_empty()).then_some(out)
    }

    /// Pool de boucles par waypoints, puis `iters` mutations. Renvoie la meilleure boucle
    /// (réalisable si possible) ou None.
    pub fn run(&mut self, iters: u64) -> Option<Vec<Step>> {
        let p = self.p;
        // Constructions : ~15 % du budget en temps côté Python, au plus 300 boucles.
        let mut pool = Vec::new();
        let mut tries = 0;
        while tries < (iters / 15).max(20) && pool.len() < 300 && !self.late() {
            tries += 1;
            if let Some(r) = self.construct() {
                pool.push((self.evaluate(&r), r));
            }
        }
        if pool.is_empty() {
            return None;
        }
        pool.sort_by(|a, b| b.0.0.total_cmp(&a.0.0)); // tri stable, comme Python
        let ((mut cur_sc, mut cur_len, cur_dp, _), mut cur) = pool[0].clone();
        let mut best = (cur_sc, cur.clone());
        let mut best_feas: Option<(f64, Vec<Step>)> = None;
        for ((sc, _, _, fe), r) in &pool {
            if *fe && best_feas.as_ref().is_none_or(|b| *sc > b.0) {
                best_feas = Some((*sc, r.clone()));
            }
        }
        let (t0, t1) = if p.target() {
            (0.03, 0.001)
        } else {
            (0.05 * cur_dp.max(10.0), 0.002 * cur_dp.max(10.0))
        };
        let (mut it, mut stall) = (0u64, 0u32);
        while it < iters && !(it & 15 == 0 && self.late()) {
            let temp = t0 * (t1 / t0).powf(it as f64 / iters as f64);
            it += 1;
            stall += 1;
            let Some(new) = self.mutate(&cur, cur_len) else {
                continue;
            };
            let (sc, ln, _, fe) = self.evaluate(&new);
            if sc >= cur_sc || self.rng.random() < ((sc - cur_sc) / temp).exp() {
                (cur, cur_sc, cur_len) = (new, sc, ln);
                if sc > best.0 {
                    (best, stall) = ((sc, cur.clone()), 0);
                }
                if fe && best_feas.as_ref().is_none_or(|b| sc > b.0) {
                    (best_feas, stall) = (Some((sc, cur.clone())), 0);
                }
            }
            if stall > 1500 {
                // retour au meilleur connu
                cur = best_feas.as_ref().map_or(&best.1, |b| &b.1).clone();
                (cur_sc, cur_len, _, _) = self.evaluate(&cur);
                stall = 0;
            }
        }
        self.iterations += it;
        Some(best_feas.map_or(best.1, |b| b.1))
    }
}
