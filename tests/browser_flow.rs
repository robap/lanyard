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
