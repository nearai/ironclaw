//! Static file serving, CSP assembly, and the frontend HTML bundle.
//!
//! This module owns everything the browser pulls from unauthenticated or
//! project-scoped routes: the CSP directive set (single source of truth for
//! both the global response header and the per-response nonce variant), the
//! embedded asset handlers (`/`, `/style.css`, `/app.js`, `/theme.css`,
//! `/favicon.ico`, `/i18n/*`, `/admin*`), the `build_frontend_html` path
//! that splices `.system/gateway/` customizations into the embedded SPA,
//! and the authenticated `/projects/{id}/...` file-serving routes.
//!
//! No feature handlers should depend on the private pieces here — only on
//! the `pub(crate)` surface registered by `start_server()`.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use sha2::{Digest, Sha256};

use ironclaw_gateway::assets;
use ironclaw_gateway::{FrontendBundle, LayoutConfig, NONCE_PLACEHOLDER};

use crate::bootstrap::ironclaw_base_dir;
use crate::channels::web::auth::AuthenticatedUser;
use crate::channels::web::handlers::frontend::{load_resolved_widgets, read_layout_config};
use crate::channels::web::platform::state::{FrontendCacheKey, FrontendHtmlCache, GatewayState};
use crate::channels::web::types::HealthResponse;

// --- Content Security Policy ---
//
// A single source of truth for the gateway's CSP. The static value below is
// used by the global response-header layer for every endpoint.
//
// The gateway serves two flavors of CSP on the same set of directives:
//
// * The static header applied by `SetResponseHeaderLayer` to *every*
//   response (see [`BASE_CSP_HEADER`]). No inline scripts are authorized.
// * A per-response variant produced by [`build_csp`] with a `'nonce-…'`
//   source added to `script-src`, used only by `index_handler` when it
//   serves customized HTML containing inline `<script>` blocks.
//
// Both variants MUST carry the same directive set except for `script-src`
// — if one grows a new `connect-src` origin, the other silently stays on
// the old policy, and customized pages end up under a stricter CSP than
// plain pages (or vice versa). Previous versions of this file duplicated
// the full directive string in two places, so adding a CDN to one was a
// latent regression waiting to happen. Keep every directive as a named
// constant and assemble both flavors via [`build_csp`] so there is a
// single source of truth.

/// `script-src` sources other than `'self'` and the per-response nonce.
const SCRIPT_SRC_EXTRAS: &str =
    "https://cdn.jsdelivr.net https://cdnjs.cloudflare.com https://esm.sh";
const STYLE_SRC: &str = "'self' 'unsafe-inline' https://fonts.googleapis.com";
const FONT_SRC: &str = "https://fonts.gstatic.com data:";
const CONNECT_SRC: &str =
    "'self' https://esm.sh https://rpc.mainnet.near.org https://rpc.testnet.near.org";
const IMG_SRC: &str =
    "'self' data: blob: https://*.googleusercontent.com https://avatars.githubusercontent.com";
const FRAME_SRC: &str = "https://accounts.google.com https://appleid.apple.com";
const FORM_ACTION: &str =
    "'self' https://accounts.google.com https://github.com https://appleid.apple.com";

/// Build a CSP string. When `nonce` is `Some`, the resulting policy adds
/// `'nonce-{nonce}'` to `script-src` so a single inline `<script
/// nonce="{nonce}">` block on the same response is authorized. When
/// `nonce` is `None`, the policy matches the static header emitted by
/// [`BASE_CSP_HEADER`]. This is the single source of truth for the
/// gateway CSP — edit per-directive constants above, not the format
/// string here.
pub(crate) fn build_csp(nonce: Option<&str>) -> String {
    let script_nonce = match nonce {
        Some(n) => format!(" 'nonce-{n}'"),
        None => String::new(),
    };
    format!(
        "default-src 'self'; \
         script-src 'self'{script_nonce} {SCRIPT_SRC_EXTRAS}; \
         style-src {STYLE_SRC}; \
         font-src {FONT_SRC}; \
         connect-src {CONNECT_SRC}; \
         img-src {IMG_SRC}; \
         frame-src {FRAME_SRC}; \
         object-src 'none'; \
         frame-ancestors 'none'; \
         base-uri 'self'; \
         form-action {FORM_ACTION}"
    )
}

/// Static CSP header applied to every gateway response by the
/// response-header layer. Assembled at first use via [`build_csp`] with no
/// nonce. Falls back to a minimally-permissive `default-src 'self'` if the
/// assembled value somehow fails to parse as a `HeaderValue` — in practice
/// the assembled string is pure ASCII and this branch is unreachable, but
/// production code in this repo avoids panics on request-path values, so
/// we fall back instead of calling `expect`.
pub(crate) static BASE_CSP_HEADER: std::sync::LazyLock<header::HeaderValue> =
    std::sync::LazyLock::new(|| {
        header::HeaderValue::from_str(&build_csp(None))
            .unwrap_or_else(|_| header::HeaderValue::from_static("default-src 'self'"))
    });

/// Build a CSP equivalent to the static header but with `'nonce-{nonce}'`
/// added to the `script-src` directive. Thin wrapper kept for call-site
/// readability (the name is the contract the nonce handler wants).
pub(crate) fn build_csp_with_nonce(nonce: &str) -> String {
    build_csp(Some(nonce))
}

/// Generate a fresh per-response CSP nonce. 16 random bytes hex-encoded
/// (32 chars) — well above the 128-bit minimum recommended for nonces and
/// matching the `OsRng + hex` pattern used elsewhere in this module.
pub(crate) fn generate_csp_nonce() -> String {
    use rand::RngCore;
    use rand::rngs::OsRng;
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

// --- Frontend bundle assembly ---

/// Compute a cheap cache key for `build_frontend_html` — one `list` call
/// against `.system/gateway/`. The directory entry for `widgets/` carries the
/// max `updated_at` of its children, so any widget file edit naturally bubbles
/// into the key without needing to read individual manifests.
async fn compute_frontend_cache_key(workspace: &crate::workspace::Workspace) -> FrontendCacheKey {
    let Ok(entries) = workspace.list(".system/gateway/").await else {
        return FrontendCacheKey {
            layout: None,
            widgets: None,
        };
    };
    let mut key = FrontendCacheKey {
        layout: None,
        widgets: None,
    };
    for entry in entries {
        let ts = entry
            .updated_at
            .map(|t| (t.timestamp(), t.timestamp_subsec_nanos()));
        match entry.name() {
            "layout.json" if !entry.is_directory => key.layout = ts,
            "widgets" if entry.is_directory => key.widgets = ts,
            _ => {}
        }
    }
    key
}

/// Build customized HTML from the workspace gateway config.
///
/// Returns `None` if the workspace is unavailable or the loaded layout has no
/// customizations and no widgets — in that case the caller serves the embedded
/// default HTML unchanged. Custom CSS is deliberately **not** included in the
/// returned bundle: `css_handler` appends `.system/gateway/custom.css` onto
/// `/style.css` so the stylesheet is the single source of truth for CSS
/// overrides.
///
/// The assembled HTML is cached in `GatewayState::frontend_html_cache` behind
/// a fingerprint of `.system/gateway/layout.json` and `.system/gateway/widgets/`
/// mtimes (computed with a single `list()` call). A cache hit skips reading
/// every widget manifest / JS / CSS file, which would otherwise fire on every
/// page load.
///
/// **Multi-tenant safety.** In multi-user mode (`workspace_pool` set) this
/// function ALWAYS returns `None`, regardless of whether `state.workspace` is
/// also populated. The customization assembly path is fundamentally
/// single-tenant: `index_handler` (`GET /`) is the unauthenticated bootstrap
/// route — no user identity is available at request time, so there is no way
/// to resolve the *correct* per-user workspace inside this function. Reading
/// `state.workspace` instead would expose one global workspace's
/// customizations to every user, and the process-wide
/// `frontend_html_cache` would pin the leak across requests. We refuse the
/// path entirely and serve the embedded default to all users; per-user
/// customization can ride a future JS-side fetch against
/// `/api/frontend/layout`, which is authenticated and routes through
/// `resolve_workspace(&state, &user)` so it returns the right workspace.
/// See `crates/ironclaw_gateway/static/js/core/widgets.js` — the
/// layout-config IIFE already reads `window.__IRONCLAW_LAYOUT__`, which
/// a future change can populate from a `fetch('/api/frontend/layout')`
/// after auth.
///
/// **Cache key TOCTOU window (known and accepted).** The fast-path cache
/// key is computed by [`compute_frontend_cache_key`] in a single
/// `Workspace::list` call, but the slow-path data read
/// (`read_layout_config` + `load_resolved_widgets`) happens *after* that
/// key is observed, in separate workspace operations. A workspace write
/// landing between the two — operator edits `layout.json` while a
/// request is mid-rebuild — can therefore produce a cache entry whose
/// HTML was assembled from a layout *newer* than the key it's stored
/// under. The next request after the writes settle will recompute the
/// key, see a different fingerprint, and replace the cache entry, so
/// the staleness window is always self-correcting and bounded by one
/// rebuild round-trip.
///
/// This is intentional. Making the read+key+store sequence atomic would
/// require a workspace-level read lock that the rest of the gateway
/// doesn't take, and would punish the (much hotter) cache hit path with
/// extra coordination. The acceptability rests on three observations:
/// (a) the staleness window is bounded by a single `list()` call's
/// worth of wall time, (b) the cache is per-process so the staleness
/// can never outlive `Drop` of `GatewayState`, and (c) layout writes
/// are rare and operator-initiated — there is no realistic workload
/// that fires a write at the cadence required to keep the entry
/// permanently stale. If a future workload changes that calculus, the
/// right fix is a workspace version generation counter, not a lock
/// around this function.
pub(crate) async fn build_frontend_html(state: &GatewayState) -> Option<String> {
    if state.workspace_pool.is_some() {
        // Multi-tenant: refuse the assembly path entirely. See the function
        // doc comment above for the full rationale. The cache write below
        // is unreachable on this branch, so the cache stays empty and
        // cannot leak one user's customizations to another.
        return None;
    }

    let ws = state.workspace.as_ref()?;

    // Fast path — cache hit. One workspace `list()` call, no file reads.
    let cache_key = compute_frontend_cache_key(ws).await;
    {
        let cache = state.frontend_html_cache.read().await;
        if let Some(ref cached) = *cache
            && cached.key == cache_key
        {
            return cached.html.clone();
        }
    }

    // Slow path — rebuild.
    let layout = read_layout_config(ws).await;
    let widgets = load_resolved_widgets(ws, &layout).await;

    // Skip assembly when nothing is customized. `layout_has_customizations`
    // is the single source of truth so adding a new field to `LayoutConfig`
    // forces an update in one place instead of a big boolean expression here.
    let html = if widgets.is_empty() && !layout_has_customizations(&layout) {
        None
    } else {
        let bundle = FrontendBundle {
            layout,
            widgets,
            // Custom CSS is served via /style.css (css_handler) to avoid
            // double-application — see the doc comment on this function.
            custom_css: None,
        };
        Some(ironclaw_gateway::assemble_index(
            assets::INDEX_HTML,
            &bundle,
        ))
    };

    // Store in cache. If another request raced us here, either writer wins —
    // both produced the same HTML for the same key, so the cache ends up
    // consistent either way.
    *state.frontend_html_cache.write().await = Some(FrontendHtmlCache {
        key: cache_key,
        html: html.clone(),
    });

    html
}

/// Returns `true` if the layout config has any field that would affect the
/// rendered HTML. When this returns `false` and there are no widgets, the
/// gateway serves the embedded default unchanged.
fn layout_has_customizations(layout: &LayoutConfig) -> bool {
    let b = &layout.branding;
    let t = &layout.tabs;
    let c = &layout.chat;
    // `branding.colors` is opaque to this function — `BrandingColors` may
    // exist as `Some({})` (both fields `None`) or with values that the
    // `is_safe_css_color` validator strips at injection time. Treating
    // bare `colors.is_some()` as a customization forces the customized
    // HTML path (and the per-response nonce CSP that comes with it) for
    // layouts that produce zero effective branding output. Require at
    // least one trimmed-non-empty color field, mirroring what
    // `to_css_vars` actually emits.
    let has_branding_colors = b.colors.as_ref().is_some_and(|colors| {
        let nonempty = |v: &Option<String>| v.as_deref().is_some_and(|s| !s.trim().is_empty());
        nonempty(&colors.primary) || nonempty(&colors.accent)
    });
    // Same precedent for URL fields: route through the `safe_logo_url`
    // / `safe_favicon_url` getters that apply `is_safe_url`. A
    // `layout.json` with `logo_url: "javascript:alert(1)"` would
    // otherwise force the customized HTML path even though the value
    // gets dropped at consumer time. Symmetric with how branding colors
    // are gated above.
    b.title.is_some()
        || b.subtitle.is_some()
        || b.safe_logo_url().is_some()
        || b.safe_favicon_url().is_some()
        || has_branding_colors
        || t.order.is_some()
        || t.hidden.is_some()
        || t.default_tab.is_some()
        || c.suggestions.is_some()
        || c.image_upload.is_some()
        || c.upgrade_inline_json.is_some()
        || !layout.widgets.is_empty()
}

// --- Static file handlers ---
//
// All frontend assets are embedded in the `ironclaw_gateway` crate.
// These handlers serve them with appropriate MIME types and cache headers.

/// Substitute [`NONCE_PLACEHOLDER`] sentinels in the assembled HTML with a
/// fresh per-response CSP nonce.
///
/// **Why an attribute-targeted replace, not a bare string replace.** The
/// assembled HTML embeds widget JavaScript inline (so a CSP-protected
/// `<script src>` doesn't need to authenticate against `/api/frontend/widget/...`).
/// A widget author has every right to write the literal string
/// `__IRONCLAW_CSP_NONCE__` inside their own source — in a comment, a log
/// line, a test fixture, or just as a constant they happen to define. A
/// naive `html.replace(NONCE_PLACEHOLDER, nonce)` would silently rewrite
/// every such occurrence into a per-request nonce, mutating widget code
/// in a way the author didn't ask for.
///
/// The substitution here targets the full attribute form
/// `nonce="__IRONCLAW_CSP_NONCE__"`, which is the exact shape
/// `assemble_index` emits when stamping nonces onto `<script>` tags. The
/// double-quoted sentinel is unambiguous in HTML context — it can never
/// accidentally match free text in a JS module body, a comment, or a
/// JSON payload. Inline `<style>` blocks deliberately get no nonce
/// (style-src allows `'unsafe-inline'`) so they're untouched either way.
pub(crate) fn stamp_nonce_into_html(html_with_placeholder: &str, nonce: &str) -> String {
    let placeholder_attr = format!("nonce=\"{NONCE_PLACEHOLDER}\"");
    let nonce_attr = format!("nonce=\"{nonce}\"");
    html_with_placeholder.replace(&placeholder_attr, &nonce_attr)
}

pub(crate) async fn index_handler(State(state): State<Arc<GatewayState>>) -> Response {
    // Try to assemble customized HTML from workspace frontend config.
    // Falls back to embedded HTML if workspace is unavailable or has no
    // customizations — in that case there are no inline scripts and the
    // global CSP layer applies unchanged.
    let assembled = build_frontend_html(&state).await;

    let Some(html_with_placeholder) = assembled else {
        return (
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::CACHE_CONTROL, "no-cache"),
            ],
            assets::INDEX_HTML,
        )
            .into_response();
    };

    // Customized path: the assembled HTML contains inline `<script>` blocks
    // (layout config + widget modules) carrying [`NONCE_PLACEHOLDER`] in
    // their `nonce` attribute. Stamp a fresh per-response nonce in both
    // the HTML and the response's Content-Security-Policy header so the
    // browser actually executes the scripts.
    //
    // Setting `Content-Security-Policy` here suppresses the global
    // `SetResponseHeaderLayer::if_not_present` value for this response only.
    let nonce = generate_csp_nonce();
    let html = stamp_nonce_into_html(&html_with_placeholder, &nonce);
    let csp = build_csp_with_nonce(&nonce);

    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8".to_string()),
            (header::CACHE_CONTROL, "no-cache".to_string()),
            (
                header::HeaderName::from_static("content-security-policy"),
                csp,
            ),
        ],
        html,
    )
        .into_response()
}

/// Compute the strong ETag value for a CSS body.
///
/// Strong validators are quoted, sha-prefixed, and truncated to 16 hex chars
/// (64 bits) — collisions are statistically irrelevant for cache validation
/// and the short form keeps headers compact. The same scheme is used for
/// both the embedded base stylesheet and the workspace-customized variant
/// so a flip between the two flavors naturally invalidates the client's
/// cached copy.
pub(crate) fn css_etag(body: &str) -> String {
    let digest = Sha256::digest(body.as_bytes());
    let hex = hex::encode(digest);
    // 16 hex chars = 64 bits, plenty for content addressing.
    format!("\"sha256-{}\"", &hex[..16]) // safety: hex::encode is pure ASCII, char-boundary safe
}

pub(crate) async fn css_handler(
    State(state): State<Arc<GatewayState>>,
    headers: HeaderMap,
) -> Response {
    // Append custom CSS from `.system/gateway/custom.css` if it exists.
    //
    // The hot path (no workspace overlay) borrows `assets::STYLE_CSS` directly
    // via `Cow::Borrowed` so we don't allocate / copy the entire embedded
    // stylesheet on every request. We only fall through to an owned
    // `format!` when there's actually content to append.
    //
    // **Multi-tenant safety.** This must mirror the same guard
    // `build_frontend_html` already enforces (see its doc comment): in
    // multi-user mode (`workspace_pool.is_some()`) we cannot resolve a
    // per-user workspace because `/style.css` is the unauthenticated
    // bootstrap stylesheet — there is no user identity at request time.
    // Reading from `state.workspace` here would expose one global
    // workspace's `custom.css` to every user, defeating the
    // `index_handler` guard at the sibling endpoint. Refuse the overlay
    // path entirely in multi-tenant mode and serve the embedded base
    // stylesheet to all users; per-user CSS overrides can ride a future
    // authenticated `/api/frontend/custom-css` endpoint.
    let css: std::borrow::Cow<'static, str> = if state.workspace_pool.is_some() {
        std::borrow::Cow::Borrowed(assets::STYLE_CSS)
    } else {
        match &state.workspace {
            Some(ws) => match ws.read(".system/gateway/custom.css").await {
                Ok(doc) if !doc.content.trim().is_empty() => std::borrow::Cow::Owned(format!(
                    "{}\n/* --- custom overrides --- */\n{}",
                    assets::STYLE_CSS,
                    doc.content
                )),
                _ => std::borrow::Cow::Borrowed(assets::STYLE_CSS),
            },
            None => std::borrow::Cow::Borrowed(assets::STYLE_CSS),
        }
    };

    // Strong validator over the assembled body. The cache key naturally
    // tracks both base stylesheet edits (compile-time) and `custom.css`
    // edits (workspace mutation) — operators no longer need to ask users
    // to hard-refresh after tweaking branding.
    let etag = css_etag(&css);

    // Conditional GET: if the client already holds this exact body, send a
    // 304 with no body and let the browser reuse its cached copy. RFC 9110
    // §13.1.2 — `If-None-Match` is a list of validators; we accept either
    // an exact match or the literal `*`. Anything else falls through to a
    // full 200 response.
    if let Some(value) = headers.get(header::IF_NONE_MATCH)
        && let Ok(s) = value.to_str()
        && s.split(',').any(|v| {
            let v = v.trim();
            v == "*" || v == etag
        })
    {
        return (
            StatusCode::NOT_MODIFIED,
            [
                (header::ETAG, etag.as_str()),
                (header::CACHE_CONTROL, "no-cache"),
            ],
        )
            .into_response();
    }

    (
        [
            (header::CONTENT_TYPE, "text/css".to_string()),
            // Keep `no-cache` so the browser always revalidates — combined
            // with the ETag this gives us "fast 304" semantics rather than
            // a stale `max-age` window where operator edits don't show up.
            (header::CACHE_CONTROL, "no-cache".to_string()),
            (header::ETAG, etag),
        ],
        css,
    )
        .into_response()
}

pub(crate) async fn theme_css_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        assets::THEME_CSS,
    )
}

pub(crate) async fn js_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        assets::APP_JS,
    )
}

pub(crate) async fn theme_init_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        assets::THEME_INIT_JS,
    )
}

pub(crate) async fn favicon_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/x-icon"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        assets::FAVICON_ICO,
    )
}

pub(crate) async fn i18n_index_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        assets::I18N_INDEX_JS,
    )
}

pub(crate) async fn i18n_en_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        assets::I18N_EN_JS,
    )
}

pub(crate) async fn i18n_zh_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        assets::I18N_ZH_CN_JS,
    )
}

pub(crate) async fn i18n_ko_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        assets::I18N_KO_JS,
    )
}

pub(crate) async fn i18n_app_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        assets::I18N_APP_JS,
    )
}

// --- Admin panel static handlers ---

pub(crate) async fn admin_html_handler() -> impl IntoResponse {
    // Admin panel CSP — fully same-origin, no CDN allowances.
    // Delivered as an HTTP header (not a <meta> tag) so the browser enforces
    // it before any markup is parsed.
    const ADMIN_CSP: &str = "default-src 'self'; \
        script-src 'self'; \
        style-src 'self' 'unsafe-inline'; \
        font-src 'self'; \
        connect-src 'self'; \
        img-src 'self' data:; \
        object-src 'none'; \
        frame-ancestors 'none'; \
        base-uri 'self'; \
        form-action 'self'";

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-cache"),
    );
    headers.insert(
        header::HeaderName::from_static("content-security-policy"),
        header::HeaderValue::from_static(ADMIN_CSP),
    );
    (headers, assets::ADMIN_HTML)
}

pub(crate) async fn admin_css_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        assets::ADMIN_CSS,
    )
}

pub(crate) async fn admin_js_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        assets::ADMIN_JS,
    )
}

// --- Health ---

pub(crate) async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "healthy",
        channel: "gateway",
    })
}

// --- Project files (authenticated) ---

/// Redirect `/projects/{id}` to `/projects/{id}/` so relative paths in
/// the served HTML resolve within the project namespace.
pub(crate) async fn project_redirect_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(project_id): Path<String>,
) -> impl IntoResponse {
    if !verify_project_ownership(&state, &project_id, &user.user_id).await {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    }
    axum::response::Redirect::permanent(&format!("/projects/{project_id}/")).into_response()
}

/// Serve `index.html` when hitting `/projects/{project_id}/`.
pub(crate) async fn project_index_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(project_id): Path<String>,
) -> impl IntoResponse {
    if !verify_project_ownership(&state, &project_id, &user.user_id).await {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    }
    serve_project_file(&project_id, "index.html").await
}

/// Serve any file under `/projects/{project_id}/{path}`.
pub(crate) async fn project_file_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path((project_id, path)): Path<(String, String)>,
) -> impl IntoResponse {
    if !verify_project_ownership(&state, &project_id, &user.user_id).await {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    }
    serve_project_file(&project_id, &path).await
}

/// Check that a project directory belongs to a job owned by the given user.
/// Returns false if the store is unavailable or the project is not found.
async fn verify_project_ownership(state: &GatewayState, project_id: &str, user_id: &str) -> bool {
    let Some(ref store) = state.store else {
        return false;
    };
    // The project_id is a sandbox job UUID used as the directory name.
    let Ok(job_id) = project_id.parse::<uuid::Uuid>() else {
        return false;
    };
    match store.get_sandbox_job(job_id).await {
        Ok(Some(job)) => job.user_id == user_id,
        _ => false,
    }
}

/// Shared logic: resolve the file inside `~/.ironclaw/projects/{project_id}/`,
/// guard against path traversal, and stream the content with the right MIME type.
async fn serve_project_file(project_id: &str, path: &str) -> axum::response::Response {
    // Reject project_id values that could escape the projects directory.
    if project_id.contains('/')
        || project_id.contains('\\')
        || project_id.contains("..")
        || project_id.is_empty()
    {
        return (StatusCode::BAD_REQUEST, "Invalid project ID").into_response();
    }

    let base = ironclaw_base_dir().join("projects").join(project_id);

    let file_path = base.join(path);

    // Path traversal guard
    let canonical = match file_path.canonicalize() {
        Ok(p) => p,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found").into_response(),
    };
    let base_canonical = match base.canonicalize() {
        Ok(p) => p,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found").into_response(),
    };
    if !canonical.starts_with(&base_canonical) {
        return (StatusCode::FORBIDDEN, "Forbidden").into_response();
    }

    match tokio::fs::read(&canonical).await {
        Ok(contents) => {
            let mime = mime_guess::from_path(&canonical)
                .first_or_octet_stream()
                .to_string();
            ([(header::CONTENT_TYPE, mime)], contents).into_response()
        }
        Err(_) => (StatusCode::NOT_FOUND, "Not found").into_response(),
    }
}

// Tests for these helpers live alongside the route-level handler tests in
// `src/channels/web/server.rs` (for now), where the full `GatewayState`
// fixture is already in scope. They will migrate here once `server.rs` is
// further trimmed in the next ironclaw#2599 increment.
