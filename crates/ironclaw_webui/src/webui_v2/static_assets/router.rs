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

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{Path as AxumPath, State};
use axum::http::{HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use rand::RngExt as _;
use thiserror::Error;

use super::assets::{self, INDEX_HTML_TEMPLATE};

/// Placeholder substituted with the per-request CSP nonce. The
/// fork's `index.html` already declares it; we just replace it.
const NONCE_PLACEHOLDER: &str = "__IRONCLAW_CSP_NONCE__";

/// Number of random bytes per nonce. 16 bytes hex-encoded = 32
/// characters, well above the CSP-3 recommendation of 128 bits.
const NONCE_BYTES: usize = 16;

/// Lower-case hexadecimal alphabet used to encode CSP nonce bytes without a
/// temporary allocation per byte.
const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

/// Server-owned root namespaces that stay fail-closed even when optional host
/// route mounts are disabled. Composition adds roots derived from every route
/// descriptor it actually mounts.
const DEFAULT_RESERVED_ROOT_NAMESPACES: &[&str] = &["api", "auth", "v1", "webhooks"];

/// Root namespaces with explicit Rust routes owned by the static WebUI surface.
///
/// React routes intentionally share the wildcard and may be replaced when host
/// composition explicitly reserves their root. Embedded asset roots are
/// checked from the generated asset table separately so adding a new top-level
/// asset cannot silently create a host route collision.
const EXPLICIT_STATIC_ROOT_NAMESPACES: &[&str] = &["v2", "wallet"];

/// Invalid root namespace supplied to [`StaticRouterConfig`].
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum StaticRouterConfigError {
    /// The value is not one canonical literal URL path segment.
    #[error("root namespace {namespace:?} must be one canonical ASCII URL path segment")]
    NonCanonicalRootNamespace { namespace: String },
    /// The static WebUI already owns behavior or assets below this root.
    #[error("root namespace {namespace:?} is owned by the static WebUI surface")]
    StaticRootNamespaceConflict { namespace: String },
}

/// Static SPA fallback policy supplied by host composition.
///
/// The default preserves the server namespaces that must never render the SPA
/// shell. Composition should add the first literal segment from every mounted
/// route descriptor so a future host-owned namespace automatically inherits
/// the same fail-closed behavior.
#[derive(Clone, Debug)]
pub struct StaticRouterConfig {
    reserved_root_namespaces: Vec<String>,
}

impl Default for StaticRouterConfig {
    fn default() -> Self {
        Self {
            reserved_root_namespaces: DEFAULT_RESERVED_ROOT_NAMESPACES
                .iter()
                .copied()
                .map(String::from)
                .collect(),
        }
    }
}

impl StaticRouterConfig {
    /// Add literal root path segments owned by host routes.
    ///
    /// Each namespace must be one non-empty ASCII RFC 3986 `pchar` segment,
    /// excluding percent-encoding, `.` and `..`. Static-owned roots are
    /// rejected because reserving one would either shadow an asset/route or be
    /// bypassed by a more specific static route.
    pub fn try_with_additional_reserved_root_namespaces<I, S>(
        mut self,
        namespaces: I,
    ) -> Result<Self, StaticRouterConfigError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for namespace in namespaces {
            let namespace = namespace.into();
            if !is_canonical_root_namespace(&namespace) {
                return Err(StaticRouterConfigError::NonCanonicalRootNamespace { namespace });
            }
            if static_router_owns_root_namespace(&namespace) {
                return Err(StaticRouterConfigError::StaticRootNamespaceConflict { namespace });
            }
            self.reserved_root_namespaces.push(namespace);
        }
        self.reserved_root_namespaces.sort_unstable();
        self.reserved_root_namespaces.dedup();
        Ok(self)
    }
}

fn is_canonical_root_namespace(namespace: &str) -> bool {
    !namespace.is_empty()
        && namespace != "."
        && namespace != ".."
        && namespace.bytes().all(is_unencoded_ascii_pchar)
}

fn is_unencoded_ascii_pchar(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'-' | b'.'
                | b'_'
                | b'~'
                | b'!'
                | b'$'
                | b'&'
                | b'\''
                | b'('
                | b')'
                | b'*'
                | b'+'
                | b','
                | b';'
                | b'='
                | b':'
                | b'@'
        )
}

fn static_router_owns_root_namespace(namespace: &str) -> bool {
    EXPLICIT_STATIC_ROOT_NAMESPACES.contains(&namespace)
        || assets::ASSETS
            .iter()
            .any(|(path, _)| path.split('/').next().is_some_and(|root| root == namespace))
}

#[derive(Clone)]
struct StaticRouterState {
    reserved_root_namespaces: Arc<[String]>,
}

/// Build the SPA static-asset router at the gateway root.
///
/// The canonical browser routes live at `/chat`, `/settings`, and the other
/// root-level SPA paths. The legacy `/v2` surface redirects to the matching
/// root path while preserving its query string so bookmarked URLs and OAuth
/// login tickets keep working during the migration. Host-owned namespaces
/// such as `/api`, `/auth`, `/v1`, and `/webhooks` remain fail-closed.
///
/// The router state contains only the immutable fallback namespace set; each
/// request still generates a fresh nonce.
pub fn static_router() -> Router {
    static_router_with_config(StaticRouterConfig::default())
}

/// Build the root SPA router with additional host-owned root namespaces.
pub fn static_router_with_config(config: StaticRouterConfig) -> Router {
    let state = StaticRouterState {
        reserved_root_namespaces: config.reserved_root_namespaces.into(),
    };
    // Explicit routes keep `axum::Router::nest` out of the picture — nest in
    // 0.8 has quirky dispatch for exact prefixes with/without trailing slash.
    // The wildcard handler receives the root-relative path via `Path`.
    Router::new()
        .route("/", get(serve_root))
        // Keep the isolated wallet page ahead of the SPA wildcard. It carries
        // a deliberately different CSP and must never render the app shell.
        .route("/wallet/connect", get(serve_wallet_connect))
        .route("/v2", get(redirect_legacy_v2))
        .route("/v2/", get(redirect_legacy_v2))
        .route("/v2/{*path}", get(redirect_legacy_v2))
        // A custom method fallback keeps unmounted POST/PUT API paths at 404.
        // Without it, this root wildcard would turn them into 405 responses
        // merely because the SPA owns GET for the same catch-all path.
        .route(
            "/{*path}",
            get(serve_configured_wildcard).fallback(|| async { StatusCode::NOT_FOUND }),
        )
        .with_state(state)
}

/// Redirect a legacy `/v2` URL to its root-mounted equivalent.
///
/// A temporary redirect is intentional: it keeps compatibility links working
/// without leaving a browser-cached permanent redirect behind if the root mount
/// ever needs to be rolled back. Leading slashes in the suffix are collapsed
/// so an input such as `/v2//example.com` cannot become a protocol-relative
/// redirect target. Backslashes are normalized as path separators as well:
/// WHATWG URL parsing treats them like slashes for HTTP(S), so forwarding a
/// raw backslash could otherwise turn a same-origin `Location` into an
/// external navigation.
///
/// Only literal `/` and `\` need normalization here. Hyper parses the request
/// target into `http::Uri` before Axum calls this handler, and that parser
/// rejects raw ASCII whitespace and control characters. Percent-encoded bytes
/// remain encoded in `Uri::path()` and therefore stay ordinary same-origin path
/// data after the owned leading `/` below. Do not refactor this helper to use a
/// percent-decoded `Path<String>` without preserving those invariants.
async fn redirect_legacy_v2(uri: Uri) -> Redirect {
    Redirect::temporary(&legacy_v2_target(&uri))
}

fn legacy_v2_target(uri: &Uri) -> String {
    let suffix = match uri.path().strip_prefix("/v2") {
        Some(suffix) => suffix.trim_start_matches(['/', '\\']),
        None => "",
    };
    let mut target = String::from("/");
    target.push_str(&suffix.replace('\\', "/"));
    if let Some(query) = uri.query() {
        target.push('?');
        target.push_str(query);
    }
    target
}

/// Serve the isolated NEAR-wallet connect popup with its own relaxed CSP.
///
/// This page is deliberately quarantined from the SPA: it holds no session
/// bearer and no app state, connects a NEAR wallet, signs the fixed NEAR AI
/// login message, and posts the signature over a random same-origin
/// `BroadcastChannel`. The authenticated SPA relays the signature to the
/// backend, so no secret ever lives on this looser-CSP page.
pub async fn serve_wallet_connect() -> Response {
    let Some(asset) = assets::lookup("wallet-connect.html") else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut response = asset_response(asset.bytes, asset.content_type);
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    // Wallet connectors load remote executor code into sandboxed iframes and
    // reach a range of wallet relays + NEAR RPC endpoints that vary per wallet,
    // so script/connect/frame sources can't be pinned to a fixed allow-list
    // without breaking wallets. `'unsafe-inline'` is required because the
    // connector injects inline bootstrap scripts into its `srcdoc` sandbox
    // frames (which run as unique opaque origins). This is acceptable only
    // because the page is input-less and isolated; it must never gain app data.
    let csp = "default-src 'self'; \
         script-src 'self' 'unsafe-inline' https:; \
         script-src-elem 'self' 'unsafe-inline' https:; \
         style-src 'self' 'unsafe-inline'; \
         img-src 'self' data: https:; \
         connect-src 'self' https:; \
         frame-src 'self' https: data:; \
         object-src 'none'; \
         base-uri 'self'";
    response.headers_mut().insert(
        axum::http::header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(csp),
    );
    response
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
    serve_for_path(&path, DEFAULT_RESERVED_ROOT_NAMESPACES)
}

async fn serve_configured_wildcard(
    State(state): State<StaticRouterState>,
    AxumPath(path): AxumPath<String>,
) -> Response {
    serve_for_path(&path, &state.reserved_root_namespaces)
}

fn serve_for_path<T>(path: &str, reserved_root_namespaces: &[T]) -> Response
where
    T: AsRef<str>,
{
    // Axum strips exactly one slash from a wildcard capture. A request with
    // repeated leading slashes therefore reaches this helper with a leading
    // slash still present (for example, `//api/x` becomes `/api/x`). Its Path
    // extractor also percent-decodes backslashes, which browsers may treat as
    // path separators. Reject either non-canonical form instead of normalizing
    // it: otherwise the namespace check below could be bypassed and render the
    // SPA shell for a server-owned path.
    if path.starts_with('/') || path.contains('\\') {
        return StatusCode::BAD_REQUEST.into_response();
    }

    // Sanitize against `..` traversal segments even though the URL
    // table is a closed set; defense in depth keeps a future routing
    // change from leaking arbitrary file content if a host
    // misconfiguration ever permits raw query paths.
    if path.split('/').any(|seg| seg == ".." || seg == ".") {
        return StatusCode::NOT_FOUND.into_response();
    }

    // Root-mounting the SPA must not turn unknown host/API requests into a
    // successful HTML response. Exact registered routes still win in axum;
    // unmatched requests in these server-owned namespaces fail closed here.
    if path.split('/').next().is_some_and(|root| {
        reserved_root_namespaces
            .iter()
            .any(|reserved| reserved.as_ref() == root)
    }) {
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
    // (last segment has no file extension, so probably a client-side
    // route like `chat/abc` or `chat/user.123`) → serve the SPA shell
    // so react-router can render the right view. We check only the
    // last segment so a route like `profile/john.doe` doesn't get
    // misclassified as an asset request just because an earlier
    // segment happened to contain a dot.
    let last_segment = path.rsplit('/').next().unwrap_or(path);
    if !last_segment.contains('.') {
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
    // Every sub-resource the shell loads is same-origin: Vite emits the app
    // bundle and CSS under `/assets/`, and the web fonts are vendored under
    // `/vendor/` (see `frontend/public`). So `script-src` / `style-src` /
    // `font-src` collapse to `'self'` — no CDN origins, no third-party fetches.
    // `'unsafe-inline'` stays on `style-src` only: the Tailwind browser
    // runtime injects a generated `<style>` and the shell carries an
    // inline theme styles; inline scripts rely on the per-request nonce,
    // never `'unsafe-inline'` (same-origin external bundles use `'self'`).
    let csp = format!(
        "default-src 'self'; \
         script-src 'self' 'nonce-{nonce}'; \
         script-src-elem 'self' 'nonce-{nonce}'; \
         style-src 'self' 'unsafe-inline'; \
         style-src-elem 'self' 'unsafe-inline'; \
         font-src 'self'; \
         img-src 'self' data:; \
         media-src 'self' data:; \
         frame-src 'self' blob:; \
         connect-src 'self'; \
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
    rand::rng().fill(&mut buf);
    let mut out = String::with_capacity(NONCE_BYTES * 2);
    for byte in &buf {
        out.push(HEX_DIGITS[(byte >> 4) as usize] as char);
        out.push(HEX_DIGITS[(byte & 0x0f) as usize] as char);
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

    fn asset_path_with(prefix: &str, suffix: &str) -> &'static str {
        assets::ASSETS
            .iter()
            .map(|(path, _)| *path)
            .find(|path| path.starts_with(prefix) && path.ends_with(suffix))
            .unwrap_or_else(|| panic!("asset path matching {prefix:?} and {suffix:?} exists"))
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
    async fn legacy_v2_routes_redirect_to_root_and_preserve_query() {
        let app = static_router();
        for (source, target) in [
            ("/v2", "/"),
            ("/v2/", "/"),
            ("/v2?login_ticket=ticket%2B1", "/?login_ticket=ticket%2B1"),
            (
                "/v2/settings/skills?token=old%2Btoken&tab=installed",
                "/settings/skills?token=old%2Btoken&tab=installed",
            ),
            // Repeated leading slashes must not produce a protocol-relative
            // Location header (an open redirect to `evil.example`).
            ("/v2//evil.example?keep=1", "/evil.example?keep=1"),
            // Browsers normalize backslashes as URL path separators. Keep a
            // raw backslash from becoming a network-path redirect target.
            (r"/v2/\evil.example", "/evil.example"),
            (r"/v2/path\segment", "/path/segment"),
            // `http::Uri` preserves percent-encoded bytes. Even encoded ASCII
            // whitespace or separators therefore remain path data after the
            // same-origin leading slash instead of becoming a URL authority.
            ("/v2/%09//evil.example", "/%09//evil.example"),
            ("/v2/%0A//evil.example", "/%0A//evil.example"),
            ("/v2/%0D//evil.example", "/%0D//evil.example"),
            ("/v2/%0C//evil.example", "/%0C//evil.example"),
            ("/v2/%20//evil.example", "/%20//evil.example"),
            ("/v2/%2F%2Fevil.example", "/%2F%2Fevil.example"),
            ("/v2/%5C%5Cevil.example", "/%5C%5Cevil.example"),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(Method::GET)
                        .uri(source)
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("oneshot");
            assert_eq!(
                response.status(),
                StatusCode::TEMPORARY_REDIRECT,
                "GET {source}",
            );
            assert_eq!(
                response
                    .headers()
                    .get(header::LOCATION)
                    .and_then(|value| value.to_str().ok()),
                Some(target),
                "GET {source}",
            );
        }
    }

    #[test]
    fn legacy_v2_raw_ascii_whitespace_is_rejected_by_uri_parser() {
        for character in ['\t', '\n', '\r', '\u{000c}', ' '] {
            let source = format!("/v2/{character}//evil.example");
            assert!(
                source.parse::<Uri>().is_err(),
                "raw ASCII whitespace must not reach redirect construction: {source:?}",
            );
        }
    }

    #[tokio::test]
    async fn standalone_known_asset_returns_bytes() {
        let app = static_router();
        let css_path = asset_path_with("assets/app-", ".css");
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!("/{css_path}"))
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
    async fn spa_document_csp_allowlist_is_locked() {
        // Every sub-resource the SPA loads is now same-origin (the Vite
        // bundle under `/assets/` plus self-hosted fonts under `/vendor/`).
        // Lock that in:
        // a regression that re-introduced a third-party CDN origin, added
        // `unsafe-eval`, or allowed `'unsafe-inline'` scripts must fail
        // here, not ship silently. (security-parity 03 row 3b.)
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

        // No third-party origin may appear anywhere in the document CSP —
        // the SPA is fully self-hosted. The previous esm.sh / jsdelivr /
        // cdnjs / fonts.google* allowances are gone; assert they stay gone
        // (and that no other scheme-host slips in) by banning any `http`
        // token outside the same-origin keywords.
        for banned in [
            "esm.sh",
            "jsdelivr",
            "cdnjs",
            "fonts.googleapis.com",
            "fonts.gstatic.com",
            "https://",
            "http://",
        ] {
            assert!(
                !csp.contains(banned),
                "document CSP must not reference `{banned}` (all assets are same-origin); got `{csp}`",
            );
        }
        assert!(
            csp.contains("'nonce-"),
            "document `script-src` must carry a per-request nonce; got `{csp}`",
        );
        assert!(
            csp.contains("object-src 'none'"),
            "document CSP must keep `object-src 'none'`; got `{csp}`",
        );
        // Attachment preview needs inline audio (`media-src data:`) and inline
        // PDF via blob iframes (`frame-src blob:`). Assert the EXACT source
        // list per directive (not a substring) so a regression that widens them
        // — e.g. `media-src 'self' data: https://evil` — fails here.
        let directives: std::collections::HashMap<_, _> = csp
            .split(';')
            .map(str::trim)
            .filter(|directive| !directive.is_empty())
            .filter_map(|directive| directive.split_once(' '))
            .map(|(name, sources)| (name, sources.trim()))
            .collect();
        assert_eq!(
            directives.get("media-src").copied(),
            Some("'self' data:"),
            "document CSP must keep the exact media-src allowlist; got `{csp}`",
        );
        assert_eq!(
            directives.get("frame-src").copied(),
            Some("'self' blob:"),
            "document CSP must keep the exact frame-src allowlist; got `{csp}`",
        );
        assert_eq!(
            directives.get("img-src").copied(),
            Some("'self' data:"),
            "document CSP must keep the exact img-src allowlist; got `{csp}`",
        );
        // Lock the same-origin directives to their EXACT source lists. A
        // substring ban misses valid-but-remote CSP forms (`https:`, `*`,
        // `cdn.example.com`); pinning the whole source list fails closed if
        // any new source — scheme, wildcard, or host — is ever appended.
        for (directive, expected) in [
            ("default-src", "'self'"),
            ("style-src", "'self' 'unsafe-inline'"),
            ("style-src-elem", "'self' 'unsafe-inline'"),
            ("font-src", "'self'"),
            ("connect-src", "'self'"),
            ("object-src", "'none'"),
            ("frame-ancestors", "'none'"),
            ("base-uri", "'self'"),
        ] {
            assert_eq!(
                directives.get(directive).copied(),
                Some(expected),
                "document CSP must keep the exact {directive} allowlist; got `{csp}`",
            );
        }
        // Scripts must NOT be executable via eval or arbitrary inline —
        // the document relies on the nonce, not `'unsafe-inline'`.
        assert!(
            !csp.contains("'unsafe-eval'"),
            "document CSP must not allow `unsafe-eval`; got `{csp}`",
        );
        let script_src = csp
            .split(';')
            .map(str::trim)
            .find(|d| d.starts_with("script-src ") || *d == "script-src")
            .expect("script-src directive present");
        assert!(
            !script_src.contains("'unsafe-inline'"),
            "document `script-src` must not allow `'unsafe-inline'` (nonce-based); got `{script_src}`",
        );
    }

    #[tokio::test]
    async fn wallet_connect_popup_gets_relaxed_csp_and_spa_shell_stays_strict() {
        let app = static_router();

        // The isolated wallet popup carries a deliberately relaxed CSP so the
        // wallet connector can load remote executors and reach wallet relays.
        let wallet = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/wallet/connect")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("oneshot");
        assert_eq!(wallet.status(), StatusCode::OK);
        let wallet_csp = wallet
            .headers()
            .get(axum::http::header::CONTENT_SECURITY_POLICY)
            .expect("CSP on wallet popup")
            .to_str()
            .expect("CSP ASCII")
            .to_string();
        assert!(
            wallet_csp.contains("'unsafe-inline'")
                && wallet_csp.contains("connect-src 'self' https:"),
            "wallet popup CSP must be the relaxed policy — got `{wallet_csp}`",
        );

        // The SPA shell must NOT inherit that looseness: no `'unsafe-inline'`
        // scripts and connect-src stays same-origin only.
        let shell = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("oneshot");
        let shell_csp = shell
            .headers()
            .get(axum::http::header::CONTENT_SECURITY_POLICY)
            .expect("CSP on SPA shell")
            .to_str()
            .expect("CSP ASCII")
            .to_string();
        assert!(
            shell_csp.contains("connect-src 'self';")
                && !shell_csp.contains("script-src 'self' 'unsafe-inline'"),
            "SPA shell CSP must stay strict (nonce-based scripts, same-origin connect) — got `{shell_csp}`",
        );
    }

    #[tokio::test]
    async fn standalone_path_traversal_segments_return_not_found() {
        // Defense-in-depth check: the asset table is a closed set built
        // from `static/` so traversal could never escape the embedded
        // bundle, but `serve_for_path` still rejects any path with `..`
        // or `.` segments. If a future routing change starts forwarding
        // raw OS paths into the asset lookup this regression test fails
        // loudly before any leak ships.
        let app = static_router();
        for path in [
            "/../../etc/passwd",
            "/js/../../../etc/passwd",
            "/./../../etc/passwd",
            "/styles/../../../etc/passwd",
            "/foo/./bar",
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(Method::GET)
                        .uri(path)
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("oneshot");
            assert_eq!(
                response.status(),
                StatusCode::NOT_FOUND,
                "path `{path}` must be rejected with 404",
            );
        }
    }

    #[tokio::test]
    async fn standalone_reserved_host_namespaces_do_not_fall_back_to_spa() {
        let app = static_router();
        for (method, path) in [
            (Method::GET, "/api/not-a-route"),
            (Method::POST, "/api/not-a-route"),
            (Method::GET, "/auth/not-a-route"),
            (Method::POST, "/auth/not-a-route"),
            (Method::GET, "/v1/not-a-route"),
            (Method::POST, "/v1/not-a-route"),
            (Method::GET, "/webhooks/not-a-route"),
            (Method::POST, "/webhooks/not-a-route"),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method.clone())
                        .uri(path)
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("oneshot");
            assert_eq!(
                response.status(),
                StatusCode::NOT_FOUND,
                "{method} reserved host path `{path}` must not render the SPA shell",
            );
        }
    }

    #[tokio::test]
    async fn configured_host_namespace_does_not_fall_back_to_spa() {
        let app = static_router_with_config(
            StaticRouterConfig::default()
                .try_with_additional_reserved_root_namespaces(["future-host"])
                .expect("canonical unowned namespace"),
        );
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/future-host/not-a-route")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("oneshot");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn static_router_config_rejects_noncanonical_root_namespaces() {
        for namespace in [
            "",
            "/future-host",
            "future-host/child",
            r"future\host",
            ".",
            "..",
            "%66uture-host",
            "{namespace}",
            "future{namespace}",
            "future host",
            "future\nroot",
            "futuré-host",
        ] {
            let error = StaticRouterConfig::default()
                .try_with_additional_reserved_root_namespaces([namespace])
                .expect_err("noncanonical namespace must fail closed");
            assert_eq!(
                error,
                StaticRouterConfigError::NonCanonicalRootNamespace {
                    namespace: namespace.to_string(),
                },
            );
        }
    }

    #[test]
    fn static_router_config_accepts_unencoded_ascii_pchar_namespaces() {
        StaticRouterConfig::default()
            .try_with_additional_reserved_root_namespaces([
                "foo@bar",
                "oauth:callback",
                "foo!$&'()*+,;=bar",
            ])
            .expect("unencoded RFC 3986 pchar namespaces are canonical");
    }

    #[test]
    fn static_router_config_rejects_static_owned_root_namespaces() {
        for namespace in [
            "v2",
            "wallet",
            "assets",
            "vendor",
            "wallet-connect.html",
            "wallet-connect.js",
        ] {
            let error = StaticRouterConfig::default()
                .try_with_additional_reserved_root_namespaces([namespace])
                .expect_err("static-owned namespace must fail closed");
            assert_eq!(
                error,
                StaticRouterConfigError::StaticRootNamespaceConflict {
                    namespace: namespace.to_string(),
                },
            );
        }
    }

    #[tokio::test]
    async fn standalone_noncanonical_path_separators_are_rejected() {
        let app = static_router();
        for path in [
            "//api/not-a-route",
            "///auth/not-a-route",
            "//v1/not-a-route",
            "//webhooks/not-a-route",
            "/%2Fapi/not-a-route",
            r"/\api/not-a-route",
            "/%5Capi/not-a-route",
            r"/api\not-a-route",
            "/api%5Cnot-a-route",
            "//chat",
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(Method::GET)
                        .uri(path)
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("oneshot");
            assert_eq!(
                response.status(),
                StatusCode::BAD_REQUEST,
                "malformed path `{path}` must be rejected before SPA fallback",
            );
        }
    }

    #[tokio::test]
    async fn standalone_no_dot_path_falls_back_to_spa_shell() {
        // Single-segment client-side routes (e.g. `/admin`, `/login`,
        // `/settings`) have no slashes and no dots, so the fallback
        // logic must serve the SPA shell rather than 404. The
        // multi-segment case is covered by
        // `standalone_spa_fallback_for_client_route`; this guards the
        // simpler form which the wildcard handler also has to match.
        let app = static_router();
        for path in ["/admin", "/login", "/settings"] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(Method::GET)
                        .uri(path)
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("oneshot");
            assert_eq!(
                response.status(),
                StatusCode::OK,
                "single-segment client route `{path}` should fall back to SPA shell",
            );
            let body = body_string(response).await;
            assert!(body.contains("v2-root"), "`{path}` did not render shell");
        }
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

    #[tokio::test]
    async fn standalone_spa_fallback_accepts_dot_in_non_terminal_segment() {
        // A client-side route may have a dot in a middle segment
        // while the final segment has no extension (e.g. the React
        // router's segment-versioning convention). The fallback
        // decision must only look at the last segment — under the
        // previous full-path `.contains('.')` check these routes
        // would 404 instead of rendering the SPA shell.
        let app = static_router();
        for path in ["/a.b/c", "/v1.2/dashboard"] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(Method::GET)
                        .uri(path)
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("oneshot");
            assert_eq!(
                response.status(),
                StatusCode::OK,
                "client-side route `{path}` should fall back to the SPA shell",
            );
            let body = body_string(response).await;
            assert!(body.contains("v2-root"), "`{path}` did not render shell");
        }
    }

    #[test]
    fn nonce_is_unique_per_call() {
        let a = generate_nonce();
        let b = generate_nonce();
        assert_ne!(a, b);
        assert_eq!(a.len(), NONCE_BYTES * 2);
        assert!(
            a.bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        );
    }

    #[test]
    fn index_template_contains_placeholder() {
        assert!(
            INDEX_HTML_TEMPLATE.contains(NONCE_PLACEHOLDER),
            "index.html must include `{}` so CSP nonce substitution has a target",
            NONCE_PLACEHOLDER,
        );
        assert!(
            !INDEX_HTML_TEMPLATE.contains("/v2/"),
            "root-mounted SPA shell must not reference legacy /v2 assets",
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
        // Spot-check the generated SPA bundle plus committed public assets so
        // a build.rs regression that drops either class breaks loudly.
        assert!(
            assets::ASSETS
                .iter()
                .any(|(path, _)| path.starts_with("assets/app-") && path.ends_with(".js"))
        );
        assert!(
            assets::ASSETS
                .iter()
                .any(|(path, _)| path.starts_with("assets/app-") && path.ends_with(".css"))
        );
        for required in [
            "wallet-connect.js",
            "wallet-connect.html",
            "assets/favicon.svg",
            "vendor/fonts/fonts.css",
        ] {
            assert!(
                assets::lookup(required).is_some(),
                "expected `{required}` in the embedded asset table",
            );
        }
    }

    #[test]
    fn pwa_manifest_uses_root_scope_and_assets() {
        let manifest = assets::lookup("assets/site.webmanifest")
            .expect("PWA manifest is embedded in the asset table");
        let value: serde_json::Value =
            serde_json::from_slice(manifest.bytes).expect("PWA manifest is valid JSON");
        // Preserve the identity of installations created while the app lived
        // at `/v2/`, even though launches and navigation now use root paths.
        assert_eq!(value["id"], "/v2/");
        assert_eq!(value["start_url"], "/");
        assert_eq!(value["scope"], "/");
        let icons = value["icons"].as_array().expect("manifest icons array");
        assert!(!icons.is_empty(), "manifest must keep install icons");
        assert!(icons.iter().all(|icon| {
            icon["src"]
                .as_str()
                .is_some_and(|source| source.starts_with("/assets/"))
        }));
    }

    fn source_text(path: &str) -> String {
        let full = format!("{}/frontend/src/{path}", env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("read {full}: {e}"))
    }

    // Locks the WebChat v2 SSO login-ticket contract documented
    // in `frontend/src/app/auth.ts` (issue #4116 review finding #11). The
    // user-visible OAuth login path is "callback redirects to
    // `/?login_ticket=<ticket>` → SPA strips the ticket from the
    // URL → exchanges it via `/auth/session/exchange` → stores the
    // returned bearer in sessionStorage".
    //
    // No JS test runner ships in this workspace and a real
    // Playwright e2e for the OAuth flow requires Google
    // credentials. This Rust assertion is the lightweight
    // regression: it inspects the frontend source for the
    // call shapes that implement each invariant. A refactor that
    // drops any one of them fails loudly here; the deep semantics
    // belong on a follow-up e2e once the SSO mount is wired into
    // a real binary.
    #[test]
    fn auth_js_carries_login_ticket_contract() {
        let source = source_text("app/auth.ts");
        let consume_token = source
            .split("function consumeTokenFromUrl()")
            .nth(1)
            .and_then(|tail| tail.split("function consumeLoginTicketFromUrl()").next())
            .expect("auth.ts must define consumeTokenFromUrl before consumeLoginTicketFromUrl");
        let consume_ticket = source
            .split("function consumeLoginTicketFromUrl()")
            .nth(1)
            .and_then(|tail| tail.split("// Map opaque error codes").next())
            .expect("auth.ts must define consumeLoginTicketFromUrl before login error mapping");

        // 1. Reads and strips the one-time login ticket from the
        //    query string before exchanging it for the bearer.
        assert!(
            source.contains("consumeLoginTicketFromUrl"),
            "auth.js must consume login tickets; got:\n{source}",
        );
        assert!(
            source.contains("login_ticket"),
            "auth.js must read the login_ticket query param",
        );
        assert!(
            source.contains("exchangeLoginTicket"),
            "auth.js must exchange the login ticket for a bearer",
        );

        // 2. Strips consumed URL credentials via `history.replaceState`,
        //    so a copy-pasted address bar does not leak them.
        assert!(
            source.contains("history.replaceState"),
            "auth.js must call history.replaceState to clean the URL",
        );

        // 3. Refuses to overwrite an existing stored token for raw
        //    bearer URLs — `consumeTokenFromUrl` must early-return when
        //    `readStoredToken()` is truthy. This guards against the
        //    `/#token=BAD` lock-out scenario the doc-comment
        //    calls out.
        let guard_index = consume_token
            .find("if (readStoredToken())")
            .expect("consumeTokenFromUrl must check sessionStorage before storing URL tokens");
        let store_index = consume_token
            .find("storeToken(token)")
            .expect("consumeTokenFromUrl must store accepted raw bearer URL tokens");
        assert!(
            guard_index < store_index
                && consume_token[guard_index..store_index].contains("return \"\";"),
            "consumeTokenFromUrl must early-return before storeToken(token) when a stored token exists",
        );

        // 4. OAuth callback tickets are trusted, single-use login
        //    continuations. They must still be exchanged when a stale
        //    sessionStorage token exists, so an intentional relogin can
        //    replace the previous bearer.
        assert!(
            consume_ticket.contains("storeToken(\"\")"),
            "login tickets must clear stale sessionStorage before exchange fallback is possible",
        );
        assert!(
            source.contains("loginTicket ? \"\" : consumeTokenFromUrl() || readStoredToken()"),
            "login-ticket initialization must bypass any stale stored bearer before exchange",
        );
        assert!(
            source.contains("Boolean(loginTicket)"),
            "login tickets must trigger exchange even when sessionStorage already has a token",
        );
        assert!(
            source.contains("Boolean(!loginTicket && readStoredToken())"),
            "stored-token session checks must be suppressed while a login-ticket exchange is pending",
        );
        assert!(
            !source.contains("loginTicket && !readStoredToken()")
                && !source.contains("!loginTicket || readStoredToken()"),
            "login-ticket exchange must not be skipped because sessionStorage already has a token",
        );

        // 5. Logout calls the server-side revoke endpoint —
        //    locks the regression where `signOut` drops the local
        //    token without telling the server (which would let the
        //    bearer roam in other tabs until natural expiry).
        assert!(
            source.contains("logoutRequest"),
            "signOut must fire-and-forget the server-side revoke",
        );

        // 6. Surfaces the OAuth callback's `?login_error=<code>`
        //    so users who deny consent or trip a hd / state guard
        //    see an explanation instead of a blank login page.
        assert!(
            source.contains("login_error"),
            "auth.js must consume the OAuth `?login_error=` redirect",
        );
    }
}
