//! What only a handler knows, carried back to [`crate::log_layer`] on the
//! response extensions.
//!
//! The middleware times the request and reads the status. It cannot know the
//! `client_id`, the `grant_type`, the decoded parameters, the PKCE triple or
//! the claims that came out — those live inside the handler that decided them.
//! This is the courier, and it is cubby's two-part capture (`http.rs` wrapper
//! plus an `access_log.rs` hook joined through request extensions) written in
//! axum's idiom.

use axum::response::Response;
use serde_json::{Map, Value};

/// Parameters that are **lists**, not strings, however they arrived on the
/// wire. The spec is explicit that the log carries decoded parameters rather
/// than a raw query string, and `scope` is the one every reader wants split.
const LIST_VALUED: [&str; 1] = ["scope"];

/// The handler's half of an event. Every field is optional because every
/// endpoint knows a different subset.
#[derive(Clone, Debug, Default)]
pub struct LogDetail {
    pub client_id: Option<String>,
    pub grant_type: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
    pub request: Option<Map<String, Value>>,
    pub detail: Option<Map<String, Value>>,
    pub issued: Option<Map<String, Value>>,
    pub flaw: Option<String>,
    /// What is wrong with the persona sources right now, if anything.
    ///
    /// **Not something this request did** — it is the state of the world the
    /// request was answered in, riding back on the request that was about to
    /// get the wrong picker anyway. `/_/` emits no event by Phase 6's design
    /// ("a page is not a decision"), so a broken project file reaches the log
    /// on the next protocol request rather than on the page reload.
    pub warnings: Option<Vec<String>>,
}

impl LogDetail {
    /// Fill in only what is still unknown.
    ///
    /// **First writer wins**, and that is what makes the choke points work: a
    /// refusal deep inside `/oidc/token` attaches its `error` on the way out,
    /// and the outer handler then attaches the `client_id` and the form it
    /// parsed without overwriting the reason. Neither has to know about the
    /// other.
    fn fill_from(&mut self, other: LogDetail) {
        self.client_id = self.client_id.take().or(other.client_id);
        self.grant_type = self.grant_type.take().or(other.grant_type);
        self.error = self.error.take().or(other.error);
        self.error_description = self.error_description.take().or(other.error_description);
        self.request = self.request.take().or(other.request);
        self.detail = self.detail.take().or(other.detail);
        self.issued = self.issued.take().or(other.issued);
        self.flaw = self.flaw.take().or(other.flaw);
        self.warnings = self.warnings.take().or(other.warnings);
    }
}

/// The decoded request parameters, from the form or query pairs a handler
/// already parsed.
///
/// **Nothing is redacted, and that is deliberate** (spec: *Nothing is redacted*).
/// Authorization codes, `code_verifier`s and any `client_secret` a client sends
/// all appear — a `code_verifier` you cannot see is a PKCE failure you cannot
/// diagnose, and starring one out would teach a developer that lanyard checked
/// something it did not.
pub fn request_map(pairs: &[(String, String)]) -> Map<String, Value> {
    let mut out = Map::new();
    for (key, value) in pairs {
        // First occurrence wins, exactly as `Form::get` resolves a repeated
        // parameter — the log must describe the request the handler acted on.
        if out.contains_key(key) {
            continue;
        }
        let decoded = if LIST_VALUED.contains(&key.as_str()) {
            Value::from(
                value
                    .split_whitespace()
                    .map(Value::from)
                    .collect::<Vec<Value>>(),
            )
        } else {
            Value::from(value.clone())
        };
        out.insert(key.clone(), decoded);
    }
    out
}

/// A JWT's header and payload, decoded from the token that was **actually
/// emitted** rather than from the claims map that went in.
///
/// That distinction is the whole of criterion 15: `--alg-none` produces perfect
/// claims and a header saying `"alg": "none"`, and only the emitted token knows
/// that. Returns `None` for anything that is not two decodable base64url JSON
/// segments — a `--bad-signature` token still decodes, because only its
/// signature was broken.
pub fn decoded_token(jwt: &str) -> Option<Value> {
    let mut parts = jwt.split('.');
    let header = decode_segment(parts.next()?)?;
    let payload = decode_segment(parts.next()?)?;
    Some(serde_json::json!({ "header": header, "payload": payload }))
}

fn decode_segment(segment: &str) -> Option<Value> {
    serde_json::from_slice(&crate::b64::decode(segment).ok()?).ok()
}

/// Attach `detail` to `response`, merging with anything already attached.
pub fn attach(mut response: Response, detail: LogDetail) -> Response {
    let merged = match response.extensions_mut().remove::<LogDetail>() {
        Some(mut existing) => {
            existing.fill_from(detail);
            existing
        }
        None => detail,
    };
    response.extensions_mut().insert(merged);
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;
    use serde_json::json;

    fn map(v: Value) -> Option<Map<String, Value>> {
        v.as_object().cloned()
    }

    #[test]
    fn scope_is_split_into_a_list_and_everything_else_is_carried_verbatim() {
        let pairs = vec![
            ("grant_type".to_owned(), "authorization_code".to_owned()),
            ("scope".to_owned(), "openid email profile".to_owned()),
            ("code_verifier".to_owned(), "dBjftJeZ4CVP".to_owned()),
        ];
        let out = request_map(&pairs);
        assert_eq!(out["scope"], json!(["openid", "email", "profile"]));
        assert_eq!(out["grant_type"], "authorization_code");
        // Nothing is redacted — the verifier is the diagnosis.
        assert_eq!(out["code_verifier"], "dBjftJeZ4CVP");
    }

    #[test]
    fn a_repeated_parameter_resolves_to_its_first_occurrence() {
        let pairs = vec![
            ("scope".to_owned(), "openid".to_owned()),
            ("scope".to_owned(), "admin".to_owned()),
        ];
        assert_eq!(request_map(&pairs)["scope"], json!(["openid"]));
    }

    #[test]
    fn an_empty_scope_is_an_empty_list_not_a_list_of_one_empty_string() {
        let pairs = vec![("scope".to_owned(), String::new())];
        assert_eq!(request_map(&pairs)["scope"], json!([]));
    }

    #[test]
    fn a_token_decodes_to_the_header_and_payload_that_were_emitted() {
        let jwt = format!(
            "{}.{}.sig",
            crate::b64::encode(br#"{"alg":"none","typ":"JWT"}"#),
            crate::b64::encode(br#"{"sub":"ada","exp":1757080991}"#),
        );
        let decoded = decoded_token(&jwt).unwrap();
        assert_eq!(decoded["header"]["alg"], "none");
        assert_eq!(decoded["payload"]["sub"], "ada");
    }

    #[test]
    fn a_string_that_is_not_a_token_decodes_to_nothing() {
        assert!(decoded_token("not-a-jwt").is_none());
        assert!(decoded_token("aaaa.bbbb.cccc").is_none());
    }

    #[test]
    fn a_detail_rides_back_on_the_response() {
        let response = attach(
            ().into_response(),
            LogDetail {
                client_id: Some("billing-web".to_owned()),
                ..LogDetail::default()
            },
        );
        let carried = response.extensions().get::<LogDetail>().unwrap();
        assert_eq!(carried.client_id.as_deref(), Some("billing-web"));
    }

    /// The refusal wrote the reason on its way out; the outer handler adds the
    /// envelope. Neither may erase the other.
    #[test]
    fn a_later_attach_fills_gaps_and_never_overwrites() {
        let inner = attach(
            ().into_response(),
            LogDetail {
                error: Some("invalid_grant".to_owned()),
                error_description: Some("no such authorization code".to_owned()),
                ..LogDetail::default()
            },
        );
        let outer = attach(
            inner,
            LogDetail {
                client_id: Some("billing-web".to_owned()),
                grant_type: Some("authorization_code".to_owned()),
                error: Some("should not win".to_owned()),
                request: map(json!({ "code": "8Xk2" })),
                ..LogDetail::default()
            },
        );
        let carried = outer.extensions().get::<LogDetail>().unwrap();
        assert_eq!(carried.error.as_deref(), Some("invalid_grant"));
        assert_eq!(
            carried.error_description.as_deref(),
            Some("no such authorization code")
        );
        assert_eq!(carried.client_id.as_deref(), Some("billing-web"));
        assert_eq!(carried.grant_type.as_deref(), Some("authorization_code"));
        assert_eq!(carried.request.as_ref().unwrap()["code"], "8Xk2");
    }
}
