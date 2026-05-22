//! HTTP gateway composition for the Reborn WebChat v2 native surface.
//!
//! The `ironclaw_webui_v2` crate ships handlers that dispatch through
//! `RebornServicesApi` but is deliberately unaware of bearer tokens,
//! OIDC, CORS, body limits, and static security headers — its CLAUDE.md
//! lists these as "host composition still owes". This module is the
//! Reborn-side home for that work: it exposes [`webui_v2_app`], the
//! fully-composed axum [`Router`] (auth + rate limit + CORS + body
//! limit + security headers + v2 route surface). Tests drive it
//! through `tower::ServiceExt::oneshot`; the standalone
//! `ironclaw-reborn serve` subcommand (on a follow-up PR) consumes the
//! same `Router` and owns the listener lifecycle on the host side.
//!
//! ### Why no serve-and-bind helper here
//!
//! `ironclaw_reborn_composition` sits in the Reborn product/API
//! boundary enforced by
//! `crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs::
//! reborn_product_api_crates_do_not_bind_http_ingress`. Product/API
//! crates may expose `Router` / `IngressRouteDescriptor`, but they may
//! NOT bind `TcpListener`s, drive the axum `serve` future, or
//! otherwise own server lifecycle — that responsibility lives in
//! host-owned code. So the seam this PR provides is the `Router`; the
//! consuming host binary writes the listener-binding line itself.
//!
//! Everything in this module is gated on the `webui-v2-beta` Cargo
//! feature. Substrate-only callers (v1 `AppBuilder`, diagnostic
//! harnesses) stay off the feature and carry no HTTP surface code.
//!
//! The composition is intentionally Reborn-owned and does **not** share
//! middleware with the v1 gateway under `/src/channels/web/`. Path A in
//! `docs/reborn/how-to-port-channel-to-reborn.md` requires native
//! surfaces to keep host auth host-owned and route/body/CORS security
//! in gateway-owned code; the Reborn binary owns this stack itself.

use std::sync::Arc;

use axum::{
    Router,
    extract::{Request, State},
    http::{HeaderName, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_webui_v2::{WebUiV2State, webui_v2_router};
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::cors::{AllowHeaders, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::set_header::SetResponseHeaderLayer;

use crate::webui::RebornWebuiBundle;
use crate::webui_body_limit::{build_body_limit_state, enforce_body_limit};
use crate::webui_rate_limit::{build_rate_limit_state, enforce_rate_limit};
use crate::webui_ws_origin::{build_websocket_origin_state, enforce_websocket_origin};
use ironclaw_product_workflow::WebUiAuthenticatedCaller;

/// Default per-request body limit (14 MiB) — sized to cover ~10 MiB of
/// decoded attachments plus base64/JSON overhead. Mirrors the existing
/// gateway-owned limit used by host-owned surfaces today.
pub const DEFAULT_WEBUI_MAX_BODY_BYTES: usize = 14 * 1024 * 1024;

/// Default Content-Security-Policy applied to WebChat v2 responses.
/// `default-src 'self'`, `object-src 'none'`, `frame-ancestors 'none'`
/// — locked down because the v2 surface is API-only and never serves
/// untrusted HTML. The CLI can override per-deployment if it ever
/// fronts an HTML SPA on the same listener.
pub const DEFAULT_WEBUI_CSP: &str =
    "default-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'self'";

/// Authentication contract the Reborn binary supplies. The composition
/// layer is intentionally agnostic about WHERE bearer tokens come from
/// — env vars, the host's `SecretStore`, OIDC JWTs verified by the
/// caller — so the same `webui_v2_app` works for the CLI binary and
/// for any future ingress fronting the same routes.
///
/// Implementations return `Some(UserId)` on success and `None` to
/// reject. Concrete failure reasons stay inside the implementation
/// (the gateway emits a generic 401), per the
/// `docs/reborn/how-to-port-channel-to-reborn.md` Path A guidance that
/// auth evidence is host-owned and never leaks to clients.
#[async_trait::async_trait]
pub trait WebuiAuthenticator: Send + Sync + 'static {
    async fn authenticate(&self, token: &str) -> Option<UserId>;
}

/// Host-installation composition the Reborn HTTP gateway needs in
/// addition to the [`RebornWebuiBundle`] it serves over.
///
/// Fields are `pub(crate)` so the public surface is the typed builder
/// methods only. This routes every host through `new` /
/// `parse_allowed_origins` / `with_*`, which fail-closed on invalid
/// input (empty token, malformed origin, bad CSP). The fail-closed
/// defaults — empty allow-origin list, locked-down CSP, 14 MiB outer
/// body cap — apply unless an explicit builder override changes them.
///
/// Read access is intentionally not re-exposed: host binaries should
/// keep their own config sources of truth (`[webui]` TOML, env vars)
/// and feed builders, not round-trip through this struct.
#[derive(Clone)]
pub struct WebuiServeConfig {
    /// Host installation tenant id. Stamped onto every
    /// [`WebUiAuthenticatedCaller`]; the browser body cannot influence
    /// it. Matches the trusted host config rule documented in
    /// `crates/ironclaw_product_workflow/CLAUDE.md`.
    pub(crate) tenant_id: TenantId,
    /// Bearer-token verifier supplied by host composition.
    pub(crate) authenticator: Arc<dyn WebuiAuthenticator>,
    /// Outer per-request body cap applied as defense in depth for
    /// paths that don't match any v2 descriptor (e.g. axum's 404
    /// fallback). v2 routes are additionally enforced against the
    /// per-route [`BodyLimitPolicy`](ironclaw_host_api::ingress::BodyLimitPolicy)
    /// declared in `ironclaw_webui_v2::webui_v2_routes()`; that
    /// descriptor cap is always strictly tighter than this global
    /// fallback. Defaults to [`DEFAULT_WEBUI_MAX_BODY_BYTES`].
    pub(crate) max_body_bytes: usize,
    /// CORS allow-origin list. Empty means "no cross-origin requests
    /// accepted at all" — explicitly fail-closed; pre-flight checks
    /// against an empty list never echo the attacker-supplied origin.
    pub(crate) allowed_origins: Vec<HeaderValue>,
    /// Content-Security-Policy header value. Defaults to
    /// [`DEFAULT_WEBUI_CSP`] if `None`.
    pub(crate) csp_header: Option<HeaderValue>,
    /// Canonical host the WebChat v2 listener is reachable on (e.g.
    /// `"app.example.com"` or `"127.0.0.1:3000"`). When set, the
    /// WebSocket same-origin middleware compares the request's
    /// `Origin` header against this value instead of trusting the
    /// client-supplied `Host` header. A misconfigured reverse proxy
    /// that forwards an attacker-controlled Host would otherwise let
    /// the same-origin check pass for a forged Origin. Defaults to
    /// `None` (fall back to Host-header comparison + allowlist).
    pub(crate) canonical_host: Option<String>,
    /// Trusted default agent id stamped onto every
    /// [`WebUiAuthenticatedCaller`]. The browser body cannot influence
    /// this — it comes from host installation config / runtime
    /// identity. Required because the downstream `RebornServicesApi`
    /// facade builds `ThreadScope` from `caller.agent_id` for every
    /// v2 mutation and read, and a `None` agent_id collapses to a
    /// `400 InvalidRequest` before the handler reaches the workflow.
    pub(crate) default_agent_id: Option<AgentId>,
    /// Trusted default project id stamped onto every
    /// [`WebUiAuthenticatedCaller`]. Optional at the type level
    /// because the v2 facade allows projectless scopes for some
    /// flows; supply it when the host installation has a single
    /// canonical project.
    pub(crate) default_project_id: Option<ProjectId>,
}

impl WebuiServeConfig {
    /// Build a config with the body limit / CSP defaults applied and
    /// the supplied tenant, authenticator, and origin list.
    pub fn new(
        tenant_id: TenantId,
        authenticator: Arc<dyn WebuiAuthenticator>,
        allowed_origins: Vec<HeaderValue>,
    ) -> Self {
        Self {
            tenant_id,
            authenticator,
            max_body_bytes: DEFAULT_WEBUI_MAX_BODY_BYTES,
            allowed_origins,
            csp_header: None,
            canonical_host: None,
            default_agent_id: None,
            default_project_id: None,
        }
    }

    /// Set the canonical host for WebSocket same-origin checks. See
    /// [`Self::canonical_host`] for why this is more robust than
    /// trusting the request's `Host` header.
    pub fn with_canonical_host(mut self, host: impl Into<String>) -> Self {
        self.canonical_host = Some(host.into());
        self
    }

    /// Set the trusted host-installation default `AgentId`. Stamped
    /// onto every authenticated caller; required for the v2 facade to
    /// build `ThreadScope` on mutations and reads.
    pub fn with_default_agent_id(mut self, agent_id: AgentId) -> Self {
        self.default_agent_id = Some(agent_id);
        self
    }

    /// Set the trusted host-installation default `ProjectId`. Optional
    /// — supply when the host installation has a canonical project.
    pub fn with_default_project_id(mut self, project_id: ProjectId) -> Self {
        self.default_project_id = Some(project_id);
        self
    }

    /// Parse a list of allow-origin strings (typically read from
    /// operator config TOML) into the typed `HeaderValue` vector.
    /// Lets host binaries construct [`WebuiServeConfig`] without
    /// pulling axum / http as a direct workspace dependency.
    pub fn parse_allowed_origins(
        origins: &[String],
    ) -> Result<Vec<HeaderValue>, WebuiServeConfigError> {
        origins
            .iter()
            .map(|raw| {
                HeaderValue::from_str(raw).map_err(|err| {
                    WebuiServeConfigError::InvalidAllowedOrigin {
                        origin: raw.clone(),
                        reason: err.to_string(),
                    }
                })
            })
            .collect()
    }

    /// Override [`Self::max_body_bytes`] in a builder-style.
    pub fn with_max_body_bytes(mut self, bytes: usize) -> Self {
        self.max_body_bytes = bytes;
        self
    }

    /// Override [`Self::csp_header`] in a builder-style. The supplied
    /// string is parsed into a `HeaderValue`; invalid values surface
    /// as [`WebuiServeConfigError::InvalidCspHeader`].
    pub fn with_csp_header_str(mut self, csp: &str) -> Result<Self, WebuiServeConfigError> {
        let value =
            HeaderValue::from_str(csp).map_err(|err| WebuiServeConfigError::InvalidCspHeader {
                reason: err.to_string(),
            })?;
        self.csp_header = Some(value);
        Ok(self)
    }
}

/// Errors surfaced by [`WebuiServeConfig`]'s string-based helpers.
#[derive(Debug, thiserror::Error)]
pub enum WebuiServeConfigError {
    #[error("CORS allow-origin entry `{origin}` is not a valid HTTP header value: {reason}")]
    InvalidAllowedOrigin { origin: String, reason: String },
    #[error("CSP header is not a valid HTTP header value: {reason}")]
    InvalidCspHeader { reason: String },
}

/// Errors raised while composing the WebChat v2 gateway `Router`.
///
/// No I/O variant: this crate sits in the Reborn product/API boundary
/// and never binds a listener or drives the axum serve loop. Host
/// composition owns the I/O lifecycle and surfaces its own errors
/// there.
#[derive(Debug, thiserror::Error)]
pub enum WebuiServeError {
    #[error("invalid CSP header value: {0}")]
    InvalidCspHeader(String),
    #[error("rate-limit composition failed: {0}")]
    RateLimit(#[from] crate::webui_rate_limit::RateLimitConfigError),
}

/// Build the fully-composed Reborn WebChat v2 axum app:
///
/// - panic catch (outer)
/// - static security headers (`X-Content-Type-Options`, `X-Frame-Options`, CSP)
/// - CORS allow-origin list
/// - outer global request body limit (defense in depth for unmatched paths)
/// - per-route body limit, resolved from the
///   `ironclaw_webui_v2::webui_v2_routes()` descriptors (16 KiB for
///   create_thread, 1 MiB for send_message, 4 KiB for cancel_run /
///   resolve_gate, NoBody for timeline / SSE)
/// - bearer auth (+ `?token=` on the v2 SSE path) → injects
///   [`WebUiAuthenticatedCaller`]
/// - per-route rate limit, resolved from the
///   `ironclaw_webui_v2::webui_v2_routes()` descriptors (mutation
///   60/60, read 120/60, stream 12/60 per `(tenant, user)` today)
/// - WebChat v2 route set from `ironclaw_webui_v2::webui_v2_router`
///
/// The returned [`Router`] is the seam between this composition crate
/// and host-owned ingress code: tests drive it via
/// `tower::ServiceExt::oneshot`, and the standalone `ironclaw-reborn
/// serve` subcommand on a follow-up PR will hand it to axum's serve
/// loop from a host-owned listener. This crate intentionally never
/// binds a socket or drives the serve loop itself — that boundary is
/// enforced by `reborn_product_api_crates_do_not_bind_http_ingress`
/// in `ironclaw_architecture`.
pub fn webui_v2_app(
    bundle: RebornWebuiBundle,
    config: WebuiServeConfig,
) -> Result<Router, WebuiServeError> {
    let csp_value = config.csp_header.clone().map(Ok).unwrap_or_else(|| {
        HeaderValue::from_str(DEFAULT_WEBUI_CSP)
            .map_err(|err| WebuiServeError::InvalidCspHeader(err.to_string()))
    })?;

    let cors = CorsLayer::new()
        .allow_origin(config.allowed_origins.clone())
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers(AllowHeaders::list([
            header::CONTENT_TYPE,
            header::AUTHORIZATION,
        ]))
        .allow_credentials(true);

    let auth_state = AuthLayerState {
        tenant_id: config.tenant_id.clone(),
        default_agent_id: config.default_agent_id.clone(),
        default_project_id: config.default_project_id.clone(),
        authenticator: config.authenticator.clone(),
    };

    let descriptors = ironclaw_webui_v2::webui_v2_routes();
    let rate_limit_state = build_rate_limit_state(&descriptors)?;
    let body_limit_state = build_body_limit_state(&descriptors);
    let ws_origin_state = build_websocket_origin_state(
        &descriptors,
        &config.allowed_origins,
        config.canonical_host.clone(),
    );

    // Inner: the v2 route surface, retagged to `Router<()>` so it can
    // merge into the outer stateless router. `webui_v2_router` has
    // already baked its own `WebUiV2State` into every handler.
    let v2_inner: Router<()> =
        webui_v2_router(WebUiV2State::new(bundle.api.clone())).with_state(());

    // Layer order matters. `route_layer` stacks inside-out from the
    // bottom of the chain up — the LAST `.route_layer(...)` call is
    // the outermost layer and runs FIRST on inbound. That gives:
    //   ws-origin → per-route body limit → auth → rate-limit → handler
    //
    // WS-origin runs first so a forged-Origin WebSocket upgrade dies
    // before the gateway spends an auth check on it. Body limit comes
    // next so an oversized payload also short-circuits before bearer
    // validation. Auth runs before rate-limit so the limiter has a
    // real caller key and an unauthenticated request never burns a
    // rate-limit slot.
    let app = Router::new()
        .merge(v2_inner)
        .route_layer(middleware::from_fn_with_state(
            rate_limit_state,
            enforce_rate_limit,
        ))
        .route_layer(middleware::from_fn_with_state(
            auth_state,
            authenticate_request,
        ))
        .route_layer(middleware::from_fn_with_state(
            body_limit_state,
            enforce_body_limit,
        ))
        // WS upgrades skip CORS pre-flight, so origin enforcement runs
        // inline for descriptors declaring a non-NotApplicable
        // WebSocketOriginPolicy. Runs near the outside of the
        // route_layer stack so origin rejection short-circuits before
        // anything more expensive.
        .route_layer(middleware::from_fn_with_state(
            ws_origin_state,
            enforce_websocket_origin,
        ))
        // Outer global cap: applies to unmatched paths (e.g. 404 fallback)
        // as defense in depth. v2 routes are tighter via the per-route
        // body-limit middleware above.
        .layer(RequestBodyLimitLayer::new(config.max_body_bytes))
        .layer(CatchPanicLayer::custom(panic_handler))
        .layer(cors)
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            HeaderName::from_static("content-security-policy"),
            csp_value,
        ))
        // Defense in depth for the SSE `?token=` shim: browsers honor
        // Referrer-Policy when deciding whether to attach the
        // referring URL to subsequent navigation requests, third-party
        // resource loads, or downstream-link clicks. `no-referrer`
        // stops the gateway URL (which may contain `?token=…`) from
        // bleeding into any cross-origin destination's logs. Does not
        // protect against server-side access-log capture — operators
        // still need to scrub URL query strings before retention.
        .layer(SetResponseHeaderLayer::if_not_present(
            HeaderName::from_static("referrer-policy"),
            HeaderValue::from_static("no-referrer"),
        ));

    Ok(app)
}

// ─── auth middleware ──────────────────────────────────────────────────

#[derive(Clone)]
struct AuthLayerState {
    tenant_id: TenantId,
    default_agent_id: Option<AgentId>,
    default_project_id: Option<ProjectId>,
    authenticator: Arc<dyn WebuiAuthenticator>,
}

/// Resolve `Authorization: Bearer <token>` for any v2 route, OR the
/// `?token=…` query parameter only on the v2 SSE stream endpoint
/// (mirrors the browser's `EventSource` limitation — it cannot set
/// custom headers). On success, insert a [`WebUiAuthenticatedCaller`]
/// extension built from the host-installation tenant + the
/// authenticated user. On failure, return 401 before the v2 handler
/// runs.
async fn authenticate_request(
    State(state): State<AuthLayerState>,
    mut request: Request,
    next: Next,
) -> Response {
    let token = match extract_bearer_token(&request) {
        Some(token) => token,
        None => return unauthorized(),
    };

    let user_id = match state.authenticator.authenticate(&token).await {
        Some(uid) => uid,
        None => return unauthorized(),
    };

    // Stamp the trusted agent/project from host installation config
    // onto every authenticated caller. The downstream facade builds
    // `ThreadScope` from `caller.agent_id` and 400s if it's missing,
    // so a binary that fails to thread agent_id through here would
    // authenticate users only to reject every v2 mutation/read. The
    // browser body cannot influence either of these identifiers — by
    // contract `WebuiServeConfig` is host-owned.
    let caller = WebUiAuthenticatedCaller::new(
        state.tenant_id.clone(),
        user_id,
        state.default_agent_id.clone(),
        state.default_project_id.clone(),
    );
    request.extensions_mut().insert(caller);
    next.run(request).await
}

fn unauthorized() -> Response {
    (StatusCode::UNAUTHORIZED, "Invalid or missing auth token").into_response()
}

fn extract_bearer_token(request: &Request) -> Option<String> {
    if let Some(value) = request.headers().get(header::AUTHORIZATION)
        && let Ok(text) = value.to_str()
        // `text.get(..7)` returns `None` when 7 is past the end OR
        // lands inside a multi-byte UTF-8 sequence; both cases mean
        // the value cannot be `Bearer <token>`. A direct byte slice
        // would panic on a value whose first 7 bytes split a multi-byte
        // character, which is forbidden for user-supplied data.
        && let Some(prefix) = text.get(..7)
        && prefix.eq_ignore_ascii_case("Bearer ")
    {
        // Safe: `prefix.eq_ignore_ascii_case("Bearer ")` matched, so
        // the first 7 bytes are pure ASCII and byte 7 is a char
        // boundary.
        return Some(text[7..].to_string());
    }
    // `?token=` shim — only honored on the v2 SSE stream endpoint
    // because `EventSource` cannot set request headers. Mutations and
    // timeline reads stay bearer-only so a query-token leak in a
    // referer chain cannot authenticate a state change.
    //
    // **Operational warning:** the token-as-URL-parameter pattern is
    // a documented industry trade-off (SSE has no header-supplying
    // client primitive). The token value appears in the URL and will
    // therefore land in any HTTP access log, intermediate proxy log,
    // or analytics pipeline that sees the request line. Composition
    // emits `Referrer-Policy: no-referrer` on every response as
    // defense in depth, but operators MUST still scrub
    // `?token=<value>` from any log destination that retains URLs.
    // The acceptance check is narrowed to GET on the exact
    // `…/threads/{id}/events` path by `is_v2_sse_event_request` so
    // the leak surface is one route, not the whole gateway.
    if is_v2_sse_event_request(request) {
        return query_token(request);
    }
    None
}

/// Returns `true` if the request is `GET /api/webchat/v2/threads/{id}/events`.
/// The thread id must be a single non-empty path segment.
pub(crate) fn is_v2_sse_event_request(request: &Request) -> bool {
    if request.method() != Method::GET {
        return false;
    }
    let path = request.uri().path();
    let Some(rest) = path.strip_prefix("/api/webchat/v2/threads/") else {
        return false;
    };
    let Some(thread_id) = rest.strip_suffix("/events") else {
        return false;
    };
    !thread_id.is_empty() && !thread_id.contains('/')
}

fn query_token(request: &Request) -> Option<String> {
    let query = request.uri().query()?;
    url_query_value(query, "token")
}

fn url_query_value(query: &str, key: &str) -> Option<String> {
    for pair in query.split('&') {
        let mut parts = pair.splitn(2, '=');
        let candidate_key = parts.next()?;
        if candidate_key != key {
            continue;
        }
        let raw_value = parts.next().unwrap_or("");
        // Decode minimally: `+` → space, `%XX` → byte. Tokens are
        // almost always opaque ASCII so we accept the value as-is and
        // only handle the percent-decoded form when present. Empty or
        // whitespace-only values count as absent so a stray `?token=`
        // does not override a missing bearer header.
        let decoded = percent_decode(raw_value);
        let trimmed = decoded.trim();
        if trimmed.is_empty() {
            return None;
        }
        return Some(trimmed.to_string());
    }
    None
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hi = hex_value(bytes[i + 1]);
                let lo = hex_value(bytes[i + 2]);
                if let (Some(hi), Some(lo)) = (hi, lo) {
                    out.push((hi << 4) | lo);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_value(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn panic_handler(
    panic_info: Box<dyn std::any::Any + Send + 'static>,
) -> axum::http::Response<axum::body::Body> {
    let detail = if let Some(s) = panic_info.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = panic_info.downcast_ref::<&str>() {
        (*s).to_string()
    } else {
        "unknown panic".to_string()
    };
    let safe_detail = if detail.len() > 200 {
        let end = detail.floor_char_boundary(200);
        format!("{}…", &detail[..end]) // safety: end was clamped to a UTF-8 character boundary.
    } else {
        detail
    };
    tracing::error!(
        target = "ironclaw::reborn::webui_serve",
        "Handler panicked: {safe_detail}"
    );
    axum::http::Response::builder()
        .status(StatusCode::INTERNAL_SERVER_ERROR)
        .header(axum::http::header::CONTENT_TYPE, "text/plain")
        .body(axum::body::Body::from("Internal Server Error"))
        .unwrap_or_else(|_| {
            axum::http::Response::new(axum::body::Body::from("Internal Server Error"))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Method;

    fn fake_request(method: Method, path_and_query: &str) -> Request {
        Request::builder()
            .method(method)
            .uri(path_and_query)
            .body(Body::empty())
            .expect("request")
    }

    #[test]
    fn v2_sse_event_request_recognized() {
        let req = fake_request(Method::GET, "/api/webchat/v2/threads/abc/events");
        assert!(is_v2_sse_event_request(&req));
    }

    #[test]
    fn v2_sse_event_request_requires_get() {
        let req = fake_request(Method::POST, "/api/webchat/v2/threads/abc/events");
        assert!(!is_v2_sse_event_request(&req));
    }

    #[test]
    fn v2_sse_event_request_requires_single_thread_segment() {
        assert!(!is_v2_sse_event_request(&fake_request(
            Method::GET,
            "/api/webchat/v2/threads//events"
        )));
        assert!(!is_v2_sse_event_request(&fake_request(
            Method::GET,
            "/api/webchat/v2/threads/abc/events/extra"
        )));
    }

    #[test]
    fn v2_sse_event_request_rejects_other_v2_routes() {
        assert!(!is_v2_sse_event_request(&fake_request(
            Method::GET,
            "/api/webchat/v2/threads/abc/timeline"
        )));
        assert!(!is_v2_sse_event_request(&fake_request(
            Method::POST,
            "/api/webchat/v2/threads"
        )));
    }

    #[test]
    fn query_token_extracts_token_param() {
        let req = fake_request(
            Method::GET,
            "/api/webchat/v2/threads/abc/events?token=abc123",
        );
        assert_eq!(query_token(&req).as_deref(), Some("abc123"));
    }

    #[test]
    fn query_token_decodes_percent_escapes() {
        let req = fake_request(
            Method::GET,
            "/api/webchat/v2/threads/abc/events?token=a%2Bb%3Dc",
        );
        assert_eq!(query_token(&req).as_deref(), Some("a+b=c"));
    }

    #[test]
    fn query_token_treats_empty_as_absent() {
        let req = fake_request(Method::GET, "/api/webchat/v2/threads/abc/events?token=");
        assert!(query_token(&req).is_none());
        let req2 = fake_request(
            Method::GET,
            "/api/webchat/v2/threads/abc/events?token=%20%20",
        );
        assert!(query_token(&req2).is_none());
    }

    #[test]
    fn bearer_header_extraction_is_case_insensitive_on_prefix() {
        let mut req = fake_request(Method::POST, "/api/webchat/v2/threads");
        req.headers_mut().insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("bEaReR mytoken"),
        );
        assert_eq!(extract_bearer_token(&req).as_deref(), Some("mytoken"));
    }

    #[test]
    fn extract_bearer_token_rejects_query_token_on_non_sse_paths() {
        // `?token=` is an EventSource-only escape hatch on the SSE
        // route. Mutations and reads MUST stay bearer-only — a future
        // regression that widens query-token acceptance to other
        // routes would silently downgrade auth on every state change
        // (no bearer header means an attacker only needs the URL).
        // This test pins extract_bearer_token's behavior on every
        // non-SSE shape we care about.
        for (method, path_and_query) in [
            (Method::POST, "/api/webchat/v2/threads?token=stealme"),
            (
                Method::POST,
                "/api/webchat/v2/threads/abc/messages?token=stealme",
            ),
            (
                Method::GET,
                "/api/webchat/v2/threads/abc/timeline?token=stealme",
            ),
            (
                Method::POST,
                "/api/webchat/v2/threads/abc/runs/r/cancel?token=stealme",
            ),
            (
                Method::POST,
                "/api/webchat/v2/threads/abc/runs/r/gates/g/resolve?token=stealme",
            ),
            // Even on the SSE path, the wrong METHOD must reject.
            (
                Method::POST,
                "/api/webchat/v2/threads/abc/events?token=stealme",
            ),
            // List threads shares the same path as create_thread but
            // is read-only; query-token still rejected because no
            // bearer header is present.
            (Method::GET, "/api/webchat/v2/threads?token=stealme"),
        ] {
            let req = fake_request(method.clone(), path_and_query);
            assert!(
                extract_bearer_token(&req).is_none(),
                "extract_bearer_token must NOT accept ?token= on {method} {path_and_query}",
            );
        }
    }

    #[test]
    fn extract_bearer_token_accepts_query_token_only_on_sse_get() {
        // Companion to the rejection test: the one place `?token=` is
        // honored — GET on the SSE events route — must still work.
        let req = fake_request(Method::GET, "/api/webchat/v2/threads/abc/events?token=ok");
        assert_eq!(extract_bearer_token(&req).as_deref(), Some("ok"));
    }
}
