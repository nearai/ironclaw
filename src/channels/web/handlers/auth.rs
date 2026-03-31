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

/// Cookie name for OAuth browser sessions.
const SESSION_COOKIE_NAME: &str = "ironclaw_session";
/// Session lifetime: 30 days (cookie Max-Age and token expiry).
const SESSION_LIFETIME_SECS: i64 = 30 * 24 * 60 * 60;

/// Query parameters for the login redirect.
#[derive(serde::Deserialize)]
pub struct LoginParams {
    /// Optional URL to redirect to after login (default: `/`).
    redirect_after: Option<String>,
}

/// Parameters from the OAuth provider callback (query string or form body).
#[derive(serde::Deserialize)]
pub struct CallbackParams {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
    /// Apple-specific: JSON-encoded user info (name), sent only on first authorization.
    user: Option<String>,
}

/// GET /auth/providers — list enabled OAuth providers.
pub async fn providers_handler(State(state): State<Arc<GatewayState>>) -> Json<serde_json::Value> {
    let mut providers: Vec<&str> = match state.oauth_providers.as_ref() {
        Some(map) => map.keys().map(|s| s.as_str()).collect(),
        None => Vec::new(),
    };
    if state.near_nonce_store.is_some() {
        providers.push("near");
    }
    providers.sort_unstable();
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

/// GET /auth/callback/{provider} — OAuth callback (query params, used by Google/GitHub).
pub async fn callback_handler(
    State(state): State<Arc<GatewayState>>,
    Path(provider_name): Path<String>,
    Query(params): Query<CallbackParams>,
) -> Response {
    handle_callback(state, provider_name, params).await
}

/// POST /auth/callback/{provider} — OAuth callback (form post, used by Apple Sign In).
pub async fn callback_post_handler(
    State(state): State<Arc<GatewayState>>,
    Path(provider_name): Path<String>,
    axum::Form(params): axum::Form<CallbackParams>,
) -> Response {
    handle_callback(state, provider_name, params).await
}

/// Shared callback logic for both GET (query) and POST (form) callbacks.
async fn handle_callback(
    state: Arc<GatewayState>,
    provider_name: String,
    params: CallbackParams,
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
    let mut profile = match provider
        .exchange_code(code, &callback_url, &flow.code_verifier)
        .await
    {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, provider = %provider_name, "OAuth code exchange failed");
            return error_page("Failed to complete login. Please try again.");
        }
    };

    // Apple sends the user's name only on the FIRST authorization via the
    // `user` form field. Merge it into the profile if present.
    if profile.display_name.is_none()
        && let Some(ref user_json) = params.user
        && let Ok(user) = serde_json::from_str::<serde_json::Value>(user_json)
    {
        let first = user
            .get("name")
            .and_then(|n| n.get("firstName"))
            .and_then(|v| v.as_str());
        let last = user
            .get("name")
            .and_then(|n| n.get("lastName"))
            .and_then(|v| v.as_str());
        profile.display_name = match (first, last) {
            (Some(f), Some(l)) => Some(format!("{f} {l}")),
            (Some(f), None) => Some(f.to_string()),
            (None, Some(l)) => Some(l.to_string()),
            _ => None,
        };
    }

    // Validate email domain restriction.
    if !state.oauth_allowed_domains.is_empty()
        && let Err(msg) = check_email_domain(profile.email.as_deref(), &state.oauth_allowed_domains)
    {
        tracing::warn!(
            provider = %provider_name,
            email = ?profile.email,
            "OAuth login rejected by domain restriction"
        );
        return error_page(&msg);
    }

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

    let expires_at = Some(chrono::Utc::now() + chrono::Duration::seconds(SESSION_LIFETIME_SECS));
    if let Err(e) = store
        .create_api_token(&user_id, &token_name, &token_hash, token_prefix, expires_at)
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

/// POST /auth/logout — revoke session token and clear cookie.
pub async fn logout_handler(
    State(state): State<Arc<GatewayState>>,
    headers: axum::http::HeaderMap,
) -> Response {
    // Try to revoke the API token backing this session.
    if let Some(token) = extract_session_cookie(&headers)
        && let Some(ref store) = state.store
    {
        let token_hash = crate::channels::web::auth::hash_token(&token);
        if let Ok(Some((record, _user))) = store.authenticate_token(&token_hash).await {
            let _ = store.revoke_api_token(record.id, &_user.id).await;
            if let Some(ref db_auth) = state.db_auth {
                db_auth.invalidate_user(&_user.id).await;
            }
        }
    }

    let secure = state
        .oauth_base_url
        .as_deref()
        .map(is_secure)
        .unwrap_or(false);
    let cookie = build_session_cookie_clear(secure);
    let mut response = (StatusCode::OK, "Logged out").into_response();
    if let Ok(hv) = HeaderValue::from_str(&cookie) {
        response.headers_mut().insert(header::SET_COOKIE, hv);
    }
    response
}

// ── NEAR wallet auth ─────────────────────────────────────────────────────

/// GET /auth/near/challenge — generate a nonce for NEAR wallet signing.
pub async fn near_challenge_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if !state.oauth_rate_limiter.check() {
        return Err((StatusCode::TOO_MANY_REQUESTS, "Rate limited".to_string()));
    }

    let nonce_store = state.near_nonce_store.as_ref().ok_or((
        StatusCode::NOT_FOUND,
        "NEAR authentication is not enabled".to_string(),
    ))?;

    let nonce = nonce_store.generate().await;

    Ok(Json(serde_json::json!({
        "nonce": nonce,
        "message": "Sign in to IronClaw",
        "recipient": "ironclaw",
    })))
}

/// Request body for NEAR wallet verification.
#[derive(serde::Deserialize)]
pub struct NearVerifyRequest {
    pub account_id: String,
    pub public_key: String,
    pub signature: String,
    pub nonce: String,
}

/// POST /auth/near/verify — verify NEAR wallet signature and issue session.
pub async fn near_verify_handler(
    State(state): State<Arc<GatewayState>>,
    Json(body): Json<NearVerifyRequest>,
) -> Response {
    if !state.oauth_rate_limiter.check() {
        return (StatusCode::TOO_MANY_REQUESTS, "Rate limited").into_response();
    }

    let nonce_store = match state.near_nonce_store.as_ref() {
        Some(s) => s,
        None => return (StatusCode::NOT_FOUND, "NEAR auth not enabled").into_response(),
    };

    let near_rpc_url = match state.near_rpc_url.as_deref() {
        Some(u) => u,
        None => {
            return (StatusCode::SERVICE_UNAVAILABLE, "NEAR RPC not configured").into_response();
        }
    };

    let store = match state.store.as_ref() {
        Some(s) => s,
        None => return (StatusCode::SERVICE_UNAVAILABLE, "Database not available").into_response(),
    };

    // Validate nonce (single-use, TTL-checked).
    if !nonce_store.consume(&body.nonce).await {
        return (StatusCode::BAD_REQUEST, "Invalid or expired nonce").into_response();
    }

    // Validate input lengths to prevent abuse.
    if body.account_id.len() > 64 || body.public_key.len() > 128 || body.signature.len() > 256 {
        return (StatusCode::BAD_REQUEST, "Invalid input").into_response();
    }

    // Decode the public key and signature from base58/hex.
    // NEAR public keys are formatted as "ed25519:base58encoded".
    let pub_key_bytes: [u8; 32] = match decode_near_public_key(&body.public_key) {
        Ok(b) => b,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };

    let sig_bytes: [u8; 64] = match decode_near_signature(&body.signature) {
        Ok(b) => b,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };

    // The signed message is the nonce bytes (the client signs the hex nonce string).
    let message = body.nonce.as_bytes();

    // Verify the Ed25519 signature.
    if let Err(e) =
        crate::channels::web::oauth::near::verify_signature(&pub_key_bytes, &sig_bytes, message)
    {
        tracing::warn!(account_id = %body.account_id, error = %e, "NEAR signature verification failed");
        return (StatusCode::UNAUTHORIZED, "Invalid signature").into_response();
    }

    // Verify the public key is an active access key on the NEAR account.
    let http = reqwest::Client::new();
    if let Err(e) = crate::channels::web::oauth::near::verify_access_key(
        near_rpc_url,
        &body.account_id,
        &body.public_key,
        &http,
    )
    .await
    {
        tracing::warn!(
            account_id = %body.account_id,
            public_key = %body.public_key,
            error = %e,
            "NEAR access key verification failed"
        );
        return (
            StatusCode::UNAUTHORIZED,
            "Access key not valid for this account",
        )
            .into_response();
    }

    // Domain restriction check (account_id is like "user.near" — check domain part).
    if !state.oauth_allowed_domains.is_empty() {
        // NEAR account_id acts as the "email" for domain checks.
        // e.g., "alice.company.near" — check if "company.near" is allowed.
        let domain = body
            .account_id
            .rsplit_once('.')
            .map(|(_, tld)| {
                // Get the last two parts: e.g., "company.near" from "alice.company.near"
                let parts: Vec<&str> = body.account_id.rsplitn(3, '.').collect();
                if parts.len() >= 2 {
                    format!("{}.{}", parts[1], parts[0])
                } else {
                    tld.to_string()
                }
            })
            .unwrap_or_default();
        if !state
            .oauth_allowed_domains
            .iter()
            .any(|d| d.eq_ignore_ascii_case(&domain))
        {
            // If no domain match, also try the full account_id suffix
            let account_lower = body.account_id.to_ascii_lowercase();
            if !state
                .oauth_allowed_domains
                .iter()
                .any(|d| account_lower.ends_with(d))
            {
                return error_page(
                    "Your NEAR account is not authorized. Contact your administrator.",
                );
            }
        }
    }

    // Use the OAuth user resolution pipeline.
    let profile = crate::channels::web::oauth::OAuthUserProfile {
        provider_user_id: body.account_id.clone(),
        email: None,
        email_verified: false,
        display_name: Some(body.account_id.clone()),
        avatar_url: None,
        raw: serde_json::json!({
            "account_id": body.account_id,
            "public_key": body.public_key,
        }),
    };

    let (user_id, is_new) = match resolve_user(store.as_ref(), "near", &profile).await {
        Ok(result) => result,
        Err(e) => {
            tracing::error!(error = %e, "NEAR user resolution failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to create user account",
            )
                .into_response();
        }
    };

    if let Err(e) = store.record_login(&user_id).await {
        tracing::warn!(error = %e, user_id = %user_id, "Failed to record login");
    }

    // Issue API token.
    let mut token_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut token_bytes);
    let plaintext_token = hex::encode(token_bytes);
    let token_hash = crate::channels::web::auth::hash_token(&plaintext_token);
    let token_prefix = &plaintext_token[..8];

    let token_name = if is_new {
        "near-wallet-initial".to_string()
    } else {
        "near-wallet-login".to_string()
    };

    let expires_at = Some(chrono::Utc::now() + chrono::Duration::seconds(SESSION_LIFETIME_SECS));
    if let Err(e) = store
        .create_api_token(&user_id, &token_name, &token_hash, token_prefix, expires_at)
        .await
    {
        tracing::error!(error = %e, "Failed to create API token for NEAR login");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to create session",
        )
            .into_response();
    }

    if let Some(ref db_auth) = state.db_auth {
        db_auth.invalidate_user(&user_id).await;
    }

    // Return the token as JSON (the frontend sets the cookie/session).
    Json(serde_json::json!({
        "token": plaintext_token,
        "user_id": user_id,
        "account_id": body.account_id,
        "is_new": is_new,
    }))
    .into_response()
}

/// Decode a NEAR public key (format: "ed25519:base58encoded" or raw hex).
fn decode_near_public_key(key: &str) -> Result<[u8; 32], String> {
    let raw = key.strip_prefix("ed25519:").unwrap_or(key);
    // Try base58 first (NEAR standard), then hex.
    if let Ok(bytes) = bs58::decode(raw).into_vec()
        && bytes.len() == 32
    {
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        return Ok(arr);
    }
    if let Ok(bytes) = hex::decode(raw)
        && bytes.len() == 32
    {
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        return Ok(arr);
    }
    Err("Invalid public key format".to_string())
}

/// Decode a NEAR signature (base58 or hex, 64 bytes).
fn decode_near_signature(sig: &str) -> Result<[u8; 64], String> {
    if let Ok(bytes) = bs58::decode(sig).into_vec()
        && bytes.len() == 64
    {
        let mut arr = [0u8; 64];
        arr.copy_from_slice(&bytes);
        return Ok(arr);
    }
    if let Ok(bytes) = hex::decode(sig)
        && bytes.len() == 64
    {
        let mut arr = [0u8; 64];
        arr.copy_from_slice(&bytes);
        return Ok(arr);
    }
    Err("Invalid signature format".to_string())
}

/// Extract the session cookie value from request headers.
fn extract_session_cookie(headers: &axum::http::HeaderMap) -> Option<String> {
    let cookie_header = headers.get("cookie")?.to_str().ok()?;
    for pair in cookie_header.split(';') {
        let pair = pair.trim();
        if let Some(value) = pair.strip_prefix(&format!("{SESSION_COOKIE_NAME}=")) {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
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

    store
        .create_user_with_identity(&user, &identity)
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
        "{SESSION_COOKIE_NAME}={token}; HttpOnly; SameSite=Lax; Path=/; Max-Age={SESSION_LIFETIME_SECS}{secure_flag}"
    )
}

fn build_session_cookie_clear(secure: bool) -> String {
    let secure_flag = if secure { "; Secure" } else { "" };
    format!("{SESSION_COOKIE_NAME}=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0{secure_flag}")
}

fn is_secure(base_url: &str) -> bool {
    base_url.starts_with("https://")
}

/// Check that the email belongs to one of the allowed domains.
///
/// Used by both OAuth callback and OIDC middleware to enforce domain
/// restrictions.
pub(crate) fn check_email_domain(
    email: Option<&str>,
    allowed_domains: &[String],
) -> Result<(), String> {
    let email = email.ok_or_else(|| {
        "Login requires an email address, but your account does not have one.".to_string()
    })?;
    let domain = email
        .rsplit_once('@')
        .map(|(_, d)| d.to_ascii_lowercase())
        .unwrap_or_default();
    if allowed_domains
        .iter()
        .any(|d| d.eq_ignore_ascii_case(&domain))
    {
        Ok(())
    } else {
        Err(format!(
            "Your email domain '{domain}' is not authorized. \
             Contact your administrator for access."
        ))
    }
}

fn error_page(message: &str) -> Response {
    let escaped = message
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;");
    axum::response::Html(format!(
        "<html><body style='font-family: system-ui; text-align: center; padding: 60px;'>\
         <h2>Login Failed</h2>\
         <p>{escaped}</p>\
         <p><a href='/'>Return to home</a></p>\
         </body></html>"
    ))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn domains(ds: &[&str]) -> Vec<String> {
        ds.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_check_email_domain_allows_matching() {
        let allowed = domains(&["company.com", "partner.org"]);
        assert!(check_email_domain(Some("alice@company.com"), &allowed).is_ok());
        assert!(check_email_domain(Some("bob@partner.org"), &allowed).is_ok());
    }

    #[test]
    fn test_check_email_domain_rejects_non_matching() {
        let allowed = domains(&["company.com"]);
        assert!(check_email_domain(Some("alice@gmail.com"), &allowed).is_err());
    }

    #[test]
    fn test_check_email_domain_case_insensitive() {
        let allowed = domains(&["company.com"]);
        assert!(check_email_domain(Some("alice@COMPANY.COM"), &allowed).is_ok());
        assert!(check_email_domain(Some("alice@Company.Com"), &allowed).is_ok());
    }

    #[test]
    fn test_check_email_domain_rejects_missing_email() {
        let allowed = domains(&["company.com"]);
        assert!(check_email_domain(None, &allowed).is_err());
    }

    #[test]
    fn test_check_email_domain_rejects_malformed_email() {
        let allowed = domains(&["company.com"]);
        assert!(check_email_domain(Some("no-at-sign"), &allowed).is_err());
    }
}
