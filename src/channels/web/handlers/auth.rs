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
use base64::Engine;
use rand::RngCore;
use rand::rngs::OsRng;
use uuid::Uuid;

use crate::channels::web::oauth::state_store::{OAuthStateStore, new_oauth_flow};
use crate::channels::web::server::GatewayState;
use crate::db::{UserIdentityRecord, UserRecord};

use crate::channels::web::auth::SESSION_COOKIE_NAME;
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
    let mut resp = serde_json::json!({ "providers": providers });
    if let Some(ref network) = state.near_network {
        resp["near_network"] = serde_json::json!(network);
    }
    Json(resp)
}

/// GET /auth/login/{provider} — initiate OAuth flow (redirect to provider).
pub async fn login_handler(
    State(state): State<Arc<GatewayState>>,
    Path(provider_name): Path<String>,
    headers: axum::http::HeaderMap,
    Query(params): Query<LoginParams>,
) -> Result<Response, (StatusCode, String)> {
    if !state.oauth_rate_limiter.check(&rate_limit_key(&headers)) {
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
    headers: axum::http::HeaderMap,
    Query(params): Query<CallbackParams>,
) -> Response {
    handle_callback(state, provider_name, params, &headers).await
}

/// POST /auth/callback/{provider} — OAuth callback (form post, used by Apple Sign In).
pub async fn callback_post_handler(
    State(state): State<Arc<GatewayState>>,
    Path(provider_name): Path<String>,
    headers: axum::http::HeaderMap,
    axum::Form(params): axum::Form<CallbackParams>,
) -> Response {
    handle_callback(state, provider_name, params, &headers).await
}

/// Shared callback logic for both GET (query) and POST (form) callbacks.
async fn handle_callback(
    state: Arc<GatewayState>,
    provider_name: String,
    params: CallbackParams,
    headers: &axum::http::HeaderMap,
) -> Response {
    if !state.oauth_rate_limiter.check(&rate_limit_key(headers)) {
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

    // Validate email domain restriction. Only trust verified emails — an
    // unverified email could be set to any value by the user.
    if !state.oauth_allowed_domains.is_empty() {
        if !profile.email_verified {
            tracing::warn!(
                provider = %provider_name,
                email = ?profile.email,
                "OAuth login rejected: domain restriction requires a verified email"
            );
            return error_page(
                "Login requires a verified email address from an authorized domain.",
            );
        }
        if let Err(msg) = check_email_domain(profile.email.as_deref(), &state.oauth_allowed_domains)
        {
            tracing::warn!(
                provider = %provider_name,
                email = ?profile.email,
                "OAuth login rejected by domain restriction"
            );
            return error_page(&msg);
        }
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
    // Use 303 See Other (not 307 Temporary) so POST callbacks (Apple form_post)
    // are converted to GET on redirect, preventing the browser from re-POSTing.
    let cookie_value = build_session_cookie(&plaintext_token, is_secure(base_url));
    let mut response = Redirect::to(redirect_to).into_response();
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
        if let Ok(Some((record, user))) = store.authenticate_token(&token_hash).await {
            let _ = store.revoke_api_token(record.id, &user.id).await;
            if let Some(ref db_auth) = state.db_auth {
                db_auth.invalidate_user(&user.id).await;
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
    headers: axum::http::HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if !state.oauth_rate_limiter.check(&rate_limit_key(&headers)) {
        return Err((StatusCode::TOO_MANY_REQUESTS, "Rate limited".to_string()));
    }

    let nonce_store = state.near_nonce_store.as_ref().ok_or((
        StatusCode::NOT_FOUND,
        "NEAR authentication is not enabled".to_string(),
    ))?;

    let nonce = nonce_store.generate().await;
    let message = format!("Sign in to IronClaw\nNonce: {nonce}");

    Ok(Json(serde_json::json!({
        "nonce": nonce,
        "message": message,
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
    headers: axum::http::HeaderMap,
    Json(body): Json<NearVerifyRequest>,
) -> Response {
    if !state.oauth_rate_limiter.check(&rate_limit_key(&headers)) {
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

    tracing::debug!(
        account_id = %body.account_id,
        public_key = %body.public_key,
        signature_len = body.signature.len(),
        signature_prefix = &body.signature[..body.signature.len().min(20)],
        "NEAR verify: decoding credentials"
    );

    // Decode the public key and signature.
    // NEAR wallets may return base58, hex, or base64.
    let pub_key_bytes: [u8; 32] = match decode_near_public_key(&body.public_key) {
        Ok(b) => b,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };

    let sig_bytes: [u8; 64] = match decode_near_signature(&body.signature) {
        Ok(b) => b,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };

    // Build the exact NEP-413 payload the wallet signed. The nonce in the
    // challenge is hex-encoded; convert back to 32 raw bytes for NEP-413.
    let nonce_bytes: [u8; 32] = match hex::decode(&body.nonce) {
        Ok(b) if b.len() == 32 => {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&b);
            arr
        }
        _ => return (StatusCode::BAD_REQUEST, "Invalid nonce format").into_response(),
    };
    let message_str = format!("Sign in to IronClaw\nNonce: {}", body.nonce);

    // Try multiple payload formats — different wallets implement NEP-413 differently.
    if let Err(e) = crate::channels::web::oauth::near::verify_near_signature(
        &pub_key_bytes,
        &sig_bytes,
        &message_str,
        &nonce_bytes,
        "ironclaw",
    ) {
        tracing::warn!(
            account_id = %body.account_id,
            error = %e,
            "NEAR signature verification failed (all payload formats)"
        );
        return (StatusCode::UNAUTHORIZED, "Invalid signature").into_response();
    }

    // Normalize the public key to NEAR's canonical format for the RPC call.
    let canonical_pubkey = format!("ed25519:{}", bs58::encode(&pub_key_bytes).into_string());

    // Verify the public key is an active access key on the NEAR account.
    static NEAR_HTTP: std::sync::LazyLock<reqwest::Client> =
        std::sync::LazyLock::new(reqwest::Client::new);
    if let Err(e) = crate::channels::web::oauth::near::verify_access_key(
        near_rpc_url,
        &body.account_id,
        &canonical_pubkey,
        &NEAR_HTTP,
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

    // Domain restriction check for NEAR account IDs.
    // Allowed domains match as a suffix with a dot boundary:
    //   "company.near" allows "alice.company.near" but NOT "evilcompany.near".
    //   "near" allows "alice.near" (exact TLD match).
    if !state.oauth_allowed_domains.is_empty() {
        let account = body.account_id.to_ascii_lowercase();
        let allowed = state.oauth_allowed_domains.iter().any(|d| {
            let d_lower = d.to_ascii_lowercase();
            account == d_lower || account.ends_with(&format!(".{d_lower}"))
        });
        if !allowed {
            return (
                StatusCode::FORBIDDEN,
                "Your NEAR account is not authorized. Contact your administrator.",
            )
                .into_response();
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

    // Set session cookie (consistent with OAuth flow) and return user info.
    let base_url = state
        .oauth_base_url
        .as_deref()
        .unwrap_or("http://localhost");
    let cookie_value = build_session_cookie(&plaintext_token, is_secure(base_url));
    let mut response = Json(serde_json::json!({
        "user_id": user_id,
        "account_id": body.account_id,
        "is_new": is_new,
    }))
    .into_response();
    if let Ok(hv) = HeaderValue::from_str(&cookie_value) {
        response.headers_mut().insert(header::SET_COOKIE, hv);
    }
    response
}

/// Truncate a string safely for error messages (no byte-index panic on multibyte).
fn safe_truncate(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

/// Decode a NEAR public key. NEAR keys always use the `ed25519:` prefix with
/// base58 encoding. We enforce this format to avoid ambiguity with base64.
fn decode_near_public_key(key: &str) -> Result<[u8; 32], String> {
    let raw = key.strip_prefix("ed25519:").ok_or_else(|| {
        format!(
            "Expected ed25519: prefix, got: {}...",
            safe_truncate(key, 20)
        )
    })?;
    let bytes = bs58::decode(raw)
        .into_vec()
        .map_err(|e| format!("Invalid base58 in public key: {e}"))?;
    if bytes.len() != 32 {
        return Err(format!(
            "Public key decoded to {} bytes, expected 32",
            bytes.len()
        ));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(arr)
}

/// Decode a NEAR signature. Wallets return signatures as base64 (HOT, Meteor)
/// or base58 (MyNearWallet). We try base64 first (most common from near-connect),
/// then base58.
fn decode_near_signature(sig: &str) -> Result<[u8; 64], String> {
    // Try base64 standard first (most wallets via near-connect), then URL-safe,
    // then base58 (MyNearWallet). Each must decode to exactly 64 bytes.
    let candidates = [
        base64::engine::general_purpose::STANDARD.decode(sig).ok(),
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(sig)
            .ok(),
        bs58::decode(sig).into_vec().ok(),
    ];
    for bytes in candidates.into_iter().flatten() {
        if bytes.len() == 64 {
            let mut arr = [0u8; 64];
            arr.copy_from_slice(&bytes);
            return Ok(arr);
        }
    }
    Err(format!(
        "Invalid signature (expected 64 bytes base64/base58, got: {}...)",
        safe_truncate(sig, 20)
    ))
}

/// Extract the session cookie value from request headers.
fn extract_session_cookie(headers: &axum::http::HeaderMap) -> Option<String> {
    crate::channels::web::auth::extract_cookie_value(headers, SESSION_COOKIE_NAME)
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
            // Update profile fields that may have changed at the provider.
            let _ = store
                .update_identity_profile(
                    provider,
                    &profile.provider_user_id,
                    profile.display_name.as_deref(),
                    profile.avatar_url.as_deref(),
                )
                .await;
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
            // Verify the linked user is still active before linking.
            if let Some(user) = store
                .get_user(&identity.user_id)
                .await
                .map_err(|e| e.to_string())?
                && user.status == "active"
            {
                let new_identity = build_identity_record(&identity.user_id, provider, profile);
                store
                    .create_identity(&new_identity)
                    .await
                    .map_err(|e| e.to_string())?;
                return Ok((identity.user_id, false));
            }
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
    // Role is always "member" here — create_user_with_identity atomically
    // promotes to "admin" inside the DB transaction if this is the sole user,
    // preventing the TOCTOU race where two concurrent first logins both get admin.
    let role = "member";

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
        email: profile.email.as_ref().map(|e| e.to_ascii_lowercase()),
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

/// Extract a rate-limit key from request headers (X-Forwarded-For or fallback).
fn rate_limit_key(headers: &axum::http::HeaderMap) -> String {
    crate::channels::web::server::rate_limit_key_from_headers(headers)
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
    use axum::http::HeaderValue;

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

    #[test]
    fn test_extract_session_cookie_handles_quoted_neighbors() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            HeaderValue::from_static("other=\"quoted;value\"; ironclaw_session=abc123"),
        );

        assert_eq!(extract_session_cookie(&headers), Some("abc123".to_string()));
    }

    #[test]
    fn test_rate_limit_key_ignores_invalid_forwarded_for_values() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("garbage, 203.0.113.9, more-garbage"),
        );

        assert_eq!(rate_limit_key(&headers), "203.0.113.9");
    }
}
