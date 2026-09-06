//! The embedded zero app: what is served under `/_/`, and what is not served at
//! the root.

mod support;

use support::spawn;

/// Criterion 17's second half, and criterion 23's first: the log page is the
/// one page lanyard ships JavaScript for, and every asset it names lives under
/// the `/_/` mount.
#[tokio::test]
async fn the_log_page_is_the_zero_app_with_one_hashed_module_script() {
    let base = spawn().await;
    let res = support::client()
        .get(format!("{base}/_/log"))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    assert_eq!(res.headers()["content-type"], "text/html; charset=utf-8");
    assert_eq!(res.headers()["cache-control"], "no-cache");
    let html = res.text().await.unwrap();

    assert_eq!(
        html.matches("<script").count(),
        1,
        "exactly one script: {html}"
    );
    let src = html
        .split("src=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("the module script's src");
    assert!(src.starts_with("/_/assets/"), "under the mount: {src}");
    assert!(src.ends_with(".js"), "{src}");
    assert!(
        src.trim_start_matches("/_/assets/app.").len() > 3,
        "fingerprinted: {src}"
    );
    assert!(
        !html.contains("\"/assets/"),
        "no root-absolute refs survive the rewrite: {html}"
    );

    // The asset it names is actually there.
    let asset = support::client()
        .get(format!("{base}{src}"))
        .send()
        .await
        .unwrap();
    assert_eq!(asset.status(), 200);
    assert_eq!(
        asset.headers()["content-type"],
        "text/javascript; charset=utf-8"
    );
    assert!(asset.headers()["cache-control"]
        .to_str()
        .unwrap()
        .contains("immutable"));
}

/// Criterion 18's second half: with JavaScript off the page says so and names
/// the surface that works without it. It does **not** degrade to a static
/// snapshot — a page that silently stops updating is worse than one that says
/// it needs a thing.
#[tokio::test]
async fn the_log_page_names_lanyard_logs_for_a_browser_with_no_javascript() {
    let base = spawn().await;
    let html = support::client()
        .get(format!("{base}/_/log"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(html.contains("<noscript>"), "{html}");
    let noscript = html
        .split("<noscript>")
        .nth(1)
        .and_then(|rest| rest.split("</noscript>").next())
        .unwrap();
    assert!(noscript.contains("lanyard logs"), "{noscript}");
}

/// `/_/log` links back to the picker — criterion 27's second half.
#[tokio::test]
async fn the_log_page_links_back_to_the_picker() {
    let base = spawn().await;
    let html = support::client()
        .get(format!("{base}/_/log"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(html.contains("href=\"/_/\""), "{html}");
}

/// The stylesheet's font refs are rewritten too, and the fonts resolve under
/// the mount — the second half of criterion 23.
#[tokio::test]
async fn the_stylesheet_names_fonts_under_the_mount_and_they_resolve() {
    let base = spawn().await;
    let html = support::client()
        .get(format!("{base}/_/log"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let href = html
        .split("href=\"")
        .map(|rest| rest.split('"').next().unwrap_or_default())
        .find(|href| href.ends_with(".css"))
        .expect("a stylesheet");
    assert!(href.starts_with("/_/assets/"), "{href}");

    let css = support::client()
        .get(format!("{base}{href}"))
        .send()
        .await
        .unwrap();
    assert_eq!(css.headers()["content-type"], "text/css; charset=utf-8");
    let css = css.text().await.unwrap();
    assert!(css.contains("/_/.zero/fonts/"), "fonts are under the mount");
    assert!(!css.contains("(/.zero/"), "nothing root-absolute survives");

    let font = support::client()
        .get(format!(
            "{base}/_/.zero/fonts/Geist-VariableFont_wght.woff2"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(font.status(), 200);
    assert_eq!(font.headers()["content-type"], "font/woff2");
}

/// **Root stays free** — Phase 1's routing decision, which the mount-prefix
/// rewrite exists to preserve. Criterion 23's last line.
#[tokio::test]
async fn the_root_serves_none_of_it() {
    let base = spawn().await;
    for path in [
        "/.zero/fonts/Geist-VariableFont_wght.woff2",
        "/assets/app.a2256720.js",
        "/index.html",
        "/",
    ] {
        let res = support::client()
            .get(format!("{base}{path}"))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 404, "{path} must not be served at the root");
    }
}

/// An asset that is not in the bundle is a `404`, not an index page: a
/// mistyped hash must not come back `200` with HTML that a browser then fails
/// to parse as JavaScript.
#[tokio::test]
async fn a_missing_asset_is_a_404_rather_than_the_index() {
    let base = spawn().await;
    let res = support::client()
        .get(format!("{base}/_/assets/app.deadbeef.js"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 404);
}

/// Criterion 27's first half, and criterion 26's premise: the picker names the
/// log, so the banner's `Log →` line is a URL a reader has already been offered
/// once.
#[tokio::test]
async fn the_picker_links_to_the_log() {
    let base = spawn().await;
    let html = support::client()
        .get(format!("{base}/_/"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(html.contains("href=\"/_/log\""), "{html}");
    // Criterion 17's first half: `/_/` stays server-rendered with no script.
    assert_eq!(html.matches("<script").count(), 0, "{html}");
    // Criterion 24: it renders from the one hashed stylesheet, and nothing else.
    assert_eq!(html.matches("<link").count(), 1, "{html}");
    assert!(
        html.contains("<link rel=\"stylesheet\" href=\"/_/assets/app."),
        "{html}"
    );
}

/// Criterion 24, the other two pages: the rejection page and the log page come
/// from the same stylesheet the picker does.
#[tokio::test]
async fn all_three_pages_render_from_the_one_stylesheet() {
    let base = spawn().await;
    let href = lanyard_cli::embed::stylesheet_href();

    let picker = support::client()
        .get(format!("{base}/_/"))
        .send()
        .await
        .unwrap();
    let rejected = support::client()
        .get(format!(
            "{base}/oidc/authorize?client_id=c&redirect_uri=https%3A%2F%2Fevil.example.com%2Fcb\
             &response_type=code"
        ))
        .send()
        .await
        .unwrap();
    let log = support::client()
        .get(format!("{base}/_/log"))
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), 400);

    for (name, res) in [("picker", picker), ("rejection", rejected), ("log", log)] {
        let html = res.text().await.unwrap();
        assert!(html.contains(href), "{name} does not link {href}: {html}");
    }
}
