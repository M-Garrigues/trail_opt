//! Moteur de calcul de boucles de trail (port Rust de `trailopt`).
pub mod anneal;
pub mod api;
pub mod climbs;
pub mod codes;
pub mod faces;
pub mod l93;
pub mod plan;
pub mod prep;
pub mod problem;
pub mod share;
pub mod solve;
pub mod tiles;
pub mod utm;

pub use anneal::Annealer;
pub use codes::{Code, Msg};
pub use faces::{FaceSearch, Solution, build_faces};
pub use problem::Problem;
pub use solve::{Budget, Output, optimize};

/// Version du solveur : version du paquet, plus le commit si GITHUB_SHA est défini au build (CI).
pub fn solver_version() -> String {
    match option_env!("GITHUB_SHA") {
        Some(sha) if sha.len() >= 7 => format!("{}+{}", env!("CARGO_PKG_VERSION"), &sha[..7]),
        _ => env!("CARGO_PKG_VERSION").to_string(),
    }
}

/// Générateur pseudo-aléatoire SplitMix64 : déterministe, sans dépendance.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Réel uniforme dans [0, 1).
    pub fn random(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Réel uniforme dans [a, b).
    pub fn uniform(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.random()
    }

    /// Loi normale centrée réduite (Box-Muller).
    pub fn normal(&mut self) -> f64 {
        let (u1, u2) = (1.0 - self.random(), self.random());
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }

    /// Indice uniforme dans [0, n).
    pub fn below(&mut self, n: usize) -> usize {
        (self.random() * n as f64) as usize
    }
}
