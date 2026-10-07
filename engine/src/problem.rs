//! Instance du solveur lue depuis le JSON exporté par Python (`scripts/export_problem.py`,
//! contrat `.team/contracts/problem.md`), et invariants d'une boucle (port de `check_loop`).
use serde::Deserialize;

/// Version 2 (D19) : mode "min_distance", champs `q` et `climbs`. La version 1 reste lue.
pub const VERSION: u32 = 2;

/// Préférence de montées (modes.md B) : w de recherche = w + γ·β·q / H_SCALE.
pub const CLIMBS_BETA: f64 = 0.5;
pub const CLIMBS_H_SCALE: f64 = 100.0;
/// T33 (modes.md v0.3) : « longues », coût par montée dans la recherche = REL · Σq/Σw (m de D+),
/// plafonné à MAX : ville (Σq/Σw ≈ 10) 30 m, Monts d'Or 60 m, montagne (≈ 100) 60 m.
pub const CLIMBS_LONG_REL: f64 = 3.0;
pub const CLIMBS_LONG_MAX: f64 = 60.0;
/// Mode min_distance : pénalité par mètre de D+ manquant dans le score des boucles hors
/// contrainte (1 m de D+ manquant « coûte » 100 m de distance, prototype `minlen.py`).
const MD_MISS: f64 = 100.0;
const MD_OVER: f64 = 10.0;
/// D40 : voir `score_free`.
const LEN_DENSITY: f64 = 3.0;
/// Points de passage (T34) : pénalité par point manqué dans `score`, au-dessus de tout écart
/// réaliste (la réalisabilité l'exige de toute façon).
pub const VIA_MISS: f64 = 1e6;
/// Mode cible : écart relatif admis sur la distance et sur le D+ pour une autre boucle (un peu
/// sous les 10 % du message `target_not_reached`, l'aller-retour d'accès s'ajoutant ensuite).
pub const TARGET_TOL: f64 = 0.08;
/// Préférence de type de voie (api.md v1.7, `surface`) : jamais un filtre, un prix sur la longueur
/// parcourue hors du type voulu (`Problem::off`). Les contraintes restent sur les valeurs réelles.
/// Pas en mode min_distance : la boucle la plus courte prime.
/// D63 : poids de CONFORT par mode (modes.md § Confort). Unité commune : le « mètre de confort »
/// (`Problem::off` hors grands axes : mètre hors du type voulu, étiquettes) ; chaque mode lui donne
/// un prix dans son objectif, comparé par C = part de l'objectif que coûte une sortie entièrement
/// hors confort. Max D+ et Le plus court : performance, confort LÉGER ; Cible : confort FORT.
/// Mode max : un mètre de confort vaut `off_price` m de D+ (posé par `plan`) = `SURF_MAX` × la
/// densité de D+ des meilleures arêtes de la zone (borne du sac à dos / Lmax, ≈ 2 × la densité de
/// la meilleure boucle) : C ≈ 2 × `SURF_MAX`. (0,6 avant D63 : C ≈ 120 %.)
pub const SURF_MAX: f64 = 0.1;
/// Mode cible : la distance et le D+ restent tenus (sous `SURF_TARGET_BAND` d'erreur relative,
/// distance + D+, l'erreur ne compte plus) ; le confort compte toujours, en plus : une boucle
/// entièrement hors confort « coûte » `SURF_TARGET` d'erreur (C ; 3 % avant D63).
pub const SURF_TARGET_BAND: f64 = 0.02;
pub const SURF_TARGET: f64 = 0.15;
/// Le plus court (D63 : « un peu » de préférence, 0 avant) : un mètre de confort vaut
/// `COMFORT_MD` m de longueur (C).
pub const COMFORT_MD: f64 = 0.05;
/// D57 : prix d'un mètre de grand axe en mode max, en multiples de la densité de D+ de la borne du
/// sac à dos (0,6 = poids 1 de D57). RETIRÉ du mode max (0) le 07/10 : la 2e sortie de Massy 10 km
/// perdait 3 à 9 % de D+ selon le prix (0,1 à 0,6 ; médianes sur 8 graines, critère D52 : ≤ 3 % par
/// rang contre 290e2c7) ; sans lui +0,4 %. En max, un grand axe ne coûte donc que la préférence de
/// type (route) ; il reste fortement pénalisé en Cible et en Le plus court (`MAJOR_K`).
pub const MAJOR_PRICE_MAX: f64 = 0.0;
/// D50 (fondateur, 2026-10-07) : un mètre sur grand axe (route d'importance 1 à 3) compte `MAJOR_K`
/// mètres « hors type » de plus, dans tous les modes et toutes les préférences : emprunté
/// seulement s'il est indispensable (pont, liaison courte sans autre voie). Jamais un filtre.
/// En min_distance, c'est `MAJOR_K` mètres de longueur en plus (`score`, `search_bonus`).
pub const MAJOR_K: f64 = 4.0;
/// D57 : en mode max, poids du grand axe réduit (perte de D+ ≤ 3 % à Massy contre aucune pénalité).
pub const MAJOR_K_MAX: f64 = 1.0;
/// Part minimale de la longueur dans le coût de recherche d'une arête (`Problem::search_len`).
pub const MIN_SEARCH_LEN: f64 = 0.1;
/// Élégance (D52) : prix d'un demi-tour à un carrefour (voir `turn_cost`). Cible : en unités du score
/// des faces (`TARGET_SCALE` = 100 % d'erreur) : 1 = 0,1 % d'erreur, ne départage que dans la bande.
/// Banc du 07/10 (cible, médianes) : 1 → virages aux carrefours −58 % plaine / −47 % montagne, chemin
/// −2,4 pts ; 2 → −4,9 pts de chemin ; 5 (version douce) → −14 pts.
pub const TURN_TARGET: f64 = 1.0;
/// Mode max : prix d'un demi-tour = TURN_MAX_FRAC × densité de D+ de la borne sac à dos × 100 m.
pub const TURN_MAX_FRAC: f64 = 0.3;

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
    /// v2.1 (T24) : par arête, signe de la pente en partant de u et de v (+1 monte, −1 descend,
    /// 0 plat), hystérésis 5 m. Vide : pas de terme « longues » (choix du sens seul).
    #[serde(default)]
    pub turn: Vec<[i8; 2]>,
    /// v2.1 (T24) : par arête, ½ nombre d'extrema intérieurs (hystérésis 5 m), symétrique.
    #[serde(default)]
    pub inner: Vec<f64>,
    /// Coût par montée (m de D+) du problème de RECHERCHE (posé par `search_problem`), 0 sinon.
    #[serde(skip)]
    pub node_mu: f64,
    /// D52 : prix d'un virage aux carrefours (`turn_cost`), posé par `plan::to_problem`, 0 sinon.
    /// Terme du score seulement, jamais dans un coût de chemin (`search_len` reste > 0).
    #[serde(skip)]
    pub turn_mu: f64,
    /// Carrefours (degré >= 3) pour `node_costs`, posé avec `turn_mu` ; vide = recalculé.
    #[serde(skip)]
    pub junction: Vec<bool>,
    /// Directions (cos, sin) au départ de u puis de v, pour `turn_cost` ; vide = calculées.
    #[serde(skip)]
    pub dir: Vec<[f64; 4]>,
    /// D62 : paquet « élégance » (virages, pétales, une direction par sortie) actif. Faux : le
    /// moteur se comporte exactement comme avant D52 (`enable_smooth`).
    #[serde(skip)]
    pub smooth: bool,
    /// Points de passage obligatoires (T34, api.md v1.5) : nœuds que la boucle doit toucher.
    #[serde(default)]
    pub via: Vec<usize>,
    /// Par arête, longueur (m) « hors type » pondérée : hors du type de voie voulu, plus `MAJOR_K`
    /// fois la longueur sur grand axe (D50). Vide : rien à pénaliser.
    #[serde(default)]
    pub off: Vec<f64>,
    /// Mode max : valeur (m de D+) d'un mètre sur le type voulu.
    #[serde(default)]
    pub off_price: f64,
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

    /// Bonus B par arête du recuit par faces, dans les unités de son score : préférence de montées
    /// « courtes » en min_distance (où `w` reste le D+ réel), ou type de voie (modes max et cible).
    /// Mode max : prime `off_price` par mètre SUR le bon type (à longueur égale, c'est un coût par
    /// mètre hors type ; en coût, les faces de ville seraient toutes négatives et la boucle
    /// n'atteindrait pas Lmin). Mode cible : coût par mètre hors type.
    pub fn search_bonus(&self) -> Option<Vec<f64>> {
        if self.min_distance() {
            // grands axes : coût en D+ au prix moyen D/L de la sortie cherchée (λ des faces)
            if self.off.is_empty() {
                return self.climb_bonus();
            }
            let c = self.d.unwrap_or(0.0) / self.l.max(1.0);
            let b = self.climb_bonus();
            return Some(
                (0..self.n_edges())
                    .map(|e| b.as_ref().map_or(0.0, |b| b[e]) - c * self.off[e])
                    .collect(),
            );
        }
        if self.off.is_empty() {
            return None;
        }
        let c = crate::faces::TARGET_SCALE * SURF_TARGET / self.l;
        Some(
            (0..self.n_edges())
                .map(|e| {
                    if self.target() {
                        -c * self.off[e]
                    } else {
                        self.off_price * (self.len[e] - self.off[e])
                    }
                })
                .collect(),
        )
    }

    /// Longueur de RECHERCHE de l'arête e (plus courts chemins du recuit) : longueur plus le
    /// « hors type » (`off`, négatif pour un bonus d'étiquette), jamais sous `MIN_SEARCH_LEN` ×
    /// la longueur, quels que soient les poids : un coût négatif faisait boucler Dijkstra sans fin
    /// (mémoire sans borne, gel du poste le 2026-10-07 avec des poids d'étiquettes ×5).
    pub fn search_len(&self, e: usize) -> f64 {
        let off = self.off.get(e).copied().unwrap_or(0.0);
        (self.len[e] + off).max(MIN_SEARCH_LEN * self.len[e])
    }

    /// Longueur hors du type de voie voulu (0 sans préférence).
    pub fn off_len(&self, ids: &[usize]) -> f64 {
        if self.off.is_empty() {
            return 0.0;
        }
        ids.iter().map(|&e| self.off[e]).sum()
    }

    /// Mode max avec préférence de montées : copie dont `w` est le poids de recherche (le D+
    /// rapporté se recalcule sur le vrai w). « Courtes » : w + γβq/H ; « longues » (si
    /// `turn`/`inner`) : w − μ·inner et coût μ par extremum aux nœuds. Pas en mode cible : la
    /// cible de D+ se tient sur le D+ RÉEL (avec un poids modifié, « courtes » rendait 995 m pour
    /// 600 demandés à Bourg) ; la préférence n'y choisit que le sens de parcours (`assemble`).
    pub fn search_problem(&self) -> Option<Problem> {
        if self.min_distance() || self.target() {
            return None;
        }
        let (b, mu) = (self.climb_bonus(), self.long_mu());
        if b.is_none() && mu == 0.0 {
            return None;
        }
        let mut p = self.clone();
        if let Some(b) = b {
            p.w.iter_mut().zip(b).for_each(|(w, b)| *w += b);
        }
        if mu != 0.0 {
            // montées intérieures aux arêtes : coût additif ; aux nœuds : `node_costs`
            p.w.iter_mut()
                .zip(&self.inner)
                .for_each(|(w, c)| *w -= mu * c);
            p.node_mu = mu;
        }
        p.climbs = 0;
        Some(p)
    }

    /// T33 : coût par montée (m de D+, « longues » seulement), proportionnel
    /// à l'échelle des montées du terrain Σq/Σw, plafonné ; 0 sans `turn`/`inner`/`q`.
    pub fn long_mu(&self) -> f64 {
        let m = self.n_edges();
        if self.climbs <= 0 || self.turn.len() != m || self.inner.len() != m || self.q.len() != m {
            return 0.0;
        }
        (CLIMBS_LONG_REL * self.q.iter().sum::<f64>() / self.w.iter().sum::<f64>().max(1.0))
            .min(CLIMBS_LONG_MAX)
    }

    /// T24 : ½ si le nœud a est un extremum (pic ou creux) entre les arêtes e1, e2 de la boucle.
    pub fn node_ext(&self, a: usize, e1: usize, e2: usize) -> f64 {
        let sg = |e: usize| self.turn[e][usize::from(self.u[e] != a)];
        let (s1, s2) = (sg(e1), sg(e2));
        if s1 != 0 && s1 == s2 { 0.5 } else { 0.0 }
    }

    /// D52 : coût de virage au nœud a entre les arêtes e1, e2 de la boucle : seuls les virages
    /// SERRÉS (plus de 90°) coûtent, de 0 à angle droit à 1 au demi-tour : cos de l'angle entre les
    /// deux directions de départ (`ang_u`/`ang_v`), borné à 0. Banc du 07/10 : la version douce
    /// ((1 + cos)/2, angle droit = ½) faisait perdre 14 pts de chemin en cible (virages à angle
    /// droit des sentiers comptés).
    #[inline]
    pub fn turn_cost(&self, a: usize, e1: usize, e2: usize) -> f64 {
        // cos de la différence = produit scalaire des directions (précalculées : pas de trigonométrie
        // par mouvement du recuit, −30 % de temps)
        let dir = |e: usize| {
            let k = if self.u[e] == a { 0 } else { 2 };
            if self.dir.len() == self.n_edges() {
                [self.dir[e][k], self.dir[e][k + 1]]
            } else {
                let t = if k == 0 { self.ang_u[e] } else { self.ang_v[e] };
                [t.cos(), t.sin()]
            }
        };
        let (d1, d2) = (dir(e1), dir(e2));
        (d1[0] * d2[0] + d1[1] * d2[1]).max(0.0)
    }

    /// D62 : active le paquet « élégance » : prix des virages serrés selon le mode (cible : départage
    /// dans la bande ; max : en m de D+ ; min_distance : aucun), pétales et directions (`FaceSearch`).
    pub fn enable_smooth(&mut self) {
        self.smooth = true;
        self.turn_mu = match self.mode.as_str() {
            "target" => TURN_TARGET,
            "max" => TURN_MAX_FRAC * self.dplus_upper_bound() / self.l.max(1.0) * 100.0,
            _ => 0.0,
        };
        self.prepare_turns();
    }

    /// Pose les caches du terme de virage (`junction`, `dir`) ; à appeler après avoir fixé `turn_mu`.
    pub fn prepare_turns(&mut self) {
        if self.turn_mu == 0.0 {
            return;
        }
        self.junction = self.degrees().iter().map(|&d| d >= 3).collect();
        self.dir = (0..self.n_edges())
            .map(|e| {
                let (u, v) = (self.ang_u[e], self.ang_v[e]);
                [u.cos(), u.sin(), v.cos(), v.sin()]
            })
            .collect();
    }

    /// Coût d'un nœud de degré 2 de la boucle (arêtes e1, e2) : montées « longues » (T24) + virage
    /// (D52, seulement aux carrefours du graphe : `junction` = degré >= 3 ; un nœud de degré 2 du
    /// graphe est un raccord sans choix d'itinéraire).
    #[inline]
    pub fn node_cost(&self, a: usize, e1: usize, e2: usize, junction: bool) -> f64 {
        let t = self.node_terms(a, e1, e2, junction);
        self.node_mu * t[0] + self.turn_mu * t[1]
    }

    /// (extremum T24, virage D52) NON pondérés au nœud a : les deux sommes restent séparées pour
    /// que, sans virage (`turn_mu` = 0), les calculs soient bit à bit ceux d'avant D52.
    #[inline]
    pub fn node_terms(&self, a: usize, e1: usize, e2: usize, junction: bool) -> [f64; 2] {
        [
            if self.node_mu != 0.0 {
                self.node_ext(a, e1, e2)
            } else {
                0.0
            },
            if self.turn_mu != 0.0 && junction {
                self.turn_cost(a, e1, e2)
            } else {
                0.0
            },
        ]
    }

    /// Degré de chaque nœud dans le graphe (carrefours de `node_cost`).
    pub fn degrees(&self) -> Vec<u32> {
        let mut d = vec![0u32; self.n_nodes()];
        for e in 0..self.n_edges() {
            d[self.u[e]] += 1;
            d[self.v[e]] += 1;
        }
        d
    }

    /// Σ `node_cost` sur les nœuds de degré 2 de la boucle (montées « longues » et virages).
    pub fn node_costs(&self, ids: &[usize]) -> f64 {
        if self.node_mu == 0.0 && self.turn_mu == 0.0 {
            return 0.0;
        }
        // en O(taille de la boucle) : appelée par `score` (tri des départs et des sorties)
        let deg;
        let junction: &[bool] = if self.junction.len() == self.n_nodes() {
            &self.junction
        } else {
            deg = self.degrees().iter().map(|&d| d >= 3).collect::<Vec<_>>();
            &deg
        };
        let mut inc: std::collections::HashMap<usize, (u8, usize, usize)> =
            std::collections::HashMap::with_capacity(2 * ids.len());
        for &e in ids {
            if self.u[e] != self.v[e] {
                for a in [self.u[e], self.v[e]] {
                    let t = inc.entry(a).or_insert((0, usize::MAX, usize::MAX));
                    match t.0 {
                        0 => t.1 = e,
                        1 => t.2 = e,
                        _ => {}
                    }
                    t.0 = t.0.saturating_add(1);
                }
            }
        }
        // sommes dans l'ordre des nœuds (comme avant D52 : même arrondi)
        let mut v: Vec<(usize, [f64; 2])> = inc
            .iter()
            .filter(|(_, t)| t.0 == 2)
            .map(|(&a, t)| (a, self.node_terms(a, t.1, t.2, junction[a])))
            .collect();
        v.sort_unstable_by_key(|x| x.0);
        let ext: f64 = v.iter().map(|x| x.1[0]).sum();
        let turn: f64 = v.iter().map(|x| x.1[1]).sum();
        self.node_mu * ext + self.turn_mu * turn
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
            || (!self.turn.is_empty() && self.turn.len() != m)
            || (!self.inner.is_empty() && self.inner.len() != m)
            || self.q.iter().any(|x| !x.is_finite() || *x < 0.0)
        {
            return Err("climbs hors de -1..1, ou q, turn, inner absents ou invalides".into());
        }
        let bad = |x: usize| x >= n;
        if bad(self.s)
            || self
                .u
                .iter()
                .chain(&self.v)
                .chain(&self.far)
                .chain(&self.via)
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

    /// Mode cible : la boucle tient la distance ET le D+ demandés à `TARGET_TOL` près.
    pub fn on_target(&self, ids: &[usize]) -> bool {
        let (l, d) = self.stats(ids);
        let dt = self.d.unwrap_or(1.0);
        ((l - self.l) / self.l).abs() <= TARGET_TOL && ((d - dt) / dt).abs() <= TARGET_TOL
    }

    /// (longueur, D+) d'un ensemble d'arêtes.
    pub fn stats(&self, ids: &[usize]) -> (f64, f64) {
        ids.iter()
            .fold((0.0, 0.0), |(l, d), &e| (l + self.len[e], d + self.w[e]))
    }

    /// Points de passage que la boucle `ids` ne touche pas.
    pub fn via_missed(&self, ids: &[usize]) -> Vec<usize> {
        self.via
            .iter()
            .copied()
            .filter(|&x| !ids.iter().any(|&e| self.u[e] == x || self.v[e] == x))
            .collect()
    }

    /// (score à maximiser, longueur, D+, réalisable), comme `Problem.score` en Python.
    /// Points de passage manqués : −VIA_MISS chacun, et non réalisable.
    pub fn score(&self, ids: &[usize]) -> (f64, f64, f64, bool) {
        let (sc, length, dplus, feas) = self.score_free(ids);
        let miss = self.via_missed(ids).len();
        (
            sc - VIA_MISS * miss as f64,
            length,
            dplus,
            feas && miss == 0,
        )
    }

    fn score_free(&self, ids: &[usize]) -> (f64, f64, f64, bool) {
        let (length, dplus) = self.stats(ids);
        if self.min_distance() {
            let x = self.d.unwrap();
            let (miss, over) = ((x - dplus).max(0.0), (length - self.lmax).max(0.0));
            let sc = -length - self.off_len(ids) - MD_MISS * miss - MD_OVER * over;
            return (sc, length, dplus, miss == 0.0 && over == 0.0);
        }
        let feas = self.lmin <= length && length <= self.lmax;
        if self.target() {
            let d = self.d.unwrap();
            let mut err = (length - self.l).abs() / self.l + (dplus - d).abs() / d;
            if !self.off.is_empty() {
                err = err.max(SURF_TARGET_BAND) + SURF_TARGET * self.off_len(ids) / self.l;
            }
            // D52 : virages aux carrefours, dans les unités du score des faces
            let nc = self.node_costs(ids) / crate::faces::TARGET_SCALE;
            return (-err - nc, length, dplus, feas);
        }
        let viol = (self.lmin - length).max(length - self.lmax).max(0.0);
        // T24 montées « longues » et D52 virages (0 sans l'un ni l'autre)
        let nc = self.node_costs(ids);
        // D40 : au-delà de la distance demandée, un mètre doit rapporter LEN_DENSITY × la densité
        // moyenne de D+ de la boucle (sinon c'est un détour)
        // type de voie : D+ plus la prime `off_price` par mètre sur le bon type (comme `search_bonus`)
        let val = if self.off.is_empty() {
            dplus
        } else {
            dplus + self.off_price * (length - self.off_len(ids))
        };
        let over = LEN_DENSITY * val / length.max(1.0) * (length - self.l).max(0.0);
        (val - nc - over - 0.5 * viol, length, dplus, feas)
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

#[cfg(test)]
mod tests {
    /// Poids d'étiquettes extrêmes (bonus > longueur) : la longueur de recherche reste ≥ 10 % de
    /// la longueur (gel du poste, 2026-10-07).
    #[test]
    fn search_len_stays_positive() {
        let mut p: super::Problem = serde_json::from_str(
            r#"{"version": 2, "mode": "target", "L": 1000, "Lmin": 700, "Lmax": 1300, "D": 50,
                "s": 0, "node_simple": false, "xy": [[0,0],[1,0]], "far": [], "u": [0, 0],
                "v": [1, 1], "len": [100, 200], "w": [1, 1], "ang_u": [0, 1], "ang_v": [0, 1],
                "parallel": []}"#,
        )
        .unwrap();
        assert_eq!(p.search_len(1), 200.0);
        p.off = vec![-5000.0, 30.0];
        assert_eq!(p.search_len(0), 10.0);
        assert_eq!(p.search_len(1), 230.0);
    }

    /// D52 : virage au carrefour 0 (trois arêtes partant vers l'est, le nord-est, l'ouest) : tout droit
    /// = 0, virage de 135° = cos 45°, demi-tour = 1 ; rien hors carrefour ni sans prix ; somme sur la
    /// boucle.
    #[test]
    fn turn_cost_droit_angle_demi_tour() {
        let mut p: super::Problem = serde_json::from_str(
            r#"{"version": 2, "mode": "target", "L": 1000, "Lmin": 700, "Lmax": 1300, "D": 50,
                "s": 1, "node_simple": false, "xy": [[0,0],[1,0],[0,1],[-1,0]], "far": [],
                "u": [0, 0, 0, 1], "v": [1, 2, 3, 2], "len": [1, 1, 1, 3], "w": [1, 1, 1, 1],
                "ang_u": [0, 0.7853981633974483, 3.141592653589793, 1.0],
                "ang_v": [3.141592653589793, -1.5707963267948966, 0, 2.0], "parallel": []}"#,
        )
        .unwrap();
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        assert!(close(p.turn_cost(0, 0, 2), 0.0)); // est puis ouest : tout droit
        assert!(close(p.turn_cost(0, 0, 1), 0.5f64.sqrt())); // virage serré de 135°
        assert!(close(p.turn_cost(0, 0, 0), 1.0));
        assert!(close(p.turn_cost(0, 2, 0), p.turn_cost(0, 0, 2))); // symétrique
        assert_eq!(p.node_cost(0, 0, 1, true), 0.0); // turn_mu = 0
        p.turn_mu = 4.0;
        assert!(close(p.node_cost(0, 0, 1, true), 4.0 * 0.5f64.sqrt()));
        assert_eq!(p.node_cost(0, 0, 1, false), 0.0);
        assert_eq!(p.degrees(), vec![3, 2, 2, 1]);
        // boucle 0-1-2-0 (arêtes 0, 3, 1) : seul le nœud 0 est un carrefour ; idem avec le cache
        assert!(close(p.node_costs(&[0, 3, 1]), 4.0 * 0.5f64.sqrt()));
        p.junction = vec![false; 4];
        assert_eq!(p.node_costs(&[0, 3, 1]), 0.0);
    }
}
