//! `/oidc/authorize`: the one rejection, the redirected errors, and the hop to
//! the picker.
//!
//! Every assertion here is about what a browser or an SDK actually observes —
//! the status, the `Location` header, the query parameters on it — because that
//! is the only thing the RP on the other end can react to.

mod support;

use support::{authorize, spawn, ADA_CB, ISSUER};

/// Redirects are never followed: the whole point of most of these is the
/// `Location` header itself.
#[tokio::test]
async fn a_valid_request_stores_a_pending_record_and_hops_to_the_picker() {
    let base = spawn().await;
    let res = authorize(
        &base,
        &format!("client_id=billing-web&response_type=code&redirect_uri={ADA_CB}"),
    )
    .await;

    assert_eq!(res.status().as_u16(), 302);
    let location = res.headers()["location"].to_str().unwrap();
    assert!(
        location.starts_with("/_/?req="),
        "the picker is reached by a redirect, keeping /oidc protocol-only: {location}"
    );
    let id = location.strip_prefix("/_/?req=").unwrap();
    assert!(id.len() >= 32, "the request id must be unguessable: {id}");
}

// ------------------------------------------ the two that must never redirect --

/// RFC 6749 §4.1.2.1. Sending an error to an address lanyard has just decided
/// not to trust would make the one rejection decorative.
#[tokio::test]
async fn a_non_loopback_redirect_uri_renders_a_400_with_no_location_header() {
    let base = spawn().await;
    let res = authorize(
        &base,
        "client_id=x&response_type=code\
         &redirect_uri=https%3A%2F%2Fevil.example.com%2Fcb",
    )
    .await;

    assert_eq!(res.status().as_u16(), 400);
    assert!(
        res.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html"),
        "the audience for this message is the developer reading it"
    );
    assert!(
        res.headers().get("location").is_none(),
        "there must be no Location header at all"
    );

    let body = res.text().await.unwrap();
    assert!(
        body.contains("https://evil.example.com/cb"),
        "the page names the URI it rejected: {body}"
    );
    assert!(body.contains("loopback"), "and states the rule: {body}");
}

/// It really does resolve to 127.0.0.1, and it is rejected anyway, because no
/// DNS lookup happens. Phase 0 already measured that a .NET app's own login
/// breaks on this host for an unrelated reason.
#[tokio::test]
async fn a_name_that_resolves_to_loopback_is_rejected_the_same_way() {
    let base = spawn().await;
    let res = authorize(
        &base,
        "client_id=x&response_type=code\
         &redirect_uri=http%3A%2F%2Fweb.localtest.me%3A5000%2Fcb",
    )
    .await;

    assert_eq!(res.status().as_u16(), 400);
    assert!(res.headers().get("location").is_none());
    let body = res.text().await.unwrap();
    assert!(body.contains("web.localtest.me"), "{body}");
}

#[tokio::test]
async fn a_missing_client_id_or_redirect_uri_renders_rather_than_redirecting() {
    let base = spawn().await;

    let res = authorize(&base, &format!("response_type=code&redirect_uri={ADA_CB}")).await;
    assert_eq!(res.status().as_u16(), 400);
    assert!(res.headers().get("location").is_none());
    assert!(res.text().await.unwrap().contains("client_id"));

    let res = authorize(&base, "client_id=x&response_type=code").await;
    assert_eq!(res.status().as_u16(), 400);
    assert!(res.headers().get("location").is_none());
    assert!(res.text().await.unwrap().contains("redirect_uri"));
}

/// Criterion 27's escaping applies to error pages too: the rejected value is
/// attacker-influenced in exactly the way a persona name is not.
#[tokio::test]
async fn a_rejected_uri_is_html_escaped_in_the_page() {
    let base = spawn().await;
    let res = authorize(
        &base,
        "client_id=x&response_type=code\
         &redirect_uri=https%3A%2F%2Fevil.test%2F%3Cscript%3Ealert(1)%3C%2Fscript%3E",
    )
    .await;

    let body = res.text().await.unwrap();
    assert!(!body.contains("<script>alert(1)</script>"), "{body}");
    assert!(body.contains("&lt;script&gt;"), "{body}");
}

// ----------------------------------------------- everything else redirects --

/// Phase 0's obligation #4. A `dotnet new` app with otherwise correct settings
/// sends the implicit flow, and a bare protocol noun back is not a diagnosis.
#[tokio::test]
async fn response_type_id_token_redirects_and_names_the_dotnet_setting() {
    let base = spawn().await;
    let res = authorize(
        &base,
        &format!("client_id=x&response_type=id_token&redirect_uri={ADA_CB}&state=s1"),
    )
    .await;

    assert_eq!(res.status().as_u16(), 302);
    let params = support::query_of(&res);
    assert_eq!(params["error"], "unsupported_response_type");
    assert!(
        params["error_description"].contains("options.ResponseType = \"code\""),
        "{}",
        params["error_description"]
    );
    assert_eq!(params["state"], "s1");
}

/// The .NET sentence is a diagnosis, not a signature. A response type that has
/// nothing to do with .NET's default gets the same refusal without it.
#[tokio::test]
async fn a_response_type_without_id_token_gets_no_dotnet_sentence() {
    let base = spawn().await;
    let res = authorize(
        &base,
        &format!("client_id=x&response_type=token&redirect_uri={ADA_CB}"),
    )
    .await;

    let params = support::query_of(&res);
    assert_eq!(params["error"], "unsupported_response_type");
    assert!(!params["error_description"].contains(".NET"));
    assert!(params["error_description"].contains("\"token\""));
}

#[tokio::test]
async fn response_mode_fragment_is_refused_and_form_post_is_not() {
    let base = spawn().await;
    let res = authorize(
        &base,
        &format!("client_id=x&response_type=code&redirect_uri={ADA_CB}&response_mode=fragment"),
    )
    .await;
    assert_eq!(
        support::query_of(&res)["error"],
        "unsupported_response_mode"
    );

    let res = authorize(
        &base,
        &format!("client_id=x&response_type=code&redirect_uri={ADA_CB}&response_mode=form_post"),
    )
    .await;
    assert_eq!(res.status().as_u16(), 302);
    assert!(res.headers()["location"]
        .to_str()
        .unwrap()
        .starts_with("/_/?req="));
}

#[tokio::test]
async fn a_half_supplied_or_unknown_pkce_challenge_is_invalid_request() {
    let base = spawn().await;

    // A method with no challenge would mint a code nothing could redeem.
    let res = authorize(
        &base,
        &format!("client_id=x&response_type=code&redirect_uri={ADA_CB}&code_challenge_method=S256"),
    )
    .await;
    let params = support::query_of(&res);
    assert_eq!(params["error"], "invalid_request");
    assert!(params["error_description"].contains("code_challenge"));

    // An empty challenge is an absent one, so it lands in the same place.
    let res = authorize(
        &base,
        &format!(
            "client_id=x&response_type=code&redirect_uri={ADA_CB}\
             &code_challenge=&code_challenge_method=S256"
        ),
    )
    .await;
    assert_eq!(support::query_of(&res)["error"], "invalid_request");

    let res = authorize(
        &base,
        &format!(
            "client_id=x&response_type=code&redirect_uri={ADA_CB}\
             &code_challenge=abc&code_challenge_method=S512"
        ),
    )
    .await;
    let params = support::query_of(&res);
    assert_eq!(params["error"], "invalid_request");
    assert!(params["error_description"].contains("S512"));
}

#[tokio::test]
async fn a_request_object_is_refused_by_name() {
    let base = spawn().await;
    for (param, expected) in [
        ("request=eyJ", "request_not_supported"),
        (
            "request_uri=http%3A%2F%2Fx%2Fr",
            "request_uri_not_supported",
        ),
    ] {
        let res = authorize(
            &base,
            &format!("client_id=x&response_type=code&redirect_uri={ADA_CB}&{param}"),
        )
        .await;
        assert_eq!(support::query_of(&res)["error"], expected);
    }
}

/// Criterion 9's second half. An RP that did not send `state` and receives one
/// has been told something untrue about its own request.
#[tokio::test]
async fn no_state_in_means_no_state_parameter_out() {
    let base = spawn().await;
    let res = authorize(
        &base,
        &format!("client_id=x&response_type=code&redirect_uri={ADA_CB}&response_mode=fragment"),
    )
    .await;

    let location = res.headers()["location"].to_str().unwrap();
    assert!(location.contains("error="), "{location}");
    assert!(!location.contains("state="), "{location}");
}

/// The error redirect keeps whatever query the RP's own callback URL carried.
#[tokio::test]
async fn an_error_redirect_preserves_the_callback_urls_own_query() {
    let base = spawn().await;
    let res = authorize(
        &base,
        "client_id=x&response_type=id_token\
         &redirect_uri=http%3A%2F%2Flocalhost%3A5000%2Fcb%3Ftenant%3Dacme",
    )
    .await;

    let location = res.headers()["location"].to_str().unwrap();
    assert!(location.contains("tenant=acme"), "{location}");
    assert!(
        location.contains("error=unsupported_response_type"),
        "{location}"
    );
}

/// OIDC Core §3.1.2.1 allows both verbs and some SDKs use the second.
#[tokio::test]
async fn post_works_the_same_as_get() {
    let base = spawn().await;
    let res = support::client()
        .post(format!("{base}/oidc/authorize"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(format!(
            "client_id=billing-web&response_type=code&redirect_uri={ADA_CB}"
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status().as_u16(), 302);
    assert!(res.headers()["location"]
        .to_str()
        .unwrap()
        .starts_with("/_/?req="));
}

#[tokio::test]
async fn the_issuer_is_still_not_derived_from_anything_the_request_says() {
    let base = spawn().await;
    let doc: serde_json::Value =
        reqwest::get(format!("{base}/oidc/.well-known/openid-configuration"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
    assert_eq!(doc["issuer"], ISSUER);
}
