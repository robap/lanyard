//! The embedded web UI.
//!
//! `web/dist/` is a **committed** build artifact — zero's production build —
//! compiled into the binary with `rust-embed`, so a machine with a Rust
//! toolchain and nothing else builds lanyard and `zero` is never on the build
//! path (north star 5). Regenerating the UI (`zero build`) is a developer step
//! done before committing, not part of `cargo build`. In debug builds
//! `rust-embed` reads `web/dist/` from disk, which is what makes the UI loop
//! `zero build && cargo run` with no second process and no proxy.
//!
//! **The mount-prefix problem, solved at serve time.** zero emits root-absolute
//! asset refs — `/assets/…` and `/.zero/fonts/…` — with no base-path config,
//! and Phase 1 settled that lanyard's root stays free. So those two prefixes are
//! rewritten to `/_/` **as the asset is served**, not at build time: `web/dist/`
//! stays in zero's native form (a bare `zero build` still works for a UI
//! developer), the pass is single and non-doubling, and it cannot be bypassed.
//! This is cubby's `src/embed.rs`, taken verbatim for the reason the spec gives.

use std::sync::OnceLock;

use axum::extract::Path;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use rust_embed::RustEmbed;

use crate::app::SharedState;

/// zero's production build, embedded from `web/dist/`.
#[derive(RustEmbed)]
#[folder = "web/dist/"]
struct Assets;

/// The zero app's entry document, served at `/_/log`.
const INDEX_HTML: &str = "index.html";

/// zero's build manifest: source path → hashed output path.
const MANIFEST_JSON: &str = "manifest.json";

/// The source key the server-rendered pages read their stylesheet href from.
const STYLESHEET_SOURCE: &str = "styles/app.scss";

/// Hashed bundles and fonts never change under a given URL.
const IMMUTABLE_CACHE: &str = "public, max-age=31536000, immutable";

/// The index names the current hashed bundles, so it must not be cached.
const NO_CACHE: &str = "no-cache";

pub fn routes() -> Router<SharedState> {
    Router::new()
        // **The one page lanyard ships JavaScript for.** `/_/` and the rejection
        // page stay server-rendered with no script tag, because `/authorize`
        // redirects a real browser to `/_/` and that path is committed to
        // working without JavaScript (Phase 4, criterion 26).
        .route("/log", get(log_page))
        .route("/assets/{*path}", get(asset))
        .route("/.zero/{*path}", get(zero_asset))
}

async fn log_page() -> Response {
    match Assets::get(INDEX_HTML) {
        Some(file) => (
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::CACHE_CONTROL, NO_CACHE),
            ],
            rewrite_mount_prefix(&String::from_utf8_lossy(&file.data)),
        )
            .into_response(),
        // The build embeds `index.html`; its absence means a broken build, and
        // saying so beats a blank page.
        None => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            "the web UI is missing from this build\n",
        )
            .into_response(),
    }
}

async fn asset(Path(path): Path<String>) -> Response {
    serve(&format!("assets/{path}"))
}

async fn zero_asset(Path(path): Path<String>) -> Response {
    serve(&format!(".zero/{path}"))
}

/// One embedded file, or a `404`.
///
/// A miss is **not** an index fallback: lanyard's zero app has exactly one
/// route, and answering a mistyped hash with HTML would hand a browser a
/// document where it asked for a script.
fn serve(rel_path: &str) -> Response {
    let Some(file) = Assets::get(rel_path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let headers = [
        (header::CONTENT_TYPE, content_type_for(rel_path)),
        (header::CACHE_CONTROL, IMMUTABLE_CACHE),
    ];
    if needs_prefix_rewrite(rel_path) {
        let text = rewrite_mount_prefix(&String::from_utf8_lossy(&file.data));
        return (headers, text).into_response();
    }
    (headers, file.data.into_owned()).into_response()
}

/// The hashed stylesheet the server-rendered pages `<link>` to, read once from
/// the embedded `manifest.json`.
///
/// **No hash is ever written by hand.** This is what zero's documentation
/// prescribes for a backend, and it is what lets `/_/`, the rejection page and
/// `/_/log` render from one stylesheet without the picker's markup knowing a
/// content hash.
pub fn stylesheet_href() -> &'static str {
    static HREF: OnceLock<String> = OnceLock::new();
    HREF.get_or_init(|| {
        let hashed = Assets::get(MANIFEST_JSON)
            .and_then(|file| serde_json::from_slice::<serde_json::Value>(&file.data).ok())
            .and_then(|manifest| {
                manifest
                    .get(STYLESHEET_SOURCE)
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            });
        match hashed {
            Some(path) => format!("/_/{path}"),
            // A build with no manifest has no stylesheet to name. An empty href
            // would fetch the current page as CSS, so name the source instead:
            // it 404s honestly and says which file is missing.
            None => format!("/_/{STYLESHEET_SOURCE}"),
        }
    })
}

/// Which embedded files can carry root-absolute refs. Only the HTML and the CSS
/// do; the JS bundle has none (verified against zero's output, and pinned by
/// the test below).
fn needs_prefix_rewrite(path: &str) -> bool {
    matches!(path.rsplit('.').next(), Some("html") | Some("css"))
}

/// Prefix zero's root-absolute asset refs with the `/_/` mount. Applied only to
/// zero's native (un-prefixed) output, so it is a single, non-doubling pass.
fn rewrite_mount_prefix(body: &str) -> String {
    body.replace("/assets/", "/_/assets/")
        .replace("/.zero/", "/_/.zero/")
}

/// A content type from the extension. zero emits a small, fixed set of asset
/// kinds; anything unrecognized is served as opaque bytes rather than guessed
/// at.
fn content_type_for(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Proves the committed `web/dist/` is actually embedded — the whole of
    /// criterion 20 in one assertion.
    #[test]
    fn the_committed_bundle_is_embedded() {
        let file = Assets::get(INDEX_HTML).expect("index.html is embedded");
        let html = std::str::from_utf8(&file.data).unwrap();
        assert!(html.contains(r#"id="app""#), "it mounts the app: {html}");
    }

    #[test]
    fn serving_moves_zeros_root_absolute_refs_under_the_mount() {
        let raw = Assets::get(INDEX_HTML).unwrap();
        let served = rewrite_mount_prefix(&String::from_utf8_lossy(&raw.data));
        assert!(served.contains("/_/assets/"), "{served}");
        assert!(!served.contains("\"/assets/"), "{served}");
    }

    #[test]
    fn the_rewrite_covers_both_roots_and_is_otherwise_a_noop() {
        assert_eq!(rewrite_mount_prefix("<p>hi</p>"), "<p>hi</p>");
        assert_eq!(
            rewrite_mount_prefix(r#"<script src="/assets/app.js">"#),
            r#"<script src="/_/assets/app.js">"#
        );
        assert_eq!(
            rewrite_mount_prefix("url(/.zero/fonts/geist.woff2)"),
            "url(/_/.zero/fonts/geist.woff2)"
        );
    }

    /// The JS bundle is left alone on purpose: it carries no root-absolute
    /// refs, and rewriting a megabyte of JavaScript per request to prove it
    /// would be a cost with no payer.
    #[test]
    fn only_the_text_assets_that_can_carry_refs_are_rewritten() {
        assert!(needs_prefix_rewrite("index.html"));
        assert!(needs_prefix_rewrite("assets/app.f7a2f4f4.css"));
        assert!(!needs_prefix_rewrite("assets/app.a2256720.js"));
        assert!(!needs_prefix_rewrite(".zero/fonts/Geist.woff2"));
    }

    /// The bundled JS really does contain nothing to rewrite — the assumption
    /// the line above rests on, checked against the committed bundle rather
    /// than assumed.
    #[test]
    fn the_bundled_javascript_carries_no_root_absolute_asset_refs() {
        let js = Assets::iter()
            .find(|path| path.ends_with(".js"))
            .and_then(|path| Assets::get(&path))
            .expect("a bundled script");
        let text = String::from_utf8_lossy(&js.data);
        assert!(
            !text.contains("\"/assets/"),
            "the JS would need rewriting too"
        );
        assert!(
            !text.contains("\"/.zero/"),
            "the JS would need rewriting too"
        );
    }

    /// The href comes out of the manifest, so no hash is written by hand and a
    /// rebuild cannot leave the picker pointing at a file that is gone.
    #[test]
    fn the_stylesheet_href_comes_from_the_manifest_and_is_hashed() {
        let href = stylesheet_href();
        assert!(href.starts_with("/_/assets/app."), "{href}");
        assert!(href.ends_with(".css"), "{href}");
        assert!(
            Assets::get(href.trim_start_matches("/_/")).is_some(),
            "the manifest names a file that is embedded: {href}"
        );
    }

    #[test]
    fn content_types_cover_the_kinds_zero_emits() {
        assert_eq!(content_type_for("a.js"), "text/javascript; charset=utf-8");
        assert_eq!(content_type_for("a.css"), "text/css; charset=utf-8");
        assert_eq!(content_type_for("f.woff2"), "font/woff2");
        assert_eq!(content_type_for("OFL.txt"), "text/plain; charset=utf-8");
        assert_eq!(content_type_for("x.unknown"), "application/octet-stream");
    }
}
