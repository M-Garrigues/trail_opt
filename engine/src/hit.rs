//! `POST /api/hit` (contracts/admin.md § 2, D51) : mesure d'audience sans cookie. Logique pure ;
//! le sel du jour (`share::Store::salt`) et les en-têtes viennent du binaire `lambda`.
use ring::hmac;
use serde_json::{Map, Value, json};

pub const MAX_BODY: usize = 512;
const MAX_UA: usize = 512;
const BOTS: [&str; 11] = [
    "bot",
    "crawl",
    "spider",
    "slurp",
    "headless",
    "lighthouse",
    "curl",
    "wget",
    "python",
    "go-http",
    "java",
];

/// Pays et région du visiteur (en-têtes CloudFront validés) ; clés absentes si invalides.
pub fn geo(country: Option<&str>, region: Option<&str>) -> Map<String, Value> {
    let ok = |s: &str, n: std::ops::RangeInclusive<usize>, digits: bool| {
        n.contains(&s.len())
            && s.bytes()
                .all(|b| b.is_ascii_uppercase() || (digits && b.is_ascii_digit()))
    };
    let mut m = Map::new();
    if let Some(c) = country.filter(|c| ok(c, 2..=2, false)) {
        m.insert("country".into(), json!(c));
    }
    if let Some(r) = region.filter(|r| ok(r, 1..=3, true)) {
        m.insert("region".into(), json!(r));
    }
    m
}

/// Identifiant du jour : HMAC-SHA256(sel du jour, ip ‖ "\n" ‖ ua[..512]), 64 bits en hexadécimal.
pub fn visitor(salt: &[u8], ip: &str, ua: &str) -> String {
    let k = hmac::Key::new(hmac::HMAC_SHA256, salt);
    let ua = &ua.as_bytes()[..ua.len().min(MAX_UA)];
    let tag = hmac::sign(&k, &[ip.as_bytes(), b"\n", ua].concat());
    crate::share::hex(&tag.as_ref()[..8])
}

/// Site d'origine réduit à son domaine (minuscules, sans `www.`), jamais un de nos hôtes.
fn referrer(r: &str, own: &[String]) -> Option<String> {
    let r = r.to_ascii_lowercase();
    let r = r.strip_prefix("www.").unwrap_or(&r);
    let valid = (1..=100).contains(&r.len())
        && r.contains('.')
        && r.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-');
    (valid && !own.iter().any(|h| h.strip_prefix("www.").unwrap_or(h) == r)).then(|| r.into())
}

/// Champs numériques d'un événement (D59) : (nom, min, max, tranche). Valeurs ramenées au bas
/// de leur tranche (5 km, 250 m) ou à l'entier (tranche 1 : temps à la seconde) : jamais de valeur
/// exacte qui rattacherait l'événement à la ligne `plan` du calcul (revue M4).
const NUMS: [(&str, f64, f64, f64); 11] = [
    ("km", 0.0, 300.0, 5.0),
    ("dplus_m", 0.0, 10_000.0, 250.0),
    ("max_grade_pct", 0.0, 60.0, 1.0),
    ("via_n", 0.0, 5.0, 1.0),
    ("rank", 1.0, 4.0, 1.0),
    ("got_km", 0.0, 300.0, 5.0),
    ("got_dplus_m", 0.0, 20_000.0, 250.0),
    ("trail_pct", 0.0, 100.0, 1.0),
    ("mixed_pct", 0.0, 100.0, 1.0),
    ("road_pct", 0.0, 100.0, 1.0),
    ("compute_s", 0.0, 60.0, 1.0),
];
/// Champs texte d'un événement : valeurs permises (liste blanche).
const ENUMS: [(&str, &[&str]); 5] = [
    (
        "event",
        &["share_click", "share_created", "gpx", "shared_open"],
    ),
    ("lang", &["fr", "en"]),
    ("goal", &["max_dplus", "target", "min_distance"]),
    ("surface", &["trail", "any", "road"]),
    ("climbs", &["short", "balanced", "long"]),
];

/// Événement (D59) : réglages de la demande et stats de la sortie, rien d'autre. Liste blanche
/// stricte : champ inconnu, mauvais type, hors bornes ou valeur non permise ⇒ `None` (400).
/// Aucun identifiant de calcul ni de personne, pas même le visiteur du jour (revue M4).
pub fn event(v: &Value) -> Option<Map<String, Value>> {
    let mut out = Map::new();
    for (k, x) in v.as_object()? {
        let val = if let Some((_, lo, hi, step)) = NUMS.iter().find(|n| n.0 == k) {
            let f = x.as_f64().filter(|f| (*lo..=*hi).contains(f))?;
            let v = if *step > 1.0 {
                (f / step).floor() * step
            } else {
                f.round()
            };
            json!(v as i64)
        } else if let Some((_, ok)) = ENUMS.iter().find(|e| e.0 == k) {
            json!(x.as_str().filter(|s| ok.contains(s))?)
        } else if k == "zone" || k == "no_repeat" {
            json!(x.as_bool()?)
        } else {
            return None;
        };
        out.insert(k.clone(), val);
    }
    out.contains_key("event").then_some(out)
}

/// Corps `{"page","ref"?,"lang"?}` (visite) ou `{"event",…}` (action, D59) → (statut, ligne
/// `hit` ou `event` éventuelle). 400 si corps invalide ; 204 sans ligne pour un robot (UA), sans
/// IP, ou sans sel pour une visite (`salt` : le sel du jour, appelé seulement si la visite compte).
pub fn hit(
    body: &[u8],
    ip: Option<&str>,
    ua: &str,
    geo: Map<String, Value>,
    own_hosts: &[String],
    salt: impl FnOnce() -> Option<Vec<u8>>,
) -> (u16, Option<Value>) {
    let Some(v) = (body.len() <= MAX_BODY)
        .then(|| serde_json::from_slice::<Value>(body).ok())
        .flatten()
    else {
        return (400, None);
    };
    // visite : page + site d'origine + pays/région ; événement : champs de la liste blanche seuls
    let fields = if v.get("event").is_some() {
        match event(&v) {
            Some(mut e) => {
                e.insert("msg".into(), json!("event"));
                e
            }
            None => return (400, None),
        }
    } else {
        let page = match v["page"].as_str() {
            Some(p @ ("home" | "shared")) => p,
            _ => return (400, None),
        };
        let mut m = Map::new();
        m.insert("msg".into(), json!("hit"));
        m.insert("page".into(), json!(page));
        let r = v["ref"].as_str().and_then(|r| referrer(r, own_hosts));
        let l = v["lang"].as_str().filter(|l| ["fr", "en"].contains(l));
        m.insert("ref".into(), json!(r));
        m.insert("lang".into(), json!(l));
        m.extend(geo);
        m
    };
    let lower = ua.to_ascii_lowercase();
    if ua.is_empty() || BOTS.iter().any(|b| lower.contains(b)) {
        return (204, None);
    }
    let Some(ip) = ip else {
        return (204, None);
    };
    // événement : ni visiteur ni appareil (revue M4 : rien qui le rattache à une personne) ;
    // les visiteurs uniques se comptent par les seules lignes `hit` de page
    let mut line = json!({"v": 1});
    if fields["msg"] == "hit" {
        let Some(salt) = salt() else {
            return (204, None);
        };
        line["visitor"] = json!(visitor(&salt, ip, ua));
        line["dev"] = json!(if ua.contains("Mobi") {
            "mobile"
        } else {
            "desktop"
        });
    }
    let o = line.as_object_mut().expect("objet");
    o.extend(fields);
    o.retain(|_, v| !v.is_null());
    (204, Some(line))
}

#[cfg(test)]
mod tests {
    use super::*;

    const UA: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0) Mobile/15E148 Safari/604.1";

    fn run(body: &str, ip: Option<&str>, ua: &str, salt: &[u8]) -> (u16, Option<Value>) {
        let own = vec!["optrail.eu".to_string()];
        let g = geo(Some("FR"), Some("IDF"));
        hit(body.as_bytes(), ip, ua, g, &own, || Some(salt.to_vec()))
    }

    #[test]
    fn visiteur_du_jour() {
        let (s, l) = run(
            r#"{"page":"home","lang":"fr"}"#,
            Some("1.2.3.4"),
            UA,
            b"jour1",
        );
        let l = l.unwrap();
        assert_eq!(s, 204);
        let v = l["visitor"].as_str().unwrap();
        assert!(
            v.len() == 16 && v.bytes().all(|b| b.is_ascii_hexdigit()),
            "{v}"
        );
        assert_eq!(l["dev"], "mobile");
        assert_eq!((&l["country"], &l["region"]), (&json!("FR"), &json!("IDF")));
        assert!(l.get("ref").is_none() && !l.to_string().contains("1.2.3.4"));
        // même IP + UA, même jour : même visiteur ; autre jour (autre sel) ou autre IP : différent
        assert_eq!(visitor(b"jour1", "1.2.3.4", UA), v);
        assert_ne!(visitor(b"jour2", "1.2.3.4", UA), v);
        assert_ne!(visitor(b"jour1", "1.2.3.5", UA), v);
        // UA tronqué à 512 octets
        let long = "a".repeat(600);
        assert_eq!(
            visitor(b"s", "ip", &long),
            visitor(b"s", "ip", &long[..512])
        );
    }

    #[test]
    fn robots_et_corps_invalides() {
        let ok = r#"{"page":"shared"}"#;
        let never = || -> Option<Vec<u8>> { panic!("sel inutile") };
        let own: Vec<String> = vec![];
        for ua in ["", "Googlebot/2.1", "curl/8.0", "Mozilla HeadlessChrome"] {
            assert_eq!(
                hit(ok.as_bytes(), Some("1.2.3.4"), ua, Map::new(), &own, never),
                (204, None)
            );
        }
        assert_eq!(
            hit(ok.as_bytes(), None, UA, Map::new(), &own, never),
            (204, None)
        );
        assert_eq!(
            run(ok, Some("1.2.3.4"), "Mozilla/5.0 (X11)", b"s")
                .1
                .unwrap()["dev"],
            "desktop"
        );
        let big = format!(r#"{{"page":"home","ref":"{}"}}"#, "a".repeat(MAX_BODY));
        for b in [
            "",
            "{",
            r#"{"page":"admin"}"#,
            r#"{"lang":"fr"}"#,
            big.as_str(),
        ] {
            assert_eq!(run(b, Some("1.2.3.4"), UA, b"s"), (400, None), "{b}");
        }
        // sans sel (stockage indisponible) : pas de ligne
        assert_eq!(
            hit(ok.as_bytes(), Some("1.2.3.4"), UA, Map::new(), &own, || {
                None
            }),
            (204, None)
        );
    }

    #[test]
    fn evenements_liste_blanche() {
        let ok = json!({"event": "gpx", "lang": "fr", "goal": "target", "km": 10.04, "dplus_m": 300,
            "surface": "trail", "climbs": "balanced", "max_grade_pct": 60, "zone": false, "via_n": 0,
            "rank": 2, "got_km": 10.234, "got_dplus_m": 312.4, "trail_pct": 85, "road_pct": 15,
            "compute_s": 4.26});
        let (s, l) = run(&ok.to_string(), Some("1.2.3.4"), UA, b"s");
        let l = l.unwrap();
        assert_eq!(s, 204);
        assert_eq!(
            (&l["msg"], &l["event"], &l["rank"]),
            (&json!("event"), &json!("gpx"), &json!(2))
        );
        // valeurs en tranches (5 km, 250 m, seconde), jamais exactes ; ni visiteur ni appareil (M4)
        assert_eq!(
            (
                &l["km"],
                &l["dplus_m"],
                &l["got_km"],
                &l["got_dplus_m"],
                &l["compute_s"]
            ),
            (&json!(10), &json!(250), &json!(10), &json!(250), &json!(4))
        );
        assert!(l.get("visitor").is_none() && l.get("dev").is_none(), "{l}");
        // pas de pays/région ni de page sur un événement
        assert!(l.get("country").is_none() && l.get("page").is_none());
        // tout le reste est refusé : champ inconnu, type, bornes, valeur hors liste
        for (k, bad) in [
            ("id", json!("abc")),
            ("lat", json!(45.1)),
            ("event", json!("delete")),
            ("rank", json!(0)),
            ("km", json!("10")),
            ("km", json!(1e9)),
            ("zone", json!(1)),
            ("surface", json!("rail")),
            ("compute_s", json!(-1)),
            ("goal", json!(null)),
        ] {
            let mut b = ok.clone();
            b[k] = bad;
            assert_eq!(
                run(&b.to_string(), Some("1.2.3.4"), UA, b"s"),
                (400, None),
                "{k}"
            );
        }
        assert_eq!(
            run(r#"{"event":"gpx","page":"home"}"#, Some("1"), UA, b"s"),
            (400, None)
        );
        assert!(event(&json!({"km": 10})).is_none()); // event obligatoire
        assert!(event(&json!({"event": "shared_open"})).is_some());
    }

    #[test]
    fn site_d_origine_langue_geo() {
        let r = |r: &str| {
            let b = json!({"page": "home", "ref": r, "lang": "de"}).to_string();
            run(&b, Some("::1"), UA, b"s").1.unwrap()
        };
        assert_eq!(r("WWW.Google.com")["ref"], "google.com");
        assert!(r("www.optrail.eu").get("ref").is_none());
        assert!(r("localhost").get("ref").is_none());
        assert!(r("évil.com").get("ref").is_none());
        assert!(r(&format!("{}.com", "a".repeat(100))).get("ref").is_none());
        assert!(r("google.com").get("lang").is_none());
        assert!(geo(Some("fr"), Some("IDFX")).is_empty());
        assert_eq!(geo(Some("GP"), Some("971")).len(), 2);
    }
}
