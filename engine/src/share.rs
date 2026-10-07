//! Partage des boucles (contrat `.team/contracts/api.md` v1.1) : `POST /api/loops` valide le
//! corps (schéma strict), vérifie Turnstile, tire un id aléatoire et écrit `shared/<id>.json` ;
//! `GET /api/loops/<id>` relit l'objet. Stockage : S3 (`SHARED_BUCKET`, préfixe à cycle de vie
//! 90 j, D21) ou dossier local (`SHARE_DIR`, développement). SigV4 et CSPRNG par `ring`.
use std::collections::BTreeMap;
use std::path::PathBuf;

use ring::{digest, hmac, rand::SecureRandom};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::api::{Reply, check_bot, error_body, http_status};
use crate::codes::{Code, Kind, Msg};

/// Taille max du corps (octets) et nombre max de points de la géométrie.
pub const MAX_BODY: usize = 2 << 20;
pub const MAX_POINTS: usize = 50_000;
pub const MAX_ZONE_VERTICES: usize = 10_000;
/// Id : 12 caractères base62 (≈ 71 bits).
pub const ID_LEN: usize = 12;
const B62: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// Corps de `POST /api/loops`, renvoyé tel quel par `GET` (champs de la réponse de /api/plan).
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SharedLoop {
    /// Paramètres de `/api/plan` (chaînes, nombres ou booléens), validés comme une requête.
    pub request: BTreeMap<String, Value>,
    pub data_version: String,
    pub solver_version: String,
    pub effective_start: Start,
    #[serde(default)]
    pub zone: Option<Zone>,
    pub candidate: Candidate,
    pub warnings: Vec<Warning>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Start {
    pub lat: f64,
    pub lon: f64,
    pub kind: StartKind,
    #[serde(default)]
    pub moved_m: Option<f64>,
    #[serde(default)]
    pub access_m: Option<f64>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StartKind {
    Clicked,
    Moved,
    Access,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Zone {
    pub geometry: MultiPolygon,
    pub area_km2: f64,
    #[serde(default)]
    pub reduced_radius_km: Option<f64>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MultiPolygon {
    #[serde(rename = "type")]
    pub kind: MultiPolygonType,
    pub coordinates: Vec<Vec<Vec<[f64; 2]>>>,
}

#[derive(Deserialize, Serialize)]
pub enum MultiPolygonType {
    MultiPolygon,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub length_m: f64,
    pub dplus_m: f64,
    pub feasible: bool,
    pub alt_min_m: f64,
    pub alt_max_m: f64,
    pub max_grade_pct: f64,
    /// v1.7 : part de la longueur sur « chemin » ; absente des boucles plus anciennes (et alors
    /// non sérialisée : leur signature d'origine reste valable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trail_frac: Option<f64>,
    /// v1.8 : parts (chemin naturel, intermédiaire, route) ; absentes des boucles plus anciennes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface_share: Option<[f64; 3]>,
    /// v1.8 (étiquettes des dalles) : part au calme, mètres balisés, mètres au bord de l'eau ;
    /// absents sans la colonne correspondante (et dans les boucles plus anciennes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calm_frac: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hike_m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub water_m: Option<f64>,
    #[serde(default)]
    pub target_gap: Option<Gap>,
    pub climbs: Climbs,
    /// v1.5 (T34) : points de passage, tronçons, repères ; absents des boucles plus anciennes.
    #[serde(default)]
    pub via: Vec<Via>,
    #[serde(default)]
    pub legs: Vec<Leg>,
    #[serde(default)]
    pub landmarks: Vec<Landmark>,
    pub lat: Vec<f64>,
    pub lon: Vec<f64>,
    pub ele: Vec<f64>,
    pub dist: Vec<f64>,
    /// HMAC-SHA256 (hex) posé par `/api/plan` (M3) ; exigé par `POST /api/loops`, pas stocké.
    #[serde(default, skip_serializing)]
    pub sig: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Via {
    pub n: u32,
    pub lat: f64,
    pub lon: f64,
    pub snap_m: f64,
    pub dist_m: f64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Leg {
    pub from: u32,
    pub to: u32,
    pub length_m: f64,
    pub dplus_m: f64,
    pub dminus_m: f64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Landmark {
    pub kind: String,
    pub name: String,
    pub ele_m: Option<f64>,
    pub dist_m: f64,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Gap {
    pub distance_m: f64,
    pub dplus_m: f64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Climbs {
    pub count: u32,
    pub longest_gain_m: f64,
    pub longest_len_m: f64,
    pub gbar_m: f64,
    pub mean_grade_pct: f64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Warning {
    pub code: Code,
    pub params: Map<String, Value>,
    /// D40 : réglages proposés (objet de ≤ 4 scalaires ou null), voir `Msg::suggest`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggest: Option<Map<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
}

fn bad(detail: impl Into<String>) -> Msg {
    Msg::error(Code::InvalidRequest, detail)
}

/// Signature (hex) d'une boucle : HMAC-SHA256 de tout sauf `request` et `candidate.sig`,
/// resérialisé depuis le schéma typé (nombres normalisés : `10494` et `10494.0` signent pareil).
pub fn signature(key: &hmac::Key, s: &SharedLoop) -> String {
    hex(hmac::sign(key, &signed_bytes(s)).as_ref())
}

fn signed_bytes(s: &SharedLoop) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "data_version": s.data_version, "solver_version": s.solver_version,
        "effective_start": s.effective_start, "zone": s.zone, "candidate": s.candidate, "warnings": s.warnings,
    }))
    .expect("JSON")
}

/// `/api/plan` : pose `sig` sur chaque boucle de la réponse (déjà simplifiée, telle qu'envoyée).
pub fn sign_candidates(out: &mut Value, key: &hmac::Key) -> Result<(), Msg> {
    let n = out["candidates"].as_array().map_or(0, Vec::len);
    for i in 0..n {
        let v = json!({
            "request": {}, "data_version": out["data_version"], "solver_version": out["solver_version"],
            "effective_start": out["effective_start"], "zone": out["zone"],
            "candidate": out["candidates"][i], "warnings": out["warnings"],
        });
        let s: SharedLoop = serde_json::from_value(v)
            .map_err(|e| Msg::error(Code::InvariantViolated, format!("sign: {e}")))?;
        out["candidates"][i]["sig"] = json!(signature(key, &s));
    }
    Ok(())
}

/// Corps → boucle validée (schéma strict, signature `candidate.sig` vérifiée), resérialisée
/// (seuls les champs du schéma sont stockés, sans la signature).
pub fn validate(body: &[u8], key: &hmac::Key) -> Result<Vec<u8>, Msg> {
    if body.len() > MAX_BODY {
        return Err(bad(format!("body larger than {MAX_BODY} bytes")));
    }
    let s: SharedLoop = serde_json::from_slice(body).map_err(|e| bad(e.to_string()))?;
    let sig_ok = s.candidate.sig.len() == 64
        && s.candidate.sig.is_ascii()
        && (0..32)
            .map(|i| u8::from_str_radix(&s.candidate.sig[2 * i..2 * i + 2], 16))
            .collect::<Result<Vec<u8>, _>>()
            .is_ok_and(|tag| hmac::verify(key, &signed_bytes(&s), &tag).is_ok());
    if !sig_ok {
        return Err(bad("candidate.sig: missing or invalid signature"));
    }
    let query = s
        .request
        .iter()
        .map(|(k, v)| match v {
            Value::String(x) => Ok((k.clone(), x.clone())),
            Value::Number(_) | Value::Bool(_) => Ok((k.clone(), v.to_string())),
            _ => Err(bad(format!(
                "request.{k}: string, number or boolean expected"
            ))),
        })
        .collect::<Result<Vec<_>, _>>()?;
    crate::api::parse(&query)?;
    let short = |x: &str| !x.is_empty() && x.len() <= 64 && x.bytes().all(|b| b.is_ascii_graphic());
    if !short(&s.data_version) || !short(&s.solver_version) {
        return Err(bad(
            "data_version/solver_version: 1..64 printable ASCII characters",
        ));
    }
    let c = &s.candidate;
    let n = c.lat.len();
    if !(2..=MAX_POINTS).contains(&n) || [c.lon.len(), c.ele.len(), c.dist.len()] != [n; 3] {
        return Err(bad(format!(
            "candidate: lat/lon/ele/dist of equal length 2..{MAX_POINTS}"
        )));
    }
    let wgs = |lat: f64, lon: f64| (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon);
    if !wgs(s.effective_start.lat, s.effective_start.lon)
        || !c.lat.iter().zip(&c.lon).all(|(&a, &b)| wgs(a, b))
    {
        return Err(bad("coordinates out of range"));
    }
    if let Some(z) = &s.zone {
        let pts = z.geometry.coordinates.iter().flatten().flatten();
        if pts.clone().count() > MAX_ZONE_VERTICES || !pts.clone().all(|p| wgs(p[1], p[0])) {
            return Err(bad("zone: too many vertices or out of range"));
        }
    }
    for w in &s.warnings {
        let (kind, names) = w.code.info();
        let keys_ok = w.params.keys().map(String::as_str).eq(names
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>(
        ));
        let values_ok = w.params.values().all(|v| v.is_number() || v.is_null());
        let suggest_ok = w.suggest.as_ref().is_none_or(|m| {
            m.len() <= 4
                && m.values()
                    .all(|v| v.is_string() || v.is_number() || v.is_null())
        });
        if kind != Kind::Warning || !keys_ok || !values_ok || !suggest_ok {
            return Err(bad(format!("warning {:?}: unknown code or params", w.code)));
        }
    }
    if s.warnings.len() > Code::ALL.len() {
        return Err(bad("too many warnings"));
    }
    serde_json::to_vec(&s).map_err(|e| bad(e.to_string()))
}

/// Id aléatoire (CSPRNG du système), base62 sans biais (rejet des octets ≥ 248).
pub fn new_id() -> String {
    let rng = ring::rand::SystemRandom::new();
    let mut id = String::with_capacity(ID_LEN);
    let mut buf = [0u8; 32];
    while id.len() < ID_LEN {
        rng.fill(&mut buf).expect("CSPRNG");
        for &b in buf.iter().filter(|&&b| b < 248) {
            if id.len() < ID_LEN {
                id.push(B62[(b % 62) as usize] as char);
            }
        }
    }
    id
}

pub fn valid_id(id: &str) -> bool {
    id.len() == ID_LEN && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

fn fail(m: Msg, op: &str) -> Reply {
    let status = http_status(m.code);
    let log =
        json!({"msg": "share", "op": op, "status": status, "code": m.code, "detail": m.detail});
    Reply {
        status,
        body: error_body(&m, status),
        log,
    }
}

/// `POST /api/loops` : validation (signature comprise), puis Turnstile (sauf appel interne),
/// puis écriture.
pub fn post(
    body: &[u8],
    token: Option<&str>,
    internal: bool,
    key: &hmac::Key,
    verify: impl FnOnce(&str) -> Result<bool, String>,
    put: impl FnOnce(&str, &[u8]) -> Result<(), String>,
) -> Reply {
    let data = match validate(body, key) {
        Ok(d) => d,
        Err(m) => return fail(m, "post"),
    };
    if let Err(m) = check_bot(token, internal, verify) {
        return fail(m, "post");
    }
    let id = new_id();
    if let Err(e) = put(&id, &data) {
        return fail(Msg::error(Code::Busy, e), "post");
    }
    let log = json!({"msg": "share", "op": "post", "status": 201, "bytes": data.len(), "internal": internal});
    Reply {
        status: 201,
        body: json!({ "id": id }),
        log,
    }
}

/// `GET /api/loops/<id>` : la boucle stockée, ou 404 `loop_not_found` (id mal formé, inconnu
/// ou expiré).
pub fn get(id: &str, load: impl FnOnce(&str) -> Result<Option<Vec<u8>>, String>) -> Reply {
    let missing = || fail(Msg::error(Code::LoopNotFound, "no such loop"), "get");
    if !valid_id(id) {
        return missing();
    }
    match load(id) {
        Ok(Some(b)) => match serde_json::from_slice::<Value>(&b) {
            Ok(body) => Reply {
                status: 200,
                body,
                log: json!({"msg": "share", "op": "get", "status": 200}),
            },
            Err(e) => fail(
                Msg::error(Code::Busy, format!("stored loop unreadable: {e}")),
                "get",
            ),
        },
        Ok(None) => missing(),
        Err(e) => fail(Msg::error(Code::Busy, e), "get"),
    }
}

/// Stockage des boucles partagées : clé `shared/<id>.json`.
pub enum Store {
    S3 { bucket: String, region: String },
    Dir(PathBuf),
}

impl Store {
    /// `SHARED_BUCKET` (+ `AWS_REGION`, identifiants du rôle Lambda) ; sinon dossier `SHARE_DIR`
    /// (défaut : `<tmp>/optrail-shared`).
    pub fn from_env() -> Store {
        match std::env::var("SHARED_BUCKET") {
            Ok(bucket) if !bucket.is_empty() => Store::S3 {
                bucket,
                region: std::env::var("AWS_REGION").unwrap_or_else(|_| "eu-north-1".into()),
            },
            _ => Store::Dir(
                std::env::var_os("SHARE_DIR")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| std::env::temp_dir().join("optrail-shared")),
            ),
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Store::S3 { bucket, .. } => format!("s3://{bucket}/shared/"),
            Store::Dir(d) => d.display().to_string(),
        }
    }

    pub fn put(&self, id: &str, data: &[u8]) -> Result<(), String> {
        match self {
            Store::Dir(d) => {
                std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
                std::fs::write(d.join(format!("{id}.json")), data).map_err(|e| e.to_string())
            }
            Store::S3 { .. } => self
                .s3("PUT", id, data)
                .and_then(|(status, _)| match status {
                    200 => Ok(()),
                    s => Err(format!("s3 put: HTTP {s}")),
                }),
        }
    }

    pub fn get(&self, id: &str) -> Result<Option<Vec<u8>>, String> {
        match self {
            Store::Dir(d) => match std::fs::read(d.join(format!("{id}.json"))) {
                Ok(b) => Ok(Some(b)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e.to_string()),
            },
            // sans s3:ListBucket, S3 répond 403 (et non 404) pour une clé absente
            Store::S3 { .. } => match self.s3("GET", id, &[])? {
                (200, b) => Ok(Some(b)),
                (403 | 404, _) => Ok(None),
                (s, _) => Err(format!("s3 get: HTTP {s}")),
            },
        }
    }

    /// Sel quotidien des visiteurs uniques (D51) : `salt/<jour>` (32 octets aléatoires) créé par
    /// la première instance qui le demande (écriture conditionnelle : S3 `If-None-Match: *`,
    /// fichier `create_new`), relu sinon. Oublié après 2 jours (cycle de vie S3, infra/cdn.tf).
    pub fn salt(&self, day: &str) -> Result<Vec<u8>, String> {
        let mut fresh = [0u8; 32];
        ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut fresh)
            .map_err(|_| "CSPRNG".to_string())?;
        match self {
            Store::Dir(d) => {
                use std::io::Write;
                let p = d.join("salt").join(day);
                std::fs::create_dir_all(d.join("salt")).map_err(|e| e.to_string())?;
                match std::fs::File::create_new(&p) {
                    Ok(mut f) => f.write_all(&fresh).map(|_| fresh.to_vec()),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => std::fs::read(&p),
                    Err(e) => Err(e),
                }
                .map_err(|e| e.to_string())
            }
            Store::S3 { bucket, region } => {
                let t = S3Target {
                    bucket: bucket.clone(),
                    region: region.clone(),
                    endpoint: None,
                };
                let (creds, key) = (Creds::from_env()?, format!("salt/{day}"));
                let put = [("if-none-match", "*".to_string())];
                match s3_request(&t, &creds, "PUT", &key, &fresh, &put, (5, 1024))?.0 {
                    200 => return Ok(fresh.to_vec()),
                    412 | 409 => {} // déjà créé (409 : création concurrente en cours)
                    s => return Err(format!("s3 put salt: HTTP {s}")),
                }
                match s3_request(&t, &creds, "GET", &key, &[], &[], (5, 1024))? {
                    (200, b) if b.len() == 32 => Ok(b),
                    (s, _) => Err(format!("s3 get salt: HTTP {s}")),
                }
            }
        }
    }

    /// Requête S3 signée SigV4 (identifiants temporaires du rôle Lambda).
    fn s3(&self, method: &str, id: &str, body: &[u8]) -> Result<(u16, Vec<u8>), String> {
        let Store::S3 { bucket, region } = self else {
            unreachable!()
        };
        let target = S3Target {
            bucket: bucket.clone(),
            region: region.clone(),
            endpoint: None,
        };
        let key = format!("shared/{id}.json");
        let ct = (method == "PUT").then_some("application/json");
        s3_call(
            &target,
            &Creds::from_env()?,
            method,
            &key,
            body,
            ct,
            (5, MAX_BODY as u64 + 1024),
        )
    }
}

/// Bucket S3 ; `endpoint` (http://hôte:port, adressage par chemin) : MinIO / serveur de test.
pub struct S3Target {
    pub bucket: String,
    pub region: String,
    pub endpoint: Option<String>,
}

/// Identifiants AWS (rôle Lambda : variables d'environnement).
pub struct Creds {
    pub key_id: String,
    pub secret: String,
    pub token: Option<String>,
}

impl Creds {
    pub fn from_env() -> Result<Creds, String> {
        let env = |k: &str| std::env::var(k).map_err(|_| format!("{k} missing"));
        Ok(Creds {
            key_id: env("AWS_ACCESS_KEY_ID")?,
            secret: env("AWS_SECRET_ACCESS_KEY")?,
            token: std::env::var("AWS_SESSION_TOKEN").ok(),
        })
    }
}

/// Requête S3 signée SigV4 ; `limits` = (délai en s, taille maximale de la réponse).
pub fn s3_call(
    t: &S3Target,
    creds: &Creds,
    method: &str,
    key: &str,
    body: &[u8],
    content_type: Option<&str>,
    limits: (u64, u64),
) -> Result<(u16, Vec<u8>), String> {
    let extra: Vec<(&str, String)> = content_type
        .map(|ct| ("content-type", ct.to_string()))
        .into_iter()
        .collect();
    s3_request(t, creds, method, key, body, &extra, limits)
}

/// `s3_call` avec des en-têtes signés en plus (noms en minuscules).
pub fn s3_request(
    t: &S3Target,
    creds: &Creds,
    method: &str,
    key: &str,
    body: &[u8],
    extra: &[(&str, String)],
    limits: (u64, u64),
) -> Result<(u16, Vec<u8>), String> {
    let (url, host, path) = match &t.endpoint {
        Some(e) => {
            let host = e.split("://").last().unwrap_or(e).to_string();
            let path = format!("/{}/{key}", t.bucket);
            (format!("{e}{path}"), host, path)
        }
        None => {
            let host = format!("{}.s3.{}.amazonaws.com", t.bucket, t.region);
            let path = format!("/{key}");
            (format!("https://{host}{path}"), host, path)
        }
    };
    let date = amz_date(std::time::SystemTime::now());
    let hash = hex(digest::digest(&digest::SHA256, body).as_ref());
    let mut headers = vec![
        ("host", host),
        ("x-amz-content-sha256", hash),
        ("x-amz-date", date.clone()),
    ];
    headers.extend(extra.iter().cloned());
    if let Some(tok) = &creds.token {
        headers.push(("x-amz-security-token", tok.clone()));
    }
    headers.sort();
    let auth = sigv4(
        method,
        &path,
        &headers,
        &date,
        &t.region,
        "s3",
        &creds.key_id,
        &creds.secret,
    );
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(limits.0)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut req = match method {
        "PUT" => ureq::http::Request::put(&url),
        _ => ureq::http::Request::get(&url),
    };
    for (k, v) in headers.iter().filter(|h| h.0 != "host") {
        req = req.header(*k, v);
    }
    let req = req
        .header("authorization", auth)
        .body(body.to_vec())
        .map_err(|e| e.to_string())?;
    let mut resp = agent.run(req).map_err(|e| format!("s3: {e}"))?;
    let status = resp.status().as_u16();
    let bytes = resp
        .body_mut()
        .with_config()
        .limit(limits.1)
        .read_to_vec()
        .map_err(|e| format!("s3: {e}"))?;
    Ok((status, bytes))
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// `AAAAMMJJTHHMMSSZ` (UTC) sans dépendance de date.
pub fn amz_date(t: std::time::SystemTime) -> String {
    let s = t
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs()) as i64;
    let (days, rem) = (s.div_euclid(86_400), s.rem_euclid(86_400));
    // jours → date civile (H. Hinnant, civil_from_days)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}{m:02}{d:02}T{:02}{:02}{:02}Z",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}

/// En-tête `Authorization` SigV4 (requête sans paramètres de requête) ; `headers` triés, noms
/// en minuscules, et contient `x-amz-content-sha256` (hash de la charge utile).
#[allow(clippy::too_many_arguments)]
pub fn sigv4(
    method: &str,
    path: &str,
    headers: &[(&str, String)],
    date: &str,
    region: &str,
    service: &str,
    key_id: &str,
    secret: &str,
) -> String {
    let signed = headers.iter().map(|h| h.0).collect::<Vec<_>>().join(";");
    let canon_headers: String = headers
        .iter()
        .map(|(k, v)| format!("{k}:{}\n", v.trim()))
        .collect();
    let payload = headers
        .iter()
        .find(|h| h.0 == "x-amz-content-sha256")
        .map_or("", |h| h.1.as_str());
    let canonical = format!("{method}\n{path}\n\n{canon_headers}\n{signed}\n{payload}");
    let scope = format!("{}/{region}/{service}/aws4_request", &date[..8]);
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{date}\n{scope}\n{}",
        hex(digest::digest(&digest::SHA256, canonical.as_bytes()).as_ref())
    );
    let mac = |k: &[u8], m: &str| hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, k), m.as_bytes());
    let mut k = mac(format!("AWS4{secret}").as_bytes(), &date[..8]);
    for part in [region, service, "aws4_request"] {
        k = mac(k.as_ref(), part);
    }
    let sig = hex(mac(k.as_ref(), &to_sign).as_ref());
    format!("AWS4-HMAC-SHA256 Credential={key_id}/{scope}, SignedHeaders={signed}, Signature={sig}")
}
