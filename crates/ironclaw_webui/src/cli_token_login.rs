//! `GET /login?token=<cli-token>` — the CLI-printed bootstrap link
//! into the browser session, plugging into the SAME
//! bearer/ticket-exchange flow the OAuth callback uses (see
//! `signed_session_login.rs`'s module docs for the rule this
//! follows: `WebuiAuthenticator` / `SessionStore` wiring lives in
//! this crate, not the command crate).
//!
//! Shape, mirrored from `auth::routes::callback_handler`:
//! 1. `GET /login?token=...` verifies the presented token against
//!    the host's resolved bearer authenticator (constant-time via
//!    [`crate::EnvBearerAuthenticator`], reused rather than
//!    reimplemented), mints a session bearer through the supplied
//!    [`SessionStore`] (typically [`crate::signed_session_store`]
//!    built from the same operator secret + tenant as the CLI's
//!    admin bearer minter), and redirects to
//!    `<redirect_after>?login_ticket=<ticket>` — the exact query
//!    convention the OAuth callback already produces.
//! 2. `POST /auth/session/exchange` consumes that one-time ticket
//!    and returns the real bearer as `{ "token": "..." }`, so the
//!    SPA's existing `exchangeLoginTicket` (`api.ts:747-767`)
//!    completes the hand-off with zero new frontend code.
//!
//! This mount owns its own one-time ticket store rather than the
//! OAuth login surface's (`auth::routes`'s `session_tickets` is
//! private to that module, and CLI-token login must work even when
//! no OAuth provider is configured, in which case
//! `empty_webui_v2_auth_providers_mount` — the OAuth surface's
//! provider-less fallback — never mounts `/auth/session/exchange`
//! at all). The exchange handler here is byte-for-byte the same
//! contract (`{ticket}` in, `{token}` out) as
//! `auth::routes::session_exchange_handler`, so the SPA cannot tell
//! the difference. **Integration note for whoever wires both
//! surfaces into `serve.rs`:** both mounts register `POST
//! /auth/session/exchange`; attach at most one of them per
//! deployment (this mount when there is no SSO provider — the CLI
//! onboarding case B4 targets — the OAuth mount's own exchange route
//! otherwise) to avoid a duplicate-route panic at merge time.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use chrono::Duration as ChronoDuration;
use ironclaw_host_api::NetworkMethod;
use ironclaw_host_api::TenantId;
use ironclaw_host_api::ingress::{
    AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthPolicy,
    IngressJustification, IngressPolicy, IngressPolicyParts, IngressRouteDescriptor, ListenerClass,
    RateLimitPolicy, RateLimitScope, StreamingMode, WebSocketOriginPolicy,
};
use ironclaw_reborn_composition::{PublicRouteMount, WebuiAuthenticator};
use parking_lot::Mutex;
use rand::RngExt as _;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

use crate::session::SessionStore;

/// Default landing page after a successful login — matches the OAuth
/// callback's `DEFAULT_REDIRECT_AFTER` so the SPA lands in the same
/// place regardless of which flow authenticated it.
const DEFAULT_REDIRECT_AFTER: &str = "/";

/// Default session lifetime (30 days), matching the OAuth login
/// surface's default.
const DEFAULT_SESSION_LIFETIME: ChronoDuration = ChronoDuration::seconds(30 * 24 * 60 * 60);

/// Login tickets live only long enough for the SPA to load and POST
/// the ticket back — same TTL as the OAuth surface's session tickets.
const TICKET_TTL: Duration = Duration::from_secs(60);
/// Hard cap on outstanding tickets, bounding memory if callers mint
/// links but never redeem them.
const MAX_TICKETS: usize = 1024;

const PATH_LOGIN: &str = "/login";
const PATH_SESSION_EXCHANGE: &str = "/auth/session/exchange";

const ROUTE_ID_LOGIN: &str = "webui.cli_token_login.login";
const ROUTE_ID_SESSION_EXCHANGE: &str = "webui.cli_token_login.session_exchange";

const RATE_WINDOW_SECONDS: std::num::NonZeroU32 = std::num::NonZeroU32::new(60).expect("60 != 0"); // safety: const-evaluated, literal non-zero
const LOGIN_MAX_REQUESTS: std::num::NonZeroU32 = std::num::NonZeroU32::new(30).expect("30 != 0"); // safety: const-evaluated, literal non-zero
const EXCHANGE_MAX_REQUESTS: std::num::NonZeroU32 = std::num::NonZeroU32::new(60).expect("60 != 0"); // safety: const-evaluated, literal non-zero
const EXCHANGE_BODY_LIMIT_BYTES: std::num::NonZeroU64 =
    std::num::NonZeroU64::new(1024).expect("1024 != 0"); // safety: const-evaluated, literal non-zero

/// Host-supplied input to [`build_cli_token_login`].
pub struct CliTokenLoginConfig {
    /// Host-trusted installation tenant; never browser-influenced.
    pub tenant_id: TenantId,
    /// Constant-time bearer verifier the presented `?token=` is
    /// checked against, resolving the authenticated `UserId` on
    /// success. Production callers pass the same
    /// [`crate::EnvBearerAuthenticator`] the standalone deployment
    /// already builds for its API bearer, so there is no second
    /// secret to configure.
    pub authenticator: Arc<dyn WebuiAuthenticator>,
    /// Store the minted session bearer is created through. Production
    /// callers pass [`crate::signed_session_store`] built from the
    /// same operator secret + tenant as the CLI's admin bearer
    /// minter, so the resulting bearer validates anywhere that store
    /// is reconstructed.
    pub session_store: Arc<dyn SessionStore>,
    /// Session lifetime for the minted bearer. Defaults to 30 days.
    pub session_lifetime: ChronoDuration,
    /// Path the SPA lands on after the ticket is placed in the
    /// redirect query string. Defaults to `/`.
    pub redirect_after: String,
}

impl CliTokenLoginConfig {
    pub fn new(
        tenant_id: TenantId,
        authenticator: Arc<dyn WebuiAuthenticator>,
        session_store: Arc<dyn SessionStore>,
    ) -> Self {
        Self {
            tenant_id,
            authenticator,
            session_store,
            session_lifetime: DEFAULT_SESSION_LIFETIME,
            redirect_after: DEFAULT_REDIRECT_AFTER.to_string(),
        }
    }

    pub fn with_session_lifetime(mut self, lifetime: ChronoDuration) -> Self {
        self.session_lifetime = lifetime;
        self
    }

    /// Sanitized through the same `auth::pending::sanitize_redirect` rules
    /// the OAuth login surface's own `redirect_after` query param goes
    /// through (must be relative — no absolute or scheme-relative
    /// `//host/...` target, no `#` fragment) — falling back to
    /// [`DEFAULT_REDIRECT_AFTER`] when the supplied value fails that check.
    /// No production caller sets this today (`serve` never calls this
    /// setter), but it is a public builder method, so a future
    /// browser/query-influenced caller can't smuggle an off-site redirect
    /// through it.
    pub fn with_redirect_after(mut self, redirect_after: impl Into<String>) -> Self {
        self.redirect_after = crate::auth::pending::sanitize_redirect(Some(redirect_after.into()))
            .unwrap_or_else(|| DEFAULT_REDIRECT_AFTER.to_string());
        self
    }
}

struct RouterState {
    tenant_id: TenantId,
    authenticator: Arc<dyn WebuiAuthenticator>,
    session_store: Arc<dyn SessionStore>,
    session_lifetime: ChronoDuration,
    redirect_after: String,
    tickets: LoginTicketStore,
}

type RouterStateHandle = Arc<RouterState>;

/// Build the CLI-token login mount: `GET /login?token=...` plus the
/// `POST /auth/session/exchange` that redeems the ticket it mints.
pub fn build_cli_token_login(config: CliTokenLoginConfig) -> PublicRouteMount {
    let state: RouterStateHandle = Arc::new(RouterState {
        tenant_id: config.tenant_id,
        authenticator: config.authenticator,
        session_store: config.session_store,
        session_lifetime: config.session_lifetime,
        redirect_after: config.redirect_after,
        tickets: LoginTicketStore::new(),
    });

    let router = axum::Router::new()
        .route(PATH_LOGIN, get(login_handler))
        .route(PATH_SESSION_EXCHANGE, post(session_exchange_handler))
        .with_state(state);

    PublicRouteMount::new(router, route_descriptors())
}

fn route_descriptors() -> Vec<IngressRouteDescriptor> {
    vec![
        descriptor(
            ROUTE_ID_LOGIN,
            NetworkMethod::Get,
            PATH_LOGIN,
            public_policy(BodyLimitPolicy::NoBody, LOGIN_MAX_REQUESTS),
        ),
        descriptor(
            ROUTE_ID_SESSION_EXCHANGE,
            NetworkMethod::Post,
            PATH_SESSION_EXCHANGE,
            public_policy(
                BodyLimitPolicy::Limited {
                    max_bytes: EXCHANGE_BODY_LIMIT_BYTES,
                },
                EXCHANGE_MAX_REQUESTS,
            ),
        ),
    ]
}

fn descriptor(
    route_id: &str,
    method: NetworkMethod,
    pattern: &str,
    policy: IngressPolicy,
) -> IngressRouteDescriptor {
    IngressRouteDescriptor::new(route_id.to_string(), method, pattern.to_string(), policy)
        .expect("CLI-token login route descriptor must validate at startup") // safety: ids/patterns are crate-local literals and policies are constructed by the sibling helper below.
}

fn public_policy(body_limit: BodyLimitPolicy, max_requests: std::num::NonZeroU32) -> IngressPolicy {
    IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::LocalGateway,
        auth: IngressAuthPolicy::Public {
            justification: login_justification(),
        },
        scope_source: ironclaw_host_api::IngressScopeSource::PublicRoute,
        body_limit,
        rate_limit: RateLimitPolicy::Limited {
            scope: RateLimitScope::PerIp,
            max_requests,
            window_seconds: RATE_WINDOW_SECONDS,
        },
        cors: CorsPolicy::SameOriginOnly,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming: StreamingMode::None,
        audit: AuditTraceClass::PublicCallback,
        effect_path: AllowedEffectPath::NoEffect,
    })
    .expect("CLI-token login public policy must validate") // safety: LocalGateway + Public + NoEffect is permitted; rate-limit window/max are non-zero by construction.
}

fn login_justification() -> IngressJustification {
    IngressJustification::new(
        "webui-v2 cli-token-login",
        "the CLI-printed /login?token= link is unauthenticated by design — \
         the handler itself verifies the presented token before minting a \
         session, exactly like the OAuth callback it mirrors",
    )
    .expect("CLI-token login justification literal must validate") // safety: non-empty, no leading/trailing whitespace.
}

// ─── GET /login ─────────────────────────────────────────────────────

#[derive(Deserialize)]
struct LoginParams {
    token: Option<String>,
}

/// `GET /login?token=...` — verify the presented token, mint a
/// session bearer, place a one-time ticket for it, and redirect the
/// SPA to redeem the ticket via `POST /auth/session/exchange`.
async fn login_handler(
    State(state): State<RouterStateHandle>,
    Query(params): Query<LoginParams>,
) -> Response {
    let Some(token) = params.token.filter(|t| !t.is_empty()) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    // `authenticate` (e.g. `EnvBearerAuthenticator`) already performs
    // a constant-time comparison — reused rather than reimplemented,
    // per this route's own guardrail against a second bespoke compare.
    let Some(auth) = state.authenticator.authenticate(&token).await else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    // USER-DECIDED LAW: authenticating with the webui token = operator/admin,
    // whether via raw `Authorization: Bearer` or this `/login?token=` link.
    // `auth` above is the outcome of verifying the presented token against the
    // host's operator-capable authenticator (`EnvBearerAuthenticator` in
    // production), so its `operator_webui_config` bit is exactly the
    // provenance signal `SessionStore::create_session` wants — never
    // hardcode `true` here, and never re-derive operator-ness later from the
    // bearer at validation time.
    let bearer = match state
        .session_store
        .create_session(
            state.tenant_id.clone(),
            auth.user_id,
            state.session_lifetime,
            auth.capabilities.operator_webui_config,
        )
        .await
    {
        Ok(bearer) => bearer,
        Err(err) => {
            tracing::error!(
                target = "ironclaw::reborn::webui_ingress::cli_token_login",
                error = %err,
                "session store create_session failed",
            );
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let ticket = state.tickets.insert(bearer);
    let location = build_redirect(&state.redirect_after, &ticket);
    Redirect::to(&location).into_response()
}

fn build_redirect(redirect_after: &str, ticket: &str) -> String {
    let separator = if redirect_after.contains('?') {
        '&'
    } else {
        '?'
    };
    format!(
        "{redirect_after}{separator}login_ticket={}",
        urlencoding::encode(ticket)
    )
}

// ─── POST /auth/session/exchange ─────────────────────────────────────

#[derive(Deserialize)]
struct SessionExchangeRequest {
    ticket: String,
}

#[derive(Serialize)]
struct SessionExchangeResponse {
    token: String,
}

/// `POST /auth/session/exchange` — consume the one-time ticket minted
/// by `login_handler` and return the real session bearer. Identical
/// contract to `auth::routes::session_exchange_handler`, so the SPA's
/// `exchangeLoginTicket` needs no new code to talk to either surface.
async fn session_exchange_handler(
    State(state): State<RouterStateHandle>,
    Json(request): Json<SessionExchangeRequest>,
) -> Response {
    let ticket = request.ticket.trim();
    if ticket.is_empty() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(bearer) = state.tickets.take(ticket) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    Json(SessionExchangeResponse {
        token: bearer.expose_secret().to_string(),
    })
    .into_response()
}

// ─── one-time ticket store ───────────────────────────────────────────

struct LoginTicket {
    bearer: SecretString,
    created_at: Instant,
}

/// Bounded, TTL'd, single-use bearer exchange store — the same shape
/// as `auth::pending::SessionTicketStore`, duplicated here rather than
/// shared because that store is private to the OAuth login surface
/// and (per the module doc above) this mount must work even when no
/// OAuth provider — and therefore no OAuth ticket store — exists.
struct LoginTicketStore {
    inner: Mutex<HashMap<String, LoginTicket>>,
}

impl LoginTicketStore {
    fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    fn insert(&self, bearer: SecretString) -> String {
        let ticket = mint_ticket();
        let entry = LoginTicket {
            bearer,
            created_at: Instant::now(),
        };

        let mut guard = self.inner.lock();
        if guard.len() >= MAX_TICKETS {
            guard.retain(|_, ticket| ticket.created_at.elapsed() < TICKET_TTL);
        }
        if guard.len() >= MAX_TICKETS
            && let Some(oldest) = guard
                .iter()
                .min_by_key(|(_, ticket)| ticket.created_at)
                .map(|(k, _)| k.clone())
        {
            guard.remove(&oldest);
        }
        guard.insert(ticket.clone(), entry);
        ticket
    }

    fn take(&self, ticket: &str) -> Option<SecretString> {
        let mut guard = self.inner.lock();
        let entry = guard.remove(ticket)?;
        if entry.created_at.elapsed() >= TICKET_TTL {
            return None;
        }
        Some(entry.bearer)
    }
}

fn mint_ticket() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticket_store_is_single_use() {
        let store = LoginTicketStore::new();
        let ticket = store.insert(SecretString::from("bearer-1".to_string()));
        assert_eq!(
            store.take(&ticket).map(|s| s.expose_secret().to_string()),
            Some("bearer-1".to_string())
        );
        assert!(store.take(&ticket).is_none());
    }

    #[test]
    fn unknown_ticket_returns_none() {
        let store = LoginTicketStore::new();
        assert!(store.take("nonexistent").is_none());
    }

    #[test]
    fn expired_ticket_returns_none_and_is_removed() {
        let store = LoginTicketStore::new();
        let ticket = "expired-ticket".to_string();
        {
            let mut guard = store.inner.lock();
            guard.insert(
                ticket.clone(),
                LoginTicket {
                    bearer: SecretString::from("expired-bearer".to_string()),
                    created_at: Instant::now() - TICKET_TTL - Duration::from_secs(1),
                },
            );
        }
        assert!(store.take(&ticket).is_none());
        assert!(!store.inner.lock().contains_key(&ticket));
    }

    fn test_config() -> CliTokenLoginConfig {
        let tenant = TenantId::new("tenant-a").expect("tenant");
        let session_store = crate::signed_session_store(
            &SecretString::from("test-signing-key-0123456789abcdef0123456789".to_string()),
            &tenant,
        );
        let authenticator = Arc::new(
            crate::EnvBearerAuthenticator::new(
                SecretString::from("test-bearer-0123456789abcdef0123456789".to_string()),
                ironclaw_host_api::UserId::new("operator").expect("user"),
            )
            .expect("env bearer authenticator"),
        );
        CliTokenLoginConfig::new(tenant, authenticator, session_store)
    }

    #[test]
    fn with_redirect_after_rejects_an_absolute_url_and_falls_back_to_the_default() {
        let config = test_config().with_redirect_after("https://evil.example");
        assert_eq!(config.redirect_after, DEFAULT_REDIRECT_AFTER);
    }

    #[test]
    fn with_redirect_after_rejects_a_scheme_relative_url_and_falls_back_to_the_default() {
        let config = test_config().with_redirect_after("//evil.example");
        assert_eq!(config.redirect_after, DEFAULT_REDIRECT_AFTER);
    }

    #[test]
    fn with_redirect_after_preserves_a_safe_relative_path() {
        let config = test_config().with_redirect_after("/v2/foo");
        assert_eq!(config.redirect_after, "/v2/foo");
    }

    #[test]
    fn build_redirect_appends_login_ticket_query_param() {
        assert_eq!(build_redirect("/", "abc"), "/?login_ticket=abc");
        assert_eq!(
            build_redirect("/?tab=settings", "abc"),
            "/?tab=settings&login_ticket=abc"
        );
    }
}
