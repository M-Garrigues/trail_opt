//! Instance du solveur lue depuis le JSON exporté par Python (`scripts/export_problem.py`,
//! contrat `.team/contracts/problem.md`), et invariants d'une boucle (port de `check_loop`).
use serde::Deserialize;

/// Version 2 (D19) : mode "min_distance", champs `q` et `climbs`. La version 1 reste lue.
pub const VERSION: u32 = 2;

/// Préférence de montées (modes.md B) : w de recherche = w + γ·β·q / H_SCALE.
pub const CLIMBS_BETA: f64 = 0.5;
pub const CLIMBS_H_SCALE: f64 = 100.0;
/// Mode min_distance : pénalité par mètre de D+ manquant dans le score des boucles hors
/// contrainte (1 m de D+ manquant « coûte » 100 m de distance, prototype `minlen.py`).
const MD_MISS: f64 = 100.0;
const MD_OVER: f64 = 10.0;

#[derive(Deserialize, Clone)]
pub struct Problem {
    pub version: u32,
    /// "max" (maximiser le D+), "target" (viser le couple (L, D)) ou "min_distance" (boucle
    /// la plus courte de D+ >= D, sous Lmax ; Lmin = 0, L indicatif).
    pub mode: String,
    #[serde(rename = "L")]
    pub l: f64,
    #[serde(rename = "Lmin")]
    pub lmin: f64,
    #[serde(rename = "Lmax")]
    pub lmax: f64,
    #[serde(rename = "D")]
    pub d: Option<f64>,
    /// Nœud de départ.
    pub s: usize,
    /// Carrefours uniques : degré <= 2 sur les nœuds `far`.
    pub node_simple: bool,
    /// Coordonnées des nœuds (m, repère local).
    pub xy: Vec<[f64; 2]>,
    /// Nœuds hors du rayon libre autour du départ.
    pub far: Vec<usize>,
    pub u: Vec<usize>,
    pub v: Vec<usize>,
    pub len: Vec<f64>,
    /// (montée + descente) / 2 : la somme sur une boucle fermée vaut son D+.
    pub w: Vec<f64>,
    /// Angle du premier segment de l'arête en partant de u, et de v.
    pub ang_u: Vec<f64>,
    pub ang_v: Vec<f64>,
    /// Couloirs parallèles : au plus une des deux arêtes dans la boucle.
    pub parallel: Vec<[usize; 2]>,
    /// v2 : par arête, ½ (Σ G² des montées dans le sens u→v + Σ G² dans le sens v→u) (m²).
    /// Vide si `climbs` = 0.
    #[serde(default)]
    pub q: Vec<f64>,
    /// v2 : préférence de montées γ : −1 courtes, 0 équilibré, +1 longues.
    #[serde(default)]
    pub climbs: i8,
}

impl Problem {
    pub fn from_json(text: &str) -> Result<Problem, String> {
        let p: Problem = serde_json::from_str(text).map_err(|e| format!("JSON invalide : {e}"))?;
        p.validate()?;
        Ok(p)
    }

    pub fn n_nodes(&self) -> usize {
        self.xy.len()
    }

    pub fn n_edges(&self) -> usize {
        self.len.len()
    }

    pub fn target(&self) -> bool {
        self.mode == "target"
    }

    pub fn min_distance(&self) -> bool {
        self.mode == "min_distance"
    }

    /// Bonus de préférence de montées par arête (γ·β·q/H), seulement pour « courtes » (γ < 0) ;
    /// « longues » = seul choix du sens de parcours (D23).
    pub fn climb_bonus(&self) -> Option<Vec<f64>> {
        let g = self.climbs as f64 * CLIMBS_BETA / CLIMBS_H_SCALE;
        (self.climbs < 0).then(|| self.q.iter().map(|&q| g * q).collect())
    }

    /// Modes max et cible avec préférence de montées : copie dont `w` est le poids de
    /// recherche w + γβq/H (le D+ rapporté se recalcule sur le vrai w).
    pub fn search_problem(&self) -> Option<Problem> {
        let b = self.climb_bonus().filter(|_| !self.min_distance())?;
        let mut p = self.clone();
        p.w.iter_mut().zip(b).for_each(|(w, b)| *w += b);
        p.climbs = 0;
        Some(p)
    }

    /// reach[e] = d(s,u) + len[e] + d(v,s) : longueur minimale d'une boucle qui prend e.
    pub fn reach(&self) -> Vec<f64> {
        let mut adj = vec![Vec::new(); self.n_nodes()];
        for e in 0..self.n_edges() {
            adj[self.u[e]].push((e, self.v[e]));
            adj[self.v[e]].push((e, self.u[e]));
        }
        let (d, _) = crate::faces::shortest(&adj, |e| self.len[e], self.s, None, None, None);
        (0..self.n_edges())
            .map(|e| d[self.u[e]] + self.len[e] + d[self.v[e]])
            .collect()
    }

    /// Cohérence des tailles et des indices (le fichier vient d'un autre processus).
    fn validate(&self) -> Result<(), String> {
        if !(1..=VERSION).contains(&self.version) {
            return Err(format!("version {} (attendue 1 à {VERSION})", self.version));
        }
        let (n, m) = (self.n_nodes(), self.n_edges());
        if [
            self.u.len(),
            self.v.len(),
            self.w.len(),
            self.ang_u.len(),
            self.ang_v.len(),
        ]
        .iter()
        .any(|&k| k != m)
        {
            return Err("tableaux d'arêtes de tailles différentes".into());
        }
        if !matches!(self.mode.as_str(), "max" | "target" | "min_distance") {
            return Err(format!("mode inconnu : {}", self.mode));
        }
        if (self.target() || self.min_distance()) && !self.d.is_some_and(|d| d > 0.0) {
            return Err("D+ cible requis en modes target et min_distance".into());
        }
        if !(-1..=1).contains(&self.climbs)
            || (self.climbs < 0 && self.q.len() != m)
            || self.q.iter().any(|x| !x.is_finite() || *x < 0.0)
        {
            return Err("climbs hors de -1..1, ou q absent ou invalide".into());
        }
        let bad = |x: usize| x >= n;
        if bad(self.s)
            || self
                .u
                .iter()
                .chain(&self.v)
                .chain(&self.far)
                .any(|&x| bad(x))
        {
            return Err("indice de nœud hors bornes".into());
        }
        if self.parallel.iter().flatten().any(|&e| e >= m) {
            return Err("indice d'arête hors bornes dans parallel".into());
        }
        if self
            .len
            .iter()
            .chain(&self.w)
            .any(|x| !x.is_finite() || *x < 0.0)
        {
            return Err("longueur ou poids invalide".into());
        }
        Ok(())
    }

    /// par[e] : arêtes du même couloir que e.
    pub fn par_lists(&self) -> Vec<Vec<usize>> {
        let mut par = vec![Vec::new(); self.n_edges()];
        for &[a, b] in &self.parallel {
            par[a].push(b);
            par[b].push(a);
        }
        par
    }

    /// Au plus une arête par couloir parallèle.
    pub fn parallel_ok(&self, ids: &[usize]) -> bool {
        let mut inx = vec![false; self.n_edges()];
        ids.iter().for_each(|&e| inx[e] = true);
        !self.parallel.iter().any(|&[a, b]| inx[a] && inx[b])
    }

    /// (longueur, D+) d'un ensemble d'arêtes.
    pub fn stats(&self, ids: &[usize]) -> (f64, f64) {
        ids.iter()
            .fold((0.0, 0.0), |(l, d), &e| (l + self.len[e], d + self.w[e]))
    }

    /// (score à maximiser, longueur, D+, réalisable), comme `Problem.score` en Python.
    pub fn score(&self, ids: &[usize]) -> (f64, f64, f64, bool) {
        let (length, dplus) = self.stats(ids);
        if self.min_distance() {
            let x = self.d.unwrap();
            let (miss, over) = ((x - dplus).max(0.0), (length - self.lmax).max(0.0));
            let sc = -length - MD_MISS * miss - MD_OVER * over;
            return (sc, length, dplus, miss == 0.0 && over == 0.0);
        }
        let feas = self.lmin <= length && length <= self.lmax;
        if self.target() {
            let d = self.d.unwrap();
            let err = (length - self.l).abs() / self.l + (dplus - d).abs() / d;
            return (-err, length, dplus, feas);
        }
        let viol = (self.lmin - length).max(length - self.lmax).max(0.0);
        (dplus - 0.5 * viol, length, dplus, feas)
    }

    /// Sac à dos fractionnaire (w/l décroissant, capacité Lmax) : borne sur le D+.
    pub fn dplus_upper_bound(&self) -> f64 {
        knapsack_ub(&self.len, &self.w, self.lmax)
    }

    /// Circuit eulérien [(arête, de, vers)] partant de s (Hierholzer, port de
    /// `graph.euler_circuit`) ; erreur si `ids` n'est pas une boucle fermée connexe.
    pub fn euler(&self, ids: &[usize]) -> Result<Vec<(usize, usize, usize)>, String> {
        let mut adj: Vec<Vec<(usize, usize)>> = vec![Vec::new(); self.n_nodes()];
        for &e in ids {
            adj[self.u[e]].push((e, self.v[e]));
            if self.u[e] != self.v[e] {
                adj[self.v[e]].push((e, self.u[e]));
            }
        }
        let mut used = vec![false; self.n_edges()];
        let mut ptr = vec![0usize; self.n_nodes()];
        let mut stack = vec![(self.s, usize::MAX, usize::MAX)];
        let mut circuit = Vec::new();
        while let Some(&(n, e_in, from)) = stack.last() {
            while ptr[n] < adj[n].len() && used[adj[n][ptr[n]].0] {
                ptr[n] += 1;
            }
            if ptr[n] < adj[n].len() {
                let (e, nb) = adj[n][ptr[n]];
                used[e] = true;
                stack.push((nb, e, n));
            } else {
                stack.pop();
                if e_in != usize::MAX {
                    circuit.push((e_in, from, n));
                }
            }
        }
        circuit.reverse();
        let ok = !circuit.is_empty()
            && circuit.len() == ids.len()
            && circuit[0].1 == self.s
            && circuit[circuit.len() - 1].2 == self.s;
        if !ok {
            return Err("sous-graphe non eulérien ou non connexe".into());
        }
        Ok(circuit)
    }

    /// Invariants d'une boucle (port de `tests/conftest.py::check_loop`, partie topologique) :
    /// non vide, arêtes uniques, degrés pairs (donc fermée), passe par s, connexe, au plus une
    /// arête par couloir parallèle, degré <= 2 sur les nœuds lointains en mode carrefours uniques.
    pub fn check(&self, ids: &[usize]) -> Result<(), String> {
        let (n, m) = (self.n_nodes(), self.n_edges());
        if ids.is_empty() {
            return Err("boucle vide".into());
        }
        let mut inx = vec![false; m];
        let mut deg = vec![0u32; n];
        let mut adj = vec![Vec::new(); n];
        for &e in ids {
            if e >= m {
                return Err(format!("arête {e} hors bornes"));
            }
            if inx[e] {
                return Err(format!("arête répétée : {e}"));
            }
            inx[e] = true;
            let (a, b) = (self.u[e], self.v[e]);
            deg[a] += 1;
            deg[b] += 1;
            adj[a].push(b);
            adj[b].push(a);
        }
        if let Some(x) = (0..n).find(|&x| deg[x] % 2 == 1) {
            return Err(format!("degré impair au nœud {x}"));
        }
        if deg[self.s] == 0 {
            return Err("la boucle ne passe pas par le départ".into());
        }
        let mut seen = vec![false; n];
        let mut stack = vec![self.s];
        seen[self.s] = true;
        while let Some(x) = stack.pop() {
            for &y in &adj[x] {
                if !seen[y] {
                    seen[y] = true;
                    stack.push(y);
                }
            }
        }
        if (0..n).any(|x| deg[x] > 0 && !seen[x]) {
            return Err("boucle non connexe".into());
        }
        if let Some([a, b]) = self.parallel.iter().find(|[a, b]| inx[*a] && inx[*b]) {
            return Err(format!("couloir parallèle emprunté deux fois : {a}, {b}"));
        }
        if self.node_simple
            && let Some(x) = self.far.iter().find(|&&x| deg[x] > 2)
        {
            return Err(format!("carrefour repassé hors du rayon libre : nœud {x}"));
        }
        Ok(())
    }
}

/// Sac à dos fractionnaire inverse : longueur minimale d'un ensemble d'arêtes de Σw >= x
/// (w/l décroissant, dernière arête au prorata). None si Σw < x.
fn knap_len(len: &[f64], w: &[f64], ids: &[usize], x: f64) -> Option<f64> {
    let mut order: Vec<usize> = ids.iter().copied().filter(|&e| w[e] > 0.0).collect();
    order.sort_by(|&a, &b| (w[b] / len[b].max(1e-9)).total_cmp(&(w[a] / len[a].max(1e-9))));
    let (mut cw, mut cl) = (0.0, 0.0);
    for e in order {
        if cw + w[e] >= x {
            return Some(cl + len[e] * (x - cw) / w[e]);
        }
        cw += w[e];
        cl += len[e];
    }
    None
}

/// Borne inférieure de la longueur d'une boucle de D+ >= x (contrat modes.md A) : une boucle
/// de longueur L n'emprunte que des arêtes de reach <= L, donc
/// LB = min_i max(r_i, knap({e : reach[e] <= r_i}, x)), knap décroissant en r_i : dichotomie.
/// None si Σw < x (aucune distance ne suffit).
pub fn length_lower_bound(len: &[f64], w: &[f64], reach: &[f64], x: f64) -> Option<f64> {
    let mut o: Vec<usize> = (0..len.len()).filter(|&e| reach[e].is_finite()).collect();
    o.sort_by(|&a, &b| reach[a].total_cmp(&reach[b]));
    let r: Vec<f64> = o.iter().map(|&e| reach[e]).collect();
    // arêtes de reach <= r[i] : o[..fin(i)]
    let fin = |i: usize| r.partition_point(|&x| x <= r[i]);
    let at = |i: usize| knap_len(len, w, &o[..fin(i)], x).map(|k| k.max(r[i]));
    let n = r.len();
    at(n.checked_sub(1)?)?;
    let (mut lo, mut hi) = (0, n - 1);
    while lo < hi {
        // premier i où knap(S_i) <= r_i
        let mid = (lo + hi) / 2;
        if knap_len(len, w, &o[..fin(mid)], x).is_some_and(|k| k <= r[mid]) {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    let a = at(lo).unwrap_or(f64::INFINITY);
    let b = lo.checked_sub(1).and_then(at).unwrap_or(f64::INFINITY);
    Some(a.min(b))
}

/// Sac à dos fractionnaire : Σw maximale pour une longueur `cap` (w/l décroissant).
pub fn knapsack_ub(len: &[f64], w: &[f64], cap: f64) -> f64 {
    let mut order: Vec<usize> = (0..len.len()).collect();
    order.sort_by(|&a, &b| (w[b] / len[b].max(1e-9)).total_cmp(&(w[a] / len[a].max(1e-9))));
    let (mut room, mut ub) = (cap, 0.0);
    for e in order {
        if len[e] <= room {
            room -= len[e];
            ub += w[e];
        } else {
            ub += w[e] * room / len[e].max(1e-9);
            break;
        }
    }
    ub
}
