//! Codes stables des erreurs et avertissements (décision D14). Le client affiche
//! `m.err[code](params)` ; jamais de phrase affichable ici, `detail` anglais pour les logs.
//! Un code n'est jamais renommé ni réutilisé : la fixture `engine/codes.json` (test
//! `codes_stables`) le vérifie. Pour en ajouter un : variante + ligne de `info()` + `ALL`.
use serde::Serialize;
use serde_json::{Map, Value};

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Code {
    // Erreurs de saisie (pipeline.validate et plan_loop)
    ZoneInvalid,
    StartOutsideZone,
    ModeUnknown,
    RoadsUnknown,
    SourceUnknown,
    TargetDplusRequired,
    ToleranceOutOfRange,
    MaxGradeInvalid,
    DistanceOutOfRange,
    TimeOutOfRange,
    ZoneTooLarge,
    // Erreurs de calcul
    NoWayInZone,
    NoLoopFromStart,
    NoLoopOfDistance,
    Cancelled,
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
    ProvenInfeasible,
    DistanceOutOfTolerance,
    TargetDplusAboveBound,
    TargetProvenUnreachable,
    TargetDplusProbablyUnreachable,
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Error,
    Warning,
}

impl Code {
    pub const ALL: [Code; 29] = {
        use Code::*;
        [
            ZoneInvalid,
            StartOutsideZone,
            ModeUnknown,
            RoadsUnknown,
            SourceUnknown,
            TargetDplusRequired,
            ToleranceOutOfRange,
            MaxGradeInvalid,
            DistanceOutOfRange,
            TimeOutOfRange,
            ZoneTooLarge,
            NoWayInZone,
            NoLoopFromStart,
            NoLoopOfDistance,
            Cancelled,
            InvalidProblem,
            NoLoopFound,
            InvariantViolated,
            LongDistance,
            ZoneReduced,
            AccessRoundTrip,
            StartMoved,
            StartFarFromNetwork,
            ProfileMismatch,
            ProvenInfeasible,
            DistanceOutOfTolerance,
            TargetDplusAboveBound,
            TargetProvenUnreachable,
            TargetDplusProbablyUnreachable,
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
            ZoneInvalid | StartOutsideZone | ModeUnknown | RoadsUnknown | SourceUnknown
            | TargetDplusRequired | ToleranceOutOfRange | MaxGradeInvalid | NoWayInZone
            | NoLoopFromStart | NoLoopOfDistance | Cancelled | InvalidProblem | NoLoopFound
            | InvariantViolated => (Error, &[]),
            LongDistance => (Warning, &["km"]),
            ZoneReduced => (Warning, &["radius_km", "area_km2"]),
            AccessRoundTrip => (Warning, &["access_m"]),
            StartMoved | StartFarFromNetwork => (Warning, &["distance_m"]),
            ProfileMismatch => (Warning, &["profile_m", "sum_w_m"]),
            ProvenInfeasible => (Warning, &["min_km", "max_km"]),
            DistanceOutOfTolerance | TargetDplusProbablyUnreachable => (Warning, &[]),
            TargetDplusAboveBound => (Warning, &["max_dplus_m", "max_km"]),
            TargetProvenUnreachable => (Warning, &["min_error"]),
        }
    }
}

/// Message transmis au client : {"code", "params", "detail"?}.
#[derive(Serialize, Debug)]
pub struct Msg {
    pub code: Code,
    pub params: Map<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
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
