use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use launcher_core::markdown::escape;
use launcher_core::{Action, Icon, Item, Plugin, Section};
use serde_json::{Map, Value};

pub const NAME: &str = "JSON Web Token";

/// Decodes a pasted JSON Web Token. It doesn't verify the signature, which needs the issuer's key;
/// the point is to read the claims.
pub struct Jwt;

impl Plugin for Jwt {
    fn name(&self) -> &'static str {
        NAME
    }

    fn query(&mut self, query: &str) -> Vec<Item> {
        claims(query)
            .map(|token| token.item(Timestamp::now()))
            .into_iter()
            .collect()
    }

    /// Decoded again, as most tokens are never opened. Only here is the zone read, for local
    /// times.
    fn detail(&mut self, query: &str, _item: &Item) -> Vec<Section> {
        claims(query)
            .map(|token| token.detail(Timestamp::now(), &TimeZone::system()))
            .unwrap_or_default()
    }
}

/// A token's header and payload. It may come as a copied header line, `Authorization: Bearer …`.
fn claims(query: &str) -> Option<Token> {
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
    Some(Token {
        header: object(header)?,
        payload: object(payload)?,
    })
}

/// A token's decoded claims.
struct Token {
    header: Map<String, Value>,
    payload: Map<String, Value>,
}

impl Token {
    fn item(&self, now: Timestamp) -> Item {
        let name = match self.payload.get("sub") {
            Some(Value::String(subject)) if !subject.is_empty() => subject.clone(),
            Some(Value::Number(subject)) => subject.to_string(),
            _ => self
                .header
                .get("alg")
                .and_then(Value::as_str)
                .unwrap_or("JWT")
                .to_owned(),
        };
        let status = match (self.time("exp"), self.time("nbf")) {
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
        Item {
            title: format!("{name} · {status}"),
            subtitle: String::new(),
            icon: Icon::Token,
            action: Action::Copy(self.payload_json()),
            has_detail: true,
            // Three base64url JSON objects can only be a token.
            relevance: 100,
        }
    }

    /// The header, payload and times, in `zone`, the times as a table's rows.
    fn detail(&self, now: Timestamp, zone: &TimeZone) -> Vec<Section> {
        let times: Vec<String> = ["exp", "iat", "nbf"]
            .into_iter()
            .filter_map(|claim| {
                let (time, relative) = match self.time(claim)? {
                    Ok(time) => {
                        let relative = if time <= now {
                            format!("{} ago", span(now.duration_since(time)))
                        } else {
                            format!("in {}", span(time.duration_since(now)))
                        };
                        let local = time
                            .to_zoned_datetime(zone.clone())
                            .strftime("%Y-%m-%d %H:%M:%S %Z");
                        (local.to_string(), relative)
                    }
                    Err(raw) => (raw, "unreadable".into()),
                };
                Some(format!(
                    "| {claim} | {} | {} |",
                    escape(&time),
                    escape(&relative)
                ))
            })
            .collect();
        let mut detail = vec![
            Section::mono(
                "Header · signature not verified",
                format!("{:#}", Value::Object(self.header.clone())),
            ),
            Section::mono("Payload", self.payload_json()),
        ];
        if !times.is_empty() {
            detail.push(Section::markdown(
                "Times",
                format!(
                    "| Claim | Time | Relative |\n|---|---|---|\n{}",
                    times.join("\n")
                ),
            ));
        }
        detail
    }

    fn payload_json(&self) -> String {
        format!("{:#}", Value::Object(self.payload.clone()))
    }

    /// `None` when the claim is absent; its raw JSON when it isn't seconds since the epoch, as
    /// when an issuer wrongly writes milliseconds.
    fn time(&self, claim: &str) -> Option<Result<Timestamp, String>> {
        let value = self.payload.get(claim)?;
        Some(
            value
                .as_f64()
                .and_then(|seconds| SignedDuration::try_from_secs_f64(seconds).ok())
                .and_then(|since_epoch| Timestamp::from_duration(since_epoch).ok())
                .ok_or_else(|| value.to_string()),
        )
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
    use launcher_core::plain_text;
    use serde_json::json;

    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn token(header: Value, payload: Value) -> String {
        let part = |value: Value| URL_SAFE_NO_PAD.encode(value.to_string());
        format!("{}.{}.c2lnbmF0dXJl", part(header), part(payload))
    }

    /// The token's row and its detail, in UTC.
    fn decoded(query: &str) -> Option<(Item, Vec<Section>)> {
        let now = Timestamp::from_second(NOW).ok()?;
        claims(query).map(|token| (token.item(now), token.detail(now, &TimeZone::UTC)))
    }

    fn title(header: Value, payload: Value) -> Option<String> {
        decoded(&token(header, payload)).map(|(item, _)| item.title)
    }

    #[test]
    fn valid_token() {
        let token = token(
            json!({"alg": "HS256", "typ": "JWT"}),
            json!({"sub": "ada", "iat": NOW - 3_600, "exp": NOW + 720}),
        );
        let Some((item, detail)) = decoded(&token) else {
            panic!("{token} didn't decode");
        };
        assert_eq!(item.title, "ada · valid for 12 min");
        assert_eq!(item.subtitle, "", "the group's header names it");
        assert_eq!(item.icon, Icon::Token);
        let payload = "{\n  \"sub\": \"ada\",\n  \"iat\": 1799996400,\n  \"exp\": 1800000720\n}";
        assert_eq!(item.action, Action::Copy(payload.into()));
        assert!(item.has_detail);
        assert_eq!(
            detail
                .iter()
                .map(|section| (section.title.as_str(), plain_text(&section.blocks())))
                .collect::<Vec<_>>(),
            [
                (
                    "Header · signature not verified",
                    "{\n  \"alg\": \"HS256\",\n  \"typ\": \"JWT\"\n}".into()
                ),
                ("Payload", payload.into()),
                (
                    "Times",
                    "Claim\tTime\tRelative\n\
                     exp\t2027-01-15 08:12:00 UTC\tin 12 min\n\
                     iat\t2027-01-15 07:00:00 UTC\t1 h ago"
                        .into()
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
            decoded(&format!("{header}.{payload}.sig")).map(|(item, _)| item.title),
            Some("ada · no expiry".into())
        );
    }

    #[test]
    fn the_detail_comes_when_asked_for() {
        let token = token(json!({"alg": "HS256"}), json!({"sub": "ada"}));
        let mut jwt = Jwt;
        let items = jwt.query(&token);
        let detail: Vec<String> = items
            .iter()
            .flat_map(|item| jwt.detail(&token, item))
            .map(|section| section.title)
            .collect();
        assert_eq!(detail, ["Header · signature not verified", "Payload"]);
    }

    #[test]
    fn unreadable_expiry_is_not_no_expiry() {
        let Some((item, detail)) =
            decoded(&token(json!({"alg": "HS256"}), json!({"exp": NOW * 1_000})))
        else {
            panic!("didn't decode");
        };
        assert_eq!(item.title, "HS256 · unreadable expiry");
        let times =
            |detail: Vec<Section>| detail.last().map(|section| plain_text(&section.blocks()));
        assert_eq!(
            times(detail),
            Some("Claim\tTime\tRelative\nexp\t1800000000000\tunreadable".into())
        );
        let Some((item, detail)) = decoded(&token(
            json!({"alg": "HS256"}),
            json!({"exp": "soon | *later*"}),
        )) else {
            panic!("didn't decode");
        };
        assert_eq!(item.title, "HS256 · unreadable expiry");
        assert_eq!(
            times(detail),
            Some("Claim\tTime\tRelative\nexp\t\"soon | *later*\"\tunreadable".into()),
            "the raw value is escaped, so it stays in its cell"
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
        let Some((item, _)) = decoded(&token(
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
