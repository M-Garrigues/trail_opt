//! Codes stables des erreurs et avertissements (décision D14). Le client affiche
//! `m.err[code](params)` ; jamais de phrase affichable ici, `detail` anglais pour les logs.
//! Un code n'est jamais renommé ni réutilisé : la fixture `engine/codes.json` (test
//! `codes_stables`) le vérifie. Pour en ajouter un : variante + ligne de `info()` + `ALL`.
//! Un code jamais émis est retiré (variante supprimée) et son nom va dans `RETIRED`.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Code {
    // Erreurs de saisie (pipeline.validate et plan_loop)
    ZoneInvalid,
    StartOutsideZone,
    ModeUnknown,
    RoadsUnknown,
    TargetDplusRequired,
    ToleranceOutOfRange,
    MaxGradeInvalid,
    DistanceOutOfRange,
    TimeOutOfRange,
    ZoneTooLarge,
    // Erreurs de calcul
    NoWayInZone,
    NoLoopOfDistance,
    // Erreurs du moteur
    InvalidProblem,
    NoLoopFound,
    InvariantViolated,
    // Avertissements
    LongDistance,
    ZoneReduced,
    AccessRoundTrip,
    StartMoved,
    StartFarFromNetwork,
    ProfileMismatch,
    DistanceOutOfTolerance,
    TargetDplusAboveBound,
    TargetDplusProbablyUnreachable,
    // Ajouts étape 3 (moteur depuis les dalles, contrat api.md v0)
    InvalidRequest,
    OutsideCoverage,
    Busy,
    Timeout,
    BotCheckFailed,
    ServicePaused,
    // Modes D19 (contracts/modes.md)
    DplusOutOfRange,
    DplusUnreachableProven,
    DplusNotReached,
    ClimbsUnknown,
    // Partage (api.md v1.1)
    LoopNotFound,
    // Revues T25/T26 (api.md v1.3)
    CandidatesReduced,
    CoverageEdge,
    // Points de passage (api.md v1.5, T34)
    ViaTooFar,
    ViaOutsideZone,
    ViaUnreachable,
    ViaMissed,
    // Messages importants (api.md v1.5, D40)
    TargetNotReached,
    FewerLoops,
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Error,
    Warning,
}

impl Code {
    pub const ALL: [Code; 43] = {
        use Code::*;
        [
            ZoneInvalid,
            StartOutsideZone,
            ModeUnknown,
            RoadsUnknown,
            TargetDplusRequired,
            ToleranceOutOfRange,
            MaxGradeInvalid,
            DistanceOutOfRange,
            TimeOutOfRange,
            ZoneTooLarge,
            NoWayInZone,
            NoLoopOfDistance,
            InvalidProblem,
            NoLoopFound,
            InvariantViolated,
            LongDistance,
            ZoneReduced,
            AccessRoundTrip,
            StartMoved,
            StartFarFromNetwork,
            ProfileMismatch,
            DistanceOutOfTolerance,
            TargetDplusAboveBound,
            TargetDplusProbablyUnreachable,
            InvalidRequest,
            OutsideCoverage,
            Busy,
            Timeout,
            BotCheckFailed,
            ServicePaused,
            DplusOutOfRange,
            DplusUnreachableProven,
            DplusNotReached,
            ClimbsUnknown,
            LoopNotFound,
            CandidatesReduced,
            CoverageEdge,
            ViaTooFar,
            ViaOutsideZone,
            ViaUnreachable,
            ViaMissed,
            TargetNotReached,
            FewerLoops,
        ]
    };

    /// Genre et noms des paramètres (unités dans le nom).
    pub fn info(self) -> (Kind, &'static [&'static str]) {
        use Code::*;
        use Kind::*;
        match self {
            DistanceOutOfRange => (Error, &["min_km", "max_km"]),
            TimeOutOfRange => (Error, &["min_s", "max_s"]),
            ZoneTooLarge => (Error, &["area_km2", "max_km2"]),
            ZoneInvalid | StartOutsideZone | ModeUnknown | RoadsUnknown | TargetDplusRequired
            | ToleranceOutOfRange | MaxGradeInvalid | NoWayInZone | NoLoopOfDistance
            | InvalidProblem | NoLoopFound | InvariantViolated | InvalidRequest
            | OutsideCoverage | Busy | Timeout | BotCheckFailed | ServicePaused | ClimbsUnknown
            | LoopNotFound => (Error, &[]),
            DplusOutOfRange => (Error, &["min_m", "max_m"]),
            // min_km : null quand Σw du graphe < D+ (aucune distance ne suffit)
            DplusUnreachableProven => (Error, &["dplus_m", "min_km"]),
            DplusNotReached => (Warning, &["dplus_m", "best_dplus_m", "max_km"]),
            LongDistance => (Warning, &["km"]),
            ZoneReduced => (Warning, &["radius_km", "area_km2"]),
            AccessRoundTrip => (Warning, &["access_m"]),
            StartMoved | StartFarFromNetwork => (Warning, &["distance_m"]),
            ProfileMismatch => (Warning, &["profile_m", "sum_w_m"]),
            DistanceOutOfTolerance | TargetDplusProbablyUnreachable => (Warning, &[]),
            TargetDplusAboveBound => (Warning, &["max_dplus_m", "max_km"]),
            CandidatesReduced => (Warning, &["max_n", "km"]),
            CoverageEdge => (Warning, &[]),
            ViaTooFar => (Error, &["n", "max_km"]),
            ViaOutsideZone | ViaUnreachable => (Error, &["n"]),
            ViaMissed => (Warning, &["n"]),
            // null : sans objet (mode max : pas de D+ demandé)
            TargetNotReached => (Warning, &["dplus_m", "best_dplus_m", "km", "best_km"]),
            FewerLoops => (Warning, &["asked", "got"]),
        }
    }
}

/// Codes retirés (jamais émis par le moteur Rust, T28) : noms à ne jamais réutiliser.
pub const RETIRED: [&str; 5] = [
    "source_unknown",
    "no_loop_from_start",
    "cancelled",
    "proven_infeasible",
    "target_proven_unreachable",
];

/// Message transmis au client : {"code", "params", "detail"?, "suggest"?, "checked"?}.
/// `suggest` (D40) : réglages de la requête à changer pour lever la contrainte limitante
/// (noms de paramètres de l'API, `null` = retirer) ; `checked` : confirmé par un calcul.
#[derive(Serialize, Debug)]
pub struct Msg {
    pub code: Code,
    pub params: Map<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggest: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
}

impl Msg {
    /// `params` : objet JSON dont les clés sont exactement celles de `code.info()`.
    pub fn new(code: Code, params: Value) -> Msg {
        let params = match params {
            Value::Object(m) => m,
            _ => Map::new(),
        };
        debug_assert!(
            params.keys().eq(code
                .info()
                .1
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>()),
            "paramètres de {code:?}"
        );
        Msg {
            code,
            params,
            detail: None,
            suggest: None,
            checked: None,
        }
    }

    /// Erreur avec paramètres et détail (anglais, logs).
    pub fn with_detail(code: Code, params: Value, detail: impl Into<String>) -> Msg {
        Msg {
            detail: Some(detail.into()),
            ..Msg::new(code, params)
        }
    }

    pub fn error(code: Code, detail: impl Into<String>) -> Msg {
        Msg {
            detail: Some(detail.into()),
            ..Msg::new(code, Value::Null)
        }
    }
}

/// Liste exportée pour le front : [{"code", "kind", "params"}].
pub fn export() -> Value {
    Value::Array(
        Code::ALL
            .iter()
            .map(|&c| {
                let (kind, params) = c.info();
                serde_json::json!({"code": c, "kind": kind, "params": params})
            })
            .collect(),
    )
}
