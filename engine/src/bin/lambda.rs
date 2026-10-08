//! Handler Lambda (provided.al2023, arm64) : `GET /api/plan`, `POST /api/loops`,
//! `GET /api/loops/<id>`, `POST /api/hit`, `GET /api/admin/stats` (contracts/admin.md)
//! derrière une Function URL AWS_IAM appelée par CloudFront (OAC).
//! cargo-lambda produit l'exécutable `bootstrap`.
//! Environnement :
//!   TILES_S3           s3://bucket/tiles/<DATA_VERSION>/ : dalles lues à la demande (cache /tmp/tiles/<version>,
//!                      TILES_CACHE ; plafond TILES_CACHE_MB, défaut 1 500 ; délai d'un GET
//!                      TILES_GET_TIMEOUT_S, défaut 5 ; S3_ENDPOINT = serveur compatible S3) ;
//!                      prioritaire sur TILES_DIR
//!   TILES_DIR          dossier des dalles (défaut : `tiles/` à côté de l'exécutable, D10)
//!   DATA_VERSION       version attendue des dalles (contrôle au démarrage, log seulement)
//!   TURNSTILE_SECRET   clé secrète Cloudflare Turnstile (D8) ; clés de test refusées en release
//!   TURNSTILE_HOSTNAMES  noms d'hôte acceptés (`hostname` de siteverify), séparés par des
//!                      virgules, ex. `optrail.eu` ; obligatoire en release
//!   LOOP_SIGNING_KEY   clé HMAC des boucles rendues (≥ 32 octets, M3) ; obligatoire en release,
//!                      aléatoire au démarrage en build debug si absente
//!   ADMIN_KEY          clé de /api/admin/* (en-tête x-admin-key) ; vide : routes en 404 ; en release,
//!                      moins de 32 caractères refusés au démarrage (en debug : toute clé, ex. `123`)
//!   AWS_LAMBDA_LOG_GROUP_NAME  groupe interrogé par /api/admin/stats (posé par Lambda)
//!   Sel des visiteurs de /api/hit : `salt/<jour>` dans SHARED_BUCKET (ou SHARE_DIR), D51.
//!   Build debug seulement (profil `local` de scripts/dev.sh), ignorés en release :
//!   AWS_PROFILE         /api/admin/stats passe par l'AWS CLI de ce profil (lecture seule)
//!   TURNSTILE_DISABLED=1  désactive Turnstile
//!   FORCE_TURNSTILE=1  traite aussi les appels sans authorizer comme externes (tester Turnstile
//!                      de bout en bout avec `cargo lambda watch`)
//!   SHARED_BUCKET       bucket des boucles partagées (`shared/<id>.json`) ; absent : dossier
//!                      SHARE_DIR (défaut `<tmp>/optrail-shared`, développement)
//! Appel interne : un événement sans `requestContext.authorizer` vient d'un `lambda invoke`
//! direct (IAM, smoke test de deploy.yml) ; via la Function URL AWS_IAM, AWS remplit toujours
//! `authorizer.iam`, donc un client ne peut pas s'en faire passer.
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use engine::admin::{self, Admin};
use engine::share::{self, Store};
use engine::tiles::TileStore;
use engine::{api, hit};
use lambda_http::request::RequestContext;
use lambda_http::{Body, Error, Request, RequestExt, Response, run, service_fn};
use serde_json::json;

/// Plafond de lignes `hit`/`event` par IP et par heure, par instance (revue F3 : un script qui
/// change d'User-Agent ne crée pas plus de HIT_MAX « visiteurs » par heure et par instance).
const HIT_MAX: u32 = 60;
const SITEVERIFY: &str = "https://challenges.cloudflare.com/turnstile/v0/siteverify";

fn tiles_dir() -> PathBuf {
    std::env::var_os("TILES_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let exe = std::env::current_exe().unwrap_or_default();
            exe.parent()
                .map(|p| p.join("tiles"))
                .unwrap_or_else(|| "tiles".into())
        })
}

/// Variable d'environnement de développement : lue seulement en build debug.
fn dev_flag(name: &str) -> bool {
    cfg!(debug_assertions) && std::env::var(name).is_ok_and(|v| v == "1")
}

/// Configuration Turnstile, contrôlée au démarrage.
struct Turnstile {
    secret: String,
    hostnames: Vec<String>,
}

impl Turnstile {
    /// En release : secret présent et pas une clé de test Cloudflare (`?x000…0AA`), liste
    /// d'hôtes non vide. En debug : tout est permis (clés de test de scripts/dev.sh).
    fn from_env() -> Result<Turnstile, String> {
        let secret = std::env::var("TURNSTILE_SECRET").unwrap_or_default();
        let hostnames: Vec<String> = std::env::var("TURNSTILE_HOSTNAMES")
            .unwrap_or_default()
            .split(',')
            .map(|h| h.trim().to_ascii_lowercase())
            .filter(|h| !h.is_empty())
            .collect();
        if !cfg!(debug_assertions) {
            if secret.is_empty() || secret.ends_with("x0000000000000000000000000000000AA") {
                return Err("TURNSTILE_SECRET missing or a Cloudflare test key".into());
            }
            if hostnames.is_empty() {
                return Err("TURNSTILE_HOSTNAMES missing".into());
            }
        }
        Ok(Turnstile { secret, hostnames })
    }

    /// POST siteverify (2 s max, E3) ; succès ET nom d'hôte attendu (si une liste est fixée).
    fn verify(&self, token: &str, remoteip: Option<&str>) -> Result<bool, String> {
        if dev_flag("TURNSTILE_DISABLED") {
            return Ok(true);
        }
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(2)))
            .build()
            .into();
        let mut form = vec![("secret", self.secret.as_str()), ("response", token)];
        if let Some(ip) = remoteip {
            form.push(("remoteip", ip));
        }
        let mut resp = agent
            .post(SITEVERIFY)
            .send_form(form)
            .map_err(|e| format!("siteverify: {e}"))?;
        let v: serde_json::Value = resp
            .body_mut()
            .read_json()
            .map_err(|e| format!("siteverify: {e}"))?;
        let host = v["hostname"].as_str().unwrap_or("").to_ascii_lowercase();
        if v["success"] == json!(true)
            && !self.hostnames.is_empty()
            && !self.hostnames.contains(&host)
        {
            return Err(format!("siteverify: unexpected hostname {host:?}"));
        }
        Ok(v["success"] == json!(true))
    }
}

/// Clé HMAC des boucles (M3) : `LOOP_SIGNING_KEY` (≥ 32 octets) ; en debug, aléatoire si absente.
fn signing_key() -> Result<ring::hmac::Key, String> {
    let k = std::env::var("LOOP_SIGNING_KEY").unwrap_or_default();
    if k.len() >= 32 {
        return Ok(ring::hmac::Key::new(ring::hmac::HMAC_SHA256, k.as_bytes()));
    }
    if cfg!(debug_assertions) && k.is_empty() {
        return ring::hmac::Key::generate(
            ring::hmac::HMAC_SHA256,
            &ring::rand::SystemRandom::new(),
        )
        .map_err(|_| "CSPRNG".to_string());
    }
    Err("LOOP_SIGNING_KEY missing or shorter than 32 bytes".into())
}

/// IP du visiteur pour siteverify : en-tête `CloudFront-Viewer-Address` (`ip:port`, posé par
/// CloudFront si l'origin request policy le transmet) ; sinon rien.
fn viewer_ip(req: &Request) -> Option<String> {
    let v = req
        .headers()
        .get("cloudfront-viewer-address")?
        .to_str()
        .ok()?;
    let ip = v.rsplit_once(':')?.0.trim_matches(['[', ']']);
    ip.parse::<std::net::IpAddr>().ok().map(|ip| ip.to_string())
}

fn internal(req: &Request) -> bool {
    if dev_flag("FORCE_TURNSTILE") {
        return false;
    }
    match req.request_context_ref() {
        Some(RequestContext::ApiGatewayV2(c)) => c.authorizer.is_none(),
        _ => false,
    }
}

struct Ctx {
    store: TileStore,
    loops: Store,
    turnstile: Turnstile,
    key: ring::hmac::Key,
    admin: Option<Admin>,
    /// lignes de mesure d'audience par IP (revue F3)
    hits: admin::Limiter,
    /// sel du jour des visiteurs (D51), mis en cache par instance : (jour, sel)
    salt: Mutex<Option<(String, Vec<u8>)>>,
    log_group: String,
}

fn today() -> i64 {
    let s = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    s as i64 / 86_400
}

/// Sel du jour (`salt/<jour>` du stockage des partages), relu une fois par jour et par instance.
fn day_salt(ctx: &Ctx) -> Option<Vec<u8>> {
    let day = admin::day_str(today());
    let mut c = ctx.salt.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((d, s)) = &*c
        && *d == day
    {
        return Some(s.clone());
    }
    match ctx.loops.salt(&day) {
        Ok(s) => {
            *c = Some((day, s.clone()));
            Some(s)
        }
        Err(e) => {
            println!("{}", json!({"msg": "hit_error", "detail": e}));
            None
        }
    }
}

fn header<'a>(req: &'a Request, name: &str) -> Option<&'a str> {
    req.headers().get(name).and_then(|v| v.to_str().ok())
}

async fn handler(ctx: Arc<Ctx>, req: Request) -> Result<Response<Body>, Error> {
    let json_resp = |status: u16, body: String| {
        Response::builder()
            .status(status)
            .header("content-type", "application/json")
            .header("cache-control", "no-store")
            .body(Body::from(body))
    };
    let token = header(&req, "x-turnstile-token").map(str::to_string);
    let internal = internal(&req);
    let ip = viewer_ip(&req);
    // pays/région du visiteur (CloudFront), jamais l'IP ni l'UA dans une ligne `plan`
    let geo = hit::geo(
        header(&req, "cloudfront-viewer-country"),
        header(&req, "cloudfront-viewer-country-region"),
    );
    let path = req.uri().path().to_string();
    let query: Vec<(String, String)> = req
        .query_string_parameters_ref()
        .map(|q| {
            q.iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        })
        .unwrap_or_default();
    // calcul, S3 et Turnstile bloquants : hors de la boucle asynchrone
    let task = match (req.method().as_str(), path.as_str()) {
        ("GET", "/api/plan") => tokio::task::spawn_blocking(move || {
            let verify = |t: &str| ctx.turnstile.verify(t, ip.as_deref());
            api::handle(
                &query,
                token.as_deref(),
                internal,
                &ctx.store,
                &ctx.key,
                verify,
            )
        }),
        ("POST", "/api/loops") => {
            let body = req.body().to_vec();
            tokio::task::spawn_blocking(move || {
                let verify = |t: &str| ctx.turnstile.verify(t, ip.as_deref());
                share::post(
                    &body,
                    token.as_deref(),
                    internal,
                    &ctx.key,
                    verify,
                    |id, d| ctx.loops.put(id, d),
                )
            })
        }
        ("GET", p) if p.starts_with("/api/loops/") => {
            let id = p["/api/loops/".len()..].to_string();
            tokio::task::spawn_blocking(move || share::get(&id, |id| ctx.loops.get(id)))
        }
        // mesure d'audience (admin.md § 2, D51) : pas de Turnstile, robots filtrés par UA
        ("POST", "/api/hit") => {
            let body = req.body().to_vec();
            let ua = header(&req, "user-agent").unwrap_or("").to_string();
            let geo = geo.clone();
            tokio::task::spawn_blocking(move || {
                let hosts = &ctx.turnstile.hostnames;
                let (status, mut line) =
                    hit::hit(&body, ip.as_deref(), &ua, geo, hosts, || day_salt(&ctx));
                // revue F3 : au plus HIT_MAX lignes par IP et par heure (par instance) ; au-delà, 204 muet
                let who = ip.as_deref().unwrap_or("");
                if line.is_some() && !ctx.hits.allow(who, std::time::Instant::now()) {
                    line = None;
                }
                let m = engine::Msg::error(engine::Code::InvalidRequest, "invalid hit");
                api::Reply {
                    status,
                    body: api::error_body(&m, status),
                    log: line.unwrap_or_default(),
                }
            })
        }
        // admin (admin.md § 3–4) : 404 si ADMIN_KEY est absente
        ("GET", "/api/admin/stats") if ctx.admin.is_some() => {
            let given = header(&req, "x-admin-key").map(str::to_string);
            tokio::task::spawn_blocking(move || {
                let Some(adm) = &ctx.admin else {
                    unreachable!()
                };
                let region = std::env::var("AWS_REGION").unwrap_or_else(|_| "eu-north-1".into());
                // local (build de développement + AWS_PROFILE) : AWS CLI, sinon API signée
                let cli = cfg!(debug_assertions) && std::env::var_os("AWS_PROFILE").is_some();
                // quotas Logs : 5 appels/s (API) ; AWS CLI : ~1 s par appel, tout en parallèle
                let pace = if cli {
                    (9, Duration::from_millis(300))
                } else {
                    (5, Duration::from_secs(1))
                };
                let call = |op: &str, b: &serde_json::Value| {
                    if cli {
                        admin::logs_cli(&region, op, b)
                    } else {
                        admin::logs_api(&region, op, b)
                    }
                };
                let who = ip.as_deref().unwrap_or("");
                admin::handle(adm, given.as_deref(), who, &query, today(), |f, t| {
                    admin::run(f, t, &ctx.log_group, call, pace, admin::DEADLINE)
                })
            })
        }
        // consommation AWS du mois (admin, même clé), sources en lecture, cache 3 h
        ("GET", "/api/admin/usage") if ctx.admin.is_some() => {
            let given = header(&req, "x-admin-key").map(str::to_string);
            tokio::task::spawn_blocking(move || {
                let adm = ctx.admin.as_ref().expect("admin");
                let region = std::env::var("AWS_REGION").unwrap_or_else(|_| "eu-north-1".into());
                let now = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs());
                let who = ip.as_deref().unwrap_or("");
                admin::handle_usage(adm, given.as_deref(), who, &query, || {
                    admin::usage(&ctx.log_group, &region, now, admin::aws)
                })
            })
        }
        // CloudFront transmet toutes les méthodes sur /api/loops* et /api/hit (jeu imposé) : 405
        (_, p) => {
            let status = if p.starts_with("/api/loops") || p == "/api/hit" {
                405
            } else {
                404
            };
            let m = engine::Msg::error(engine::Code::InvalidRequest, "unknown route or method");
            return Ok(json_resp(status, api::error_body(&m, status).to_string())?);
        }
    };
    let mut reply = task.await?;
    if path == "/api/plan"
        && let Some(o) = reply.log.as_object_mut()
    {
        o.extend(geo);
    }
    if !reply.log.is_null() {
        println!("{}", reply.log);
    }
    if reply.status == 204 {
        let r = Response::builder()
            .status(204)
            .header("cache-control", "no-store");
        return Ok(r.body(Body::Empty)?);
    }
    let mut r = json_resp(reply.status, reply.body.to_string())?;
    if path.starts_with("/api/admin/") {
        r.headers_mut()
            .insert("x-robots-tag", "noindex".parse().expect("en-tête"));
    }
    Ok(r)
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    // TILES_S3 (s3://bucket/tiles/<version>/) : dalles lues à la demande dans /tmp/tiles/<version>
    let (dir, store) = match std::env::var("TILES_S3").ok().filter(|s| !s.is_empty()) {
        Some(url) => {
            let cache = std::env::var_os("TILES_CACHE").map_or("/tmp/tiles".into(), PathBuf::from);
            let store = TileStore::open_s3(&url, &cache)?;
            (PathBuf::from(url), store)
        }
        None => {
            let dir = tiles_dir();
            let store = TileStore::open(&dir)?;
            (dir, store)
        }
    };
    let loops = Store::from_env();
    // E3, M3 : configuration de production vérifiée au démarrage (refus = pas de service)
    let turnstile = Turnstile::from_env()?;
    let key = signing_key()?;
    // admin.md § 3 : clé courte refusée en release (démarrage en échec) ; vide = admin désactivé
    let admin = Admin::from_key(
        &std::env::var("ADMIN_KEY").unwrap_or_default(),
        cfg!(debug_assertions),
    )?;
    let log_group = std::env::var("AWS_LAMBDA_LOG_GROUP_NAME")
        .unwrap_or_else(|_| "/aws/lambda/optrail-api".into());
    let expected = std::env::var("DATA_VERSION").unwrap_or_default();
    println!(
        "{}",
        json!({"msg": "init", "tiles_dir": dir, "data_version": store.manifest.data_version,
               "data_version_env": expected, "mismatch": !expected.is_empty() && expected != store.manifest.data_version,
               "tiles": store.manifest.tiles.len(), "solver_version": engine::solver_version(),
               "share_store": loops.describe(), "debug_build": cfg!(debug_assertions),
               "turnstile_hostnames": turnstile.hostnames, "admin": admin.is_some(),
               "log_group": log_group})
    );
    let ctx = Arc::new(Ctx {
        store,
        loops,
        turnstile,
        key,
        admin,
        salt: Mutex::new(None),
        hits: admin::Limiter::new(HIT_MAX, Duration::from_secs(3600)),
        log_group,
    });
    run(service_fn(move |req| handler(ctx.clone(), req))).await
}
