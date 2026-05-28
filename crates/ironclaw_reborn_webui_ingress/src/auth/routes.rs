//! HTTP route handlers for the WebChat v2 OAuth login flow.
//!
//! Mounted by composition as an UNAUTHENTICATED route group — the
//! browser hits `/auth/providers`, `/auth/login/{provider}`, and
//! `/auth/callback/{provider}` before it has a session, so the
//! bearer-auth middleware must not run in front of them.
//!
//! `/auth/logout` accepts an `Authorization: Bearer <token>` header
//! (the session token the SPA stored after a previous callback) and
//! revokes the underlying session record. Composition mounts it in
//! the SAME public group as the login routes for symmetry — logout
//! has its own per-route bearer check inside the handler so a bare
//! request without a header is harmless.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use chrono::Duration as ChronoDuration;
use ironclaw_host_api::TenantId;
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};

use super::error::OAuthError;
use super::google::OAuthProvider;
use super::pending::{PendingFlowStore, sanitize_redirect};
use super::user_directory::{UserDirectory, UserDirectoryError};
use crate::session::SessionStore;

/// Default landing page after a successful OAuth callback. The SPA
/// reads `#token=` from the fragment and stores it in sessionStorage.
const DEFAULT_REDIRECT_AFTER: &str = "/v2";

/// Default session lifetime (30 days). Matches the v1 gateway's
/// `SESSION_LIFETIME_SECS`; production deployments can override via
/// [`OAuthRouterConfig::session_lifetime`].
const DEFAULT_SESSION_LIFETIME: ChronoDuration = ChronoDuration::seconds(30 * 24 * 60 * 60);

/// Owner-supplied config for the OAuth router.
///
/// `base_url` is the externally-visible origin the v2 listener is
/// reachable at (e.g. `https://app.example.com`). It is used to
/// build the OAuth `redirect_uri` Google calls back to and so it
/// must match what was registered in the Google Cloud Console.
#[derive(Clone)]
pub struct OAuthRouterConfig {
    pub tenant_id: TenantId,
    pub session_store: Arc<dyn SessionStore>,
    pub user_directory: Arc<dyn UserDirectory>,
    pub providers: Vec<Arc<dyn OAuthProvider>>,
    pub base_url: String,
    pub session_lifetime: ChronoDuration,
}

impl OAuthRouterConfig {
    /// Build a config with the default 30-day session lifetime.
    pub fn new(
        tenant_id: TenantId,
        session_store: Arc<dyn SessionStore>,
        user_directory: Arc<dyn UserDirectory>,
        providers: Vec<Arc<dyn OAuthProvider>>,
        base_url: impl Into<String>,
    ) -> Self {
        Self {
            tenant_id,
            session_store,
            user_directory,
            providers,
            base_url: base_url.into(),
            session_lifetime: DEFAULT_SESSION_LIFETIME,
        }
    }

    pub fn with_session_lifetime(mut self, lifetime: ChronoDuration) -> Self {
        self.session_lifetime = lifetime;
        self
    }
}

/// Internal state shared across all `/auth/*` handlers.
struct RouterState {
    tenant_id: TenantId,
    session_store: Arc<dyn SessionStore>,
    user_directory: Arc<dyn UserDirectory>,
    providers: Vec<Arc<dyn OAuthProvider>>,
    base_url: String,
    session_lifetime: ChronoDuration,
    pending: PendingFlowStore,
}

impl RouterState {
    fn provider(&self, name: &str) -> Option<Arc<dyn OAuthProvider>> {
        self.providers
            .iter()
            .find(|p| p.name() == name)
            .map(Arc::clone)
    }

    fn callback_url(&self, provider_name: &str) -> String {
        format!("{}/auth/callback/{provider_name}", self.base_url)
    }
}

type RouterStateHandle = Arc<RouterState>;

/// Build the unauthenticated axum sub-router that mounts the OAuth
/// login endpoints. Composition merges this router as a public route
/// group alongside the bearer-protected WebChat v2 routes.
pub fn webui_v2_auth_router(config: OAuthRouterConfig) -> axum::Router {
    let state: RouterStateHandle = Arc::new(RouterState {
        tenant_id: config.tenant_id,
        session_store: config.session_store,
        user_directory: config.user_directory,
        providers: config.providers,
        base_url: config.base_url,
        session_lifetime: config.session_lifetime,
        pending: PendingFlowStore::new(),
    });

    axum::Router::new()
        .route("/auth/providers", get(providers_handler))
        .route("/auth/login/{provider}", get(login_handler))
        .route("/auth/callback/{provider}", get(callback_handler))
        .route("/auth/logout", post(logout_handler))
        .with_state(state)
}

// ─── /auth/providers ──────────────────────────────────────────────────

#[derive(Serialize)]
struct ProvidersResponse {
    providers: Vec<&'static str>,
}

/// `GET /auth/providers` — list the providers configured at startup.
/// Empty list when no provider was wired. The SPA filters this list
/// against its supported set so a future backend that adds new
/// providers without a matching SPA build still renders cleanly.
async fn providers_handler(State(state): State<RouterStateHandle>) -> Json<ProvidersResponse> {
    let mut providers: Vec<&'static str> = state.providers.iter().map(|p| p.name()).collect();
    providers.sort_unstable();
    Json(ProvidersResponse { providers })
}

// ─── /auth/login/{provider} ───────────────────────────────────────────

#[derive(Deserialize)]
struct LoginParams {
    /// Optional same-origin path the SPA should land on after the
    /// callback completes. Validated through `sanitize_redirect` to
    /// block open redirects.
    redirect_after: Option<String>,
}

/// `GET /auth/login/{provider}` — initiate the OAuth flow. Mints a
/// pending-flow entry and redirects the browser to the provider's
/// authorization URL.
async fn login_handler(
    State(state): State<RouterStateHandle>,
    Path(provider_name): Path<String>,
    Query(params): Query<LoginParams>,
) -> Response {
    let Some(provider) = state.provider(&provider_name) else {
        return (
            StatusCode::NOT_FOUND,
            format!("Unknown OAuth provider: {provider_name}"),
        )
            .into_response();
    };

    let redirect_after = sanitize_redirect(params.redirect_after);
    let (csrf_state, flow) = state.pending.insert(provider.name(), redirect_after);
    let callback_url = state.callback_url(provider.name());
    // `flow.code_challenge` is the SHA-256 the pending store
    // pre-computed at mint time — no second hash per login redirect.
    let auth_url = provider.authorization_url(&callback_url, &csrf_state, &flow.code_challenge);

    Redirect::temporary(&auth_url).into_response()
}

// ─── /auth/callback/{provider} ────────────────────────────────────────

#[derive(Deserialize)]
struct CallbackParams {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

/// `GET /auth/callback/{provider}` — handle the provider's callback,
/// exchange the code, resolve the user, issue a session, redirect
/// back to the SPA with the session token in the URL fragment.
async fn callback_handler(
    State(state): State<RouterStateHandle>,
    Path(provider_name): Path<String>,
    Query(params): Query<CallbackParams>,
) -> Response {
    // Provider-initiated denial (user clicked "cancel" on the consent
    // screen, account suspended, etc.). Surface a generic redirect
    // back to the SPA with `?login_error=denied` so the login page
    // can render an error banner without exposing the provider's
    // description verbatim.
    if let Some(error) = params.error {
        tracing::info!(
            target = "ironclaw::reborn::webui_ingress::auth",
            provider = %provider_name,
            error = %error,
            description = ?params.error_description,
            "OAuth provider returned an error",
        );
        return spa_error_redirect("denied").into_response();
    }

    let Some(code) = params.code.filter(|c| !c.is_empty()) else {
        return spa_error_redirect("invalid_request").into_response();
    };
    let Some(csrf_state) = params.state.filter(|s| !s.is_empty()) else {
        return spa_error_redirect("invalid_request").into_response();
    };

    let Some(flow) = state.pending.take(&csrf_state) else {
        // Unknown state token: either expired (>5 min in the pending
        // store) or a replay of an already-consumed callback. Fail
        // closed — never re-use a state token.
        return spa_error_redirect("invalid_state").into_response();
    };
    if flow.provider != provider_name {
        // Cross-provider state replay (e.g. GitHub state arriving on
        // the Google callback). Fail closed.
        return spa_error_redirect("provider_mismatch").into_response();
    }

    let Some(provider) = state.provider(&provider_name) else {
        return spa_error_redirect("invalid_request").into_response();
    };

    let callback_url = state.callback_url(provider.name());
    let profile = match provider
        .exchange_code(&code, &callback_url, flow.code_verifier.expose_secret())
        .await
    {
        Ok(profile) => profile,
        Err(err) => {
            log_oauth_error(&provider_name, &err);
            return spa_error_redirect(error_code_for(&err)).into_response();
        }
    };

    let user_id = match state
        .user_directory
        .resolve(provider.name(), &profile)
        .await
    {
        Ok(uid) => uid,
        Err(UserDirectoryError::Unknown) => {
            tracing::info!(
                target = "ironclaw::reborn::webui_ingress::auth",
                provider = %provider_name,
                email = ?profile.email,
                "user directory rejected unknown profile",
            );
            return spa_error_redirect("unauthorized").into_response();
        }
        Err(UserDirectoryError::Backend(reason)) => {
            tracing::warn!(
                target = "ironclaw::reborn::webui_ingress::auth",
                provider = %provider_name,
                error = %reason,
                "user directory backend failure",
            );
            return spa_error_redirect("server_error").into_response();
        }
    };

    let bearer = match state
        .session_store
        .create_session(state.tenant_id.clone(), user_id, state.session_lifetime)
        .await
    {
        Ok(token) => token,
        Err(err) => {
            tracing::error!(
                target = "ironclaw::reborn::webui_ingress::auth",
                provider = %provider_name,
                error = %err,
                "session store create_session failed",
            );
            return spa_error_redirect("server_error").into_response();
        }
    };

    let redirect_after = flow
        .redirect_after
        .as_deref()
        .unwrap_or(DEFAULT_REDIRECT_AFTER);
    let location = build_success_redirect(redirect_after, bearer.expose_secret());
    Redirect::to(&location).into_response()
}

// ─── /auth/logout ─────────────────────────────────────────────────────

/// `POST /auth/logout` — revoke the bearer session and clear it from
/// the durable session store. Honors `Authorization: Bearer <token>`
/// only — query-token shim is reserved for the SSE route per the
/// composition's `extract_bearer_token` policy. Returns `204` whether
/// or not a token was present so the client UX is unconditional.
async fn logout_handler(
    State(state): State<RouterStateHandle>,
    headers: axum::http::HeaderMap,
) -> Response {
    if let Some(token) = extract_bearer(&headers)
        && let Err(err) = state.session_store.revoke(&token).await
    {
        // A revoke failure is operator-relevant (durable store may
        // have disconnected) but the client UX is still "you are
        // signed out locally" — return 204 so the SPA clears its
        // sessionStorage either way.
        tracing::warn!(
            target = "ironclaw::reborn::webui_ingress::auth",
            error = %err,
            "session store revoke failed during logout",
        );
    }
    StatusCode::NO_CONTENT.into_response()
}

// ─── helpers ──────────────────────────────────────────────────────────

fn extract_bearer(headers: &axum::http::HeaderMap) -> Option<String> {
    let value = headers.get(header::AUTHORIZATION)?;
    let text = value.to_str().ok()?;
    let prefix = text.get(..7)?;
    if !prefix.eq_ignore_ascii_case("Bearer ") {
        return None;
    }
    let candidate = text[7..].trim();
    if candidate.is_empty() {
        None
    } else {
        Some(candidate.to_string())
    }
}

/// Build the success redirect URL: `<redirect_after>#token=<bearer>`.
/// The fragment is the SPA's contract — `app/auth.js::consumeTokenFromUrl`
/// reads it on load. Fragments are never sent to the server in
/// subsequent navigation, so the bearer cannot leak through HTTP
/// access logs or `Referer` headers.
fn build_success_redirect(redirect_after: &str, bearer: &str) -> String {
    // `redirect_after` was already validated by `sanitize_redirect`
    // to start with `/` and to contain only RFC-3986 path chars.
    // Encode the bearer to be safe inside the fragment (uuid v4
    // strings are already URL-safe, but a future SessionStore impl
    // might mint a different shape).
    format!("{redirect_after}#token={}", urlencoding::encode(bearer))
}

/// Build a redirect back to the SPA login route with an opaque error
/// code in the query string. The SPA maps the code to a localized
/// error banner.
fn spa_error_redirect(code: &str) -> Redirect {
    let target = format!("/v2?login_error={}", urlencoding::encode(code));
    Redirect::to(&target)
}

fn error_code_for(err: &OAuthError) -> &'static str {
    match err {
        OAuthError::CodeExchange(_) | OAuthError::ProfileFetch(_) => "exchange_failed",
        OAuthError::Denied(_) => "unauthorized",
    }
}

fn log_oauth_error(provider_name: &str, err: &OAuthError) {
    // Provider error bodies and JWT decode details are operator-only
    // — never echoed back to the client. Logged at `warn!` so they
    // appear in production logs without spamming `info!` on every
    // user-cancelled login.
    tracing::warn!(
        target = "ironclaw::reborn::webui_ingress::auth",
        provider = %provider_name,
        error = %err,
        "OAuth flow failed",
    );
}
