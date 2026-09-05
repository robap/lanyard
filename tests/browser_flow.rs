//! The browser flow end to end, over a real socket and with a real cookie jar:
//! `/authorize` → `/_/` → `/_/pick` → `/oidc/token`.
//!
//! These are the inner loop for the acceptance criteria that a human drives in
//! a browser. They do not replace those — a .NET app is the only thing that can
//! prove a .NET app logs in — but they are what keeps the wiring honest between
//! acceptance runs.

mod support;

use support::{client, spawn, spawn_with, ADA_CB};

const CB: &str = "http://localhost:5000/signin-oidc";

/// Start a login and return the pending request id from the redirect.
async fn start(base: &str, http: &reqwest::Client, query: &str) -> String {
    let res = http
        .get(format!("{base}/oidc/authorize?{query}"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 302, "authorize should hop to /_/");
    res.headers()["location"]
        .to_str()
        .unwrap()
        .strip_prefix("/_/?req=")
        .expect("the picker is the destination")
        .to_string()
}

// ------------------------------------------------------------- the picker --

/// Criterion 25. The banner prints this URL, so it has to be honest about there
/// being nothing to do.
#[tokio::test]
async fn the_picker_with_no_req_lists_everyone_and_says_nothing_is_happening() {
    let base = spawn().await;
    let res = reqwest::get(format!("{base}/_/")).await.unwrap();

    assert_eq!(res.status().as_u16(), 200);
    assert!(res.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/html"));

    let body = res.text().await.unwrap();
    for name in ["Ada Bell", "Mira Okonkwo", "nobody"] {
        assert!(body.contains(name), "{name} is missing from the picker");
    }
    assert!(body.contains("No login is in progress"), "{body}");
}

/// `nest("/_")` answers `/_` and not `/_/`, so `/_/` is routed at the top level
/// too. This is the test that fails if that second route is ever tidied away.
#[tokio::test]
async fn both_spellings_of_the_ui_root_answer() {
    let base = spawn().await;
    for path in ["/_", "/_/"] {
        let res = reqwest::get(format!("{base}{path}")).await.unwrap();
        assert_eq!(res.status().as_u16(), 200, "{path} must not 404");
    }
}

#[tokio::test]
async fn the_picker_with_a_req_offers_one_form_per_person() {
    let base = spawn().await;
    let http = client();
    let req = start(
        &base,
        &http,
        &format!("client_id=billing-web&response_type=code&redirect_uri={ADA_CB}"),
    )
    .await;

    let body = reqwest::get(format!("{base}/_/?req={req}"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    for id in ["ada", "mira", "nobody"] {
        assert!(
            body.contains(&format!("name=\"persona\" value=\"{id}\"")),
            "no form for {id}"
        );
    }
    assert!(body.contains("action=\"/_/pick\""));
    assert!(
        body.contains("billing-web"),
        "the page names who is waiting"
    );
    // Criterion 29: nothing is fetched, so a machine with no network and no
    // `node` renders the same page.
    assert!(!body.contains("<script src"), "no external script");
    assert!(!body.contains("cdn."), "no CDN");
}

/// Criterion 27. A persona file can come out of a fixture generator, and a
/// picker that executes its own persona list is a bad look for a tool whose
/// pitch is "it catches your bugs".
#[tokio::test]
async fn a_persona_named_like_a_script_tag_renders_as_text() {
    let personas = lanyard_cli::persona::Personas::parse(
        "personas:\n  - id: xss\n    name: \"<script>alert(1)</script>\"\n",
        std::path::Path::new("/tmp/users.yaml"),
    )
    .unwrap();
    let base = spawn_with(personas).await;

    let body = reqwest::get(format!("{base}/_/"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!body.contains("<script>alert(1)</script>"), "{body}");
    assert!(
        body.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
        "{body}"
    );
}

/// The picker can be reloaded, bookmarked mid-flow, or opened in another tab.
/// That is what peeking rather than taking buys.
#[tokio::test]
async fn the_picker_can_be_reloaded_without_consuming_the_request() {
    let base = spawn().await;
    let http = client();
    let req = start(
        &base,
        &http,
        &format!("client_id=x&response_type=code&redirect_uri={ADA_CB}"),
    )
    .await;

    for _ in 0..3 {
        let res = reqwest::get(format!("{base}/_/?req={req}")).await.unwrap();
        assert_eq!(res.status().as_u16(), 200);
    }
    // And it still completes.
    let res = http
        .post(format!("{base}/_/pick"))
        .form(&[("req", req.as_str()), ("persona", "ada")])
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 302);
}

#[tokio::test]
async fn a_req_that_names_nothing_says_so_rather_than_rendering_an_empty_picker() {
    let base = spawn().await;
    let res = reqwest::get(format!("{base}/_/?req=00000000000000000000000000000000"))
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 400);
    assert!(res.text().await.unwrap().contains("no longer in progress"));
}

// --------------------------------------------------------------- the pick --

#[tokio::test]
async fn picking_a_person_redirects_to_the_rp_with_a_code_and_the_state() {
    let base = spawn().await;
    let http = client();
    let req = start(
        &base,
        &http,
        &format!("client_id=billing-web&response_type=code&redirect_uri={ADA_CB}&state=s1"),
    )
    .await;

    let res = http
        .post(format!("{base}/_/pick"))
        .form(&[("req", req.as_str()), ("persona", "ada")])
        .send()
        .await
        .unwrap();

    assert_eq!(res.status().as_u16(), 302);
    let params = support::query_of(&res);
    assert!(params.contains_key("code"), "{params:?}");
    assert_eq!(params["state"], "s1");
    assert!(res.headers()["location"].to_str().unwrap().starts_with(CB));
}

/// Criterion 9, asserted on the **decoded** value. `serde_urlencoded` writes a
/// space as `+` and a real RP form-decodes, so comparing raw query strings
/// would fail the criterion for a reason that is not a bug.
#[tokio::test]
async fn state_round_trips_byte_for_byte_once_decoded() {
    let base = spawn().await;
    let http = client();
    let hostile = "a b/c&d=e%2F";
    let req = start(
        &base,
        &http,
        &format!(
            "client_id=x&response_type=code&redirect_uri={ADA_CB}&state={}",
            urlencode(hostile)
        ),
    )
    .await;

    let res = http
        .post(format!("{base}/_/pick"))
        .form(&[("req", req.as_str()), ("persona", "ada")])
        .send()
        .await
        .unwrap();
    assert_eq!(support::query_of(&res)["state"], hostile);
}

#[tokio::test]
async fn no_state_in_means_no_state_out_of_a_successful_login_too() {
    let base = spawn().await;
    let http = client();
    let req = start(
        &base,
        &http,
        &format!("client_id=x&response_type=code&redirect_uri={ADA_CB}"),
    )
    .await;

    let res = http
        .post(format!("{base}/_/pick"))
        .form(&[("req", req.as_str()), ("persona", "ada")])
        .send()
        .await
        .unwrap();
    let location = res.headers()["location"].to_str().unwrap();
    assert!(location.contains("code="), "{location}");
    assert!(!location.contains("state="), "{location}");
}

/// Criterion 17, read from the wire rather than from source. Phase 0's
/// obligation #1: a `Secure` cookie is silently dropped over `http://` and the
/// symptom is "it asks me to pick a persona every single time".
#[tokio::test]
async fn the_session_cookie_has_no_secure_and_no_expiry() {
    let base = spawn().await;
    let http = client();
    let req = start(
        &base,
        &http,
        &format!("client_id=x&response_type=code&redirect_uri={ADA_CB}"),
    )
    .await;

    let res = http
        .post(format!("{base}/_/pick"))
        .form(&[("req", req.as_str()), ("persona", "ada")])
        .send()
        .await
        .unwrap();

    let cookie = res.headers()["set-cookie"].to_str().unwrap();
    assert!(cookie.starts_with("lanyard_session="), "{cookie}");
    assert!(cookie.contains("Path=/"), "{cookie}");
    assert!(cookie.contains("HttpOnly"), "{cookie}");
    assert!(cookie.contains("SameSite=Lax"), "{cookie}");
    assert!(!cookie.contains("Secure"), "{cookie}");
    assert!(!cookie.contains("Max-Age"), "{cookie}");
    assert!(!cookie.contains("Expires"), "{cookie}");
}

/// The pending record is consumed. A back button and a second click must not
/// mint a second code for the same request.
#[tokio::test]
async fn a_pending_request_can_only_be_answered_once() {
    let base = spawn().await;
    let http = client();
    let req = start(
        &base,
        &http,
        &format!("client_id=x&response_type=code&redirect_uri={ADA_CB}"),
    )
    .await;

    let first = http
        .post(format!("{base}/_/pick"))
        .form(&[("req", req.as_str()), ("persona", "ada")])
        .send()
        .await
        .unwrap();
    assert_eq!(first.status().as_u16(), 302);

    let second = http
        .post(format!("{base}/_/pick"))
        .form(&[("req", req.as_str()), ("persona", "ada")])
        .send()
        .await
        .unwrap();
    assert_eq!(second.status().as_u16(), 400);
    assert!(second.headers().get("location").is_none());
}

// --------------------------------------------------------- form_post mode --

#[tokio::test]
async fn form_post_returns_a_self_submitting_page_that_degrades_to_a_button() {
    let base = spawn().await;
    let http = client();
    let req = start(
        &base,
        &http,
        &format!(
            "client_id=x&response_type=code&redirect_uri={ADA_CB}\
             &response_mode=form_post&state=s9"
        ),
    )
    .await;

    let res = http
        .post(format!("{base}/_/pick"))
        .form(&[("req", req.as_str()), ("persona", "ada")])
        .send()
        .await
        .unwrap();

    assert_eq!(
        res.status().as_u16(),
        200,
        "form_post is a page, not a redirect"
    );
    assert!(res.headers().get("location").is_none());
    let body = res.text().await.unwrap();
    assert!(
        body.contains(&format!("method=\"post\" action=\"{CB}\"")),
        "{body}"
    );
    assert!(body.contains("name=\"code\""), "{body}");
    assert!(body.contains("name=\"state\" value=\"s9\""), "{body}");
    assert!(body.contains("<noscript>"), "criterion 26 needs the button");
}

fn urlencode(raw: &str) -> String {
    raw.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

// ------------------------------------------------------- the code exchange --

use serde_json::Value;

/// Run a whole login in a **fresh browser** and hand back the code. Most tests
/// here are about the exchange rather than about sessions, and a shared jar
/// would make the second login skip the picker — which is a feature, and a
/// different test's subject.
async fn login(base: &str, query: &str) -> String {
    login_with(base, &client(), query).await
}

/// The same, in a browser the caller owns, for the tests that are about what
/// the jar remembers.
async fn login_with(base: &str, http: &reqwest::Client, query: &str) -> String {
    let req = start(base, http, query).await;
    let res = http
        .post(format!("{base}/_/pick"))
        .form(&[("req", req.as_str()), ("persona", "ada")])
        .send()
        .await
        .unwrap();
    support::query_of(&res)["code"].clone()
}

async fn exchange(base: &str, params: &[(&str, &str)]) -> (u16, Value) {
    let res = client()
        .post(format!("{base}/oidc/token"))
        .form(params)
        .send()
        .await
        .unwrap();
    (res.status().as_u16(), res.json().await.unwrap())
}

fn payload(token: &str) -> Value {
    let segment = token.split('.').nth(1).expect("compact JWS");
    serde_json::from_slice(&lanyard_cli::b64::decode(segment).unwrap()).unwrap()
}

#[tokio::test]
async fn a_code_exchanges_for_an_access_token_and_an_id_token() {
    let base = spawn().await;
    let code = login(
        &base,
        &format!(
            "client_id=billing-web&response_type=code&redirect_uri={ADA_CB}\
             &scope=openid%20email%20profile&nonce=n-0S6&state=s1"
        ),
    )
    .await;

    let (status, body) = exchange(
        &base,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", CB),
        ],
    )
    .await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(body["token_type"], "Bearer");
    assert_eq!(
        body["expires_in"], 60,
        "the access token is still 60 seconds"
    );
    assert_eq!(body["scope"], "openid email profile");

    let id = payload(body["id_token"].as_str().unwrap());
    assert_eq!(
        id["aud"], "billing-web",
        "the client_id, not the API audience"
    );
    assert_eq!(id["sub"], "ada");
    assert_eq!(id["nonce"], "n-0S6");
    assert_eq!(
        id["exp"].as_u64().unwrap() - id["iat"].as_u64().unwrap(),
        300,
        "a receipt, not a credential"
    );
    assert!(id["auth_time"].is_u64());
}

/// OIDC Core §3.1.3.6: base64url of the leftmost **128 bits** of the SHA-256.
/// Emitting a full digest is the classic way to produce a hash every library
/// rejects, so this recomputes both rather than trusting the shape.
#[tokio::test]
async fn at_hash_and_c_hash_are_the_leftmost_half_of_the_sha256() {
    use sha2::{Digest as _, Sha256};

    let base = spawn().await;
    let code = login(
        &base,
        &format!("client_id=x&response_type=code&redirect_uri={ADA_CB}&scope=openid"),
    )
    .await;

    let (_, body) = exchange(
        &base,
        &[("grant_type", "authorization_code"), ("code", &code)],
    )
    .await;

    let half = |value: &str| lanyard_cli::b64::encode(&Sha256::digest(value.as_bytes())[..16]);
    let id = payload(body["id_token"].as_str().unwrap());
    assert_eq!(id["at_hash"], half(body["access_token"].as_str().unwrap()));
    assert_eq!(id["c_hash"], half(&code));
}

/// Criterion 14. Without `openid` this is a plain OAuth 2.0 code flow and gets
/// a plain OAuth 2.0 response — which is honest, and a thing worth testing.
#[tokio::test]
async fn a_request_without_openid_gets_no_id_token() {
    let base = spawn().await;
    let code = login(
        &base,
        &format!("client_id=x&response_type=code&redirect_uri={ADA_CB}&scope=profile%20email"),
    )
    .await;

    let (status, body) = exchange(
        &base,
        &[("grant_type", "authorization_code"), ("code", &code)],
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert!(body.get("id_token").is_none(), "{body}");
    assert!(body["access_token"].is_string());
}

/// Criterion 15. Three logins, three scope sets, one persona.
#[tokio::test]
async fn scope_decides_which_persona_claims_reach_the_id_token() {
    let base = spawn().await;

    for (scope, email, name) in [
        ("openid", false, false),
        ("openid%20email", true, false),
        ("openid%20email%20profile", true, true),
    ] {
        let code = login(
            &base,
            &format!("client_id=x&response_type=code&redirect_uri={ADA_CB}&scope={scope}"),
        )
        .await;
        let (_, body) = exchange(
            &base,
            &[("grant_type", "authorization_code"), ("code", &code)],
        )
        .await;
        let id = payload(body["id_token"].as_str().unwrap());

        assert_eq!(id["sub"], "ada", "{scope}");
        assert_eq!(id["roles"], serde_json::json!(["admin", "user"]), "{scope}");
        assert_eq!(id.get("email").is_some(), email, "{scope}: {id}");
        assert_eq!(id.get("email_verified").is_some(), email, "{scope}: {id}");
        assert_eq!(id.get("name").is_some(), name, "{scope}: {id}");
        assert_eq!(
            id.get("preferred_username").is_some(),
            name,
            "{scope}: {id}"
        );
    }
}

/// The persona that breaks applications is a persona you can log in as.
#[tokio::test]
async fn nobody_completes_a_login_and_carries_only_sub() {
    let base = spawn().await;
    let http = client();
    let req = start(
        &base,
        &http,
        &format!(
            "client_id=x&response_type=code&redirect_uri={ADA_CB}&scope=openid%20email%20profile"
        ),
    )
    .await;
    let res = http
        .post(format!("{base}/_/pick"))
        .form(&[("req", req.as_str()), ("persona", "nobody")])
        .send()
        .await
        .unwrap();
    let code = support::query_of(&res)["code"].clone();

    let (status, body) = exchange(
        &base,
        &[("grant_type", "authorization_code"), ("code", &code)],
    )
    .await;
    assert_eq!(status, 200, "the login itself must succeed: {body}");

    let id = payload(body["id_token"].as_str().unwrap());
    assert_eq!(id["sub"], "nobody");
    assert!(id.get("email").is_none());
    assert!(id.get("name").is_none());
}

// ------------------------------------------------------------------- PKCE --

/// RFC 7636 §B's worked example, so the transformation is checked against the
/// RFC rather than against lanyard's own hashing.
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const S256_CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

#[tokio::test]
async fn s256_verifies_and_one_character_different_does_not() {
    let base = spawn().await;

    let code = login(
        &base,
        &format!(
            "client_id=x&response_type=code&redirect_uri={ADA_CB}\
             &code_challenge={S256_CHALLENGE}&code_challenge_method=S256"
        ),
    )
    .await;
    let (status, body) = exchange(
        &base,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("code_verifier", VERIFIER),
        ],
    )
    .await;
    assert_eq!(status, 200, "{body}");

    let code = login(
        &base,
        &format!(
            "client_id=x&response_type=code&redirect_uri={ADA_CB}\
             &code_challenge={S256_CHALLENGE}&code_challenge_method=S256"
        ),
    )
    .await;
    let mut wrong = VERIFIER.to_string();
    wrong.replace_range(0..1, "e");
    let (status, body) = exchange(
        &base,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("code_verifier", &wrong),
        ],
    )
    .await;
    assert_eq!(status, 400);
    assert_eq!(body["error"], "invalid_grant");
    assert!(
        body["error_description"]
            .as_str()
            .unwrap()
            .contains("code_verifier"),
        "{body}"
    );
}

/// `plain`, and the omitted method that RFC 7636 §4.4.1 says *is* plain.
#[tokio::test]
async fn plain_works_both_when_named_and_when_omitted() {
    let base = spawn().await;

    for method in ["&code_challenge_method=plain", ""] {
        let code = login(
            &base,
            &format!(
                "client_id=x&response_type=code&redirect_uri={ADA_CB}\
                 &code_challenge={VERIFIER}{method}"
            ),
        )
        .await;
        let (status, body) = exchange(
            &base,
            &[
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("code_verifier", VERIFIER),
            ],
        )
        .await;
        assert_eq!(status, 200, "method {method:?}: {body}");

        let code = login(
            &base,
            &format!(
                "client_id=x&response_type=code&redirect_uri={ADA_CB}\
                 &code_challenge={VERIFIER}{method}"
            ),
        )
        .await;
        let (status, body) = exchange(
            &base,
            &[
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("code_verifier", "not-the-verifier"),
            ],
        )
        .await;
        assert_eq!(status, 400, "method {method:?}: {body}");
    }
}

#[tokio::test]
async fn a_missing_verifier_is_refused_when_a_challenge_was_recorded() {
    let base = spawn().await;
    let code = login(
        &base,
        &format!(
            "client_id=x&response_type=code&redirect_uri={ADA_CB}\
             &code_challenge={S256_CHALLENGE}&code_challenge_method=S256"
        ),
    )
    .await;

    let (status, body) = exchange(
        &base,
        &[("grant_type", "authorization_code"), ("code", &code)],
    )
    .await;
    assert_eq!(status, 400);
    assert_eq!(body["error"], "invalid_grant");
    assert!(body["error_description"]
        .as_str()
        .unwrap()
        .contains("code_verifier is required"));
}

/// A public client with no secret anywhere is the `node-spa` shape, and it is
/// the ordinary case rather than a special one.
#[tokio::test]
async fn a_public_client_with_pkce_and_no_secret_completes() {
    let base = spawn().await;
    let code = login(
        &base,
        &format!(
            "client_id=node-spa&response_type=code&redirect_uri={ADA_CB}&scope=openid%20email\
             &code_challenge={S256_CHALLENGE}&code_challenge_method=S256"
        ),
    )
    .await;

    let (status, body) = exchange(
        &base,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("code_verifier", VERIFIER),
            ("client_id", "node-spa"),
        ],
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        payload(body["id_token"].as_str().unwrap())["aud"],
        "node-spa"
    );
}

// ----------------------------------------------------------- code hygiene --

/// Criterion 12. Three refusals, three different descriptions — Phase 6's log
/// can only report what this phase distinguishes.
#[tokio::test]
async fn reuse_expiry_and_a_mismatched_redirect_uri_each_say_something_different() {
    let base = spawn().await;

    // Reused.
    let code = login(
        &base,
        &format!("client_id=x&response_type=code&redirect_uri={ADA_CB}"),
    )
    .await;
    let (status, _) = exchange(
        &base,
        &[("grant_type", "authorization_code"), ("code", &code)],
    )
    .await;
    assert_eq!(status, 200);
    let (status, reused) = exchange(
        &base,
        &[("grant_type", "authorization_code"), ("code", &code)],
    )
    .await;
    assert_eq!(status, 400);
    assert_eq!(reused["error"], "invalid_grant");

    // A mismatched redirect_uri.
    let code = login(
        &base,
        &format!("client_id=x&response_type=code&redirect_uri={ADA_CB}"),
    )
    .await;
    let (status, mismatched) = exchange(
        &base,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", "http://localhost:5000/somewhere-else"),
        ],
    )
    .await;
    assert_eq!(status, 400);
    assert_eq!(mismatched["error"], "invalid_grant");

    // Never issued.
    let (status, unknown) = exchange(
        &base,
        &[
            ("grant_type", "authorization_code"),
            ("code", "00000000000000000000000000000000"),
        ],
    )
    .await;
    assert_eq!(status, 400);
    assert_eq!(unknown["error"], "invalid_grant");

    // Expired, on a server whose codes die on arrival.
    let short = support::spawn_with_short_lived_codes().await;
    let code = login(
        &short,
        &format!("client_id=x&response_type=code&redirect_uri={ADA_CB}"),
    )
    .await;
    let (status, expired) = exchange(
        &short,
        &[("grant_type", "authorization_code"), ("code", &code)],
    )
    .await;
    assert_eq!(status, 400);
    assert_eq!(expired["error"], "invalid_grant");

    let descriptions: Vec<&str> = [&reused, &mismatched, &unknown, &expired]
        .iter()
        .map(|body| body["error_description"].as_str().unwrap())
        .collect();
    let mut unique = descriptions.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(
        unique.len(),
        4,
        "each refusal must name what actually failed: {descriptions:?}"
    );
    assert!(
        descriptions[0].contains("already been exchanged"),
        "{descriptions:?}"
    );
    assert!(
        descriptions[1].contains("does not match"),
        "{descriptions:?}"
    );
    assert!(descriptions[2].contains("no such"), "{descriptions:?}");
    assert!(descriptions[3].contains("expired"), "{descriptions:?}");
}

/// An SDK that does not resend `redirect_uri` is not making a mistake, and
/// refusing it would be a registration-shaped "no" on an endpoint that has none.
#[tokio::test]
async fn an_absent_redirect_uri_at_the_token_endpoint_is_accepted() {
    let base = spawn().await;
    let code = login(
        &base,
        &format!("client_id=x&response_type=code&redirect_uri={ADA_CB}"),
    )
    .await;
    let (status, body) = exchange(
        &base,
        &[("grant_type", "authorization_code"), ("code", &code)],
    )
    .await;
    assert_eq!(status, 200, "{body}");
}

/// Criterion 23's property, at the unit of the two callers rather than the two
/// command lines: one persona, one `issue()`, one claim set.
#[tokio::test]
async fn the_browser_access_token_matches_the_client_credentials_one() {
    let base = spawn().await;
    let code = login(
        &base,
        &format!(
            "client_id=billing-web&response_type=code&redirect_uri={ADA_CB}\
             &audience=billing-api&scope=openid"
        ),
    )
    .await;
    let (_, browser) = exchange(
        &base,
        &[("grant_type", "authorization_code"), ("code", &code)],
    )
    .await;
    let (_, cli) = exchange(
        &base,
        &[
            ("grant_type", "client_credentials"),
            ("persona", "ada"),
            ("audience", "billing-api"),
        ],
    )
    .await;

    let strip = |token: &str| {
        let mut claims = payload(token);
        for volatile in ["iat", "nbf", "exp", "jti", "client_id", "scope"] {
            claims.as_object_mut().unwrap().remove(volatile);
        }
        claims
    };
    assert_eq!(
        strip(browser["access_token"].as_str().unwrap()),
        strip(cli["access_token"].as_str().unwrap()),
        "two callers of issue(), one claim set"
    );
}

/// Phase 2's grant is untouched by all of this.
#[tokio::test]
async fn client_credentials_still_returns_sixty_seconds_and_no_id_token() {
    let base = spawn().await;
    let (status, body) = exchange(
        &base,
        &[
            ("grant_type", "client_credentials"),
            ("persona", "ada"),
            ("scope", "openid email profile"),
        ],
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["expires_in"], 60);
    assert!(
        body.get("id_token").is_none(),
        "client credentials has no user authentication event to attest to"
    );
    // And still unfiltered, despite the openid scope.
    let claims = payload(body["access_token"].as_str().unwrap());
    assert_eq!(claims["email"], "ada@example.test");
}

// --------------------------------------------------------------- userinfo --

/// Criterion 16. The body equals the ID token's scope-filtered persona claims —
/// `sub` included, everything protocol-shaped excluded.
#[tokio::test]
async fn userinfo_returns_the_same_claims_the_id_token_carried() {
    let base = spawn().await;
    let code = login(
        &base,
        &format!(
            "client_id=x&response_type=code&redirect_uri={ADA_CB}&scope=openid%20email%20profile"
        ),
    )
    .await;
    let (_, tokens) = exchange(
        &base,
        &[("grant_type", "authorization_code"), ("code", &code)],
    )
    .await;

    let res = client()
        .get(format!("{base}/oidc/userinfo"))
        .bearer_auth(tokens["access_token"].as_str().unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 200);
    let info: Value = res.json().await.unwrap();

    let mut expected = payload(tokens["id_token"].as_str().unwrap());
    for protocol in [
        "iss",
        "aud",
        "exp",
        "iat",
        "nbf",
        "jti",
        "nonce",
        "at_hash",
        "c_hash",
        "auth_time",
    ] {
        expected.as_object_mut().unwrap().remove(protocol);
    }
    assert_eq!(info, expected);
    assert_eq!(info["sub"], "ada");
}

#[tokio::test]
async fn userinfo_is_scope_filtered_like_the_id_token() {
    let base = spawn().await;
    let code = login(
        &base,
        &format!("client_id=x&response_type=code&redirect_uri={ADA_CB}&scope=openid"),
    )
    .await;
    let (_, tokens) = exchange(
        &base,
        &[("grant_type", "authorization_code"), ("code", &code)],
    )
    .await;

    let info: Value = client()
        .get(format!("{base}/oidc/userinfo"))
        .bearer_auth(tokens["access_token"].as_str().unwrap())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(info["sub"], "ada");
    assert!(info.get("email").is_none(), "{info}");
    assert!(info.get("name").is_none(), "{info}");
    assert_eq!(info["roles"], serde_json::json!(["admin", "user"]));
}

#[tokio::test]
async fn userinfo_answers_a_post_as_well_as_a_get() {
    let base = spawn().await;
    let code = login(
        &base,
        &format!("client_id=x&response_type=code&redirect_uri={ADA_CB}&scope=openid%20email"),
    )
    .await;
    let (_, tokens) = exchange(
        &base,
        &[("grant_type", "authorization_code"), ("code", &code)],
    )
    .await;

    let res = client()
        .post(format!("{base}/oidc/userinfo"))
        .bearer_auth(tokens["access_token"].as_str().unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 200);
    let info: Value = res.json().await.unwrap();
    assert_eq!(info["email"], "ada@example.test");
}

/// A mock that answers an expired token hides the bug in the app that presented
/// it. Every one of these is a `401` carrying `WWW-Authenticate: Bearer`.
#[tokio::test]
async fn userinfo_refuses_a_missing_malformed_unsigned_or_expired_token() {
    let base = spawn().await;

    let bare = client()
        .get(format!("{base}/oidc/userinfo"))
        .send()
        .await
        .unwrap();
    assert_eq!(bare.status().as_u16(), 401);
    assert!(bare.headers()["www-authenticate"]
        .to_str()
        .unwrap()
        .starts_with("Bearer error=\"invalid_token\""));

    for (label, token) in [
        ("garbage", "not-a-token".to_string()),
        (
            "expired",
            seam_token(&base, "?persona=ada&flaw=expired").await,
        ),
        (
            "unsigned",
            seam_token(&base, "?persona=ada&flaw=alg-none").await,
        ),
        (
            "broken signature",
            seam_token(&base, "?persona=ada&flaw=bad-signature").await,
        ),
        (
            "another issuer",
            seam_token(&base, "?persona=ada&flaw=wrong-iss").await,
        ),
    ] {
        let res = client()
            .get(format!("{base}/oidc/userinfo"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status().as_u16(), 401, "{label} was accepted");
        assert!(res.headers().contains_key("www-authenticate"), "{label}");
        let body: Value = res.json().await.unwrap();
        assert_eq!(body["error"], "invalid_token", "{label}");
    }
}

/// A one-off identity from "mint one now" is in no persona file, and its
/// UserInfo response still has to be the person the token says it is.
#[tokio::test]
async fn userinfo_answers_for_an_identity_that_is_in_no_file() {
    let base = spawn().await;
    let http = client();
    let req = start(
        &base,
        &http,
        &format!("client_id=x&response_type=code&redirect_uri={ADA_CB}&scope=openid%20email"),
    )
    .await;
    let res = http
        .post(format!("{base}/_/pick"))
        .form(&[
            ("req", req.as_str()),
            ("sub", "zed"),
            ("email", "zed@example.test"),
            ("attributes", r#"{"department":"ops"}"#),
        ])
        .send()
        .await
        .unwrap();
    let code = support::query_of(&res)["code"].clone();
    let (_, tokens) = exchange(
        &base,
        &[("grant_type", "authorization_code"), ("code", &code)],
    )
    .await;

    let info: Value = client()
        .get(format!("{base}/oidc/userinfo"))
        .bearer_auth(tokens["access_token"].as_str().unwrap())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(info["sub"], "zed");
    assert_eq!(info["email"], "zed@example.test");
    assert_eq!(info["department"], "ops");
    assert!(info.get("iss").is_none(), "no protocol claims: {info}");
    assert!(info.get("exp").is_none(), "{info}");
}

/// The seam is how a flawed token gets minted without going through the CLI.
async fn seam_token(base: &str, query: &str) -> String {
    let body: Value = client()
        .post(format!("{base}/_/api/token{query}"))
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    body["token"].as_str().unwrap().to_string()
}

// -------------------------------------------------------------- sessions --

/// Criterion 18, and the first observable form of "three services, one
/// instance, zero setup between them" (CONCEPT §3).
#[tokio::test]
async fn a_second_login_skips_the_picker_for_that_client_id_only() {
    let base = spawn().await;
    let http = client();

    // First login: the picker.
    let code = login_with(
        &base,
        &http,
        &format!("client_id=billing-web&response_type=code&redirect_uri={ADA_CB}"),
    )
    .await;
    assert!(!code.is_empty());

    // Second, same jar, same client: straight through.
    let res = http
        .get(format!(
            "{base}/oidc/authorize?client_id=billing-web&response_type=code&redirect_uri={ADA_CB}"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 302);
    let location = res.headers()["location"].to_str().unwrap();
    assert!(!location.contains("/_/"), "no picker: {location}");
    assert!(location.contains("code="), "{location}");

    // Same jar, a different client: the picker, because the selection is
    // remembered per client_id and not per browser.
    let res = http
        .get(format!(
            "{base}/oidc/authorize?client_id=spike-php&response_type=code&redirect_uri={ADA_CB}"
        ))
        .send()
        .await
        .unwrap();
    assert!(res.headers()["location"]
        .to_str()
        .unwrap()
        .starts_with("/_/?req="));
}

/// Criterion 4's shape without a browser: two apps, one jar, two people, and
/// neither disturbs the other.
#[tokio::test]
async fn two_apps_in_one_browser_stay_logged_in_as_different_people() {
    let base = spawn().await;
    let http = client();

    for (client_id, persona) in [("billing-web", "ada"), ("spike-php", "mira")] {
        let req = start(
            &base,
            &http,
            &format!("client_id={client_id}&response_type=code&redirect_uri={ADA_CB}"),
        )
        .await;
        http.post(format!("{base}/_/pick"))
            .form(&[("req", req.as_str()), ("persona", persona)])
            .send()
            .await
            .unwrap();
    }

    for (client_id, expected) in [("billing-web", "ada"), ("spike-php", "mira")] {
        let res = http
            .get(format!(
                "{base}/oidc/authorize?client_id={client_id}&response_type=code\
                 &redirect_uri={ADA_CB}&scope=openid"
            ))
            .send()
            .await
            .unwrap();
        let location = res.headers()["location"].to_str().unwrap();
        assert!(
            !location.contains("/_/"),
            "{client_id} saw the picker: {location}"
        );

        let code = support::query_of(&res)["code"].clone();
        let (_, tokens) = exchange(
            &base,
            &[("grant_type", "authorization_code"), ("code", &code)],
        )
        .await;
        assert_eq!(
            payload(tokens["id_token"].as_str().unwrap())["sub"],
            expected,
            "{client_id} is logged in as the wrong person"
        );
    }
}

/// Criterion 22. Sessions are in memory, so a restart logs everybody out — and
/// that is a property worth having, not an omission.
#[tokio::test]
async fn a_cookie_from_a_previous_process_gets_the_picker() {
    let base = spawn().await;
    let http = client();
    login_with(
        &base,
        &http,
        &format!("client_id=billing-web&response_type=code&redirect_uri={ADA_CB}"),
    )
    .await;

    // A different server is what "restarted" looks like from the outside.
    let restarted = spawn().await;
    let res = http
        .get(format!(
            "{restarted}/oidc/authorize?client_id=billing-web&response_type=code\
             &redirect_uri={ADA_CB}"
        ))
        .send()
        .await
        .unwrap();
    assert!(res.headers()["location"]
        .to_str()
        .unwrap()
        .starts_with("/_/?req="));
}

/// Criterion 19.
#[tokio::test]
async fn prompt_login_asks_again_and_prompt_none_never_renders() {
    let base = spawn().await;
    let http = client();
    login_with(
        &base,
        &http,
        &format!("client_id=x&response_type=code&redirect_uri={ADA_CB}"),
    )
    .await;

    for prompt in ["login", "select_account"] {
        let res = http
            .get(format!(
                "{base}/oidc/authorize?client_id=x&response_type=code\
                 &redirect_uri={ADA_CB}&prompt={prompt}"
            ))
            .send()
            .await
            .unwrap();
        assert!(
            res.headers()["location"]
                .to_str()
                .unwrap()
                .starts_with("/_/?req="),
            "prompt={prompt} must show the picker"
        );
    }

    // With a selection: a code, and no HTML anywhere.
    let res = http
        .get(format!(
            "{base}/oidc/authorize?client_id=x&response_type=code\
             &redirect_uri={ADA_CB}&prompt=none"
        ))
        .send()
        .await
        .unwrap();
    let location = res.headers()["location"].to_str().unwrap().to_string();
    assert!(location.contains("code="), "{location}");
    assert!(res.text().await.unwrap().is_empty() || !location.contains("/_/"));

    // With an empty jar: login_required, and a body with no picker in it.
    let empty = client();
    let res = empty
        .get(format!(
            "{base}/oidc/authorize?client_id=x&response_type=code\
             &redirect_uri={ADA_CB}&prompt=none&state=s2"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 302);
    let params = support::query_of(&res);
    assert_eq!(params["error"], "login_required");
    assert_eq!(params["state"], "s2");
    let body = res.text().await.unwrap();
    assert!(!body.contains("persona"), "no picker markup: {body}");
}

/// Criterion 20.
#[tokio::test]
async fn max_age_is_compared_against_the_remembered_auth_time() {
    let base = spawn().await;
    let http = client();
    login_with(
        &base,
        &http,
        &format!("client_id=x&response_type=code&redirect_uri={ADA_CB}"),
    )
    .await;

    // A selection made just now is inside an hour.
    let res = http
        .get(format!(
            "{base}/oidc/authorize?client_id=x&response_type=code\
             &redirect_uri={ADA_CB}&max_age=3600"
        ))
        .send()
        .await
        .unwrap();
    assert!(res.headers()["location"]
        .to_str()
        .unwrap()
        .contains("code="));

    // And outside a window that has already elapsed. RFC-literal: the picker
    // appears when the elapsed time is *greater than* `max_age`, so this waits
    // past a whole second rather than relying on `max_age=0` meaning "always".
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let res = http
        .get(format!(
            "{base}/oidc/authorize?client_id=x&response_type=code\
             &redirect_uri={ADA_CB}&max_age=0"
        ))
        .send()
        .await
        .unwrap();
    assert!(res.headers()["location"]
        .to_str()
        .unwrap()
        .starts_with("/_/?req="));

    // A stale selection plus prompt=none cannot ask, so it says so.
    let res = http
        .get(format!(
            "{base}/oidc/authorize?client_id=x&response_type=code\
             &redirect_uri={ADA_CB}&max_age=0&prompt=none"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(support::query_of(&res)["error"], "login_required");
}

/// Criterion 21. One flag for the whole browser: it covers both client ids, and
/// unchecking it restores the skip.
#[tokio::test]
async fn always_ask_covers_every_client_id_and_can_be_turned_off_again() {
    let base = spawn().await;
    let http = client();

    for client_id in ["billing-web", "spike-php"] {
        let req = start(
            &base,
            &http,
            &format!("client_id={client_id}&response_type=code&redirect_uri={ADA_CB}"),
        )
        .await;
        http.post(format!("{base}/_/pick"))
            .form(&[("req", req.as_str()), ("persona", "ada")])
            .send()
            .await
            .unwrap();
    }

    let skips = |http: reqwest::Client, base: String, client_id: &'static str| async move {
        let res = http
            .get(format!(
                "{base}/oidc/authorize?client_id={client_id}&response_type=code\
                 &redirect_uri={ADA_CB}"
            ))
            .send()
            .await
            .unwrap();
        res.headers()["location"]
            .to_str()
            .unwrap()
            .contains("code=")
    };

    assert!(skips(http.clone(), base.clone(), "billing-web").await);

    // On.
    http.post(format!("{base}/_/session"))
        .form(&[("always_ask", "1")])
        .send()
        .await
        .unwrap();
    assert!(!skips(http.clone(), base.clone(), "billing-web").await);
    assert!(!skips(http.clone(), base.clone(), "spike-php").await);

    // Off — an unchecked checkbox is not submitted at all, which is how HTML
    // says "off".
    http.post(format!("{base}/_/session"))
        .form::<[(&str, &str); 0]>(&[])
        .send()
        .await
        .unwrap();
    assert!(skips(http.clone(), base.clone(), "billing-web").await);
    assert!(skips(http.clone(), base.clone(), "spike-php").await);
}

/// The toggle comes back to where the human was, with the login still waiting.
#[tokio::test]
async fn saving_the_toggle_mid_login_returns_to_the_same_picker() {
    let base = spawn().await;
    let http = client();
    let req = start(
        &base,
        &http,
        &format!("client_id=x&response_type=code&redirect_uri={ADA_CB}"),
    )
    .await;

    let res = http
        .post(format!("{base}/_/session"))
        .form(&[("always_ask", "1"), ("req", req.as_str())])
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 302);
    assert_eq!(
        res.headers()["location"].to_str().unwrap(),
        format!("/_/?req={req}")
    );

    // And the checkbox comes back checked.
    let body = http
        .get(format!("{base}/_/?req={req}"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        body.contains("name=\"always_ask\" value=\"1\" checked"),
        "{body}"
    );
}

/// Criterion 28's second half. Remembering a one-off would mean the session
/// record growing from an id into a claim blob.
#[tokio::test]
async fn a_one_off_identity_is_not_remembered() {
    let base = spawn().await;
    let http = client();
    let req = start(
        &base,
        &http,
        &format!(
            "client_id=x&response_type=code&redirect_uri={ADA_CB}\
             &scope=openid%20email%20profile"
        ),
    )
    .await;
    let res = http
        .post(format!("{base}/_/pick"))
        .form(&[
            ("req", req.as_str()),
            ("sub", "zed"),
            ("name", "Zed Quinn"),
            ("email", "zed@example.test"),
            ("roles", "ops, oncall"),
            ("attributes", r#"{"department":"ops"}"#),
        ])
        .send()
        .await
        .unwrap();

    let code = support::query_of(&res)["code"].clone();
    let (_, tokens) = exchange(
        &base,
        &[("grant_type", "authorization_code"), ("code", &code)],
    )
    .await;
    let id = payload(tokens["id_token"].as_str().unwrap());
    assert_eq!(id["sub"], "zed");
    assert_eq!(id["email"], "zed@example.test");
    assert_eq!(id["department"], "ops");
    assert_eq!(id["roles"], serde_json::json!(["ops", "oncall"]));

    // The next login from that client shows the picker again.
    let res = http
        .get(format!(
            "{base}/oidc/authorize?client_id=x&response_type=code&redirect_uri={ADA_CB}"
        ))
        .send()
        .await
        .unwrap();
    assert!(res.headers()["location"]
        .to_str()
        .unwrap()
        .starts_with("/_/?req="));
}

/// A one-off whose extra claims do not parse is refused, not silently minted
/// without them. A typo that produced a persona missing the claim is the exact
/// failure this project exists not to be.
#[tokio::test]
async fn a_one_off_with_unparseable_extra_claims_is_refused() {
    let base = spawn().await;
    let http = client();
    let req = start(
        &base,
        &http,
        &format!("client_id=x&response_type=code&redirect_uri={ADA_CB}"),
    )
    .await;
    let res = http
        .post(format!("{base}/_/pick"))
        .form(&[
            ("req", req.as_str()),
            ("sub", "zed"),
            ("attributes", "{not json"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 400);
    assert!(res.text().await.unwrap().contains("not valid JSON"));
}

// ------------------------------------------------------------------- CORS --
//
// The minimum a browser public client needs, and the assertions that keep it
// minimum. See `src/oidc/cors.rs` for why this is here a phase early.

#[tokio::test]
async fn the_protocol_endpoints_answer_a_cross_origin_fetch() {
    let base = spawn().await;
    for path in [
        "/oidc/.well-known/openid-configuration",
        "/oidc/jwks",
        "/oidc/userinfo",
    ] {
        let res = client()
            .get(format!("{base}{path}"))
            .header("origin", "http://localhost:5173")
            .send()
            .await
            .unwrap();
        assert_eq!(
            res.headers()["access-control-allow-origin"],
            "http://localhost:5173",
            "{path} is unreachable from a SPA"
        );
        assert_eq!(res.headers()["vary"], "Origin", "{path}");
    }
}

#[tokio::test]
async fn a_preflight_is_answered_without_reaching_a_handler() {
    let base = spawn().await;
    let res = client()
        .request(reqwest::Method::OPTIONS, format!("{base}/oidc/token"))
        .header("origin", "http://localhost:5173")
        .header("access-control-request-method", "POST")
        .header("access-control-request-headers", "content-type")
        .send()
        .await
        .unwrap();

    assert_eq!(res.status().as_u16(), 204, "not a 405");
    assert_eq!(
        res.headers()["access-control-allow-origin"],
        "http://localhost:5173"
    );
    assert!(res.headers()["access-control-allow-headers"]
        .to_str()
        .unwrap()
        .contains("content-type"));
    assert!(res.headers()["access-control-allow-methods"]
        .to_str()
        .unwrap()
        .contains("POST"));
}

/// A public client authenticates with a bearer token, not with lanyard's
/// cookie. Allowing credentialed cross-origin requests would let any page a
/// developer visits drive their session, and this test exists to fail the day
/// somebody adds the header to "fix" something.
#[tokio::test]
async fn cross_origin_requests_are_never_credentialed() {
    let base = spawn().await;
    let res = client()
        .get(format!("{base}/oidc/jwks"))
        .header("origin", "http://evil.example.com")
        .send()
        .await
        .unwrap();
    assert!(res
        .headers()
        .get("access-control-allow-credentials")
        .is_none());
}

/// The UI is for a human and the seam is for a test on this machine. Neither
/// answers another origin, and neither needs to.
#[tokio::test]
async fn the_ui_and_the_seam_are_not_cross_origin_readable() {
    let base = spawn().await;
    for path in ["/_/", "/_/api/personas"] {
        let res = client()
            .get(format!("{base}{path}"))
            .header("origin", "http://localhost:5173")
            .send()
            .await
            .unwrap();
        assert!(
            res.headers().get("access-control-allow-origin").is_none(),
            "{path} must not be readable from another origin"
        );
    }
}

// -------------------------------------------------------- refresh tokens --
//
// `offline_access` gates the refresh token exactly as `openid` gates the ID
// token: a request parameter with a documented effect is not registration, and
// a request without it gets a plain OAuth 2.0 response.

const OFFLINE: &str = "openid%20email%20offline_access";

/// A code flow that asked for `offline_access`, exchanged, in one browser.
async fn login_offline(base: &str, http: &reqwest::Client) -> Value {
    let code = login_with(
        base,
        http,
        &format!(
            "client_id=billing-web&response_type=code&redirect_uri={ADA_CB}\
             &scope={OFFLINE}&nonce=n-orig"
        ),
    )
    .await;
    let (status, body) = exchange(
        base,
        &[("grant_type", "authorization_code"), ("code", &code)],
    )
    .await;
    assert_eq!(status, 200, "{body}");
    body
}

/// Criterion 10. Auth0, Okta and Entra all require `offline_access`; an app
/// that forgets it in production gets no refresh token and breaks, and a
/// lanyard that hands one over unasked hides exactly that bug.
#[tokio::test]
async fn offline_access_gates_the_refresh_token() {
    let base = spawn().await;

    let body = login_offline(&base, &client()).await;
    assert!(
        body["refresh_token"].is_string(),
        "offline_access was asked for: {body}"
    );
    assert_eq!(body["scope"], "openid email offline_access");

    let code = login(
        &base,
        &format!(
            "client_id=billing-web&response_type=code&redirect_uri={ADA_CB}&scope=openid%20email"
        ),
    )
    .await;
    let (_, body) = exchange(
        &base,
        &[("grant_type", "authorization_code"), ("code", &code)],
    )
    .await;
    assert!(
        body.get("refresh_token").is_none(),
        "no offline_access, so not even the key: {body}"
    );
}

/// The refresh token is **opaque**, not a JWT. It is presented to exactly one
/// endpoint, which has the record, and an app that "validates" one has a bug a
/// JWT-shaped refresh token would hide.
#[tokio::test]
async fn the_refresh_token_is_opaque_and_unguessable() {
    let base = spawn().await;
    let one = login_offline(&base, &client()).await;
    let two = login_offline(&base, &client()).await;

    let token = one["refresh_token"].as_str().unwrap();
    assert!(!token.contains('.'), "not a compact JWS: {token}");
    assert!(token.len() >= 32, "a v4 uuid's worth of unguessability");
    assert_ne!(token, two["refresh_token"].as_str().unwrap());
}

/// Criterion 14. RFC 6749 §4.4.3 says a client credentials response MUST NOT
/// include a refresh token, and the reason is good: there is no user, so there
/// is nothing a refresh could be on behalf of.
#[tokio::test]
async fn client_credentials_never_returns_a_refresh_token() {
    let base = spawn().await;
    let (status, body) = exchange(
        &base,
        &[
            ("grant_type", "client_credentials"),
            ("persona", "ada"),
            ("scope", "offline_access"),
        ],
    )
    .await;

    assert_eq!(status, 200, "{body}");
    let mut keys: Vec<&str> = body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["access_token", "expires_in", "scope", "token_type"],
        "the `scope` key is Phase 2's echo; there is no refresh_token"
    );
}

/// Criterion 11. **North star 3**: a refreshed token's claims come out of
/// `issue()` like every other token's, so the only differences from the
/// original are the ones OIDC Core §12.2 requires.
#[tokio::test]
async fn the_refresh_grant_mints_a_new_access_token_and_a_new_id_token() {
    use sha2::{Digest as _, Sha256};

    let base = spawn().await;
    let first = login_offline(&base, &client()).await;
    let original_id = payload(first["id_token"].as_str().unwrap());

    let (status, body) = exchange(
        &base,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", first["refresh_token"].as_str().unwrap()),
        ],
    )
    .await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(body["token_type"], "Bearer");
    assert_eq!(body["expires_in"], 60, "60 seconds, as always");
    assert_eq!(body["scope"], "openid email offline_access");
    assert_ne!(
        body["access_token"], first["access_token"],
        "a new token, not the old one handed back"
    );

    let access = payload(body["access_token"].as_str().unwrap());
    assert_eq!(access["sub"], "ada");
    assert_eq!(access["client_id"], "billing-web");

    let id = payload(body["id_token"].as_str().unwrap());
    assert_eq!(id["sub"], original_id["sub"]);
    assert_eq!(
        id["auth_time"], original_id["auth_time"],
        "a refresh is not a new login"
    );
    assert_eq!(
        id["nonce"], "n-orig",
        "OIDC Core §12.2: the nonce of the original authorization request"
    );
    assert_eq!(id["aud"], "billing-web");
    assert_eq!(
        id["at_hash"],
        lanyard_cli::b64::encode(
            &Sha256::digest(body["access_token"].as_str().unwrap().as_bytes())[..16]
        ),
        "hashed over the NEW access token"
    );
    assert!(
        id.get("c_hash").is_none(),
        "there is no code this time: {id}"
    );
}

/// Criterion 12. Rotation is what Auth0, Okta and Entra do for public clients,
/// and it is the strictly more demanding shape: an app that handles rotation
/// handles a static token too.
#[tokio::test]
async fn every_refresh_rotates_and_a_replay_is_told_it_was_already_exchanged() {
    let base = spawn().await;
    let first = login_offline(&base, &client()).await;
    let presented = first["refresh_token"].as_str().unwrap().to_string();

    let (_, body) = exchange(
        &base,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &presented),
        ],
    )
    .await;
    let rotated = body["refresh_token"].as_str().unwrap().to_string();
    assert_ne!(
        rotated, presented,
        "every successful refresh returns a new one"
    );

    // The new one works.
    let (status, _) = exchange(
        &base,
        &[("grant_type", "refresh_token"), ("refresh_token", &rotated)],
    )
    .await;
    assert_eq!(status, 200);

    // The presented one does not, and says why.
    let (status, body) = exchange(
        &base,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &presented),
        ],
    )
    .await;
    assert_eq!(status, 400);
    assert_eq!(body["error"], "invalid_grant");
    let description = body["error_description"].as_str().unwrap();
    assert!(
        description.contains("already been exchanged"),
        "a replay has to be told apart from an unknown token: {description}"
    );
}

/// Four distinct `invalid_grant` descriptions, because Phase 6's log can only
/// report what this phase distinguishes.
#[tokio::test]
async fn the_four_refusals_of_the_refresh_grant_are_told_apart() {
    let base = spawn().await;

    let (status, body) = exchange(&base, &[("grant_type", "refresh_token")]).await;
    assert_eq!(status, 400);
    assert_eq!(body["error"], "invalid_request", "{body}");
    assert!(body["error_description"]
        .as_str()
        .unwrap()
        .contains("refresh_token is required"));

    let (status, body) = exchange(
        &base,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", "nobody-issued-this"),
        ],
    )
    .await;
    assert_eq!(status, 400);
    assert_eq!(body["error"], "invalid_grant");
    assert!(
        body["error_description"]
            .as_str()
            .unwrap()
            .contains("no such refresh token"),
        "{body}"
    );

    // Criterion 18: revoked is its own answer, not "unknown".
    let issued = login_offline(&base, &client()).await;
    let token = issued["refresh_token"].as_str().unwrap();
    let res = client()
        .post(format!("{base}/oidc/revoke"))
        .form(&[("token", token)])
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 200);

    let (status, body) = exchange(
        &base,
        &[("grant_type", "refresh_token"), ("refresh_token", token)],
    )
    .await;
    assert_eq!(status, 400);
    assert_eq!(body["error"], "invalid_grant");
    assert!(
        body["error_description"]
            .as_str()
            .unwrap()
            .contains("revoked"),
        "{body}"
    );
}

/// The expired arm, on a server whose refresh tokens die on arrival — so the
/// description is covered without an eight-hour test.
#[tokio::test]
async fn an_expired_refresh_token_says_expired_rather_than_unknown() {
    let base = support::spawn_configured(
        lanyard_cli::persona::Personas::builtin(),
        lanyard_cli::store::Stores::with_ttls(
            std::time::Duration::from_secs(300),
            std::time::Duration::from_secs(60),
            std::time::Duration::ZERO,
        ),
    )
    .await;

    let issued = login_offline(&base, &client()).await;
    let (status, body) = exchange(
        &base,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", issued["refresh_token"].as_str().unwrap()),
        ],
    )
    .await;

    assert_eq!(status, 400);
    assert_eq!(body["error"], "invalid_grant");
    let description = body["error_description"].as_str().unwrap();
    assert!(description.contains("expired"), "{description}");
    assert!(
        description.contains("eight hours"),
        "the lifetime is in the message a developer reads: {description}"
    );
}

/// No `openid` in the original grant means no ID token to refresh — the same
/// gate, read again from the stored scope rather than from a second decision.
#[tokio::test]
async fn a_refresh_of_a_grant_without_openid_returns_no_id_token() {
    let base = spawn().await;
    let code = login(
        &base,
        &format!(
            "client_id=billing-web&response_type=code&redirect_uri={ADA_CB}\
             &scope=email%20offline_access"
        ),
    )
    .await;
    let (_, body) = exchange(
        &base,
        &[("grant_type", "authorization_code"), ("code", &code)],
    )
    .await;
    assert!(body.get("id_token").is_none(), "{body}");

    let (status, body) = exchange(
        &base,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", body["refresh_token"].as_str().unwrap()),
        ],
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert!(body["access_token"].is_string());
    assert!(body.get("id_token").is_none(), "{body}");
}

/// Client authentication is accepted in any form and checked in none, on this
/// arm too.
#[tokio::test]
async fn the_refresh_grant_accepts_any_client_authentication() {
    let base = spawn().await;
    let issued = login_offline(&base, &client()).await;
    let (status, body) = exchange(
        &base,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", issued["refresh_token"].as_str().unwrap()),
            ("client_id", "whoever"),
            ("client_secret", "nonsense"),
        ],
    )
    .await;
    assert_eq!(status, 200, "{body}");
}

/// Criterion 13. RFC 6749 §6: a refresh may narrow the grant and may not widen
/// it. The same class of check as PKCE — cryptographically free, catches a real
/// bug, and nothing to do with registration.
#[tokio::test]
async fn a_refresh_may_narrow_the_scope_and_the_response_echoes_the_narrowed_one() {
    let base = spawn().await;
    let issued = login_offline(&base, &client()).await;

    let (status, body) = exchange(
        &base,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", issued["refresh_token"].as_str().unwrap()),
            ("scope", "openid"),
        ],
    )
    .await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(body["scope"], "openid", "the narrowed set is echoed");
    assert_eq!(
        payload(body["access_token"].as_str().unwrap())["scope"],
        "openid",
        "and it is what the token carries"
    );
    // `email` was granted and not asked for this time, so the ID token must not
    // carry it: the narrowed scope is the filter too.
    let id = payload(body["id_token"].as_str().unwrap());
    assert!(id.get("email").is_none(), "{id}");
    assert!(
        body["refresh_token"].is_string(),
        "a narrowed refresh still rotates"
    );
}

/// Widening is `400 invalid_scope` **naming the offending value**, because a
/// developer reading it has to know which of the scopes they sent was the
/// problem.
#[tokio::test]
async fn a_refresh_may_not_widen_the_scope_and_the_refusal_names_it() {
    let base = spawn().await;
    let issued = login_offline(&base, &client()).await;
    let token = issued["refresh_token"].as_str().unwrap();

    let (status, body) = exchange(
        &base,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", token),
            ("scope", "openid admin"),
        ],
    )
    .await;

    assert_eq!(status, 400);
    assert_eq!(body["error"], "invalid_scope");
    let description = body["error_description"].as_str().unwrap();
    assert!(description.contains("admin"), "{description}");
    assert!(
        description.contains("openid email offline_access"),
        "and says what was granted: {description}"
    );

    // A refusal must not spend the token: the grant is unchanged, so the
    // presented refresh token still works.
    let (status, _) = exchange(
        &base,
        &[("grant_type", "refresh_token"), ("refresh_token", token)],
    )
    .await;
    assert_eq!(status, 200, "a rejected narrowing is not a rotation");
}

// --------------------------------------------------------- RP-initiated --
//
// `/oidc/end_session`. **Logging out never fails**: no session, an unknown
// session, a session from before a restart — all of them are the same redirect
// or the same page. There is no state in which a logout produces an error the
// user has to read, except the one rejection.

const LOGOUT_CB: &str = "http://localhost:5000/signout-callback-oidc";

async fn end_session(base: &str, http: &reqwest::Client, query: &str) -> reqwest::Response {
    http.get(format!("{base}/oidc/end_session?{query}"))
        .send()
        .await
        .unwrap()
}

/// Whether this browser would see the picker for `client_id`, which is the only
/// observable difference a logout makes to an RP.
async fn sees_the_picker(base: &str, http: &reqwest::Client, client_id: &str) -> bool {
    let res = http
        .get(format!(
            "{base}/oidc/authorize?client_id={client_id}&response_type=code\
             &redirect_uri={ADA_CB}&scope=openid"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 302);
    res.headers()["location"]
        .to_str()
        .unwrap()
        .starts_with("/_/?req=")
}

fn cleared_cookie(res: &reqwest::Response) -> bool {
    res.headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .any(|v| v.starts_with("lanyard_session=;") && v.contains("Max-Age=0"))
}

/// Criteria 1–3, as far as a test can go without a browser: the redirect back
/// to the RP, the expired cookie, and **the picker** on the next login.
#[tokio::test]
async fn end_session_redirects_home_expires_the_cookie_and_forgets_the_browser() {
    let base = spawn().await;
    let http = client();
    login_with(
        &base,
        &http,
        &format!("client_id=billing-web&response_type=code&redirect_uri={ADA_CB}&scope=openid"),
    )
    .await;
    assert!(
        !sees_the_picker(&base, &http, "billing-web").await,
        "the selection is remembered before the logout"
    );

    let res = end_session(
        &base,
        &http,
        &format!("post_logout_redirect_uri={}", urlencode(LOGOUT_CB)),
    )
    .await;

    assert_eq!(res.status().as_u16(), 302);
    assert_eq!(res.headers()["location"], LOGOUT_CB);
    assert!(cleared_cookie(&res), "{:?}", res.headers());
    assert!(
        sees_the_picker(&base, &http, "billing-web").await,
        "criterion 1: the next login shows the picker rather than signing you \
         straight back in"
    );
}

/// Criterion 6, and the decision on the record: **it drops everything.** Every
/// real IdP has one SSO session and clears all of it, and the cost — logging
/// out of one app logs you out of the other two — is made visible on purpose.
#[tokio::test]
async fn logging_out_from_one_client_clears_the_whole_browser_session() {
    let base = spawn().await;
    let http = client();
    for client_id in ["billing-web", "spike-php"] {
        login_with(
            &base,
            &http,
            &format!("client_id={client_id}&response_type=code&redirect_uri={ADA_CB}&scope=openid"),
        )
        .await;
    }

    end_session(
        &base,
        &http,
        &format!("post_logout_redirect_uri={}", urlencode(LOGOUT_CB)),
    )
    .await;

    assert!(sees_the_picker(&base, &http, "billing-web").await);
    assert!(
        sees_the_picker(&base, &http, "spike-php").await,
        "one SSO session, cleared all at once — that is the cost, on purpose"
    );
}

/// Criterion 5. The one rejection, applied to a second parameter — **and it
/// logs nobody out**, because a rejected logout is a request lanyard never
/// acted on.
#[tokio::test]
async fn a_non_loopback_post_logout_redirect_uri_is_rendered_and_logs_nobody_out() {
    let base = spawn().await;

    for bad in [
        "https://evil.example.com/",
        // Resolves to 127.0.0.1 and is rejected anyway: the host is compared
        // literally and no DNS lookup happens.
        "http://web.localtest.me:5000/",
    ] {
        let http = client();
        login_with(
            &base,
            &http,
            &format!("client_id=billing-web&response_type=code&redirect_uri={ADA_CB}&scope=openid"),
        )
        .await;

        let res = end_session(
            &base,
            &http,
            &format!("post_logout_redirect_uri={}", urlencode(bad)),
        )
        .await;

        assert_eq!(res.status().as_u16(), 400, "{bad}");
        assert!(res.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html"));
        assert!(
            res.headers().get("location").is_none(),
            "no Location: the browser stays on lanyard"
        );
        let body = res.text().await.unwrap();
        assert!(body.contains(&lanyard_cli::ui::html::escape(bad)), "{body}");
        assert!(body.contains("loopback"), "the rule is stated: {body}");
        assert!(
            body.contains("post_logout_redirect_uri"),
            "and names the parameter it is about: {body}"
        );

        assert!(
            !sees_the_picker(&base, &http, "billing-web").await,
            "a rejected logout logs nobody out"
        );
    }
}

/// Criterion 7. An RP that sends no return address has to land somewhere, and
/// landing on a blank page or a bare `302` to `/` would be lanyard being
/// unhelpful at exactly the moment a developer is watching.
#[tokio::test]
async fn with_no_return_address_lanyard_renders_its_own_signed_out_page() {
    let base = spawn().await;
    let http = client();
    login_with(
        &base,
        &http,
        &format!("client_id=billing-web&response_type=code&redirect_uri={ADA_CB}&scope=openid"),
    )
    .await;

    let res = end_session(&base, &http, "").await;
    assert_eq!(res.status().as_u16(), 200);
    assert!(res.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/html"));
    assert!(cleared_cookie(&res));
    assert!(res.headers().get("location").is_none());

    let body = res.text().await.unwrap();
    assert!(body.contains("signed out"), "{body}");
    assert!(body.contains("/_/"), "and links to the picker: {body}");
    assert!(sees_the_picker(&base, &http, "billing-web").await);
}

/// Criterion 8. Echoed byte-for-byte when sent, absent when not. Never
/// invented — an RP that did not send `state` and receives one has been told
/// something untrue about its own request.
#[tokio::test]
async fn state_is_echoed_byte_for_byte_and_never_invented() {
    let base = spawn().await;
    let http = client();
    let awkward = "a b/c&d=e%2F";

    let res = end_session(
        &base,
        &http,
        &format!(
            "post_logout_redirect_uri={}&state={}",
            urlencode(LOGOUT_CB),
            urlencode(awkward)
        ),
    )
    .await;
    let location = res.headers()["location"].to_str().unwrap();
    let url = url::Url::parse(location).unwrap();
    assert_eq!(
        url.query_pairs()
            .find(|(k, _)| k == "state")
            .map(|(_, v)| v.into_owned()),
        Some(awkward.to_string()),
        "{location}"
    );

    let res = end_session(
        &base,
        &http,
        &format!("post_logout_redirect_uri={}", urlencode(LOGOUT_CB)),
    )
    .await;
    assert_eq!(
        res.headers()["location"],
        LOGOUT_CB,
        "no state was sent, so there is no state parameter at all"
    );
}

/// Criterion 9. `id_token_hint` is **read, never required**: absent, expired,
/// foreign and garbage all log out identically, and none of them renders a
/// confirmation screen.
#[tokio::test]
async fn an_absent_expired_or_garbage_id_token_hint_all_log_out_identically() {
    let base = spawn().await;
    let expired = {
        let res = client()
            .post(format!("{base}/_/api/token?persona=ada&flaw=expired"))
            .header("content-type", "application/json")
            .body("{}")
            .send()
            .await
            .unwrap();
        res.json::<Value>().await.unwrap()["token"]
            .as_str()
            .unwrap()
            .to_string()
    };

    for hint in [
        String::new(),
        format!("&id_token_hint={expired}"),
        "&id_token_hint=garbage".to_string(),
    ] {
        let http = client();
        login_with(
            &base,
            &http,
            &format!("client_id=billing-web&response_type=code&redirect_uri={ADA_CB}&scope=openid"),
        )
        .await;

        let res = end_session(
            &base,
            &http,
            &format!("post_logout_redirect_uri={}{hint}", urlencode(LOGOUT_CB)),
        )
        .await;
        assert_eq!(res.status().as_u16(), 302, "hint {hint:?}");
        assert_eq!(res.headers()["location"], LOGOUT_CB);
        assert!(sees_the_picker(&base, &http, "billing-web").await);
    }
}

/// RP-Initiated Logout §2 allows both verbs, .NET sends the first and some SDKs
/// send the second.
#[tokio::test]
async fn end_session_answers_post_as_well_as_get() {
    let base = spawn().await;
    let http = client();
    let res = http
        .post(format!("{base}/oidc/end_session"))
        .form(&[("post_logout_redirect_uri", LOGOUT_CB)])
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 302);
    assert_eq!(res.headers()["location"], LOGOUT_CB);
}

/// Logging out with no session at all is the same answer, not an error.
#[tokio::test]
async fn logging_out_a_browser_that_was_never_logged_in_is_not_an_error() {
    let base = spawn().await;
    let res = end_session(
        &base,
        &client(),
        &format!("post_logout_redirect_uri={}", urlencode(LOGOUT_CB)),
    )
    .await;
    assert_eq!(res.status().as_u16(), 302);
    assert!(cleared_cookie(&res));
}

/// Criterion 19, and open question 2: a "full logout" that leaves a working
/// refresh token in an app's local storage is not one.
#[tokio::test]
async fn logging_out_revokes_the_refresh_tokens_that_session_was_issued() {
    let base = spawn().await;
    let http = client();
    let mine = login_offline(&base, &http).await;
    let theirs = login_offline(&base, &client()).await;

    end_session(
        &base,
        &http,
        &format!("post_logout_redirect_uri={}", urlencode(LOGOUT_CB)),
    )
    .await;

    let introspect = |token: String| {
        let base = base.clone();
        async move {
            client()
                .post(format!("{base}/oidc/introspect"))
                .form(&[("token", token.as_str())])
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    };

    assert_eq!(
        introspect(mine["refresh_token"].as_str().unwrap().to_string()).await,
        serde_json::json!({ "active": false }),
        "the session's refresh token went with it"
    );
    assert_eq!(
        introspect(theirs["refresh_token"].as_str().unwrap().to_string()).await["active"],
        true,
        "another browser's did not"
    );

    let (status, body) = exchange(
        &base,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", mine["refresh_token"].as_str().unwrap()),
        ],
    )
    .await;
    assert_eq!(status, 400);
    assert!(
        body["error_description"]
            .as_str()
            .unwrap()
            .contains("revoked"),
        "{body}"
    );
}

// ------------------------------------------------------- the /_/ controls --

/// The same login, as whoever is asked for.
async fn login_as(base: &str, http: &reqwest::Client, client_id: &str, persona: &str) {
    let req = start(
        base,
        http,
        &format!("client_id={client_id}&response_type=code&redirect_uri={ADA_CB}&scope=openid"),
    )
    .await;
    let res = http
        .post(format!("{base}/_/pick"))
        .form(&[("req", req.as_str()), ("persona", persona)])
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 302, "the pick should complete");
}

async fn picker_page(base: &str, http: &reqwest::Client) -> String {
    http.get(format!("{base}/_/"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap()
}

/// Criterion 20. One row per `client_id`, naming the persona — and an honest
/// empty state, because the banner's `UI →` line lands on this page too.
#[tokio::test]
async fn the_this_browser_panel_lists_one_row_per_client_id() {
    let base = spawn().await;
    let http = client();

    let empty = picker_page(&base, &http).await;
    assert!(empty.contains("This browser"), "{empty}");
    assert!(
        empty.contains("not signed in to any application"),
        "the empty state has to say so plainly: {empty}"
    );

    login_as(&base, &http, "billing-web", "ada").await;
    login_as(&base, &http, "spike-php", "mira").await;

    let body = picker_page(&base, &http).await;
    assert!(body.contains("billing-web"), "{body}");
    assert!(body.contains("Ada Bell"), "{body}");
    assert!(body.contains("spike-php"), "{body}");
    assert!(body.contains("Mira Okonkwo"), "{body}");
    assert!(
        !body.contains("not signed in to any application"),
        "the empty state is gone once there is something to show"
    );
    // Rendered on the mid-login version of the page too.
    let req = start(
        &base,
        &http,
        &format!("client_id=third-app&response_type=code&redirect_uri={ADA_CB}"),
    )
    .await;
    let mid = http
        .get(format!("{base}/_/?req={req}"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        mid.contains("billing-web") && mid.contains("Ada Bell"),
        "{mid}"
    );
}

/// A `client_id` is developer-supplied and goes into the page like everything
/// else on it: escaped. A picker that executes its own session record is the
/// same bad look as one that executes its persona list.
#[tokio::test]
async fn a_client_id_shaped_like_a_script_tag_renders_as_text() {
    let base = spawn().await;
    let http = client();
    login_as(&base, &http, "%3Cscript%3Ealert(1)%3C%2Fscript%3E", "ada").await;

    let body = picker_page(&base, &http).await;
    assert!(!body.contains("<script>alert(1)</script>"), "{body}");
    assert!(
        body.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
        "{body}"
    );
}

async fn control(
    base: &str,
    http: &reqwest::Client,
    path: &str,
    fields: &[(&str, &str)],
) -> reqwest::Response {
    let res = http
        .post(format!("{base}/_/{path}"))
        .form(fields)
        .send()
        .await
        .unwrap();
    assert_eq!(
        res.status().as_u16(),
        302,
        "a control redirects back to /_/"
    );
    res
}

async fn introspection(base: &str, token: &str) -> Value {
    client()
        .post(format!("{base}/oidc/introspect"))
        .form(&[("token", token)])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

/// Criterion 21. The whole session: every selection, and every refresh token it
/// was issued.
#[tokio::test]
async fn log_out_of_lanyard_drops_the_whole_session() {
    let base = spawn().await;
    let http = client();
    let issued = login_offline(&base, &http).await;
    login_as(&base, &http, "spike-php", "mira").await;

    let res = control(&base, &http, "logout", &[]).await;
    assert_eq!(res.headers()["location"], "/_/");
    assert!(cleared_cookie(&res), "{:?}", res.headers());

    assert!(sees_the_picker(&base, &http, "billing-web").await);
    assert!(sees_the_picker(&base, &http, "spike-php").await);
    assert_eq!(
        introspection(&base, issued["refresh_token"].as_str().unwrap()).await,
        serde_json::json!({ "active": false })
    );

    let body = picker_page(&base, &http).await;
    assert!(
        body.contains("not signed in to any application"),
        "the This browser section is empty: {body}"
    );
}

/// Criterion 22. The per-`client_id` variant lives **here**, on lanyard's own
/// UI where a developer reaches for it deliberately, and not on `/end_session`
/// where an SDK would reach it by accident.
#[tokio::test]
async fn forget_drops_one_client_and_leaves_the_others() {
    let base = spawn().await;
    let http = client();
    login_as(&base, &http, "billing-web", "ada").await;
    login_as(&base, &http, "spike-php", "mira").await;

    control(&base, &http, "forget", &[("client_id", "billing-web")]).await;

    assert!(sees_the_picker(&base, &http, "billing-web").await);
    assert!(
        !sees_the_picker(&base, &http, "spike-php").await,
        "the other row is untouched"
    );
}

/// Criterion 23, and CONCEPT §6's control. **The tokens died, the person did
/// not** — keeping the selection is the whole point, because the question it
/// answers is "what does my application do when its token dies", not "what does
/// the picker look like".
#[tokio::test]
async fn expire_now_kills_the_tokens_and_keeps_the_selection() {
    let base = spawn().await;
    let http = client();
    let issued = login_offline(&base, &http).await;
    login_as(&base, &http, "spike-php", "mira").await;
    let other = {
        let http = client();
        login_offline(&base, &http).await
    };

    let access = issued["access_token"].as_str().unwrap();
    assert_eq!(introspection(&base, access).await["active"], true);

    control(&base, &http, "expire", &[("client_id", "billing-web")]).await;

    assert_eq!(
        introspection(&base, access).await,
        serde_json::json!({ "active": false }),
        "the access token this browser holds is dead"
    );
    let res = client()
        .get(format!("{base}/oidc/userinfo"))
        .header("authorization", format!("Bearer {access}"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 401, "and /userinfo agrees");

    assert_eq!(
        introspection(&base, issued["refresh_token"].as_str().unwrap()).await,
        serde_json::json!({ "active": false }),
        "so is its refresh token — otherwise the app renews straight past this"
    );

    assert!(
        !sees_the_picker(&base, &http, "billing-web").await,
        "criterion 23: a fresh /authorize still returns a code with no picker"
    );
    assert!(
        !sees_the_picker(&base, &http, "spike-php").await,
        "and the other application is untouched"
    );
    assert_eq!(
        introspection(&base, other["access_token"].as_str().unwrap()).await["active"],
        true,
        "as is another browser's token for the same client_id"
    );
}

/// A control carries a login in progress across itself, so pressing one in the
/// middle of a login does not abandon it.
#[tokio::test]
async fn a_control_pressed_mid_login_comes_back_to_the_login() {
    let base = spawn().await;
    let http = client();
    let req = start(
        &base,
        &http,
        &format!("client_id=billing-web&response_type=code&redirect_uri={ADA_CB}"),
    )
    .await;

    let res = control(
        &base,
        &http,
        "forget",
        &[("client_id", "billing-web"), ("req", &req)],
    )
    .await;
    assert_eq!(res.headers()["location"], format!("/_/?req={req}"));
}

/// The three controls appear on the page, as plain forms. No JavaScript, same
/// as everything else here — and `SameSite=Lax` is why none of them needs a
/// CSRF token, the same reasoning Phase 4 recorded for `/_/pick`.
#[tokio::test]
async fn the_controls_are_rendered_as_plain_forms_with_no_script() {
    let base = spawn().await;
    let http = client();
    login_as(&base, &http, "billing-web", "ada").await;

    let body = picker_page(&base, &http).await;
    for action in ["/_/logout", "/_/forget", "/_/expire"] {
        assert!(
            body.contains(&format!("action=\"{action}\"")),
            "{action}: {body}"
        );
    }
    assert!(body.contains("Log out of lanyard"), "{body}");
    assert!(body.contains(">Forget<"), "{body}");
    assert!(body.contains(">Expire now<"), "{body}");
    assert!(!body.contains("<script"), "no script anywhere on this page");
}
