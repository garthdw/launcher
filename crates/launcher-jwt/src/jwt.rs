use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use launcher_core::{Action, Item, Plugin, Section};
use serde_json::{Map, Value};

/// Decodes a pasted JSON Web Token. It doesn't verify the signature, which needs the issuer's key;
/// the point is to read the claims.
pub struct Jwt;

impl Plugin for Jwt {
    fn name(&self) -> &'static str {
        "JSON Web Token"
    }

    fn query(&mut self, query: &str) -> Vec<Item> {
        // The clock and zone are read only for a token, so other queries never load the zone.
        claims(query)
            .map(|(header, payload)| item(header, payload, Timestamp::now(), &TimeZone::system()))
            .into_iter()
            .collect()
    }
}

/// A token's header and payload. It may come as a copied header line, `Authorization: Bearer …`.
fn claims(query: &str) -> Option<(Map<String, Value>, Map<String, Value>)> {
    let query = match query.split_at_checked("authorization:".len()) {
        Some((name, value)) if name.eq_ignore_ascii_case("authorization:") => value.trim_start(),
        _ => query,
    };
    let token = match query.split_once(char::is_whitespace) {
        Some((scheme, token)) if scheme.eq_ignore_ascii_case("bearer") => token.trim(),
        _ => query,
    };
    let mut parts = token.split('.');
    let (Some(header), Some(payload), Some(signature), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    // Rejects most other queries before decoding anything.
    let base64url = |part: &str| {
        part.bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    };
    if header.is_empty()
        || payload.is_empty()
        || ![header, payload, signature].into_iter().all(base64url)
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
    let name = match payload.get("sub") {
        Some(Value::String(subject)) if !subject.is_empty() => subject.clone(),
        Some(Value::Number(subject)) => subject.to_string(),
        _ => header
            .get("alg")
            .and_then(Value::as_str)
            .unwrap_or("JWT")
            .to_owned(),
    };
    // `None` when the claim is absent; its raw JSON when it isn't seconds since the epoch, as
    // when an issuer wrongly writes milliseconds.
    let time = |claim| {
        let value = payload.get(claim)?;
        Some(
            value
                .as_f64()
                .and_then(|seconds| SignedDuration::try_from_secs_f64(seconds).ok())
                .and_then(|since_epoch| Timestamp::from_duration(since_epoch).ok())
                .ok_or_else(|| value.to_string()),
        )
    };
    let status = match (time("exp"), time("nbf")) {
        (Some(Ok(exp)), _) if exp <= now => {
            format!("expired {} ago", span(now.duration_since(exp)))
        }
        (_, Some(Ok(nbf))) if nbf > now => {
            format!("not valid for {}", span(nbf.duration_since(now)))
        }
        (Some(Ok(exp)), _) => format!("valid for {}", span(exp.duration_since(now))),
        (Some(Err(_)), _) => "unreadable expiry".into(),
        (None, _) => "no expiry".into(),
    };
    let times: Vec<String> = ["exp", "iat", "nbf"]
        .into_iter()
        .filter_map(|claim| {
            Some(match time(claim)? {
                Ok(time) => {
                    let relative = if time <= now {
                        format!("{} ago", span(now.duration_since(time)))
                    } else {
                        format!("in {}", span(time.duration_since(now)))
                    };
                    let local = time
                        .to_zoned_datetime(zone.clone())
                        .strftime("%Y-%m-%d %H:%M:%S %Z");
                    format!("{claim}  {local}  ({relative})")
                }
                Err(raw) => format!("{claim}  {raw}  (unreadable)"),
            })
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
        title: format!("{name} · {status}"),
        subtitle: "JWT".into(),
        action: Action::Copy(payload),
        detail,
        // Three base64url JSON objects can only be a token.
        relevance: 100,
    }
}

/// A duration in its largest whole unit, as in "3 h" or "12 min".
fn span(duration: SignedDuration) -> String {
    let seconds = duration.as_secs();
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
        let payload = "{\n  \"sub\": \"ada\",\n  \"iat\": 1799996400,\n  \"exp\": 1800000720\n}";
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
        assert_eq!(decoded(&format!("Bearer\t{token}")), decoded(&token));
        assert_eq!(
            decoded(&format!("Authorization: Bearer {token}")),
            decoded(&token)
        );
        assert!(decoded(&token).is_some());
    }

    #[test]
    fn json_that_does_not_encode_to_eyj() {
        // `{"$` encodes to `eyIk` and `{ "` to `eyAi`, not the usual `eyJ`.
        let header = URL_SAFE_NO_PAD.encode(r#"{"$alg":"HS256"}"#);
        let payload = URL_SAFE_NO_PAD.encode(r#"{ "sub": "ada"}"#);
        assert_eq!(
            decoded(&format!("{header}.{payload}.sig")).map(|item| item.title),
            Some("ada · no expiry".into())
        );
    }

    #[test]
    fn unreadable_expiry_is_not_no_expiry() {
        let Some(item) = decoded(&token(json!({"alg": "HS256"}), json!({"exp": NOW * 1_000})))
        else {
            panic!("didn't decode");
        };
        assert_eq!(item.title, "HS256 · unreadable expiry");
        assert_eq!(
            item.detail.last().map(|section| section.text.as_str()),
            Some("exp  1800000000000  (unreadable)")
        );
        assert_eq!(
            title(json!({"alg": "HS256"}), json!({"exp": "soon"})),
            Some("HS256 · unreadable expiry".into())
        );
    }

    #[test]
    fn expiry_outranks_not_before() {
        assert_eq!(
            title(
                json!({"alg": "HS256"}),
                json!({"nbf": NOW + 300, "exp": NOW - 60})
            ),
            Some("HS256 · expired 1 min ago".into())
        );
    }

    #[test]
    fn expiry_within_the_last_second_is_still_valid() {
        assert_eq!(
            title(json!({"alg": "HS256"}), json!({"exp": NOW as f64 + 0.4})),
            Some("HS256 · valid for 0 s".into())
        );
    }

    #[test]
    fn subject_names() {
        assert_eq!(
            title(json!({"alg": "HS256"}), json!({"sub": 12345})),
            Some("12345 · no expiry".into())
        );
        assert_eq!(
            title(json!({"alg": "HS256"}), json!({"sub": ""})),
            Some("HS256 · no expiry".into())
        );
    }

    #[test]
    fn payload_keeps_the_token_order() {
        let Some(item) = decoded(&token(
            json!({"alg": "HS256"}),
            json!({"sub": "ada", "aud": "launcher"}),
        )) else {
            panic!("didn't decode");
        };
        assert_eq!(
            item.action,
            Action::Copy("{\n  \"sub\": \"ada\",\n  \"aud\": \"launcher\"\n}".into())
        );
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
