//! `GET /login?token=<cli-token>` — CLI-printed bootstrap link into the
//! browser session, sharing the OAuth callback's bearer/ticket-exchange
//! contract (see `signed_session_login.rs` module docs for why this
//! wiring lives in this crate, not the command crate).
//!
//! Flow (mirrors `auth::routes::callback_handler`):
//! - `GET /login?token=...`: verify via [`crate::EnvBearerAuthenticator`],
//!   mint a bearer via [`SignedTokenSessionStore`], redirect to
//!   `<redirect_after>?login_ticket=<ticket>` (same convention as OAuth).
//! - `POST /auth/session/exchange`: consumes the ticket, returns
//!   `{ "token": "..." }` — byte-for-byte the same contract as
//!   `auth::routes::session_exchange_handler`, so the SPA's
//!   `exchangeLoginTicket` needs no new frontend code.
//!
//! Owns its own one-time ticket store rather than `auth::routes`'s
//! (private to that module) because CLI-token login must work even with
//! no OAuth provider configured.
//!
//! Integration note: both this mount and the OAuth mount register `POST
//! /auth/session/exchange` — attach at most one per deployment or routes
//! collide at merge time.

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
use ironclaw_host_ingress::PublicRouteMount;
use parking_lot::Mutex;
use rand::RngExt as _;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

use crate::WebuiAuthenticator;
use crate::signed_session_login::SignedTokenSessionStore;

/// Matches the OAuth callback's default so the SPA lands in the same
/// place regardless of which flow authenticated it.
const DEFAULT_REDIRECT_AFTER: &str = "/";

/// Matches the OAuth login surface's default (30 days).
const DEFAULT_SESSION_LIFETIME: ChronoDuration = ChronoDuration::seconds(30 * 24 * 60 * 60);

/// Same TTL as the OAuth surface's session tickets.
const TICKET_TTL: Duration = Duration::from_secs(60);
/// Bounds memory if callers mint links but never redeem them.
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
    /// Constant-time verifier for the presented `?token=`, resolving the
    /// authenticated `UserId`. Prod passes the same
    /// [`crate::EnvBearerAuthenticator`] used for the API bearer — no
    /// second secret to configure.
    pub authenticator: Arc<dyn WebuiAuthenticator>,
    /// Store the minted session bearer is created through. Prod passes
    /// [`crate::signed_session_store`] built from the same operator
    /// secret + tenant as the CLI's admin bearer minter, so the bearer
    /// validates anywhere that store is reconstructed.
    pub session_store: Arc<SignedTokenSessionStore>,
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
        session_store: Arc<SignedTokenSessionStore>,
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

    // `redirect_after` is fixed to `DEFAULT_REDIRECT_AFTER` until a real
    // caller needs it — a `with_redirect_after` setter was removed as
    // speculative surface (zero production callers) in a security-sensitive
    // module; re-add it (with `auth::pending::sanitize_redirect`) if one
    // shows up.
}

struct RouterState {
    tenant_id: TenantId,
    authenticator: Arc<dyn WebuiAuthenticator>,
    session_store: Arc<SignedTokenSessionStore>,
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

    // `authenticate` already does a constant-time compare (reused, not
    // reimplemented).
    let Some(auth) = state.authenticator.authenticate(&token).await else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    // USER-DECIDED LAW: webui-token auth = operator/admin, same as raw
    // `Authorization: Bearer`.
    // - `auth.capabilities.operator_webui_config` is the provenance signal
    //   `create_session` wants — never hardcode `true`, never re-derive
    //   operator-ness later at validation time.
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

/// Bounded, TTL'd, single-use bearer exchange store — same shape as
/// `auth::pending::SessionTicketStore`, duplicated because that store is
/// private to the OAuth login surface and this mount must work with no
/// OAuth provider (and thus no OAuth ticket store) present.
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

    #[test]
    fn ticket_store_evicts_oldest_at_capacity() {
        let store = LoginTicketStore::new();
        // Seed to capacity with deterministic, strictly increasing
        // timestamps so eviction order is unambiguous regardless of
        // `Instant::now()` resolution on this platform.
        let mut tickets = Vec::with_capacity(MAX_TICKETS);
        {
            let mut guard = store.inner.lock();
            let base = Instant::now();
            for i in 0..MAX_TICKETS {
                let ticket = format!("ticket-{i}");
                guard.insert(
                    ticket.clone(),
                    LoginTicket {
                        bearer: SecretString::from(format!("bearer-{i}")),
                        created_at: base + Duration::from_millis(i as u64),
                    },
                );
                tickets.push(ticket);
            }
        }

        let newest = store.insert(SecretString::from("bearer-newest".to_string()));

        let oldest = &tickets[0];
        assert!(
            store.take(oldest).is_none(),
            "oldest ticket must be evicted once the store is at capacity"
        );
        assert_eq!(
            store.take(&newest).map(|s| s.expose_secret().to_string()),
            Some("bearer-newest".to_string())
        );
    }

    #[test]
    fn ticket_store_single_redemption_under_concurrent_take() {
        // `take()` holds the store's `parking_lot::Mutex` for its whole
        // critical section, so redemption is atomic by construction — this
        // proves it holds under real concurrent access (Arc-shared across
        // OS threads with a barrier to force contention), pinning the
        // single-redemption invariant rather than just calling `take()`
        // twice sequentially from one thread.
        let store = Arc::new(LoginTicketStore::new());
        let ticket = store.insert(SecretString::from("bearer-1".to_string()));
        let barrier = Arc::new(std::sync::Barrier::new(2));

        let results: Vec<Option<SecretString>> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..2)
                .map(|_| {
                    let store = Arc::clone(&store);
                    let ticket = ticket.clone();
                    let barrier = Arc::clone(&barrier);
                    scope.spawn(move || {
                        barrier.wait();
                        store.take(&ticket)
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });

        let redeemed = results.iter().filter(|r| r.is_some()).count();
        assert_eq!(
            redeemed, 1,
            "exactly one concurrent taker must redeem the ticket"
        );
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
