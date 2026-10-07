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
    let starts = queries().map(|(_, q)| {
        json!({"logGroupName": group, "startTime": from * 86_400,
               "endTime": to * 86_400 + 86_399, "queryString": q, "limit": 10_000})
    });
    let mut pending = Vec::new();
    for ((name, _), r) in queries()
        .into_iter()
        .zip(calls("StartQuery", starts.into()))
    {
        let id = r?["queryId"]
            .as_str()
            .ok_or("StartQuery: no queryId")?
            .to_string();
        pending.push((name, id));
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
    use crate::share::{Creds, amz_date, hex, sigv4};
    let creds = Creds::from_env()?;
    let host = format!("logs.{region}.amazonaws.com");
    let body = body.to_string();
    let date = amz_date(std::time::SystemTime::now());
    let hash = hex(ring::digest::digest(&ring::digest::SHA256, body.as_bytes()).as_ref());
    let mut headers = vec![
        ("content-type", "application/x-amz-json-1.1".to_string()),
        ("host", host.clone()),
        ("x-amz-content-sha256", hash),
        ("x-amz-date", date.clone()),
        ("x-amz-target", format!("Logs_20140328.{op}")),
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
        "logs",
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
    let mut resp = agent.run(req).map_err(|e| format!("logs: {e}"))?;
    let status = resp.status().as_u16();
    let v: Value = resp
        .body_mut()
        .with_config()
        .limit(16 << 20)
        .read_json()
        .map_err(|e| format!("logs: {e}"))?;
    match status {
        200 => Ok(v),
        _ if v["__type"]
            .as_str()
            .is_some_and(|t| t.contains("ExpiredToken")) =>
        {
            Err(EXPIRED.into())
        }
        _ => Err(format!("logs {op}: HTTP {status} {}", v["__type"])),
    }
}

/// Transport local (build de développement) : AWS CLI avec le profil `AWS_PROFILE`, sans
/// manipuler d'identifiants ; session expirée → `EXPIRED`.
pub fn logs_cli(region: &str, op: &str, body: &Value) -> Result<Value, String> {
    let sub = if op == "StartQuery" {
        "start-query"
    } else {
        "get-query-results"
    };
    let out = std::process::Command::new("aws")
        .args([
            "logs",
            sub,
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
    let reply = |m: Msg, outcome: &str, range: Option<(i64, i64)>| {
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
    };
    if let Err(c) = admin.check(given, who, Instant::now()) {
        let outcome = if c == Code::AdminLocked {
            "locked"
        } else {
            "denied"
        };
        return reply(Msg::error(c, "admin key"), outcome, None);
    }
    let (from, to) = match parse_range(query, today) {
        Ok(r) => r,
        Err(m) => return reply(m, "error", None),
    };
    match run_views(from, to) {
        Ok((views, mb)) => Reply {
            status: 200,
            body: shape(from, to, &views, mb),
            log: log("ok", Some((from, to)), Some(mb), t0),
        },
        Err(e) => reply(Msg::error(Code::Busy, e), "error", Some((from, to))),
    }
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
