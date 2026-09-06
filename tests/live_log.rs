//! The live request log: the one emit point, what it excludes, and the two
//! endpoints that serve the stream.

mod support;

use support::{logged, spawn_logged};

/// How long an assertion that expects **no** frames waits before believing it.
/// Everything under test here is in-process and on loopback, so a frame that
/// has not arrived in this long was never sent.
const QUIET: std::time::Duration = std::time::Duration::from_millis(400);

/// **One request, one event.** The middleware is the only thing that publishes,
/// so the count is the assertion that matters as much as the contents.
#[tokio::test]
async fn a_protocol_request_publishes_exactly_one_event() {
    let (base, events) = spawn_logged().await;

    support::client()
        .post(format!("{base}/oidc/token"))
        .form(&[
            ("grant_type", "client_credentials"),
            ("client_id", "billing-web"),
            ("persona", "ada"),
        ])
        .send()
        .await
        .unwrap();

    let seen = logged(&events);
    assert_eq!(seen.len(), 1, "one request, one event: {seen:#?}");
    let event = &seen[0];
    assert_eq!(event.method, "POST");
    assert_eq!(event.endpoint, "/oidc/token");
    assert_eq!(event.status, 200);
    assert_eq!(event.id, 1);
    assert!(event.ts > 0, "a wall-clock stamp");
}

/// Discovery and JWKS are static, and an SDK polls them: logging either would
/// drown every event a developer opened the log to read.
#[tokio::test]
async fn discovery_and_jwks_are_excluded() {
    let (base, events) = spawn_logged().await;

    for path in ["/oidc/.well-known/openid-configuration", "/oidc/jwks"] {
        support::client()
            .get(format!("{base}{path}"))
            .send()
            .await
            .unwrap();
    }

    assert!(logged(&events).is_empty(), "{:#?}", logged(&events));
}

/// **The `/_/` surface is a page, not a decision.** Only `POST /_/pick` and the
/// seam's `POST /_/api/token` emit; the picker, the session controls, the log
/// page, its assets and — load-bearingly — the event stream itself do not.
#[tokio::test]
async fn the_ui_surface_is_excluded_except_the_two_routes_that_decide_something() {
    let (base, events) = spawn_logged().await;

    for path in [
        "/_/",
        "/_/log",
        "/_/api/events",
        "/_/api/personas",
        "/_/assets/app.deadbeef.css",
        "/_/.zero/fonts/Geist.woff2",
    ] {
        support::client()
            .get(format!("{base}{path}"))
            .timeout(std::time::Duration::from_millis(300))
            .send()
            .await
            .ok();
    }

    assert!(logged(&events).is_empty(), "{:#?}", logged(&events));
}

/// **The one funnel.** Every `invalid_grant` Phases 4 and 5 wrote passes
/// through `bad_request`, so attaching there is what makes "no error can be
/// returned without being logged" true rather than aspirational — and the whole
/// sentence reaches stdout for a developer who never opens the UI.
#[tokio::test]
async fn a_refusal_carries_its_error_and_the_whole_description() {
    let (base, events) = spawn_logged().await;

    support::client()
        .post(format!("{base}/oidc/token"))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", "never-issued"),
            ("client_id", "billing-web"),
        ])
        .send()
        .await
        .unwrap();

    let seen = logged(&events);
    assert_eq!(seen.len(), 1);
    let event = &seen[0];
    assert_eq!(event.status, 400);
    assert_eq!(event.error.as_deref(), Some("invalid_grant"));
    assert_eq!(
        event.error_description.as_deref(),
        Some(
            "no such authorization code; it was never issued, or lanyard \
             was restarted since it was"
        )
    );
    assert!(
        lanyard_cli::events::pretty(event).contains("lanyard was restarted since it was"),
        "the stdout line carries the sentence whole: {}",
        lanyard_cli::events::pretty(event)
    );
}

/// Criterion 1's token half and criterion 15's first half. `client_id`,
/// `grant_type`, the decoded form with `scope` as a **list**, the flaw that was
/// asked for, and the decoded header and payload of what came out.
#[tokio::test]
async fn a_mint_carries_the_grant_the_parameters_and_what_it_issued() {
    let (base, events) = spawn_logged().await;

    support::client()
        .post(format!("{base}/oidc/token"))
        .form(&[
            ("grant_type", "client_credentials"),
            ("client_id", "lanyard-cli"),
            ("persona", "ada"),
            ("audience", "billing-api"),
            ("scope", "openid email profile"),
            ("flaw", "alg-none"),
        ])
        .send()
        .await
        .unwrap();

    let seen = logged(&events);
    assert_eq!(seen.len(), 1);
    let event = &seen[0];
    assert_eq!(event.client_id.as_deref(), Some("lanyard-cli"));
    assert_eq!(event.grant_type.as_deref(), Some("client_credentials"));
    assert_eq!(event.flaw.as_deref(), Some("alg-none"));

    let request = event.request.as_ref().expect("the decoded parameters");
    assert_eq!(request["persona"], "ada");
    assert_eq!(
        request["scope"],
        serde_json::json!(["openid", "email", "profile"]),
        "scope is a list, not a raw string"
    );

    let issued = event.issued.as_ref().expect("what was minted");
    let access = &issued["access_token"];
    assert_eq!(
        access["header"]["alg"], "none",
        "the flaw is visible in the header that was actually emitted"
    );
    assert_eq!(access["payload"]["sub"], "ada");
    assert_eq!(access["payload"]["aud"], "billing-api");

    let line = lanyard_cli::events::pretty(event);
    assert!(line.contains("client_credentials"), "{line}");
    assert!(line.contains("scope=openid,email,profile"), "{line}");
    assert!(line.contains("flaw=alg-none"), "{line}");
}

/// A browser login's exchange: the `client_id` comes off the code record, not
/// off a form field the RP may not have resent, and the ID token is decoded
/// beside the access token.
#[tokio::test]
async fn an_authorization_code_exchange_names_the_client_and_both_tokens() {
    let (base, events) = spawn_logged().await;
    let code = support::login(&base, "ada", "billing-web", "openid email").await;

    support::client()
        .post(format!("{base}/oidc/token"))
        .form(&[("grant_type", "authorization_code"), ("code", &code)])
        .send()
        .await
        .unwrap();

    let exchange = logged(&events)
        .into_iter()
        .find(|e| e.endpoint == "/oidc/token")
        .expect("the token event");
    assert_eq!(exchange.client_id.as_deref(), Some("billing-web"));
    assert_eq!(exchange.grant_type.as_deref(), Some("authorization_code"));
    let issued = exchange.issued.as_ref().expect("what was minted");
    assert_eq!(issued["access_token"]["header"]["alg"], "RS256");
    assert_eq!(issued["id_token"]["payload"]["aud"], "billing-web");
    assert_eq!(issued["id_token"]["payload"]["email"], "ada@example.test");
}

/// Criterion 10's payload. A real code, a `code_verifier` one character off,
/// and the event stacks all three values — the verifier presented, the S256
/// computed from it, and the challenge recorded at `/authorize`. The UI only
/// has to render this.
#[tokio::test]
async fn a_pkce_mismatch_carries_all_three_values() {
    use base64::Engine as _;
    use sha2::Digest as _;

    let (base, events) = spawn_logged().await;

    let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(sha2::Sha256::digest(verifier.as_bytes()));

    let client = support::client();
    let res = client
        .get(format!(
            "{base}/oidc/authorize?client_id=billing-web&redirect_uri={}\
             &response_type=code&code_challenge={challenge}&code_challenge_method=S256",
            support::ADA_CB
        ))
        .send()
        .await
        .unwrap();
    let req = res.headers()["location"]
        .to_str()
        .unwrap()
        .split("req=")
        .nth(1)
        .unwrap()
        .to_owned();
    let res = client
        .post(format!("{base}/_/pick"))
        .form(&[("req", req.as_str()), ("persona", "ada")])
        .send()
        .await
        .unwrap();
    let code = url::Url::parse(res.headers()["location"].to_str().unwrap())
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == "code")
        .map(|(_, v)| v.into_owned())
        .unwrap();

    // One character off, which is the failure a developer actually has.
    let wrong = format!("{}X", &verifier[..verifier.len() - 1]);
    client
        .post(format!("{base}/oidc/token"))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("code_verifier", &wrong),
        ])
        .send()
        .await
        .unwrap();

    let exchange = logged(&events)
        .into_iter()
        .find(|e| e.endpoint == "/oidc/token")
        .expect("the token event");
    assert_eq!(exchange.status, 400);
    // **The row is labelled even though the exchange named no client.** The
    // code record knows whose login this was, and a failed row a developer
    // cannot attribute is the one row the log page exists to attribute.
    assert_eq!(exchange.client_id.as_deref(), Some("billing-web"));
    let pkce = &exchange.detail.as_ref().expect("the detail")["pkce"];
    assert_eq!(pkce["method"], "S256");
    assert_eq!(pkce["verifier_presented"], wrong);
    assert_eq!(pkce["challenge_recorded"], challenge);
    let computed = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(sha2::Sha256::digest(wrong.as_bytes()));
    assert_eq!(pkce["challenge_computed"], computed);
    assert_ne!(
        pkce["challenge_computed"], pkce["challenge_recorded"],
        "the mismatched pair is visibly the one that differs"
    );
}

/// The authorize half of criterion 1, plus the decoded-parameters promise: a
/// list for `scope`, the challenge beside its method, and the response mode
/// that was **decided** rather than the one that was sent.
#[tokio::test]
async fn an_authorize_carries_its_decoded_parameters() {
    let (base, events) = spawn_logged().await;

    support::client()
        .get(format!(
            "{base}/oidc/authorize?client_id=billing-web&redirect_uri={}\
             &response_type=code&scope=openid%20email%20profile&nonce=n-0S6\
             &code_challenge=K2-ltc83acc4h0c9w6ESC&code_challenge_method=S256",
            support::ADA_CB
        ))
        .send()
        .await
        .unwrap();

    let seen = logged(&events);
    assert_eq!(seen.len(), 1);
    let event = &seen[0];
    assert_eq!(event.client_id.as_deref(), Some("billing-web"));
    assert_eq!(event.status, 302);
    let request = event.request.as_ref().expect("the decoded parameters");
    assert_eq!(request["response_type"], "code");
    assert_eq!(request["response_mode"], "query", "decided, not sent");
    assert_eq!(
        request["scope"],
        serde_json::json!(["openid", "email", "profile"])
    );
    assert_eq!(request["nonce"], "n-0S6");
    assert_eq!(request["code_challenge"], "K2-ltc83acc4h0c9w6ESC");
    assert_eq!(request["code_challenge_method"], "S256");
    assert!(request["redirect_uri"]
        .as_str()
        .unwrap()
        .contains("localhost:5000"));

    let line = lanyard_cli::events::pretty(event);
    assert!(line.contains("scope=openid,email,profile"), "{line}");
    assert!(line.contains("pkce=S256"), "{line}");
}

/// A remembered selection completes `/authorize` without the picker, and the
/// line says who — criterion 1's "the authorize line naming the persona".
#[tokio::test]
async fn a_remembered_login_names_the_persona_on_the_authorize_line() {
    let (base, events) = spawn_logged().await;
    let client = support::client();

    // First login: the picker chooses, and the browser keeps the cookie.
    let res = client
        .get(format!(
            "{base}/oidc/authorize?client_id=billing-web&redirect_uri={}&response_type=code",
            support::ADA_CB
        ))
        .send()
        .await
        .unwrap();
    let req = res.headers()["location"]
        .to_str()
        .unwrap()
        .split("req=")
        .nth(1)
        .unwrap()
        .to_owned();
    client
        .post(format!("{base}/_/pick"))
        .form(&[("req", req.as_str()), ("persona", "ada")])
        .send()
        .await
        .unwrap();

    // Second login: no picker, so the authorize event is the whole decision.
    client
        .get(format!(
            "{base}/oidc/authorize?client_id=billing-web&redirect_uri={}&response_type=code",
            support::ADA_CB
        ))
        .send()
        .await
        .unwrap();

    let last = logged(&events).pop().expect("an event");
    assert_eq!(last.endpoint, "/oidc/authorize");
    assert_eq!(last.detail.as_ref().expect("the detail")["persona"], "ada");
    assert!(lanyard_cli::events::pretty(&last).contains(" ada"));
}

/// Criterion 12: a rejection the RP never sees is still in the log, and it
/// names the host that failed the loopback check.
#[tokio::test]
async fn a_refused_redirect_uri_is_logged_with_the_host_that_failed() {
    let (base, events) = spawn_logged().await;

    let res = support::client()
        .get(format!(
            "{base}/oidc/authorize?client_id=billing-web\
             &redirect_uri=https%3A%2F%2Fevil.example.com%2Fcb&response_type=code"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 400, "nothing was redirected");

    let event = logged(&events).pop().expect("an event");
    assert_eq!(event.endpoint, "/oidc/authorize");
    assert_eq!(event.status, 400);
    assert!(event.error.is_some(), "{event:#?}");
    let detail = event.detail.as_ref().expect("the detail");
    assert_eq!(detail["redirect_uri"]["host"], "evil.example.com");
    assert!(lanyard_cli::events::pretty(&event).contains("evil.example.com"));
}

/// The four remaining protocol endpoints. Each names the `client_id` it acted
/// for and says what happened — a logout and a revoke each print a line naming
/// what it did, which is the whole reason they emit at all.
#[tokio::test]
async fn the_rest_of_the_protocol_surface_names_its_client_and_its_outcome() {
    let (base, events) = spawn_logged().await;
    let client = support::client();
    let code = support::login_as(&client, &base, "ada", "billing-web", "openid email").await;
    let tokens: serde_json::Value = client
        .post(format!("{base}/oidc/token"))
        .form(&[("grant_type", "authorization_code"), ("code", &code)])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let access = tokens["access_token"].as_str().unwrap().to_owned();

    client
        .get(format!("{base}/oidc/userinfo"))
        .bearer_auth(&access)
        .send()
        .await
        .unwrap();
    client
        .post(format!("{base}/oidc/introspect"))
        .form(&[("token", access.as_str())])
        .send()
        .await
        .unwrap();
    client
        .post(format!("{base}/oidc/revoke"))
        .form(&[("token", access.as_str())])
        .send()
        .await
        .unwrap();
    client
        .get(format!("{base}/oidc/end_session"))
        .send()
        .await
        .unwrap();

    let seen = logged(&events);
    let of = |endpoint: &str| {
        seen.iter()
            .find(|e| e.endpoint == endpoint)
            .unwrap_or_else(|| panic!("no {endpoint} event in {seen:#?}"))
            .clone()
    };

    let userinfo = of("/oidc/userinfo");
    assert_eq!(userinfo.status, 200);
    assert_eq!(userinfo.client_id.as_deref(), Some("billing-web"));
    assert_eq!(userinfo.detail.as_ref().unwrap()["sub"], "ada");

    let introspect = of("/oidc/introspect");
    assert_eq!(introspect.client_id.as_deref(), Some("billing-web"));
    assert_eq!(introspect.detail.as_ref().unwrap()["active"], true);

    let revoke = of("/oidc/revoke");
    assert_eq!(revoke.client_id.as_deref(), Some("billing-web"));
    assert_eq!(revoke.detail.as_ref().unwrap()["revoked"], true);

    let logout = of("/oidc/end_session");
    assert_eq!(logout.detail.as_ref().unwrap()["signed_out"], true);
}

/// An unrecognized token is the answer RFC 7662 §2.2 requires, and the log says
/// which answer it was — otherwise a developer cannot tell "lanyard said no"
/// from "lanyard was never called".
#[tokio::test]
async fn an_unrecognized_token_is_logged_as_inactive_rather_than_as_nothing() {
    let (base, events) = spawn_logged().await;
    support::client()
        .post(format!("{base}/oidc/introspect"))
        .form(&[("token", "not-a-token")])
        .send()
        .await
        .unwrap();
    let event = logged(&events).pop().unwrap();
    assert_eq!(event.detail.as_ref().unwrap()["active"], false);
    assert_eq!(event.client_id, None);
}

/// **The one human moment that gets an event.** "Why am I signed in as the
/// wrong person" is a question the log has to answer, and `POST /_/pick` is
/// where that stops being "the picker appeared" and becomes "you are Ada".
#[tokio::test]
async fn picking_a_person_names_them() {
    let (base, events) = spawn_logged().await;
    support::login(&base, "mira", "php-demo", "openid").await;

    let pick = logged(&events)
        .into_iter()
        .find(|e| e.endpoint == "/_/pick")
        .expect("the pick event");
    assert_eq!(pick.client_id.as_deref(), Some("php-demo"));
    assert_eq!(pick.detail.as_ref().unwrap()["persona"], "mira");
    assert!(lanyard_cli::events::pretty(&pick).contains("mira"));
}

/// The test seam mints, so it says what it minted and which flaw it was asked
/// for — Phase 3's deferred item, paid on both surfaces that can ask.
#[tokio::test]
async fn the_test_seam_names_its_flaw_and_its_claims() {
    let (base, events) = spawn_logged().await;

    support::client()
        .post(format!("{base}/_/api/token?persona=ada&flaw=alg-none"))
        .header("content-type", "application/json")
        .body(r#"{"aud":"billing-api"}"#)
        .send()
        .await
        .unwrap();

    let event = logged(&events).pop().expect("an event");
    assert_eq!(event.endpoint, "/_/api/token");
    assert_eq!(event.flaw.as_deref(), Some("alg-none"));
    let issued = event.issued.as_ref().expect("what was minted");
    assert_eq!(issued["access_token"]["header"]["alg"], "none");
    assert_eq!(issued["access_token"]["payload"]["aud"], "billing-api");
    assert_eq!(event.request.as_ref().unwrap()["persona"], "ada");
}

// ------------------------------------------------------ the two endpoints --

/// Criterion 5: SSE frames carry `id:` and `data:`, and the ring is replayed
/// before the stream goes live — criterion 6's "three logins, *then* start
/// reading" in miniature.
#[tokio::test]
async fn the_stream_replays_the_ring_as_sse_frames() {
    let (base, _events) = spawn_logged().await;
    mint(&base, "ada").await;
    mint(&base, "mira").await;

    let res = support::client()
        .get(format!("{base}/_/api/events"))
        .header("accept", "text/event-stream")
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    assert_eq!(
        res.headers()["content-type"],
        "text/event-stream; charset=utf-8"
    );
    assert_eq!(res.headers()["cache-control"], "no-cache");
    assert_eq!(res.headers()["x-accel-buffering"], "no");

    let body = support::read_stream(res, 2).await;
    assert!(body.starts_with("id: 1\ndata: {"), "{body}");
    assert!(body.contains("\nid: 2\ndata: {"), "{body}");
    assert!(body.contains("\"endpoint\":\"/oidc/token\""), "{body}");
    assert!(
        body.ends_with("\n\n"),
        "frames end blank-line terminated: {body}"
    );
}

/// Reconnect replay: `Last-Event-ID: 2` delivers 3 onward and replays none
/// before it.
#[tokio::test]
async fn last_event_id_resumes_strictly_after_the_given_id() {
    let (base, _events) = spawn_logged().await;
    for persona in ["ada", "mira", "ada"] {
        mint(&base, persona).await;
    }

    let res = support::client()
        .get(format!("{base}/_/api/events"))
        .header("last-event-id", "2")
        .send()
        .await
        .unwrap();

    let body = support::read_stream(res, 1).await;
    assert!(body.starts_with("id: 3\ndata: {"), "{body}");
    assert!(!body.contains("id: 1"), "{body}");
    assert!(!body.contains("id: 2\n"), "{body}");
}

/// One `client_credentials` mint, for a test that only needs an event to exist.
async fn mint(base: &str, persona: &str) {
    support::client()
        .post(format!("{base}/oidc/token"))
        .form(&[
            ("grant_type", "client_credentials"),
            ("client_id", "lanyard-cli"),
            ("persona", persona),
        ])
        .send()
        .await
        .unwrap();
}

/// Criterion 2: the same objects, one JSON per line, from the same handler.
#[tokio::test]
async fn the_same_stream_comes_out_as_ndjson() {
    let (base, _events) = spawn_logged().await;
    mint(&base, "ada").await;

    let res = support::client()
        .get(format!("{base}/_/api/events?format=ndjson"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        res.headers()["content-type"],
        "application/x-ndjson; charset=utf-8"
    );
    assert_eq!(res.headers()["cache-control"], "no-cache");

    let body = support::read_stream(res, 1).await;
    let line = body.lines().next().expect("one line");
    assert!(!line.contains("data:"), "no SSE framing: {line}");
    let event: serde_json::Value = serde_json::from_str(line).expect("one JSON object per line");
    assert_eq!(event["client_id"], "lanyard-cli");
    assert_eq!(event["grant_type"], "client_credentials");
}

/// Criterion 8's second half. A reader that stops reading is ordinary — a
/// backgrounded tab, a `curl` into a full pipe — and the fan-out is lossy at
/// the subscriber and never at the source. What it must not be is *silent*: a
/// log with an invisible hole is worse than no log, because the developer
/// concludes the request never happened.
#[tokio::test]
async fn a_lagged_reader_is_told_how_many_it_missed() {
    let (base, events) = spawn_logged().await;

    let res = support::client()
        .get(format!("{base}/_/api/events?format=ndjson"))
        .send()
        .await
        .unwrap();

    // Far more than the broadcast channel holds, published while nothing is
    // draining the socket.
    for _ in 0..5_000 {
        events.publish(lanyard_cli::events::EventDraft {
            endpoint: "/oidc/token".to_owned(),
            method: "POST".to_owned(),
            status: 200,
            ..lanyard_cli::events::EventDraft::default()
        });
    }

    let body = support::read_stream(res, 6_000).await;
    let dropped = body
        .lines()
        .find(|line| line.contains("\"dropped\""))
        .expect("a dropped marker rather than a silent hole");
    let marker: serde_json::Value = serde_json::from_str(dropped).unwrap();
    assert!(
        marker["dropped"].as_u64().unwrap() > 0,
        "it names how many: {marker}"
    );
}

/// Criterion 9. Draining the ring — rather than only the client's own list — is
/// what makes the clear survive a reconnect replay and reach every open tab.
#[tokio::test]
async fn clearing_empties_the_ring_and_reaches_an_open_stream() {
    let (base, _events) = spawn_logged().await;
    mint(&base, "ada").await;

    // A tab that is already watching.
    let open = support::client()
        .get(format!("{base}/_/api/events?format=ndjson"))
        .send()
        .await
        .unwrap();

    let cleared = support::client()
        .post(format!("{base}/_/api/events/clear"))
        .send()
        .await
        .unwrap();
    assert_eq!(cleared.status(), 204);

    // The already-open stream is told, so it empties its own view.
    let body = support::read_stream(open, 2).await;
    assert!(body.contains("{\"clear\":true}"), "{body}");

    // And a reconnect replays nothing.
    let fresh = support::client()
        .get(format!("{base}/_/api/events?format=ndjson"))
        .send()
        .await
        .unwrap();
    let replayed = support::read_stream_for(fresh, 1, QUIET).await;
    assert!(replayed.is_empty(), "nothing is replayed: {replayed:?}");
}

/// **The stream must not log itself.** Two clients connected: neither sees an
/// event from the other's connection, or one connected tab would feed the next
/// and the log would become its own traffic.
#[tokio::test]
async fn two_connected_readers_see_nothing_from_each_others_connections() {
    let (base, _events) = spawn_logged().await;

    let first = support::client()
        .get(format!("{base}/_/api/events?format=ndjson"))
        .send()
        .await
        .unwrap();
    let second = support::client()
        .get(format!("{base}/_/api/events?format=ndjson"))
        .send()
        .await
        .unwrap();

    // Reading with nothing to read: the helper gives back what arrived before
    // its deadline, which must be nothing at all.
    assert!(support::read_stream_for(first, 1, QUIET).await.is_empty());
    assert!(support::read_stream_for(second, 1, QUIET).await.is_empty());
}

// ---------------------------------------- the ten sentences, all logged --

/// **Criterion 11.** Phases 4 and 5 wrote ten distinct `invalid_grant`
/// descriptions on purpose, each naming exactly one thing that went wrong — and
/// every one of them was written to a `400` on a back-channel call the
/// developer never sees. This is the assertion that each now reaches the log
/// **verbatim**, attached to the request that produced it.
///
/// Two of the ten are only reachable with a clock: a code expires after 60
/// seconds and a refresh token after eight hours, so those two arms run against
/// a server whose stores were built with zero TTLs. Everything else is the
/// ordinary server.
#[tokio::test]
async fn every_invalid_grant_description_reaches_the_log_verbatim() {
    let (base, events) = spawn_logged().await;
    let client = support::client();

    // 1. A code that was never issued.
    exchange(&client, &base, &[("code", "never-issued")]).await;

    // 2. A code exchanged twice.
    let code = support::login_as(
        &client,
        &base,
        "ada",
        "billing-web",
        "openid offline_access",
    )
    .await;
    let tokens: serde_json::Value = exchange(&client, &base, &[("code", &code)])
        .await
        .json()
        .await
        .unwrap();
    exchange(&client, &base, &[("code", &code)]).await;

    // 3. A redirect_uri that is not the one the code was issued against.
    let code = support::login_as(&client, &base, "ada", "billing-web", "openid").await;
    exchange(
        &client,
        &base,
        &[
            ("code", &code),
            ("redirect_uri", "http://localhost:9999/cb"),
        ],
    )
    .await;

    // 4 and 5. A code bound to a challenge: no verifier, then a wrong one.
    let code = login_with_pkce(
        &client,
        &base,
        "K2-ltc83acc4h0c9w6ESC_rEMTJ3F50BXVuGJSstw-cM",
    )
    .await;
    exchange(&client, &base, &[("code", &code)]).await;
    let code = login_with_pkce(
        &client,
        &base,
        "K2-ltc83acc4h0c9w6ESC_rEMTJ3F50BXVuGJSstw-cM",
    )
    .await;
    exchange(
        &client,
        &base,
        &[("code", &code), ("code_verifier", "not-the-verifier")],
    )
    .await;

    // 6. A refresh token that was never issued.
    refresh(&client, &base, "never-issued").await;

    // 7. A refresh token used twice — lanyard rotates.
    let token = tokens["refresh_token"].as_str().unwrap().to_owned();
    let rotated: serde_json::Value = refresh(&client, &base, &token).await.json().await.unwrap();
    refresh(&client, &base, &token).await;

    // 8. A refresh token revoked at /oidc/revoke.
    let live = rotated["refresh_token"].as_str().unwrap().to_owned();
    client
        .post(format!("{base}/oidc/revoke"))
        .form(&[("token", live.as_str())])
        .send()
        .await
        .unwrap();
    refresh(&client, &base, &live).await;

    let logged_descriptions: Vec<String> = logged(&events)
        .into_iter()
        .filter_map(|e| e.error_description)
        .collect();

    for expected in [
        "no such authorization code; it was never issued, or lanyard was restarted since it was",
        "that authorization code has already been exchanged; codes are single-use",
        "redirect_uri \"http://localhost:9999/cb\" does not match the one this code was issued \
         against, \"http://localhost:5000/signin-oidc\"",
        "this code was issued against a S256 code_challenge, so a code_verifier is required",
        "the code_verifier does not match the S256 code_challenge this code was issued against",
        "no such refresh token; it was never issued, or lanyard was restarted since it was",
        "that refresh token has already been exchanged; lanyard rotates refresh tokens, so each \
         one works once and the response carries its replacement",
        "that refresh token has been revoked, either at /oidc/revoke or by logging out of lanyard",
    ] {
        assert!(
            logged_descriptions.iter().any(|got| got == expected),
            "not in the log verbatim: {expected:?}\nlogged: {logged_descriptions:#?}"
        );
    }

    // The two that need a clock.
    assert!(
        expired_code_descriptions().await.contains(
            &"that authorization code has expired; codes live 60 seconds, which is the redirect \
              and the exchange and nothing else"
                .to_string()
        ),
        "the expired-code sentence"
    );
    assert!(
        expired_refresh_descriptions().await.contains(
            &"that refresh token has expired; refresh tokens live eight hours, which is a working \
              day"
            .to_string()
        ),
        "the expired-refresh sentence"
    );
}

/// The sentence a code's own clock produces: a server whose codes die the
/// instant they are issued.
async fn expired_code_descriptions() -> Vec<String> {
    let (base, events) =
        short_lived(std::time::Duration::ZERO, lanyard_cli::store::REFRESH_TTL).await;
    let client = support::client();
    let code = support::login_as(&client, &base, "ada", "billing-web", "openid").await;
    exchange(&client, &base, &[("code", &code)]).await;
    descriptions(&events)
}

/// The sentence a refresh token's own clock produces. **A separate server**,
/// because the code store has to keep working long enough to issue the refresh
/// token that is then found expired.
async fn expired_refresh_descriptions() -> Vec<String> {
    let (base, events) = short_lived(lanyard_cli::store::CODE_TTL, std::time::Duration::ZERO).await;
    let client = support::client();
    let code = support::login_as(
        &client,
        &base,
        "ada",
        "billing-web",
        "openid offline_access",
    )
    .await;
    let issued: serde_json::Value = exchange(&client, &base, &[("code", &code)])
        .await
        .json()
        .await
        .unwrap();
    let token = issued["refresh_token"]
        .as_str()
        .expect("offline_access mints a refresh token");
    refresh(&client, &base, token).await;
    descriptions(&events)
}

async fn short_lived(
    codes: std::time::Duration,
    refresh: std::time::Duration,
) -> (String, lanyard_cli::events::EventBus) {
    support::spawn_observed(
        lanyard_cli::persona::Personas::builtin(),
        lanyard_cli::store::Stores::with_ttls(std::time::Duration::from_secs(300), codes, refresh),
    )
    .await
}

fn descriptions(events: &lanyard_cli::events::EventBus) -> Vec<String> {
    logged(events)
        .into_iter()
        .filter_map(|e| e.error_description)
        .collect()
}

async fn login_with_pkce(client: &reqwest::Client, base: &str, challenge: &str) -> String {
    let res = client
        .get(format!(
            "{base}/oidc/authorize?client_id=billing-web&redirect_uri={}&response_type=code\
             &code_challenge={challenge}&code_challenge_method=S256&prompt=login",
            support::ADA_CB
        ))
        .send()
        .await
        .unwrap();
    let req = res.headers()["location"]
        .to_str()
        .unwrap()
        .split("req=")
        .nth(1)
        .unwrap()
        .to_owned();
    let res = client
        .post(format!("{base}/_/pick"))
        .form(&[("req", req.as_str()), ("persona", "ada")])
        .send()
        .await
        .unwrap();
    url::Url::parse(res.headers()["location"].to_str().unwrap())
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == "code")
        .map(|(_, v)| v.into_owned())
        .unwrap()
}

async fn exchange(
    client: &reqwest::Client,
    base: &str,
    extra: &[(&str, &str)],
) -> reqwest::Response {
    let mut form = vec![("grant_type", "authorization_code")];
    form.extend_from_slice(extra);
    client
        .post(format!("{base}/oidc/token"))
        .form(&form)
        .send()
        .await
        .unwrap()
}

async fn refresh(client: &reqwest::Client, base: &str, token: &str) -> reqwest::Response {
    client
        .post(format!("{base}/oidc/token"))
        .form(&[("grant_type", "refresh_token"), ("refresh_token", token)])
        .send()
        .await
        .unwrap()
}
