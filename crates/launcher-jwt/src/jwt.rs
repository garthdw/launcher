use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jiff::Timestamp;
use jiff::tz::TimeZone;
use launcher_core::{Action, Item, Plugin, Section};
use serde_json::{Map, Value};

/// Decodes a pasted JSON Web Token. It doesn't verify the signature, which needs the issuer's key;
/// the point is to read the claims.
pub struct Jwt;

impl Plugin for Jwt {
    fn query(&mut self, query: &str) -> Vec<Item> {
        // The clock and zone are read only for a token, so other queries never load the zone.
        claims(query)
            .map(|(header, payload)| item(header, payload, Timestamp::now(), &TimeZone::system()))
            .into_iter()
            .collect()
    }
}

/// A token's header and payload, with or without a `Bearer` prefix.
fn claims(query: &str) -> Option<(Map<String, Value>, Map<String, Value>)> {
    let token = match query.split_once(' ') {
        Some((scheme, token)) if scheme.eq_ignore_ascii_case("bearer") => token.trim(),
        _ => query,
    };
    let mut parts = token.split('.');
    let (Some(header), Some(payload), Some(signature), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    // `eyJ` is `{"` encoded, so anything else can't be a token and is rejected before decoding.
    if !header.starts_with("eyJ")
        || !payload.starts_with("eyJ")
        || !signature
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return None;
    }
    let object = |part: &str| serde_json::from_slice(&URL_SAFE_NO_PAD.decode(part).ok()?).ok();
    Some((object(header)?, object(payload)?))
}

fn item(
    header: Map<String, Value>,
    payload: Map<String, Value>,
    now: Timestamp,
    zone: &TimeZone,
) -> Item {
    let name = payload
        .get("sub")
        .and_then(Value::as_str)
        .or_else(|| header.get("alg").and_then(Value::as_str))
        .unwrap_or("JWT");
    let time = |claim| {
        payload
            .get(claim)
            .and_then(Value::as_f64)
            .and_then(|seconds| Timestamp::from_second(seconds as i64).ok())
    };
    // Seconds from the claim's time to now: negative while it's still ahead.
    let elapsed = |time: Timestamp| now.duration_since(time).as_secs();
    let status = match (time("nbf").map(elapsed), time("exp").map(elapsed)) {
        (Some(elapsed), _) if elapsed < 0 => format!("not valid for {}", span(-elapsed)),
        (_, Some(elapsed)) if elapsed >= 0 => format!("expired {} ago", span(elapsed)),
        (_, Some(elapsed)) => format!("valid for {}", span(-elapsed)),
        _ => "no expiry".into(),
    };
    let title = format!("{name} · {status}");
    let times: Vec<String> = ["exp", "iat", "nbf"]
        .into_iter()
        .filter_map(|claim| {
            let time = time(claim)?;
            let elapsed = elapsed(time);
            let relative = if elapsed >= 0 {
                format!("{} ago", span(elapsed))
            } else {
                format!("in {}", span(-elapsed))
            };
            let local = time.to_zoned(zone.clone()).strftime("%Y-%m-%d %H:%M:%S %Z");
            Some(format!("{claim}  {local}  ({relative})"))
        })
        .collect();

    let payload = format!("{:#}", Value::Object(payload));
    let mut detail = vec![
        Section {
            title: "Header · signature not verified".into(),
            text: format!("{:#}", Value::Object(header)),
        },
        Section {
            title: "Payload".into(),
            text: payload.clone(),
        },
    ];
    if !times.is_empty() {
        detail.push(Section {
            title: "Times".into(),
            text: times.join("\n"),
        });
    }
    Item {
        title,
        subtitle: "JWT".into(),
        action: Action::Copy(payload),
        detail,
    }
}

/// A duration in its largest whole unit, as in "3 h" or "12 min".
fn span(seconds: i64) -> String {
    if seconds < 60 {
        format!("{seconds} s")
    } else if seconds < 3_600 {
        format!("{} min", seconds / 60)
    } else if seconds < 86_400 {
        format!("{} h", seconds / 3_600)
    } else {
        format!("{} d", seconds / 86_400)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn token(header: Value, payload: Value) -> String {
        let part = |value: Value| URL_SAFE_NO_PAD.encode(value.to_string());
        format!("{}.{}.c2lnbmF0dXJl", part(header), part(payload))
    }

    fn decoded(query: &str) -> Option<Item> {
        let now = Timestamp::from_second(NOW).ok()?;
        claims(query).map(|(header, payload)| item(header, payload, now, &TimeZone::UTC))
    }

    fn title(header: Value, payload: Value) -> Option<String> {
        decoded(&token(header, payload)).map(|item| item.title)
    }

    #[test]
    fn valid_token() {
        let token = token(
            json!({"alg": "HS256", "typ": "JWT"}),
            json!({"sub": "ada", "iat": NOW - 3_600, "exp": NOW + 720}),
        );
        let Some(item) = decoded(&token) else {
            panic!("{token} didn't decode");
        };
        assert_eq!(item.title, "ada · valid for 12 min");
        let payload = "{\n  \"exp\": 1800000720,\n  \"iat\": 1799996400,\n  \"sub\": \"ada\"\n}";
        assert_eq!(item.action, Action::Copy(payload.into()));
        assert_eq!(
            item.detail
                .iter()
                .map(|section| (section.title.as_str(), section.text.as_str()))
                .collect::<Vec<_>>(),
            [
                (
                    "Header · signature not verified",
                    "{\n  \"alg\": \"HS256\",\n  \"typ\": \"JWT\"\n}"
                ),
                ("Payload", payload),
                (
                    "Times",
                    "exp  2027-01-15 08:12:00 UTC  (in 12 min)\n\
                     iat  2027-01-15 07:00:00 UTC  (1 h ago)"
                ),
            ]
        );
    }

    #[test]
    fn expired_token_without_subject_names_the_algorithm() {
        assert_eq!(
            title(json!({"alg": "RS256"}), json!({"exp": NOW - 3 * 3_600})),
            Some("RS256 · expired 3 h ago".into())
        );
    }

    #[test]
    fn not_yet_valid_and_no_expiry() {
        assert_eq!(
            title(json!({"alg": "HS256"}), json!({"nbf": NOW + 300})),
            Some("HS256 · not valid for 5 min".into())
        );
        assert_eq!(
            title(json!({"alg": "HS256"}), json!({"iss": "launcher"})),
            Some("HS256 · no expiry".into())
        );
    }

    #[test]
    fn bearer_prefix() {
        let token = token(json!({"alg": "HS256"}), json!({"sub": "ada"}));
        assert_eq!(decoded(&format!("Bearer {token}")), decoded(&token));
        assert_eq!(decoded(&format!("bearer  {token}")), decoded(&token));
        assert!(decoded(&token).is_some());
    }

    #[test]
    fn malformed_input_shows_nothing() {
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"HS256"}"#);
        for query in [
            "firefox".to_owned(),
            "a.b.c".to_owned(),
            format!("{header}.{header}"),
            format!("{header}.{header}.sig.extra"),
            format!("{header}.eyJub3QganNvbg.sig"),
            format!("{header}.{}.sig", URL_SAFE_NO_PAD.encode("[1]")),
            format!("{header}.{header}.not a signature"),
            format!("Basic {header}.{header}.sig"),
        ] {
            assert_eq!(decoded(&query), None, "{query}");
        }
    }
}
