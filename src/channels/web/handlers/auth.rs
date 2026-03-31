//! OAuth authentication handlers.
//!
//! Public (no auth required) endpoints for initiating and completing
//! OAuth login flows via configured providers (Google, GitHub).

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
};
use rand::RngCore;
use rand::rngs::OsRng;
use uuid::Uuid;

use crate::channels::web::oauth::state_store::{OAuthStateStore, new_oauth_flow};
use crate::channels::web::server::GatewayState;
use crate::db::{UserIdentityRecord, UserRecord};

/// Query parameters for the login redirect.
#[derive(serde::Deserialize)]
pub struct LoginParams {
    /// Optional URL to redirect to after login (default: `/`).
    redirect_after: Option<String>,
}

/// Query parameters from the OAuth provider callback.
#[derive(serde::Deserialize)]
pub struct CallbackParams {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

/// GET /auth/providers — list enabled OAuth providers.
pub async fn providers_handler(State(state): State<Arc<GatewayState>>) -> Json<serde_json::Value> {
    let providers: Vec<&str> = match state.oauth_providers.as_ref() {
        Some(map) => map.keys().map(|s| s.as_str()).collect(),
        None => Vec::new(),
    };
    Json(serde_json::json!({ "providers": providers }))
}

/// GET /auth/login/{provider} — initiate OAuth flow (redirect to provider).
pub async fn login_handler(
    State(state): State<Arc<GatewayState>>,
    Path(provider_name): Path<String>,
    Query(params): Query<LoginParams>,
) -> Result<Response, (StatusCode, String)> {
    if !state.oauth_rate_limiter.check() {
        return Err((StatusCode::TOO_MANY_REQUESTS, "Rate limited".to_string()));
    }

    let providers = state
        .oauth_providers
        .as_ref()
        .ok_or((StatusCode::NOT_FOUND, "OAuth is not enabled".to_string()))?;

    let provider = providers.get(&provider_name).ok_or((
        StatusCode::NOT_FOUND,
        format!("Unknown OAuth provider: {provider_name}"),
    ))?;

    let state_store = state.oauth_state_store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "OAuth state store not available".to_string(),
    ))?;

    let base_url = state.oauth_base_url.as_deref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "OAuth base URL not configured".to_string(),
    ))?;

    let flow = new_oauth_flow(provider_name.clone(), params.redirect_after);
    let code_challenge = OAuthStateStore::code_challenge(&flow.code_verifier);
    let csrf_state = state_store.insert(flow).await;

    let callback_url = format!("{base_url}/auth/callback/{provider_name}");
    let auth_url = provider.authorization_url(&callback_url, &csrf_state, &code_challenge);

    Ok(Redirect::temporary(&auth_url).into_response())
}

/// GET /auth/callback/{provider} — OAuth callback (exchange code, issue session).
pub async fn callback_handler(
    State(state): State<Arc<GatewayState>>,
    Path(provider_name): Path<String>,
    Query(params): Query<CallbackParams>,
) -> Response {
    if !state.oauth_rate_limiter.check() {
        return error_page("Too many requests. Please try again later.");
    }

    // Check for error from the OAuth provider (e.g. user denied consent).
    if let Some(ref error) = params.error {
        let desc = params
            .error_description
            .as_deref()
            .unwrap_or(error.as_str());
        return error_page(desc);
    }

    let code = match params.code.as_deref() {
        Some(c) if !c.is_empty() => c,
        _ => return error_page("Missing authorization code"),
    };

    let csrf_state = match params.state.as_deref() {
        Some(s) if !s.is_empty() => s,
        _ => return error_page("Missing state parameter"),
    };

    // Validate CSRF state and retrieve the PKCE code verifier.
    let state_store = match state.oauth_state_store.as_ref() {
        Some(s) => s,
        None => return error_page("OAuth not configured"),
    };

    let flow = match state_store.take(csrf_state).await {
        Some(f) => f,
        None => return error_page("Invalid or expired OAuth state. Please try logging in again."),
    };

    // Verify the provider matches (prevent cross-provider state replay).
    if flow.provider != provider_name {
        return error_page("OAuth provider mismatch");
    }

    let providers = match state.oauth_providers.as_ref() {
        Some(p) => p,
        None => return error_page("OAuth not configured"),
    };

    let provider = match providers.get(&provider_name) {
        Some(p) => p,
        None => return error_page("Unknown OAuth provider"),
    };

    let store = match state.store.as_ref() {
        Some(s) => s,
        None => return error_page("Database not available"),
    };

    let base_url = match state.oauth_base_url.as_deref() {
        Some(u) => u,
        None => return error_page("OAuth base URL not configured"),
    };

    let callback_url = format!("{base_url}/auth/callback/{provider_name}");

    // Exchange the authorization code for a user profile.
    let profile = match provider
        .exchange_code(code, &callback_url, &flow.code_verifier)
        .await
    {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, provider = %provider_name, "OAuth code exchange failed");
            return error_page("Failed to complete login. Please try again.");
        }
    };

    // Resolve user: find existing, link by email, or create new.
    let (user_id, is_new) = match resolve_user(store.as_ref(), &provider_name, &profile).await {
        Ok(result) => result,
        Err(e) => {
            tracing::error!(error = %e, "OAuth user resolution failed");
            return error_page("Failed to create or link user account.");
        }
    };

    // Record login.
    if let Err(e) = store.record_login(&user_id).await {
        tracing::warn!(error = %e, user_id = %user_id, "Failed to record login");
    }

    // Generate an API token for the new session.
    let mut token_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut token_bytes);
    let plaintext_token = hex::encode(token_bytes);
    let token_hash = crate::channels::web::auth::hash_token(&plaintext_token);
    let token_prefix = &plaintext_token[..8];

    let token_name = if is_new {
        format!("oauth-{provider_name}-initial")
    } else {
        format!("oauth-{provider_name}-login")
    };

    if let Err(e) = store
        .create_api_token(&user_id, &token_name, &token_hash, token_prefix, None)
        .await
    {
        tracing::error!(error = %e, "Failed to create API token for OAuth login");
        return error_page("Failed to create session. Please try again.");
    }

    // Invalidate the DbAuthenticator cache so the new token is immediately usable.
    if let Some(ref db_auth) = state.db_auth {
        db_auth.invalidate_user(&user_id).await;
    }

    let redirect_to = flow.redirect_after.as_deref().unwrap_or("/");

    // Build the response with a session cookie.
    let cookie_value = build_session_cookie(&plaintext_token, is_secure(base_url));
    let mut response = Redirect::temporary(redirect_to).into_response();
    if let Ok(hv) = HeaderValue::from_str(&cookie_value) {
        response.headers_mut().insert(header::SET_COOKIE, hv);
    }

    response
}

/// POST /auth/logout — clear session cookie.
pub async fn logout_handler() -> Response {
    let cookie = "ironclaw_session=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0";
    let mut response = (StatusCode::OK, "Logged out").into_response();
    if let Ok(hv) = HeaderValue::from_str(cookie) {
        response.headers_mut().insert(header::SET_COOKIE, hv);
    }
    response
}

// ── User resolution ──────────────────────────────────────────────────────

/// Resolve the OAuth profile to an existing or new user.
///
/// Returns `(user_id, is_new_user)`.
async fn resolve_user(
    store: &dyn crate::db::Database,
    provider: &str,
    profile: &crate::channels::web::oauth::OAuthUserProfile,
) -> Result<(String, bool), String> {
    // 1. Check if this provider identity is already linked.
    if let Some(existing) = store
        .get_identity_by_provider(provider, &profile.provider_user_id)
        .await
        .map_err(|e| e.to_string())?
    {
        // Verify the user is still active.
        if let Some(user) = store
            .get_user(&existing.user_id)
            .await
            .map_err(|e| e.to_string())?
        {
            if user.status != "active" {
                return Err(format!("Account is {}", user.status));
            }
            return Ok((existing.user_id, false));
        }
    }

    // 2. Try to link by verified email.
    if let Some(ref email) = profile.email
        && profile.email_verified
    {
        // Check user_identities for a verified email match.
        if let Some(identity) = store
            .find_identity_by_verified_email(email)
            .await
            .map_err(|e| e.to_string())?
        {
            // Link this new provider to the existing user.
            let new_identity = build_identity_record(&identity.user_id, provider, profile);
            store
                .create_identity(&new_identity)
                .await
                .map_err(|e| e.to_string())?;
            return Ok((identity.user_id, false));
        }

        // Check the users table directly for email match.
        if let Some(user) = store
            .get_user_by_email(email)
            .await
            .map_err(|e| e.to_string())?
            && user.status == "active"
        {
            let new_identity = build_identity_record(&user.id, provider, profile);
            store
                .create_identity(&new_identity)
                .await
                .map_err(|e| e.to_string())?;
            return Ok((user.id, false));
        }
    }

    // 3. Create a new user.
    let is_first_user = !store.has_any_users().await.map_err(|e| e.to_string())?;
    let role = if is_first_user { "admin" } else { "member" };

    let user_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now();

    let display_name = profile
        .display_name
        .clone()
        .unwrap_or_else(|| profile.email.clone().unwrap_or_else(|| "User".to_string()));

    let user = UserRecord {
        id: user_id.clone(),
        email: profile.email.clone(),
        display_name,
        status: "active".to_string(),
        role: role.to_string(),
        created_at: now,
        updated_at: now,
        last_login_at: Some(now),
        created_by: None,
        metadata: serde_json::json!({}),
    };

    let identity = build_identity_record(&user_id, provider, profile);

    let mut token_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut token_bytes);
    let token_hash = crate::channels::web::auth::hash_token(&hex::encode(token_bytes));
    let token_prefix = &hex::encode(&token_bytes[..4]);

    store
        .create_user_with_identity_and_token(
            &user,
            &identity,
            &format!("oauth-{provider}-signup"),
            &token_hash,
            token_prefix,
        )
        .await
        .map_err(|e| e.to_string())?;

    Ok((user_id, true))
}

fn build_identity_record(
    user_id: &str,
    provider: &str,
    profile: &crate::channels::web::oauth::OAuthUserProfile,
) -> UserIdentityRecord {
    let now = chrono::Utc::now();
    UserIdentityRecord {
        id: Uuid::new_v4(),
        user_id: user_id.to_string(),
        provider: provider.to_string(),
        provider_user_id: profile.provider_user_id.clone(),
        email: profile.email.clone(),
        email_verified: profile.email_verified,
        display_name: profile.display_name.clone(),
        avatar_url: profile.avatar_url.clone(),
        raw_profile: profile.raw.clone(),
        created_at: now,
        updated_at: now,
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn build_session_cookie(token: &str, secure: bool) -> String {
    let secure_flag = if secure { "; Secure" } else { "" };
    format!(
        "ironclaw_session={token}; HttpOnly; SameSite=Lax; Path=/; Max-Age=2592000{secure_flag}"
    )
}

fn is_secure(base_url: &str) -> bool {
    base_url.starts_with("https://")
}

fn error_page(message: &str) -> Response {
    let escaped = message
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;");
    axum::response::Html(format!(
        "<html><body style='font-family: system-ui; text-align: center; padding: 60px;'>\
         <h2>Login Failed</h2>\
         <p>{escaped}</p>\
         <p><a href='/'>Return to home</a></p>\
         </body></html>"
    ))
    .into_response()
}
