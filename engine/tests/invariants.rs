//! Invariants des boucles (port de `check_loop`) sur une grille synthétique avec relief
//! analytique, comme `tests/conftest.py`.
use engine::{Annealer, Budget, FaceSearch, Problem, build_faces, codes, optimize};

const N: usize = 7;
const H: f64 = 100.0;

fn relief(x: f64, y: f64) -> f64 {
    40.0 * (x / 250.0).sin() + 30.0 * (y / 180.0).cos() + 0.05 * x + 100.0
}

fn pos(a: usize) -> [f64; 2] {
    let (i, j) = ((a / N) as f64, (a % N) as f64);
    let c = (N / 2) as f64;
    [(j - c) * H, (i - c) * H]
}

/// (longueur, w) d'une polyligne : w = (montée + descente) / 2, profil échantillonné tous les 5 m.
fn profile(pts: &[[f64; 2]]) -> (f64, f64) {
    let (mut len, mut updown) = (0.0, 0.0);
    for p in pts.windows(2) {
        let d = (p[1][0] - p[0][0]).hypot(p[1][1] - p[0][1]);
        let k = (d / 5.0).ceil() as usize;
        let z = |t: f64| {
            relief(
                p[0][0] + t * (p[1][0] - p[0][0]),
                p[0][1] + t * (p[1][1] - p[0][1]),
            )
        };
        updown += (0..k)
            .map(|i| (z((i + 1) as f64 / k as f64) - z(i as f64 / k as f64)).abs())
            .sum::<f64>();
        len += d;
    }
    (len, updown / 2.0)
}

/// Grille N×N, départ au centre, plus une arête courbe parallèle à l'arête 0-1.
fn grid(l: f64, target: Option<f64>, node_simple: bool) -> Problem {
    let mut polys: Vec<(usize, usize, Vec<[f64; 2]>)> = Vec::new();
    for a in 0..N * N {
        if a % N + 1 < N {
            polys.push((a, a + 1, vec![pos(a), pos(a + 1)]));
        }
        if a / N + 1 < N {
            polys.push((a, a + N, vec![pos(a), pos(a + N)]));
        }
    }
    let mid = [pos(0)[0] + 50.0, pos(0)[1] - 40.0];
    polys.push((0, 1, vec![pos(0), mid, pos(1)]));
    let ang = |p: [f64; 2], q: [f64; 2]| (q[1] - p[1]).atan2(q[0] - p[0]);
    let s = (N / 2) * N + N / 2;
    let xy: Vec<[f64; 2]> = (0..N * N).map(pos).collect();
    let (lo, hi) = if target.is_some() {
        (0.7, 1.3)
    } else {
        (0.95, 1.05)
    };
    let mut p = Problem {
        version: 1,
        mode: if target.is_some() { "target" } else { "max" }.into(),
        l,
        lmin: l * lo,
        lmax: l * hi,
        d: target,
        s,
        node_simple,
        far: (0..N * N)
            .filter(|&n| (xy[n][0] - xy[s][0]).hypot(xy[n][1] - xy[s][1]) > 200.0)
            .collect(),
        xy,
        u: polys.iter().map(|x| x.0).collect(),
        v: polys.iter().map(|x| x.1).collect(),
        len: Vec::new(),
        w: Vec::new(),
        ang_u: polys.iter().map(|x| ang(x.2[0], x.2[1])).collect(),
        ang_v: polys
            .iter()
            .map(|x| ang(x.2[x.2.len() - 1], x.2[x.2.len() - 2]))
            .collect(),
        parallel: vec![[0, polys.len() - 1]],
        q: Vec::new(),
        climbs: 0,
    };
    (p.len, p.w) = polys.iter().map(|x| profile(&x.2)).unzip();
    p
}

fn edge(p: &Problem, a: usize, b: usize) -> usize {
    (0..p.n_edges())
        .find(|&e| (p.u[e], p.v[e]) == (a.min(b), a.max(b)))
        .unwrap()
}

/// Carré unitaire de coin inférieur gauche (i, j).
fn square(p: &Problem, i: usize, j: usize) -> Vec<usize> {
    let a = i * N + j;
    vec![
        edge(p, a, a + 1),
        edge(p, a + 1, a + 1 + N),
        edge(p, a + N, a + N + 1),
        edge(p, a, a + N),
    ]
}

#[test]
fn faces_de_la_grille() {
    let p = grid(2400.0, None, false);
    let (faces, foe) = build_faces(&p.u, &p.v, &p.ang_u, &p.ang_v, &p.len, p.lmax);
    assert!(faces.len() >= (N - 1) * (N - 1)); // 36 mailles + la lunule du couloir
    for f in &faces {
        assert_eq!(f[0].1, f[f.len() - 1].2, "face non fermée");
        assert!(f.windows(2).all(|x| x[0].2 == x[1].1), "face discontinue");
    }
    assert!(foe.iter().all(|x| x.len() <= 2));
}

#[test]
fn boucles_valides() {
    for node_simple in [false, true] {
        for target in [None, Some(120.0)] {
            let p = grid(2400.0, target, node_simple);
            let fs = FaceSearch::new(&p);
            for seed in 0..4 {
                let sol = fs.solve(200_000, seed, None).expect("aucune boucle");
                p.check(&sol.ids).unwrap();
                assert_eq!(p.stats(&sol.ids), (sol.length, sol.dplus));
                if target.is_none() {
                    assert!(
                        sol.feasible && sol.dplus > 0.0,
                        "{node_simple} {seed} {}",
                        sol.length
                    );
                }
            }
        }
    }
}

#[test]
fn deterministe() {
    let p = grid(2400.0, None, true);
    let fs = FaceSearch::new(&p);
    assert_eq!(
        fs.solve(50_000, 7, None).unwrap().ids,
        fs.solve(50_000, 7, None).unwrap().ids
    );
}

#[test]
fn check_rejette_les_boucles_invalides() {
    let p = grid(2400.0, None, true);
    let s = p.s;
    let sq = square(&p, s / N, s % N); // carré qui part du départ
    p.check(&sq).unwrap();
    let err = |ids: &[usize]| p.check(ids).unwrap_err();
    assert!(err(&[]).contains("vide"));
    assert!(err(&sq[..3]).contains("impair"));
    assert!(err(&[sq.clone(), vec![sq[0]]].concat()).contains("répétée"));
    assert!(err(&square(&p, 0, 0)).contains("départ"));
    assert!(err(&[sq.clone(), square(&p, 0, 0)].concat()).contains("connexe"));
    // lunule formée par l'arête 0-1 et sa parallèle courbe, départ au nœud 0
    let p0 = Problem {
        s: 0,
        ..grid(2400.0, None, true)
    };
    assert!(
        p0.check(&[0, p0.n_edges() - 1])
            .unwrap_err()
            .contains("parallèle")
    );
    // huit : deux carrés qui se touchent au nœud (1, 1), hors du rayon libre
    let eight = [square(&p, 0, 0), square(&p, 1, 1)].concat();
    let p2 = Problem {
        s: N + 1,
        ..grid(2400.0, None, true)
    };
    assert!(p2.check(&eight).unwrap_err().contains("carrefour"));
    let p3 = Problem {
        s: N + 1,
        ..grid(2400.0, None, false)
    };
    p3.check(&eight).unwrap();
}

#[test]
fn recuit_classique_valide() {
    for node_simple in [false, true] {
        for target in [None, Some(120.0)] {
            let p = grid(2400.0, target, node_simple);
            for seed in 0..3 {
                let r = Annealer::new(&p, seed).run(2_000).expect("aucune boucle");
                let ids: Vec<usize> = r.iter().map(|s| s.0).collect();
                p.check(&ids).unwrap();
                // circuit continu, fermé en s
                assert_eq!((r[0].1, r[r.len() - 1].2), (p.s, p.s));
                assert!(r.windows(2).all(|x| x[0].2 == x[1].1));
            }
        }
    }
}

fn budget(candidates: usize) -> Budget {
    Budget {
        iters: 300_000,
        anneal_iters: 2_000,
        seed: 1,
        candidates,
        deadline: None,
    }
}

#[test]
fn optimize_modes_et_candidats() {
    // mode cible : recuit puis affinage par faces
    let p = grid(2400.0, Some(120.0), true);
    let o = optimize(&p, &budget(1)).unwrap();
    p.check(&o.ids).unwrap();
    // plusieurs candidats : valides, dans les bornes, recouvrement <= 50 %, triés par D+
    let p = grid(2400.0, None, false);
    let o = optimize(&p, &budget(3)).unwrap();
    assert!(o.feasible);
    let fs = FaceSearch::new(&p);
    let loops: Vec<&Vec<usize>> = std::iter::once(&o.ids).chain(&o.alternatives).collect();
    assert!(!o.alternatives.is_empty());
    for (i, a) in loops.iter().enumerate() {
        p.check(a).unwrap();
        assert!(p.score(a).3);
        for b in &loops[..i] {
            assert!(fs.overlap(a, b) <= 0.5);
        }
    }
    let d: Vec<f64> = o.alternatives.iter().map(|a| p.stats(a).1).collect();
    assert!(d.windows(2).all(|x| x[0] >= x[1]));
}

/// Codes stables (D14) : la fixture `codes.json` ne perd jamais un code ni ne change ses
/// paramètres. Après ajout d'un code : `UPDATE_CODES=1 cargo test`.
#[test]
fn codes_stables() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/codes.json");
    let now = codes::export();
    let old: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap_or("[]".into())).unwrap();
    for c in old.as_array().unwrap() {
        let retired = codes::RETIRED.contains(&c["code"].as_str().unwrap());
        assert!(
            retired || now.as_array().unwrap().contains(c),
            "code retiré ou modifié : {c}"
        );
    }
    for c in now.as_array().unwrap() {
        assert!(
            !codes::RETIRED.contains(&c["code"].as_str().unwrap()),
            "code retiré réutilisé : {c}"
        );
    }
    if std::env::var_os("UPDATE_CODES").is_some() {
        std::fs::write(path, serde_json::to_string_pretty(&now).unwrap() + "\n").unwrap();
    } else {
        assert_eq!(old, now, "nouveaux codes : relancer avec UPDATE_CODES=1");
    }
}

/// Grille en mode min_distance (D19) : D+ visé `x`, distance max `cap`.
fn md_grid(x: f64, cap: f64, node_simple: bool) -> Problem {
    Problem {
        mode: "min_distance".into(),
        d: Some(x),
        lmin: 0.0,
        lmax: cap,
        l: cap / 2.0,
        ..grid(cap, None, node_simple)
    }
}

/// Mode min_distance : boucles valides, D+ >= X, L <= Lcap, au-dessus de la borne inférieure,
/// et plus courtes que la boucle du mode max qui a ce D+ ; candidats triés par longueur.
#[test]
fn min_distance_invariants() {
    let max = optimize(&grid(2400.0, None, false), &budget(1)).unwrap();
    for node_simple in [false, true] {
        let x = 0.7 * max.dplus;
        let p = md_grid(x, 6000.0, node_simple);
        let lb = engine::solve::md_lower_bound(&p).unwrap();
        let o = optimize(&p, &budget(3)).unwrap();
        p.check(&o.ids).unwrap();
        assert!(
            o.feasible && o.dplus >= x && o.length <= p.lmax,
            "{node_simple}"
        );
        assert!(lb <= o.length + 1e-6, "borne {lb} > {}", o.length);
        assert!(o.warnings.is_empty());
        if !node_simple {
            assert!(
                o.length <= max.length + 1e-6,
                "{} > {}",
                o.length,
                max.length
            );
        }
        let l: Vec<f64> = o.alternatives.iter().map(|a| p.stats(a).0).collect();
        assert!(l.windows(2).all(|w| w[0] <= w[1]));
        for a in &o.alternatives {
            p.check(a).unwrap();
            assert!(p.score(a).3);
        }
    }
}

/// Refus prouvés (aucun calcul) : Σw < X, ou borne inférieure > Lcap.
#[test]
fn min_distance_refus() {
    let total: f64 = grid(2400.0, None, false).w.iter().sum();
    let e = optimize(&md_grid(total + 1.0, 6000.0, false), &budget(1))
        .err()
        .unwrap();
    assert_eq!(e.code, engine::Code::DplusUnreachableProven);
    assert!(e.params["min_km"].is_null());
    let p = md_grid(0.5 * total, 1000.0, false);
    let e = optimize(&p, &budget(1)).err().unwrap();
    assert_eq!(e.code, engine::Code::DplusUnreachableProven);
    assert!(e.params["min_km"].as_f64().unwrap() > 1.0);
}

/// Borne inférieure : sac à dos inverse restreint aux arêtes de reach <= L.
#[test]
fn borne_inferieure_de_distance() {
    use engine::problem::length_lower_bound as lb;
    let (len, w) = ([100.0, 100.0, 1000.0], [50.0, 10.0, 10.0]);
    // tout est atteignable : 50 m de D+ en 100 m, puis 5 m au prorata de l'arête 2 (10 %)
    assert_eq!(lb(&len, &w, &[0.0; 3], 55.0), Some(150.0));
    // l'arête raide n'est prise que par une boucle de 5 km : reach domine
    assert_eq!(lb(&len, &w, &[5000.0, 0.0, 0.0], 15.0), Some(600.0));
    assert_eq!(lb(&len, &w, &[5000.0, 0.0, 0.0], 50.0), Some(5000.0));
    assert_eq!(lb(&len, &w, &[0.0; 3], 71.0), None);
}

/// Préférence de montées en mode max : recherche sur w + γβq/H, D+ rapporté sur le vrai w.
#[test]
fn climbs_poids_de_recherche() {
    for gamma in [-1, 1] {
        let mut p = grid(2400.0, None, false);
        p.q = p.w.iter().map(|w| w * w).collect();
        p.climbs = gamma;
        let o = optimize(&p, &budget(2)).unwrap();
        p.check(&o.ids).unwrap();
        assert_eq!((o.length, o.dplus), p.stats(&o.ids));
        let d: Vec<f64> = o.alternatives.iter().map(|a| p.stats(a).1).collect();
        assert!(d.windows(2).all(|x| x[0] >= x[1]));
    }
    // q absent : instance refusée
    let json = serde_json::to_string(&serde_json::json!({
        "version": 2, "mode": "max", "L": 1.0, "Lmin": 0.0, "Lmax": 2.0, "D": null, "s": 0,
        "node_simple": false, "xy": [[0.0, 0.0], [1.0, 0.0]], "far": [], "u": [0], "v": [1],
        "len": [1.0], "w": [1.0], "ang_u": [0.0], "ang_v": [3.1], "parallel": [], "climbs": -1
    }))
    .unwrap();
    assert!(Problem::from_json(&json).err().unwrap().contains("q"));
}

/// D30 : pente max affichée sur 50 m. Un ressaut de 20 m sur 25 m (80 %) dans une pente de 10 %
/// donne (20 + 2,5) / 50 = 45 %, plus 80 %.
#[test]
fn max_grade_window_50m() {
    let s: Vec<f64> = (0..=40).map(|i| 5.0 * i as f64).collect();
    let z: Vec<f64> = s
        .iter()
        .map(|&x| {
            0.1 * x
                + if x >= 100.0 {
                    17.5
                } else if x > 75.0 {
                    0.7 * (x - 75.0)
                } else {
                    0.0
                }
        })
        .collect();
    let g = engine::plan::max_grade(&z, &s);
    assert!((g - 0.45).abs() < 1e-9, "{g}");
    assert_eq!(engine::plan::GRADE_WINDOW_M, 50.0);
    // plus court que la fenêtre : pente moyenne
    assert!((engine::plan::max_grade(&[0.0, 3.0, 6.0], &[0.0, 10.0, 20.0]) - 0.3).abs() < 1e-12);
}
