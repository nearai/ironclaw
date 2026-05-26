//! Axum router that serves the embedded SPA bundle.
//!
//! Two concerns: serve raw embedded asset bytes for known paths, and
//! return the `index.html` shell (with a fresh per-request CSP
//! nonce substituted into the `__IRONCLAW_CSP_NONCE__` placeholder)
//! for the SPA root and any client-side route.
//!
//! Security headers, CORS, body/rate limits, and bearer auth are NOT
//! the router's concern — host composition wraps this Router with
//! its own middleware stack.

use axum::Router;
use axum::body::Body;
use axum::extract::Path as AxumPath;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use rand::RngCore;

use crate::assets::{self, INDEX_HTML_TEMPLATE};

/// Placeholder substituted with the per-request CSP nonce. The
/// fork's `index.html` already declares it; we just replace it.
const NONCE_PLACEHOLDER: &str = "__IRONCLAW_CSP_NONCE__";

/// Number of random bytes per nonce. 16 bytes hex-encoded = 32
/// characters, well above the CSP-3 recommendation of 128 bits.
const NONCE_BYTES: usize = 16;

/// Build the SPA static-asset router.
///
/// Mounted by host composition under `/v2`. The router owns no
/// per-instance state; each request generates a fresh nonce.
///
/// A single fallback handles every path so nesting semantics
/// (with or without a trailing slash on the mount prefix) resolve
/// uniformly to either an embedded asset, the SPA shell, or 404.
pub fn static_router() -> Router {
    // Three explicit routes keep `axum::Router::nest` out of the
    // picture — nest in 0.8 has quirky dispatch for the exact prefix
    // with/without trailing slash. The wildcard handler reads the
    // matched suffix via `Path` so the path passed downstream is
    // already prefix-stripped, no matter what mount the caller used.
    Router::new()
        .route("/", get(serve_root))
        .route("/{*path}", get(serve_wildcard))
}

/// Render the SPA shell with a freshly-substituted CSP nonce. Used
/// for the mount prefix's exact root and any client-side route the
/// SPA owns (e.g. `/chat/<id>`).
pub async fn serve_root() -> Response {
    render_index_with_nonce()
}

/// Resolve the wildcard suffix (post-prefix path) against the asset
/// table. Falls back to the SPA shell for client-side routes (any
/// path that has no file extension), 404 for unknown asset paths
/// that do look like asset requests.
pub async fn serve_wildcard(AxumPath(path): AxumPath<String>) -> Response {
    serve_for_path(&path)
}

fn serve_for_path(path: &str) -> Response {
    // Sanitize against `..` traversal segments even though the URL
    // table is a closed set; defense in depth keeps a future routing
    // change from leaking arbitrary file content if a host
    // misconfiguration ever permits raw query paths.
    if path.split('/').any(|seg| seg == ".." || seg == ".") {
        return StatusCode::NOT_FOUND.into_response();
    }

    // Empty path (only reachable through unusual routings) → SPA shell.
    if path.is_empty() {
        return render_index_with_nonce();
    }

    if let Some(asset) = assets::lookup(path) {
        return asset_response(asset.bytes, asset.content_type);
    }

    // Unknown path that does not look like a real asset request
    // (no `.` in any segment, so probably a client-side route like
    // `chat/123`) → serve the SPA shell so react-router can render
    // the right view.
    if !path.contains('.') {
        return render_index_with_nonce();
    }

    StatusCode::NOT_FOUND.into_response()
}

fn render_index_with_nonce() -> Response {
    let nonce = generate_nonce();
    let body = INDEX_HTML_TEMPLATE.replace(NONCE_PLACEHOLDER, &nonce);
    let mut response = Response::new(Body::from(body));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    // The browser must not cache the shell — the nonce changes per
    // request and the CSP header (set below) will reject a stale
    // nonce on the next load.
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    // CSP for the SPA shell. Per-request, scoped to this exact
    // response so the nonce attribute in the HTML matches the
    // `nonce-...` source the browser will accept. The composition
    // crate sets a stricter default CSP for JSON routes via
    // `SetResponseHeaderLayer::if_not_present`, which honors the
    // header we set here instead of overwriting it.
    //
    // The CDN origins below match `index.html`: React + react-router
    // + react-query + htm + react-hook-form from esm.sh, Tailwind
    // browser runtime from jsdelivr, dompurify + marked + highlight.js
    // from cdnjs, Google Fonts CSS + woff files.
    let csp = format!(
        "default-src 'self'; \
         script-src 'self' 'nonce-{nonce}' https://esm.sh https://cdn.jsdelivr.net https://cdnjs.cloudflare.com; \
         script-src-elem 'self' 'nonce-{nonce}' https://esm.sh https://cdn.jsdelivr.net https://cdnjs.cloudflare.com; \
         style-src 'self' 'unsafe-inline' https://fonts.googleapis.com https://cdn.jsdelivr.net; \
         style-src-elem 'self' 'unsafe-inline' https://fonts.googleapis.com https://cdn.jsdelivr.net; \
         font-src 'self' https://fonts.gstatic.com data:; \
         img-src 'self' data:; \
         connect-src 'self' https://esm.sh https://cdn.jsdelivr.net https://cdnjs.cloudflare.com; \
         object-src 'none'; \
         frame-ancestors 'none'; \
         base-uri 'self'",
    );
    // `HeaderValue::from_str` cannot fail for the literal+hex-nonce
    // input above; if a future edit introduces a non-ASCII byte the
    // request fails closed with 500 rather than serving the SPA shell
    // without a CSP header (silent fail-open is banned by the
    // error-handling rule).
    let value = match HeaderValue::from_str(&csp) {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(?error, "csp header build produced invalid HeaderValue");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    response
        .headers_mut()
        .insert(axum::http::header::CONTENT_SECURITY_POLICY, value);
    response
}

fn asset_response(bytes: &'static [u8], content_type: &'static str) -> Response {
    let mut response = Response::new(Body::from(bytes));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        // content_type strings come from build.rs and are static
        // ASCII; from_static cannot panic on the values we emit.
        HeaderValue::from_static(content_type),
    );
    response
}

fn generate_nonce() -> String {
    let mut buf = [0u8; NONCE_BYTES];
    rand::thread_rng().fill_bytes(&mut buf);
    let mut out = String::with_capacity(NONCE_BYTES * 2);
    for byte in &buf {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::{Method, Request};
    use tower::ServiceExt;

    async fn body_string(response: Response) -> String {
        let bytes = to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("body");
        String::from_utf8_lossy(&bytes).into_owned()
    }

    #[tokio::test]
    async fn standalone_root_returns_spa_shell() {
        let app = static_router();
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("oneshot");
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_string(response).await;
        assert!(body.contains("v2-root"));
        assert!(!body.contains("__IRONCLAW_CSP_NONCE__"));
    }

    #[tokio::test]
    async fn standalone_known_asset_returns_bytes() {
        let app = static_router();
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/styles/app.css")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("oneshot");
        assert_eq!(response.status(), StatusCode::OK);
        let ct = response
            .headers()
            .get(header::CONTENT_TYPE)
            .map(|v| v.to_str().unwrap().to_string())
            .unwrap_or_default();
        assert!(ct.starts_with("text/css"), "got `{ct}`");
    }

    #[tokio::test]
    async fn standalone_spa_shell_carries_matching_csp_nonce() {
        let app = static_router();
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("oneshot");
        assert_eq!(response.status(), StatusCode::OK);
        let csp = response
            .headers()
            .get(axum::http::header::CONTENT_SECURITY_POLICY)
            .expect("CSP header on SPA shell")
            .to_str()
            .expect("CSP ASCII")
            .to_string();
        let body = body_string(response).await;
        // Pull the nonce attribute from the HTML and assert the same
        // value appears inside the CSP's `nonce-...` source. Browsers
        // require an exact match; this regression-guards against a
        // future refactor that emits the CSP with a different nonce
        // than the one substituted into the document.
        let html_nonce = {
            let marker = "nonce=\"";
            let start = body.find(marker).expect("nonce attribute in HTML");
            let after = &body[start + marker.len()..];
            let end = after.find('"').expect("nonce attribute closed");
            after[..end].to_string()
        };
        assert!(
            csp.contains(&format!("'nonce-{html_nonce}'")),
            "CSP must allow the exact nonce embedded in the HTML — got `{csp}`",
        );
    }

    #[tokio::test]
    async fn standalone_spa_fallback_for_client_route() {
        let app = static_router();
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/chat/abc")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("oneshot");
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_string(response).await;
        assert!(body.contains("v2-root"));
    }

    #[test]
    fn nonce_is_unique_per_call() {
        let a = generate_nonce();
        let b = generate_nonce();
        assert_ne!(a, b);
        assert_eq!(a.len(), NONCE_BYTES * 2);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn index_template_contains_placeholder() {
        assert!(
            INDEX_HTML_TEMPLATE.contains(NONCE_PLACEHOLDER),
            "index.html must include `{}` so CSP nonce substitution has a target",
            NONCE_PLACEHOLDER,
        );
    }

    #[test]
    fn index_rendering_replaces_every_placeholder() {
        let nonce = generate_nonce();
        let rendered = INDEX_HTML_TEMPLATE.replace(NONCE_PLACEHOLDER, &nonce);
        assert!(rendered.contains(&nonce));
        assert!(!rendered.contains(NONCE_PLACEHOLDER));
    }

    #[test]
    fn asset_table_includes_known_files() {
        // Spot-check core SPA entry points — the chat surface (in-scope
        // for #3886) plus one representative page-tree file (extensions,
        // which wires the 9th v2 endpoint) — so a build.rs regression
        // that drops a whole subtree breaks loudly.
        for required in [
            "styles/app.css",
            "js/main.js",
            "js/lib/api.js",
            "js/app/app.js",
            "js/pages/chat/chat-page.js",
            "js/pages/extensions/extensions-page.js",
        ] {
            assert!(
                assets::lookup(required).is_some(),
                "expected `{required}` in the embedded asset table",
            );
        }
    }
}
