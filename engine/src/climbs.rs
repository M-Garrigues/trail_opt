//! Définition UNIQUE d'une « montée » et d'une « descente » (modes.md v0.3, T33) : affichage
//! (`climbs`), choix du sens de parcours (Ḡ) et coût de recherche (`plan::edge_turns`) la
//! partagent. Une descente est une montée du profil renversé en altitude : même fonction, signe.
//!
//! 1. Découpe de base (anti-bruit du MNT) : une montée s'ouvre quand on s'élève de `OPEN_M` au-dessus
//!    du creux courant et ne se ferme que si l'on redescend d'au moins `tol(G)` sous son sommet
//!    (`tol(G) = clamp(15 % · G, 10 m, 25 m)`) : replat, escalier, pont ne la coupent pas.
//! 2. Fusion hiérarchique, du plus petit creux au plus grand (façon proéminence) : l'interruption
//!    entre deux montées voisines (gains G₁, G₂, longueurs l₁, l₂) est absorbée si, RELATIVEMENT à
//!    elles, elle est courte et peu profonde : `longueur ≤ 25 % · min(l₁, l₂)` ET
//!    `perte ≤ 30 % · min(G₁, G₂)`. Pas de plafond absolu : 1 km, creux de 20 m, 1 km = UNE montée
//!    (gain 180 m). La montée fusionnée peut absorber à son tour.

pub const OPEN_M: f64 = 10.0;
const TOL_MIN: f64 = 10.0;
const TOL_MAX: f64 = 25.0;
const TOL_REL: f64 = 0.15;
/// Une interruption est absorbée si sa longueur ≤ `GAP_LEN` · la plus courte des voisines…
const GAP_LEN: f64 = 0.25;
/// … et sa perte d'altitude ≤ `GAP_LOSS` · le plus petit des deux gains.
const GAP_LOSS: f64 = 0.30;

/// Tampon local (m) qui ferme une montée de gain `gain` (aussi utilisé par `plan::edge_turns`,
/// approximation locale par arête du coût de recherche : la fusion y est inutilisable).
pub fn tol(gain: f64) -> f64 {
    (TOL_REL * gain).clamp(TOL_MIN, TOL_MAX)
}

/// Montées d'un profil (altitudes `z`, abscisses `s`) : [(gain, longueur)].
pub fn scan(z: &[f64], s: &[f64]) -> Vec<(f64, f64)> {
    scan_signed(z, s, 1.0)
}

/// Descentes d'un profil : [(perte, longueur)], même définition que les montées.
pub fn descents(z: &[f64], s: &[f64]) -> Vec<(f64, f64)> {
    scan_signed(z, s, -1.0)
}

/// `sign` = +1 (montées) ou −1 (descentes : montées du profil renversé en altitude).
fn scan_signed(z: &[f64], s: &[f64], sign: f64) -> Vec<(f64, f64)> {
    let z: Vec<f64> = z.iter().map(|v| sign * v).collect();
    let mut c = base(&z);
    // fusion : toujours l'interruption la plus petite (rapport au seuil), tant qu'un rapport ≤ 1
    loop {
        let ratio = |a: f64, b: f64| if a <= 0.0 { 0.0 } else { a / b };
        let best = (1..c.len())
            .map(|k| {
                let ((a, b), (c2, d)) = (c[k - 1], c[k]);
                let (g1, g2) = (z[b] - z[a], z[d] - z[c2]);
                let l = ratio(s[c2] - s[b], GAP_LEN * (s[b] - s[a]).min(s[d] - s[c2]));
                let g = ratio(z[b] - z[c2], GAP_LOSS * g1.min(g2));
                (l.max(g), k)
            })
            .filter(|x| x.0 <= 1.0)
            .min_by(|x, y| x.0.total_cmp(&y.0));
        let Some((_, k)) = best else { break };
        c[k - 1].1 = c[k].1;
        c.remove(k);
    }
    c.iter().map(|&(a, b)| (z[b] - z[a], s[b] - s[a])).collect()
}

/// Découpe de base : indices (creux, sommet) de chaque montée.
fn base(z: &[f64]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let (mut lo, mut hi): (usize, Option<usize>) = (0, None);
    for i in 1..z.len() {
        match hi {
            None if z[i] < z[lo] => lo = i,
            None if z[i] - z[lo] >= OPEN_M => hi = Some(i),
            Some(h) if z[i] > z[h] => hi = Some(i),
            Some(h) if z[h] - z[i] >= tol(z[h] - z[lo]) => {
                out.push((lo, h));
                (lo, hi) = (i, None);
            }
            _ => {}
        }
    }
    if let Some(h) = hi {
        out.push((lo, h));
    }
    out
}
