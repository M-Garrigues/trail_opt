//! Handler Lambda (provided.al2023, arm64) : `GET /api/plan`, `POST /api/loops`,
//! `GET /api/loops/<id>` derrière une Function URL AWS_IAM appelée par CloudFront (OAC).
//! cargo-lambda produit l'exécutable `bootstrap`.
//! Environnement :
//!   TILES_DIR          dossier des dalles (défaut : `tiles/` à côté de l'exécutable, D10)
//!   DATA_VERSION       version attendue des dalles (contrôle au démarrage, log seulement)
//!   TURNSTILE_SECRET   clé secrète Cloudflare Turnstile (D8) ; clés de test refusées en release
//!   TURNSTILE_HOSTNAMES  noms d'hôte acceptés (`hostname` de siteverify), séparés par des
//!                      virgules, ex. `optrail.eu` ; obligatoire en release
//!   LOOP_SIGNING_KEY   clé HMAC des boucles rendues (≥ 32 octets, M3) ; obligatoire en release,
//!                      aléatoire au démarrage en build debug si absente
//!   Build debug seulement (profil `local` de scripts/dev.sh), ignorés en release :
//!   TURNSTILE_DISABLED=1  désactive Turnstile
//!   FORCE_TURNSTILE=1  traite aussi les appels sans authorizer comme externes (tester Turnstile
//!                      de bout en bout avec `cargo lambda watch`)
//!   SHARED_BUCKET       bucket des boucles partagées (`shared/<id>.json`) ; absent : dossier
//!                      SHARE_DIR (défaut `<tmp>/optrail-shared`, développement)
//! Appel interne : un événement sans `requestContext.authorizer` vient d'un `lambda invoke`
//! direct (IAM, smoke test de deploy.yml) ; via la Function URL AWS_IAM, AWS remplit toujours
//! `authorizer.iam`, donc un client ne peut pas s'en faire passer.
use std::path::PathBuf;
use std::sync::Arc;

use engine::api;
use engine::share::{self, Store};
use engine::tiles::TileStore;
use lambda_http::request::RequestContext;
use lambda_http::{Body, Error, Request, RequestExt, Response, run, service_fn};
use serde_json::json;

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
}

async fn handler(ctx: Arc<Ctx>, req: Request) -> Result<Response<Body>, Error> {
    let json_resp = |status: u16, body: String| {
        Response::builder()
            .status(status)
            .header("content-type", "application/json")
            .header("cache-control", "no-store")
            .body(Body::from(body))
    };
    let token = req
        .headers()
        .get("x-turnstile-token")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let internal = internal(&req);
    let ip = viewer_ip(&req);
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
        // CloudFront transmet toutes les méthodes sur /api/loops* (jeu imposé) : 405
        (_, p) => {
            let status = if p.starts_with("/api/loops") {
                405
            } else {
                404
            };
            let m = engine::Msg::error(engine::Code::InvalidRequest, "unknown route or method");
            return Ok(json_resp(status, api::error_body(&m, status).to_string())?);
        }
    };
    let reply = task.await?;
    println!("{}", reply.log);
    Ok(json_resp(reply.status, reply.body.to_string())?)
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let dir = tiles_dir();
    let store = TileStore::open(&dir)?;
    let loops = Store::from_env();
    // E3, M3 : configuration de production vérifiée au démarrage (refus = pas de service)
    let turnstile = Turnstile::from_env()?;
    let key = signing_key()?;
    let expected = std::env::var("DATA_VERSION").unwrap_or_default();
    println!(
        "{}",
        json!({"msg": "init", "tiles_dir": dir, "data_version": store.manifest.data_version,
               "data_version_env": expected, "mismatch": !expected.is_empty() && expected != store.manifest.data_version,
               "tiles": store.manifest.tiles.len(), "solver_version": engine::solver_version(),
               "share_store": loops.describe(), "debug_build": cfg!(debug_assertions),
               "turnstile_hostnames": turnstile.hostnames})
    );
    let ctx = Arc::new(Ctx {
        store,
        loops,
        turnstile,
        key,
    });
    run(service_fn(move |req| handler(ctx.clone(), req))).await
}
