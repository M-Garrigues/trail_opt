//! Admin (contracts/admin.md § 3–4, D47) : clé `x-admin-key` comparée à temps constant avec
//! limitation des essais, et `GET /api/admin/stats` (requêtes CloudWatch Logs Insights mises en
//! forme). Le transport Logs est injecté (`call`) : API signée SigV4 en Lambda, AWS CLI en local.
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use ring::hmac;
use serde_json::{Map, Value, json};

use crate::api::{Reply, error_body, http_status};
use crate::codes::{Code, Msg};

/// Longueur minimale de `ADMIN_KEY` en release (`openssl rand -hex 32`).
pub const MIN_KEY_LEN: usize = 32;
pub const MAX_FAILS: u32 = 5;
pub const WINDOW: Duration = Duration::from_secs(15 * 60);
pub const MAX_DAYS: i64 = 400;
/// Échéance des requêtes Insights (Lambda : 30 s) ; vues non terminées : `null` + `incomplete`.
pub const DEADLINE: Duration = Duration::from_secs(20);
/// Message d'erreur des identifiants expirés (local : `aws login --profile optrail`), lu par la page.
pub const EXPIRED: &str = "aws credentials expired";

/// Clé admin : seule son empreinte HMAC (clé aléatoire de l'instance) est gardée ; `fails` =
/// (début de la fenêtre, échecs) de cette instance.
pub struct Admin {
    k: hmac::Key,
    tag: hmac::Tag,
    /// échecs par adresse IP (revue F5 : un tiers ne peut plus verrouiller l'admin pour tous)
    fails: Limiter,
}

/// Compteur par adresse IP et par instance Lambda : au plus `max` dans une fenêtre `window`
/// ouverte au premier comptage. Mémoire bornée : au-delà de 10 000 adresses, les fenêtres
/// échues sont oubliées. ponytail: par instance (≤ 10), sans état partagé ; DynamoDB si besoin.
pub struct Limiter {
    map: Mutex<HashMap<String, (Instant, u32)>>,
    max: u32,
    window: Duration,
}

impl Limiter {
    pub fn new(max: u32, window: Duration) -> Limiter {
        Limiter {
            map: Mutex::new(HashMap::new()),
            max,
            window,
        }
    }

    /// `who` a-t-il atteint le plafond dans sa fenêtre en cours ?
    pub fn blocked(&self, who: &str, now: Instant) -> bool {
        let m = self.map.lock().unwrap_or_else(|e| e.into_inner());
        m.get(who)
            .is_some_and(|&(t, n)| now.saturating_duration_since(t) < self.window && n >= self.max)
    }

    /// Compte une occurrence ; `false` si le plafond est dépassé.
    pub fn allow(&self, who: &str, now: Instant) -> bool {
        let mut m = self.map.lock().unwrap_or_else(|e| e.into_inner());
        if m.len() > 10_000 {
            m.retain(|_, (t, _)| now.saturating_duration_since(*t) < self.window);
        }
        let e = m.entry(who.to_string()).or_insert((now, 0));
        if now.saturating_duration_since(e.0) >= self.window {
            *e = (now, 0);
        }
        e.1 += 1;
        e.1 <= self.max
    }
}

impl Admin {
    /// `ADMIN_KEY` vide : admin désactivé (routes en 404). En release (`dev = false`), une clé
    /// de moins de `MIN_KEY_LEN` caractères est refusée au démarrage ; en build local, toute
    /// clé non vide (ex. `123`) est acceptée.
    pub fn from_key(key: &str, dev: bool) -> Result<Option<Admin>, String> {
        if key.is_empty() {
            return Ok(None);
        }
        if !dev && key.len() < MIN_KEY_LEN {
            return Err(format!("ADMIN_KEY shorter than {MIN_KEY_LEN} characters"));
        }
        let k = hmac::Key::generate(hmac::HMAC_SHA256, &ring::rand::SystemRandom::new())
            .map_err(|_| "CSPRNG".to_string())?;
        let tag = hmac::sign(&k, key.as_bytes());
        Ok(Some(Admin {
            k,
            tag,
            fails: Limiter::new(MAX_FAILS, WINDOW),
        }))
    }

    /// Temps constant (indépendant de la longueur) ; après `MAX_FAILS` échecs d'une même adresse
    /// `who` dans `WINDOW`, `AdminLocked` pour elle seule (même bonne clé) jusqu'à la fin de la fenêtre.
    pub fn check(&self, given: Option<&str>, who: &str, now: Instant) -> Result<(), Code> {
        if self.fails.blocked(who, now) {
            return Err(Code::AdminLocked);
        }
        if hmac::verify(&self.k, given.unwrap_or("").as_bytes(), self.tag.as_ref()).is_ok() {
            return Ok(());
        }
        self.fails.allow(who, now);
        Err(Code::AdminDenied)
    }
}

/// Jours depuis le 1970-01-01 → `AAAA-MM-JJ`.
pub fn day_str(days: i64) -> String {
    let d =
        crate::share::amz_date(std::time::UNIX_EPOCH + Duration::from_secs(days as u64 * 86_400));
    format!("{}-{}-{}", &d[..4], &d[4..6], &d[6..8])
}

/// `AAAA-MM-JJ` → jours depuis le 1970-01-01 (date réelle, années 2000–2100).
pub fn parse_day(s: &str) -> Option<i64> {
    let p: Vec<i64> = s
        .split('-')
        .map(|x| x.parse().ok())
        .collect::<Option<_>>()?;
    let [y, m, d]: [i64; 3] = p.as_slice().try_into().ok()?;
    if s.len() != 10
        || !(2000..=2100).contains(&y)
        || !(1..=12).contains(&m)
        || !(1..=31).contains(&d)
    {
        return None;
    }
    // H. Hinnant, days_from_civil ; aller-retour pour refuser le 30 février
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    (day_str(days) == s).then_some(days)
}

/// `from`/`to` (jours UTC inclus) ; défaut : les 30 derniers jours jusqu'à `today`.
pub fn parse_range(query: &[(String, String)], today: i64) -> Result<(i64, i64), Msg> {
    let bad = |d: &str| Msg::error(Code::InvalidRequest, d.to_string());
    let get = |k: &str| -> Result<Option<i64>, Msg> {
        match query
            .iter()
            .filter(|x| x.0 == k)
            .collect::<Vec<_>>()
            .as_slice()
        {
            [] => Ok(None),
            [x] => parse_day(&x.1)
                .map(Some)
                .ok_or_else(|| bad("date: YYYY-MM-DD expected")),
            _ => Err(bad("repeated parameter")),
        }
    };
    let (from, to) = (get("from")?, get("to")?);
    if let Some((k, _)) = query.iter().find(|x| x.0 != "from" && x.0 != "to") {
        return Err(bad(&format!("unknown parameter {k}")));
    }
    let to = to.unwrap_or(today);
    let from = from.unwrap_or(to - 29);
    if from > to || to - from > MAX_DAYS {
        return Err(bad("from <= to, at most 400 days"));
    }
    Ok((from, to))
}

/// Lignes `plan` des calculs publics (v2 : `phase` ; v1, avant D47 : sans `phase` ni `diagnose`).
const PLAN: &str = r#"msg = "plan" and internal = 0 and accepted = 1 and (phase = "plan" or (not ispresent(phase) and not ispresent(diagnose)))"#;

/// Les 11 vues (admin.md § 4, D59), tolérantes aux lignes `plan` v1 (pas de `phase`, `length_m`,
/// `dplus_res_m`) : `calcs` regroupe par `accepted` et non `phase` (même partition).
pub fn queries() -> [(&'static str, String); 11] {
    [
        ("visitors", r#"filter msg = "hit" | stats count_distinct(visitor) as visitors, count(*) as hits by bin(1d) as day | sort day asc"#.into()),
        ("geo", r#"filter msg = "hit" | stats count_distinct(visitor) as visitors, count(*) as hits by country, region, dev | sort hits desc | limit 1000"#.into()),
        ("refs", r#"filter msg = "hit" and ispresent(ref) | stats count(*) as hits by ref | sort hits desc | limit 50"#.into()),
        ("calcs", r#"filter msg = "plan" and internal = 0 and (phase in ["plan", "rejected"] or (not ispresent(phase) and not ispresent(diagnose))) | stats count(*) as n by bin(1d) as day, accepted, code | sort day asc"#.into()),
        ("perf_day", format!("filter {PLAN} and status = 200 | stats pct(compute_s, 50) as p50, pct(compute_s, 95) as p95, count(*) as n by bin(1d) as day | sort day asc")),
        ("perf", format!("filter {PLAN} and status = 200 | stats pct(compute_s, 50) as p50, pct(compute_s, 95) as p95, count(*) as n")),
        ("mix", format!("filter {PLAN} | stats count(*) as n by goal, surface, climbs")),
        // aussi par type, voie et nombre de sorties rendues : dénominateurs des taux d'action (D59)
        ("hist", format!("filter {PLAN} and status = 200 | fields goal, surface, n_got, floor(coalesce(got_km, length_m / 1000) / 5) * 5 as km_bin, floor(coalesce(got_dplus_m, dplus_res_m) / 250) * 250 as dplus_bin | stats count(*) as n by goal, surface, n_got, km_bin, dplus_bin")),
        ("events_day", r#"filter msg = "event" | stats count(*) as n by bin(1d) as day, event | sort day asc"#.into()),
        ("events_mix", r#"filter msg = "event" and event in ["share_created", "gpx"] | fields event, goal, surface, rank, floor(got_km / 5) * 5 as km_bin, floor(got_dplus_m / 250) * 250 as dplus_bin | stats count(*) as n by event, goal, surface, rank, km_bin, dplus_bin | sort n desc | limit 1000"#.into()),
        ("starts", r#"filter msg = "plan" and ispresent(start_lat) and (phase = "plan" or code = "outside_coverage") | stats count(*) as n by start_lat, start_lon, code | sort n desc | limit 10000"#.into()),
    ]
}

/// Ligne de résultat Insights : champ → valeur (chaînes).
pub type Row = BTreeMap<String, String>;
/// Vues par nom (`None` : non terminée) et Mo analysés.
pub type Views = (BTreeMap<&'static str, Option<Vec<Row>>>, f64);

fn s<'a>(r: &'a Row, k: &str) -> &'a str {
    r.get(k).map_or("-", String::as_str)
}

fn n(r: &Row, k: &str) -> f64 {
    r.get(k).and_then(|v| v.parse().ok()).unwrap_or(0.0)
}

/// Somme du champ `val` par clé, triée par valeur décroissante : `[{<name>: clé, <val>: somme}]`.
fn sum_by(rows: &[Row], key: impl Fn(&Row) -> String, val: &str, name: &str) -> Value {
    let mut m: BTreeMap<String, f64> = BTreeMap::new();
    for r in rows {
        *m.entry(key(r)).or_default() += n(r, val);
    }
    let mut v: Vec<_> = m.into_iter().collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1));
    v.into_iter()
        .map(|(k, x)| json!({name: k, val: x}))
        .collect()
}

/// Taux d'action par clé : actions (`ev`, rangées par `key`) rapportées aux calculs réussis
/// (`base` : `calcs(ligne, clé)` = effectif de la ligne si elle compte pour cette clé).
/// `[{key, calcs, share, gpx, share_rate, gpx_rate}]`, clés numériques triées, sinon par calculs.
fn rates(
    ev: &[Row],
    base: &[Row],
    key: impl Fn(&Row) -> String,
    calcs: impl Fn(&Row, &str) -> Option<f64>,
) -> Value {
    let mut keys: BTreeMap<String, [f64; 2]> = BTreeMap::new();
    for r in ev {
        keys.entry(key(r)).or_default()[usize::from(s(r, "event") == "gpx")] += n(r, "n");
    }
    for r in base {
        keys.entry(key(r)).or_default();
    }
    let mut out: Vec<(String, f64, [f64; 2])> = keys
        .into_iter()
        .map(|(k, a)| {
            let c: f64 = base.iter().filter_map(|r| calcs(r, &k)).sum();
            (k, c, a)
        })
        .filter(|(_, c, a)| *c > 0.0 || a[0] + a[1] > 0.0)
        .collect();
    out.sort_by(|a, b| match (a.0.parse::<f64>(), b.0.parse::<f64>()) {
        (Ok(x), Ok(y)) => x.total_cmp(&y),
        _ => b.1.total_cmp(&a.1),
    });
    let rate = |a: f64, c: f64| (c > 0.0).then(|| (a / c * 1000.0).round() / 1000.0);
    out.into_iter()
        .map(|(k, c, [sh, gpx])| {
            json!({"key": k, "calcs": c, "share": sh, "gpx": gpx,
                   "share_rate": rate(sh, c), "gpx_rate": rate(gpx, c)})
        })
        .collect()
}

fn hist(rows: &[Row], key: &str, step: f64) -> Value {
    let mut m: BTreeMap<i64, f64> = BTreeMap::new();
    for r in rows.iter().filter(|r| r.contains_key(key)) {
        *m.entry(n(r, key) as i64).or_default() += n(r, "n");
    }
    json!({"step": step, "bins": m.into_iter().map(|(f, x)| json!({"from": f, "n": x})).collect::<Vec<_>>()})
}

/// Mise en forme pure des vues (`None` : vue non terminée → `null`, `incomplete`).
pub fn shape(
    from: i64,
    to: i64,
    views: &BTreeMap<&str, Option<Vec<Row>>>,
    scanned_mb: f64,
) -> Value {
    let v = |k: &str| views.get(k).cloned().flatten();
    let day = |r: &Row| s(r, "day").chars().take(10).collect::<String>();
    let num = |x: f64| json!(x);
    let mut o = Map::new();
    o.insert("from".into(), json!(day_str(from)));
    o.insert("to".into(), json!(day_str(to)));
    o.insert(
        "incomplete".into(),
        json!(views.values().any(Option::is_none)),
    );
    o.insert(
        "scanned_mb".into(),
        json!((scanned_mb * 10.0).round() / 10.0),
    );
    let mut put = |k: &str, x: Option<Value>| {
        o.insert(k.into(), x.unwrap_or(Value::Null));
    };
    put(
        "visitors_by_day",
        v("visitors").map(|rows| {
            rows.iter()
                .map(|r| json!({"day": day(r), "visitors": n(r, "visitors"), "hits": n(r, "hits")}))
                .collect()
        }),
    );
    // calculs : `accepted` = 0 → rejetés (avant Turnstile ou jeton refusé), sinon calculs ;
    // `code` absent (v1) ou `ok` en succès
    let calcs = v("calcs");
    let ok = |r: &Row| matches!(s(r, "code"), "-" | "ok");
    let rejected = |r: &Row| s(r, "accepted") == "0";
    put(
        "calcs_by_day",
        calcs.as_ref().map(|rows| {
            let mut m: BTreeMap<String, [f64; 3]> = BTreeMap::new();
            for r in rows {
                let e = m.entry(day(r)).or_default();
                if rejected(r) {
                    e[2] += n(r, "n");
                } else {
                    e[0] += n(r, "n");
                    if ok(r) {
                        e[1] += n(r, "n");
                    }
                }
            }
            m.into_iter()
                .map(|(d, [n, ok, rej])| json!({"day": d, "n": n, "ok": ok, "rejected": rej}))
                .collect()
        }),
    );
    put(
        "codes",
        calcs.as_ref().map(|rows| {
            let plan: Vec<Row> = rows.iter().filter(|r| !rejected(r)).cloned().collect();
            let total: f64 = plan.iter().map(|r| n(r, "n")).sum();
            let fails: Vec<Row> = plan.into_iter().filter(|r| !ok(r)).collect();
            let mut c = sum_by(&fails, |r| s(r, "code").into(), "n", "code");
            for x in c.as_array_mut().into_iter().flatten() {
                x["rate"] =
                    json!((x["n"].as_f64().unwrap_or(0.0) / total * 1000.0).round() / 1000.0);
            }
            c
        }),
    );
    put(
        "rejected_codes",
        calcs.as_ref().map(|rows| {
            let rej: Vec<Row> = rows.iter().filter(|r| rejected(r)).cloned().collect();
            sum_by(&rej, |r| s(r, "code").into(), "n", "code")
        }),
    );
    let perf = v("perf").map(|rows| rows.first().cloned().unwrap_or_default());
    put(
        "compute_s",
        perf.map(|p| {
            let opt = |k: &str| p.get(k).and_then(|x| x.parse::<f64>().ok()).map(num);
            json!({"p50": opt("p50"), "p95": opt("p95"), "n": n(&p, "n"),
               "by_day": v("perf_day").map(|rows| rows.iter().map(|r| json!({
                   "day": day(r), "p50": n(r, "p50"), "p95": n(r, "p95"), "n": n(r, "n")
               })).collect::<Vec<_>>())})
        }),
    );
    let mix = v("mix");
    for (out, k) in [
        ("goals", "goal"),
        ("surfaces", "surface"),
        ("climbs", "climbs"),
    ] {
        put(
            out,
            mix.as_ref()
                .map(|rows| sum_by(rows, |r| s(r, k).into(), "n", k)),
        );
    }
    let h = v("hist");
    put("hist_km", h.as_ref().map(|rows| hist(rows, "km_bin", 5.0)));
    put(
        "hist_dplus_m",
        h.as_ref().map(|rows| hist(rows, "dplus_bin", 250.0)),
    );
    // actions (D59) : volumes par jour, taux d'action par réglage et par stats de sortie
    put(
        "events_by_day",
        v("events_day").map(|rows| {
            let mut m: BTreeMap<String, Map<String, Value>> = BTreeMap::new();
            for r in &rows {
                let e = m.entry(day(r)).or_default();
                let x = e.get(s(r, "event")).and_then(Value::as_f64).unwrap_or(0.0);
                e.insert(s(r, "event").into(), json!(x + n(r, "n")));
            }
            m.into_iter()
                .map(|(d, mut e)| {
                    e.insert("day".into(), json!(d));
                    Value::Object(e)
                })
                .collect()
        }),
    );
    let ev = v("events_mix");
    put(
        "action_rates",
        ev.as_ref().zip(h.as_ref()).map(|(ev, base)| {
            let mut o = Map::new();
            for (out, k) in [
                ("goal", "goal"),
                ("surface", "surface"),
                ("km", "km_bin"),
                ("dplus_m", "dplus_bin"),
            ] {
                o.insert(
                    out.into(),
                    rates(
                        ev,
                        base,
                        |r| s(r, k).to_string(),
                        |r, key| (s(r, k) == key).then(|| n(r, "n")),
                    ),
                );
            }
            // rang r proposé dans un calcul qui a rendu au moins r sorties (n_got absent : 1)
            o.insert(
                "rank".into(),
                rates(
                    ev,
                    base,
                    |r| s(r, "rank").to_string(),
                    |r, key| {
                        let got = r
                            .get("n_got")
                            .and_then(|x| x.parse::<f64>().ok())
                            .unwrap_or(1.0);
                        key.parse::<f64>()
                            .is_ok_and(|k| got >= k)
                            .then(|| n(r, "n"))
                    },
                ),
            );
            Value::Object(o)
        }),
    );
    put("top_combos", ev.as_ref().map(|rows| {
        let mut m: BTreeMap<[String; 5], [f64; 2]> = BTreeMap::new();
        for r in rows {
            let k = ["goal", "surface", "km_bin", "dplus_bin", "rank"].map(|k| s(r, k).to_string());
            m.entry(k).or_default()[usize::from(s(r, "event") == "gpx")] += n(r, "n");
        }
        let mut v: Vec<_> = m.into_iter().collect();
        v.sort_by(|a, b| (b.1[0] + b.1[1]).total_cmp(&(a.1[0] + a.1[1])));
        v.into_iter()
            .take(20)
            .map(|([g, su, km, dp, rk], [sh, gpx])| {
                json!({"goal": g, "surface": su, "km_bin": km, "dplus_bin": dp, "rank": rk, "share": sh, "gpx": gpx})
            })
            .collect()
    }));
    let starts = v("starts");
    for (out, outside) in [("starts", false), ("starts_outside", true)] {
        put(out, starts.as_ref().map(|rows| {
            let sel: Vec<Row> = rows
                .iter()
                .filter(|r| (s(r, "code") == "outside_coverage") == outside)
                .cloned()
                .collect();
            let cells = sum_by(&sel, |r| format!("{} {}", s(r, "start_lat"), s(r, "start_lon")), "n", "cell");
            cells
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| {
                    let (la, lo) = c["cell"].as_str()?.split_once(' ')?;
                    Some(json!({"lat": la.parse::<f64>().ok()?, "lon": lo.parse::<f64>().ok()?, "n": c["n"]}))
                })
                .collect()
        }));
    }
    // visiteurs-jours et visites par pays, par région (clé « pays région ») ; visiteurs par appareil
    let geo = v("geo");
    let pair = |rows: &[Row], key: &dyn Fn(&Row) -> String| -> Vec<(String, Value, Value)> {
        let hits = sum_by(rows, key, "hits", "k");
        let hits: BTreeMap<&str, &Value> = hits
            .as_array()
            .into_iter()
            .flatten()
            .map(|x| (x["k"].as_str().unwrap_or(""), &x["hits"]))
            .collect();
        let vis = sum_by(rows, key, "visitors", "k");
        vis.as_array()
            .into_iter()
            .flatten()
            .map(|x| {
                let k = x["k"].as_str().unwrap_or("").to_string();
                let h = hits.get(k.as_str()).map_or(json!(0), |h| (*h).clone());
                (k, x["visitors"].clone(), h)
            })
            .collect()
    };
    put(
        "countries",
        geo.as_ref().map(|rows| {
            pair(rows, &|r| s(r, "country").into())
                .into_iter()
                .map(|(c, v, h)| json!({"country": c, "visitors": v, "hits": h}))
                .collect()
        }),
    );
    put(
        "regions",
        geo.as_ref().map(|rows| {
            pair(rows, &|r| format!("{} {}", s(r, "country"), s(r, "region")))
                .into_iter()
                .map(|(k, v, h)| {
                    let (c, reg) = k.split_once(' ').unwrap_or(("-", "-"));
                    json!({"country": c, "region": reg, "visitors": v, "hits": h})
                })
                .collect()
        }),
    );
    put(
        "devices",
        geo.as_ref()
            .map(|r| sum_by(r, |r| s(r, "dev").into(), "visitors", "dev")),
    );
    put(
        "referrers",
        v("refs").map(|r| sum_by(&r, |r| s(r, "ref").into(), "hits", "ref")),
    );
    Value::Object(o)
}

/// Lance les 11 requêtes puis lit leurs résultats jusqu'à `Complete` ou `deadline`. Appels par
/// lots concurrents de `batch`, `pace` entre deux lots (quotas Logs : 5 appels/s par compte).
/// Une erreur de `StartQuery` (droits, identifiants) fait échouer l'ensemble.
pub fn run(
    from: i64,
    to: i64,
    group: &str,
    call: impl Fn(&str, &Value) -> Result<Value, String> + Sync,
    pace: (usize, Duration),
    deadline: Duration,
) -> Result<Views, String> {
    run_qs(&queries(), from, to, group, call, pace, deadline)
}

/// `run` pour les requêtes `qs` (nom, texte).
pub fn run_qs(
    qs: &[(&'static str, String)],
    from: i64,
    to: i64,
    group: &str,
    call: impl Fn(&str, &Value) -> Result<Value, String> + Sync,
    (batch, pace): (usize, Duration),
    deadline: Duration,
) -> Result<Views, String> {
    let t0 = Instant::now();
    let calls = |op: &str, reqs: Vec<Value>| -> Vec<Result<Value, String>> {
        let mut out = Vec::new();
        for chunk in reqs.chunks(batch.max(1)) {
            std::thread::scope(|s| {
                let hs: Vec<_> = chunk.iter().map(|b| s.spawn(|| call(op, b))).collect();
                out.extend(
                    hs.into_iter()
                        .map(|h| h.join().unwrap_or(Err("panic".into()))),
                );
            });
            std::thread::sleep(pace);
        }
        out
    };
    let starts = qs.iter().map(|(_, q)| {
        json!({"logGroupName": group, "startTime": from * 86_400,
               "endTime": to * 86_400 + 86_399, "queryString": q, "limit": 10_000})
    });
    let mut pending = Vec::new();
    for ((name, _), r) in qs.iter().zip(calls("StartQuery", starts.collect())) {
        let id = r?["queryId"]
            .as_str()
            .ok_or("StartQuery: no queryId")?
            .to_string();
        pending.push((*name, id));
    }
    let (mut views, mut bytes) = (BTreeMap::new(), 0.0);
    while !pending.is_empty() && t0.elapsed() < deadline {
        let ids = pending.iter().map(|p| json!({"queryId": p.1})).collect();
        let mut still = Vec::new();
        for ((name, id), r) in pending.into_iter().zip(calls("GetQueryResults", ids)) {
            // erreur passagère (quota) : nouvel essai au tour suivant
            let r = r.unwrap_or_default();
            match r["status"].as_str() {
                Some("Complete") => {
                    bytes += r["statistics"]["bytesScanned"].as_f64().unwrap_or(0.0);
                    let rows = r["results"].as_array().into_iter().flatten().map(|row| {
                        row.as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|c| {
                                Some((c["field"].as_str()?.into(), c["value"].as_str()?.into()))
                            })
                            .collect::<Row>()
                    });
                    views.insert(name, Some(rows.collect()));
                }
                Some("Failed" | "Cancelled" | "Timeout") => {
                    views.insert(name, None);
                }
                _ => still.push((name, id)),
            }
        }
        pending = still;
    }
    for (name, _) in pending {
        views.insert(name, None);
    }
    Ok((views, bytes / 1e6))
}

/// Requête de l'API CloudWatch Logs (JSON 1.1) signée SigV4 avec les identifiants du rôle Lambda.
pub fn logs_api(region: &str, op: &str, body: &Value) -> Result<Value, String> {
    aws_api("logs", region, op, body)
}

/// Appel en lecture d'une API AWS JSON (`logs`, `cloudwatch`, `budgets`, `freetier` : ces deux
/// derniers en `us-east-1`) : AWS CLI du profil en build de développement avec `AWS_PROFILE`,
/// sinon API signée avec les identifiants du rôle Lambda.
pub fn aws(service: &str, region: &str, op: &str, body: &Value) -> Result<Value, String> {
    if cfg!(debug_assertions) && std::env::var_os("AWS_PROFILE").is_some() {
        aws_cli(service, region, op, body)
    } else {
        aws_api(service, region, op, body)
    }
}

/// API AWS JSON signée SigV4 (sans SDK).
pub fn aws_api(service: &str, region: &str, op: &str, body: &Value) -> Result<Value, String> {
    use crate::share::{Creds, amz_date, hex, sigv4};
    let (sign, host, target, ver) = match service {
        "logs" => (
            "logs",
            format!("logs.{region}.amazonaws.com"),
            "Logs_20140328",
            "1.1",
        ),
        "cloudwatch" => (
            "monitoring",
            format!("monitoring.{region}.amazonaws.com"),
            "GraniteServiceVersion20100801",
            "1.0",
        ),
        "budgets" => (
            "budgets",
            "budgets.amazonaws.com".into(),
            "AWSBudgetServiceGateway",
            "1.1",
        ),
        "freetier" => (
            "freetier",
            format!("freetier.{region}.api.aws"),
            "AWSFreeTierService",
            "1.0",
        ),
        _ => return Err(format!("unknown service {service}")),
    };
    let creds = Creds::from_env()?;
    let body = body.to_string();
    let date = amz_date(std::time::SystemTime::now());
    let hash = hex(ring::digest::digest(&ring::digest::SHA256, body.as_bytes()).as_ref());
    let mut headers = vec![
        ("content-type", format!("application/x-amz-json-{ver}")),
        ("host", host.clone()),
        ("x-amz-content-sha256", hash),
        ("x-amz-date", date.clone()),
        ("x-amz-target", format!("{target}.{op}")),
    ];
    if let Some(t) = &creds.token {
        headers.push(("x-amz-security-token", t.clone()));
    }
    headers.sort();
    let auth = sigv4(
        "POST",
        "/",
        &headers,
        &date,
        region,
        sign,
        &creds.key_id,
        &creds.secret,
    );
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(5)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut req = ureq::http::Request::post(format!("https://{host}/"));
    for (k, v) in headers.iter().filter(|h| h.0 != "host") {
        req = req.header(*k, v);
    }
    let req = req
        .header("authorization", auth)
        .body(body.into_bytes())
        .map_err(|e| e.to_string())?;
    let mut resp = agent.run(req).map_err(|e| format!("{service}: {e}"))?;
    let status = resp.status().as_u16();
    let v: Value = resp
        .body_mut()
        .with_config()
        .limit(16 << 20)
        .read_json()
        .map_err(|e| format!("{service}: {e}"))?;
    match status {
        200 => Ok(v),
        _ if v["__type"]
            .as_str()
            .is_some_and(|t| t.contains("ExpiredToken")) =>
        {
            Err(EXPIRED.into())
        }
        _ => Err(format!("{service} {op}: HTTP {status} {}", v["__type"])),
    }
}

/// Transport local (build de développement) : AWS CLI avec le profil `AWS_PROFILE`, sans
/// manipuler d'identifiants ; session expirée → `EXPIRED`.
pub fn logs_cli(region: &str, op: &str, body: &Value) -> Result<Value, String> {
    aws_cli("logs", region, op, body)
}

/// AWS CLI (`StartQuery` → `aws logs start-query`), même contrat que `aws_api`.
pub fn aws_cli(service: &str, region: &str, op: &str, body: &Value) -> Result<Value, String> {
    let mut sub = String::new();
    for (i, c) in op.chars().enumerate() {
        if i > 0 && c.is_ascii_uppercase() {
            sub.push('-');
        }
        sub.push(c.to_ascii_lowercase());
    }
    let out = std::process::Command::new("aws")
        .args([
            service,
            &sub,
            "--region",
            region,
            "--output",
            "json",
            "--cli-input-json",
        ])
        .arg(body.to_string())
        .output()
        .map_err(|e| format!("aws cli: {e}"))?;
    if !out.status.success() {
        let e = String::from_utf8_lossy(&out.stderr).to_ascii_lowercase();
        let expired = ["expired", "aws login", "sso", "credentials"]
            .iter()
            .any(|w| e.contains(w));
        return Err(if expired {
            EXPIRED.into()
        } else {
            format!("aws cli: {}", e.trim())
        });
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("aws cli: {e}"))
}

/// `GET /api/admin/stats` : clé (`x-admin-key`), période, requêtes, mise en forme. La réponse
/// d'erreur garde `detail` (appelant authentifié : « identifiants AWS expirés » en local).
pub fn handle(
    admin: &Admin,
    given: Option<&str>,
    who: &str,
    query: &[(String, String)],
    today: i64,
    run_views: impl FnOnce(i64, i64) -> Result<Views, String>,
) -> Reply {
    let t0 = Instant::now();
    if let Err(r) = gate(admin, given, who, t0) {
        return r;
    }
    let (from, to) = match parse_range(query, today) {
        Ok(r) => r,
        Err(m) => return fail(m, "error", None, t0),
    };
    match run_views(from, to) {
        Ok((views, mb)) => Reply {
            status: 200,
            body: shape(from, to, &views, mb),
            log: log("ok", Some((from, to)), Some(mb), t0),
        },
        Err(e) => fail(Msg::error(Code::Busy, e), "error", Some((from, to)), t0),
    }
}

/// Réponse d'erreur admin (`detail` gardé : appelant authentifié).
fn fail(m: Msg, outcome: &str, range: Option<(i64, i64)>, t0: Instant) -> Reply {
    let status = http_status(m.code);
    let body = if status == 400 || m.code == Code::Busy {
        json!({"error": m})
    } else {
        error_body(&m, status)
    };
    Reply {
        status,
        body,
        log: log(outcome, range, None, t0),
    }
}

/// Clé admin (temps constant, essais limités) : `Err(401 | 429)` si refusée.
fn gate(admin: &Admin, given: Option<&str>, who: &str, t0: Instant) -> Result<(), Reply> {
    admin.check(given, who, Instant::now()).map_err(|c| {
        let outcome = if c == Code::AdminLocked {
            "locked"
        } else {
            "denied"
        };
        fail(Msg::error(c, "admin key"), outcome, None, t0)
    })
}

/// Ligne `{"msg":"admin",…}` (jamais la clé).
fn log(outcome: &str, range: Option<(i64, i64)>, mb: Option<f64>, t0: Instant) -> Value {
    let mut l =
        json!({"msg": "admin", "outcome": outcome, "compute_s": t0.elapsed().as_secs_f64()});
    if let Some((f, t)) = range {
        l["from"] = json!(day_str(f));
        l["to"] = json!(day_str(t));
    }
    if let Some(mb) = mb {
        l["scanned_mb"] = json!((mb * 10.0).round() / 10.0);
    }
    l
}

// ---- Consommation AWS du mois (GET /api/admin/usage) ----

/// Postes suivis : (clé, service, poste, unité, seuil gratuit par mois, prix $ par unité au-delà,
/// jauge). Prix eu-north-1 (CloudFront : Europe) lus sur l'API Pricing le 2026-10-08 ; seuils =
/// offre « toujours gratuite » d'AWS (compte au plan payant ouvert après juillet 2025 : pas d'offre
/// de 12 mois, S3 payé dès le premier octet ; `freetier:GetFreeTierUsage` y répond une liste vide).
/// Jauge (stockage, Go-mois) : mesurée à l'instant ; consommé = taille × part du mois écoulée.
/// ponytail: les 5 Go gratuits de CloudWatch Logs sont communs aux 3 postes Logs, comptés ici par
/// poste (écart nul tant que leur somme reste sous 5 Go).
pub const POSTES: [(&str, &str, &str, &str, f64, f64, bool); 8] = [
    (
        "lambda_req",
        "Lambda",
        "Requêtes",
        "requêtes",
        1e6,
        2e-7,
        false,
    ),
    (
        "lambda_gbs",
        "Lambda",
        "Calcul facturé (démarrages compris)",
        "Go·s",
        4e5,
        0.000_013_333_4,
        false,
    ),
    (
        "cf_req",
        "CloudFront",
        "Requêtes HTTPS",
        "requêtes",
        1e7,
        1.2e-6,
        false,
    ),
    (
        "cf_gb",
        "CloudFront",
        "Données sortantes",
        "Go",
        1024.0,
        0.085,
        false,
    ),
    (
        "logs_in",
        "CloudWatch Logs",
        "Ingestion des journaux",
        "Go",
        5.0,
        0.54,
        false,
    ),
    (
        "logs_scan",
        "CloudWatch Logs",
        "Analyse Insights (page admin)",
        "Go",
        5.0,
        0.0054,
        false,
    ),
    (
        "logs_gb",
        "CloudWatch Logs",
        "Stockage des journaux",
        "Go-mois",
        5.0,
        0.028,
        true,
    ),
    (
        "s3_gb",
        "S3",
        "Stockage (dalles, site, état Tofu)",
        "Go-mois",
        0.0,
        0.023,
        true,
    ),
];
/// Paliers du budget mensuel (infra/variables.tf `budget_usd`) : chacun met l'API en pause.
pub const BUDGET_STEPS: [f64; 5] = [1.0, 5.0, 10.0, 20.0, 50.0];
/// Durée du cache de `/api/admin/usage` (par instance).
pub const USAGE_TTL: Duration = Duration::from_secs(3 * 3600);
static USAGE: Mutex<Option<(Instant, Value)>> = Mutex::new(None);

/// Premier jour (jours depuis 1970) et nombre de jours du mois du jour `day`.
pub fn month_of(day: i64) -> (i64, i64) {
    let s = day_str(day);
    let first = parse_day(&format!("{}-01", &s[..7])).unwrap_or(day);
    let (y, m): (i64, i64) = (s[..4].parse().unwrap_or(2000), s[5..7].parse().unwrap_or(1));
    let next = if m == 12 {
        format!("{}-01-01", y + 1)
    } else {
        format!("{y}-{:02}-01", m + 1)
    };
    (first, parse_day(&next).unwrap_or(first + 31) - first)
}

/// Mise en forme pure : mesures (`m` : clé de `POSTES` → valeur ; absente si la source a échoué),
/// réponses `GetAccountPlanState` (`plan`) et `DescribeBudgets` (`budgets`), instant `now` (s).
/// Projection « au rythme actuel » : consommé ÷ part du mois écoulée (jauge : taille actuelle).
pub fn usage_shape(
    m: &BTreeMap<&str, f64>,
    plan: &Value,
    budgets: &Value,
    now: u64,
    errors: &[String],
) -> Value {
    let (first, ndays) = month_of(now as i64 / 86_400);
    let month_s = (ndays * 86_400) as f64;
    // au moins une heure écoulée : pas de projection infinie le 1er à minuit
    let frac = ((now as f64 - (first * 86_400) as f64) / month_s).clamp(3600.0 / month_s, 1.0);
    let r = |x: f64| (x * 1e6).round() / 1e6;
    let cost = |used: f64, free: f64, price: f64| r((used - free).max(0.0) * price);
    let mut known = 0.0;
    let mut lines: Vec<Value> = POSTES
        .iter()
        .map(|&(k, service, item, unit, free, price, gauge)| {
            let Some(&x) = m.get(k) else {
                return json!({"key": k, "service": service, "item": item, "unit": unit, "free": free, "price": price,
                              "used": null, "projected": null, "share": null, "cost": null, "cost_forecast": null});
            };
            let (used, projected) = if gauge { (x * frac, x) } else { (x, x / frac) };
            let c = cost(used, free, price);
            known += c;
            json!({"key": k, "service": service, "item": item, "unit": unit, "free": free, "price": price,
                   "now": gauge.then_some(r(x)), "used": r(used), "projected": r(projected),
                   "share": (free > 0.0).then(|| r(used / free)), "projected_share": (free > 0.0).then(|| r(projected / free)),
                   "cost": c, "cost_forecast": cost(projected, free, price)})
        })
        .collect();
    let b = budgets["Budgets"]
        .as_array()
        .and_then(|v| v.iter().find(|b| b["TimeUnit"] == "MONTHLY"));
    let amount = |b: &Value, k: &str| {
        let a = &b["CalculatedSpend"][k]["Amount"];
        a.as_str()
            .and_then(|x| x.parse::<f64>().ok())
            .or(a.as_f64())
    };
    let actual = b.and_then(|b| amount(b, "ActualSpend"));
    let forecast = b.and_then(|b| amount(b, "ForecastedSpend"));
    // reste non ventilé (requêtes S3, autres services, taxes) : dépense réelle − postes estimés
    if let Some(a) = actual {
        let rest = r((a - known).max(0.0));
        lines.push(json!({"key": "other", "service": "Autres", "item": "Non ventilé : requêtes S3, autres services, taxes",
                          "unit": null, "used": null, "projected": null, "share": null,
                          "cost": rest, "cost_forecast": r(rest / frac)}));
    }
    let spend = actual.map(|a| {
        let (f, src) = forecast.map_or((r(a / frac), "rythme"), |f| (f, "budgets"));
        json!({"actual": a, "forecast": f, "forecast_source": src,
               "limit": b.and_then(|b| b["BudgetLimit"]["Amount"].as_str().and_then(|x| x.parse::<f64>().ok())),
               "steps": BUDGET_STEPS, "next_step": BUDGET_STEPS.iter().find(|&&s| s > a)})
    });
    let credits = &plan["accountPlanRemainingCredits"]["amount"];
    json!({
        "month": &day_str(first)[..7], "renewal": day_str(first + ndays), "elapsed": r(frac), "fetched_at": now,
        "plan": plan.get("accountPlanType").map(|t| json!({"type": t, "status": plan["accountPlanStatus"], "credits_usd": credits})),
        "spend": spend, "lines": lines, "errors": errors,
    })
}

/// Lit les sources (toutes gratuites, en lecture) et met en forme ; Insights : lignes REPORT de la
/// Lambda (durée facturée, démarrages compris) et lignes `admin` (Mo analysés par la page).
/// `call(service, région, opération, corps)`. Identifiants expirés → `Err(EXPIRED)`.
pub fn usage(
    group: &str,
    region: &str,
    now: u64,
    call: impl Fn(&str, &str, &str, &Value) -> Result<Value, String> + Sync,
) -> Result<(Value, f64), String> {
    let (first, _) = month_of(now as i64 / 86_400);
    let start = first * 86_400;
    let mut m: BTreeMap<&str, f64> = BTreeMap::new();
    let mut errors = Vec::new();
    let mut put = |k: &'static str, v: Result<f64, String>| match v {
        Ok(x) => {
            m.insert(k, x);
        }
        Err(e) => errors.push(format!("{k}: {e}")),
    };
    let q = [("usage", r#"filter @type = "REPORT" or (msg = "admin" and ispresent(scanned_mb)) | stats count(@billedDuration) as n, sum(@billedDuration * @memorySize) as ms_b, sum(scanned_mb) as mb"#.to_string())];
    let views = run_qs(
        &q,
        first,
        now as i64 / 86_400,
        group,
        |op, b| call("logs", region, op, b),
        (1, Duration::from_millis(500)),
        DEADLINE,
    );
    let mut scanned = 0.0;
    match views {
        Ok((v, mb)) => {
            scanned = mb;
            match v.get("usage").cloned().flatten() {
                Some(rows) => {
                    let row = rows.first().cloned().unwrap_or_default();
                    put("lambda_req", Ok(n(&row, "n")));
                    // ms × (Mo × 10⁶) → Go·s (1 Go = 1 024 Mo)
                    put("lambda_gbs", Ok(n(&row, "ms_b") / 1.024e12));
                    put("logs_scan", Ok((n(&row, "mb") + mb) / 1024.0));
                }
                None => put("insights", Err("incomplete".into())),
            }
        }
        Err(e) => put("insights", Err(e)),
    }
    // métriques CloudWatch (GetMetricStatistics, ListMetrics : dans le million d'appels gratuits)
    let stat = |reg: &str,
                ns: &str,
                name: &str,
                dims: &Value,
                gauge: bool|
     -> Result<f64, String> {
        let (st, from, period) = if gauge {
            ("Maximum", now as i64 - 3 * 86_400, 3 * 86_400)
        } else {
            ("Sum", start, 86_400)
        };
        let r = call(
            "cloudwatch",
            reg,
            "GetMetricStatistics",
            &json!({"Namespace": ns, "MetricName": name,
            "Dimensions": dims, "StartTime": from, "EndTime": now, "Period": period, "Statistics": [st]}),
        )?;
        let xs = r["Datapoints"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|d| d[st].as_f64());
        Ok(if gauge {
            xs.fold(0.0, f64::max)
        } else {
            xs.sum()
        })
    };
    // toutes les séries d'une métrique (distributions, buckets) : dimensions par ListMetrics
    let all = |reg: &str, ns: &str, name: &str, gauge: bool| -> Result<f64, String> {
        let l = call(
            "cloudwatch",
            reg,
            "ListMetrics",
            &json!({"Namespace": ns, "MetricName": name}),
        )?;
        let mut t = 0.0;
        for x in l["Metrics"].as_array().into_iter().flatten() {
            t += stat(reg, ns, name, &x["Dimensions"], gauge)?;
        }
        Ok(t)
    };
    const GB: f64 = 1_073_741_824.0;
    put(
        "cf_req",
        all("us-east-1", "AWS/CloudFront", "Requests", false),
    );
    put(
        "cf_gb",
        all("us-east-1", "AWS/CloudFront", "BytesDownloaded", false).map(|b| b / GB),
    );
    put(
        "s3_gb",
        all(region, "AWS/S3", "BucketSizeBytes", true).map(|b| b / GB),
    );
    put(
        "logs_in",
        stat(
            region,
            "AWS/Logs",
            "IncomingBytes",
            &json!([{"Name": "LogGroupName", "Value": group}]),
            false,
        )
        .map(|b| b / GB),
    );
    put(
        "logs_gb",
        call(
            "logs",
            region,
            "DescribeLogGroups",
            &json!({"logGroupNamePrefix": "/aws/lambda/optrail-"}),
        )
        .map(|r| {
            r["logGroups"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|g| g["storedBytes"].as_f64())
                .sum::<f64>()
                / GB
        }),
    );
    let mut other = |svc: &str, op: &str, body: Value| {
        call(svc, "us-east-1", op, &body).unwrap_or_else(|e| {
            errors.push(format!("{svc}: {e}"));
            Value::Null
        })
    };
    let plan = other("freetier", "GetAccountPlanState", json!({}));
    let budgets = match plan["accountId"].as_str() {
        Some(id) => other("budgets", "DescribeBudgets", json!({"AccountId": id})),
        None => Value::Null,
    };
    if errors.iter().any(|e| e.ends_with(EXPIRED)) {
        return Err(EXPIRED.into());
    }
    Ok((usage_shape(&m, &plan, &budgets, now, &errors), scanned))
}

/// `GET /api/admin/usage` (aucun paramètre) : clé comme `/api/admin/stats`, cache `USAGE_TTL`
/// (une réponse avec sources en échec n'est pas gardée).
pub fn handle_usage(
    admin: &Admin,
    given: Option<&str>,
    who: &str,
    query: &[(String, String)],
    fetch: impl FnOnce() -> Result<(Value, f64), String>,
) -> Reply {
    let t0 = Instant::now();
    if let Err(r) = gate(admin, given, who, t0) {
        return r;
    }
    if let Some((k, _)) = query.first() {
        let m = Msg::error(Code::InvalidRequest, format!("unknown parameter {k}"));
        return fail(m, "error", None, t0);
    }
    let cached = USAGE.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if let Some((_, v)) = cached.filter(|(t, _)| t.elapsed() < USAGE_TTL) {
        return Reply {
            status: 200,
            body: v,
            log: log("ok", None, None, t0),
        };
    }
    match fetch() {
        Ok((v, mb)) => {
            if v["errors"].as_array().is_some_and(Vec::is_empty) {
                *USAGE.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some((Instant::now(), v.clone()));
            }
            Reply {
                status: 200,
                body: v,
                log: log("ok", None, Some(mb), t0),
            }
        }
        Err(e) => fail(Msg::error(Code::Busy, e), "error", None, t0),
    }
}
