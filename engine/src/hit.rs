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

/// Corps `{"page","ref"?,"lang"?}` → (statut, ligne `hit` éventuelle). 400 si corps invalide ;
/// 204 sans ligne pour un robot (UA), sans IP ou sans sel (`salt` : le sel du jour, appelé
/// seulement si la visite compte).
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
    let page = match v["page"].as_str() {
        Some(p @ ("home" | "shared")) => p,
        _ => return (400, None),
    };
    let lower = ua.to_ascii_lowercase();
    if ua.is_empty() || BOTS.iter().any(|b| lower.contains(b)) {
        return (204, None);
    }
    let (Some(ip), Some(salt)) = (ip, ip.and_then(|_| salt())) else {
        return (204, None);
    };
    let mut line = json!({
        "msg": "hit", "v": 1, "visitor": visitor(&salt, ip, ua), "page": page,
        "ref": v["ref"].as_str().and_then(|r| referrer(r, own_hosts)),
        "lang": v["lang"].as_str().filter(|l| ["fr", "en"].contains(l)),
        "dev": if ua.contains("Mobi") { "mobile" } else { "desktop" },
    });
    let o = line.as_object_mut().expect("objet");
    o.extend(geo);
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
