//! Instance du solveur lue depuis le JSON exporté par Python (`scripts/export_problem.py`,
//! contrat `.team/contracts/problem.md`), et invariants d'une boucle (port de `check_loop`).
use serde::Deserialize;

pub const VERSION: u32 = 1;

#[derive(Deserialize)]
pub struct Problem {
    pub version: u32,
    /// "max" (maximiser le D+) ou "target" (viser le couple (L, D)).
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

    /// Cohérence des tailles et des indices (le fichier vient d'un autre processus).
    fn validate(&self) -> Result<(), String> {
        if self.version != VERSION {
            return Err(format!("version {} (attendue {VERSION})", self.version));
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
        if !matches!(self.mode.as_str(), "max" | "target") {
            return Err(format!("mode inconnu : {}", self.mode));
        }
        if self.target() && !self.d.is_some_and(|d| d > 0.0) {
            return Err("D+ cible requis en mode target".into());
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
        let feas = self.lmin <= length && length <= self.lmax;
        if self.target() {
            let d = self.d.unwrap();
            let err = (length - self.l).abs() / self.l + (dplus - d).abs() / d;
            return (-err, length, dplus, feas);
        }
        let viol = (self.lmin - length).max(length - self.lmax).max(0.0);
        (dplus - 0.5 * viol, length, dplus, feas)
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
