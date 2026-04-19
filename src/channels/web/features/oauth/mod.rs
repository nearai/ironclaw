//! OAuth + channel-relay callback endpoints.
//!
//! This feature slice owns the three public gateway routes that receive
//! browser redirects or webhook callbacks for OAuth-style flows:
//!
//! - [`oauth_callback_handler`] — generic OAuth callback for installable
//!   extensions. Looks up the pending flow by CSRF state, exchanges the
//!   authorization code for tokens, persists them via the secrets store,
//!   and optionally auto-activates the extension.
//! - [`relay_events_handler`] — HMAC-signed webhook from `channel-relay`
//!   that delivers inbound events to the relay channel.
//! - [`slack_relay_oauth_callback_handler`] — Slack-specific completion
//!   flow that consumes a nonce, stores the `team_id`, and activates
//!   the relay channel.
//!
//! All three are PUBLIC routes — none require the bearer token. The
//! first two are registered under the `public` router in
//! `platform::router`; the Slack callback goes through the same router.
//!
//! The helpers below (`oauth_error_page`, `redact_oauth_state_for_logs`)
//! are slice-local and must not be called from outside this module.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use sha2::{Digest, Sha256};

use crate::channels::relay::DEFAULT_RELAY_NAME;
use crate::channels::web::platform::state::{GatewayState, rate_limit_key_from_headers};
use crate::channels::web::server::clear_auth_mode;
use crate::channels::web::types::AppEvent;
use crate::channels::web::util::web_incoming_message;
use crate::extensions::naming::extension_name_candidates;
use crate::secrets::SecretConsumeResult;

/// Render the CSRF / opaque-error landing page.
///
/// Kept as a thin wrapper over [`crate::auth::oauth::landing_html`] so the
/// callback handlers never leak internal error text to the browser —
/// every failure path lands on the same generic "Authorization Failed"
/// page, and the real reason goes to the tracing log.
fn oauth_error_page(label: &str) -> axum::response::Response {
    let html = crate::auth::oauth::landing_html(label, false);
    axum::response::Html(html).into_response()
}

/// Produce a log-safe fingerprint of an OAuth `state` parameter.
///
/// The raw `state` is a one-time CSRF token linked to an in-flight flow.
/// Logging it verbatim would (a) leak the token to anyone with log access
/// during the flow's validity window, and (b) inflate cardinality in
/// structured log sinks. The returned string is the first 6 bytes of the
/// SHA-256 digest plus the original length — enough to correlate repeated
/// callbacks for the same token in a single log stream, but not enough
/// to recover the token itself.
fn redact_oauth_state_for_logs(state: &str) -> String {
    let digest = Sha256::digest(state.as_bytes());
    let mut short_hash = String::with_capacity(12);
    for byte in &digest[..6] {
        use std::fmt::Write as _;
        let _ = write!(&mut short_hash, "{byte:02x}");
    }
    format!("sha256:{short_hash}:len={}", state.len())
}

/// OAuth callback handler for the web gateway.
///
/// This is a PUBLIC route (no Bearer token required) because OAuth providers
/// redirect the user's browser here. The `state` query parameter correlates
/// the callback with a pending OAuth flow registered by `start_wasm_oauth()`.
///
/// Used on hosted instances where `IRONCLAW_OAUTH_CALLBACK_URL` points to
/// the gateway (e.g., `https://kind-deer.agent1.near.ai/oauth/callback`).
/// Local/desktop mode continues to use the TCP listener on port 9876.
pub(crate) async fn oauth_callback_handler(
    State(state): State<Arc<GatewayState>>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    use crate::auth::oauth;

    // Check for error from OAuth provider (e.g., user denied consent)
    if let Some(error) = params.get("error") {
        let description = params
            .get("error_description")
            .cloned()
            .unwrap_or_else(|| error.clone());
        return oauth_error_page(&description);
    }

    let state_param = match params.get("state") {
        Some(s) if !s.is_empty() => s.clone(),
        _ => {
            return oauth_error_page("IronClaw");
        }
    };

    let code = match params.get("code") {
        Some(c) if !c.is_empty() => c.clone(),
        _ => {
            return oauth_error_page("IronClaw");
        }
    };

    // Look up the pending flow by CSRF state (atomic remove prevents replay)
    let ext_mgr = match state.extension_manager.as_ref() {
        Some(mgr) => mgr,
        None => {
            return oauth_error_page("IronClaw");
        }
    };

    let decoded_state = match oauth::decode_hosted_oauth_state(&state_param) {
        Ok(decoded) => decoded,
        Err(error) => {
            let redacted_state = redact_oauth_state_for_logs(&state_param);
            tracing::warn!(
                state = %redacted_state,
                error = %error,
                "OAuth callback received with malformed state"
            );
            clear_auth_mode(&state, &state.owner_id).await;
            return oauth_error_page("IronClaw");
        }
    };
    let lookup_key = decoded_state.flow_id.clone();

    let flow = ext_mgr
        .pending_oauth_flows()
        .write()
        .await
        .remove(&lookup_key);

    let flow = match flow {
        Some(f) => f,
        None => {
            let redacted_state = redact_oauth_state_for_logs(&state_param);
            let redacted_lookup_key = redact_oauth_state_for_logs(&lookup_key);
            tracing::warn!(
                state = %redacted_state,
                lookup_key = %redacted_lookup_key,
                "OAuth callback received with unknown or expired state"
            );
            return oauth_error_page("IronClaw");
        }
    };

    // Check flow expiry (5 minutes, matching TCP listener timeout)
    if flow.created_at.elapsed() > oauth::OAUTH_FLOW_EXPIRY {
        tracing::warn!(
            extension = %flow.extension_name,
            "OAuth flow expired"
        );
        // Notify UI so auth card can show error instead of staying stuck
        if let Some(ref sse) = flow.sse_manager {
            sse.broadcast_for_user(
                &flow.user_id,
                AppEvent::OnboardingState {
                    extension_name: flow.extension_name.clone(),
                    state: crate::channels::web::types::OnboardingStateDto::Failed,
                    request_id: None,
                    message: Some("OAuth flow expired. Please try again.".to_string()),
                    instructions: None,
                    auth_url: None,
                    setup_url: None,
                    onboarding: None,
                    thread_id: None,
                },
            );
        }
        clear_auth_mode(&state, &flow.user_id).await;
        return oauth_error_page(&flow.display_name);
    }

    // Exchange the authorization code for tokens.
    // Use the platform exchange proxy when configured, otherwise call the
    // provider's token URL directly.
    let exchange_proxy_url = oauth::exchange_proxy_url();

    let result: Result<(), String> = async {
        let token_response = if let Some(proxy_url) = &exchange_proxy_url {
            let oauth_proxy_auth_token = flow.oauth_proxy_auth_token().unwrap_or_default();
            oauth::exchange_via_proxy(oauth::ProxyTokenExchangeRequest {
                proxy_url,
                gateway_token: oauth_proxy_auth_token,
                token_url: &flow.token_url,
                client_id: &flow.client_id,
                client_secret: flow.client_secret.as_deref(),
                code: &code,
                redirect_uri: &flow.redirect_uri,
                code_verifier: flow.code_verifier.as_deref(),
                access_token_field: &flow.access_token_field,
                extra_token_params: &flow.token_exchange_extra_params,
            })
            .await
            .map_err(|e| e.to_string())?
        } else {
            oauth::exchange_oauth_code_with_params(
                &flow.token_url,
                &flow.client_id,
                flow.client_secret.as_deref(),
                &code,
                &flow.redirect_uri,
                flow.code_verifier.as_deref(),
                &flow.access_token_field,
                &flow.token_exchange_extra_params,
            )
            .await
            .map_err(|e| e.to_string())?
        };

        // Validate the token before storing (catches wrong account, etc.)
        if let Some(ref validation) = flow.validation_endpoint {
            oauth::validate_oauth_token(&token_response.access_token, validation)
                .await
                .map_err(|e| e.to_string())?;
        }

        // Store tokens encrypted in the secrets store
        oauth::store_oauth_tokens(
            flow.secrets.as_ref(),
            &flow.user_id,
            &flow.secret_name,
            flow.provider.as_deref(),
            &token_response.access_token,
            token_response.refresh_token.as_deref(),
            token_response.expires_in,
            &flow.scopes,
        )
        .await
        .map_err(|e| e.to_string())?;

        // Persist the client_id for flows that need it after the session ends
        // (for example DCR-based MCP refresh).
        if let Some(ref client_id_secret) = flow.client_id_secret_name {
            let params = crate::secrets::CreateSecretParams::new(client_id_secret, &flow.client_id)
                .with_provider(flow.provider.as_ref().cloned().unwrap_or_default());
            flow.secrets
                .create(&flow.user_id, params)
                .await
                .map_err(|e| {
                    tracing::warn!(
                        extension = %flow.extension_name,
                        secret_name = %client_id_secret,
                        error = %e,
                        "Failed to store OAuth client_id secret after callback"
                    );
                    "failed to store client credentials".to_string()
                })?;
        }

        if let (Some(client_secret_name), Some(client_secret)) = (
            flow.client_secret_secret_name.as_ref(),
            flow.client_secret.as_deref(),
        ) {
            let mut params =
                crate::secrets::CreateSecretParams::new(client_secret_name, client_secret)
                    .with_provider(flow.provider.as_ref().cloned().unwrap_or_default());
            if let Some(expires_at) = flow.client_secret_expires_at
                && let Some(dt) =
                    chrono::DateTime::<chrono::Utc>::from_timestamp(expires_at as i64, 0)
            {
                params = params.with_expiry(dt);
            }
            flow.secrets
                .create(&flow.user_id, params)
                .await
                .map_err(|e| {
                    tracing::warn!(
                        extension = %flow.extension_name,
                        secret_name = %client_secret_name,
                        error = %e,
                        "Failed to store OAuth client_secret secret after callback"
                    );
                    "failed to store client credentials".to_string()
                })?;
        }

        Ok(())
    }
    .await;

    let (success, message) = match &result {
        Ok(()) => (
            true,
            format!("{} authenticated successfully", flow.display_name),
        ),
        Err(e) => (
            false,
            format!("{} authentication failed: {}", flow.display_name, e),
        ),
    };

    match &result {
        Ok(()) => {
            tracing::info!(
                extension = %flow.extension_name,
                "OAuth completed successfully via gateway callback"
            );
        }
        Err(e) => {
            tracing::warn!(
                extension = %flow.extension_name,
                error = %e,
                "OAuth failed via gateway callback"
            );
        }
    }

    // Clear auth mode regardless of outcome so the next user message goes
    // through to the LLM instead of being intercepted as a token.
    clear_auth_mode(&state, &flow.user_id).await;

    // After successful OAuth, auto-activate the extension so it moves
    // from "Installed (Authenticate)" → "Active" without a second click.
    // OAuth success is independent of activation — tokens are already stored.
    // Report auth as successful and attempt activation as a bonus step.
    let final_message = if success && flow.auto_activate_extension {
        match ext_mgr
            .ensure_extension_ready(
                &flow.extension_name,
                &flow.user_id,
                crate::extensions::EnsureReadyIntent::ExplicitActivate,
            )
            .await
        {
            Ok(crate::extensions::EnsureReadyOutcome::Ready { activation, .. }) => activation
                .map(|result| result.message)
                .unwrap_or_else(|| format!("{} authenticated successfully", flow.display_name)),
            Ok(crate::extensions::EnsureReadyOutcome::NeedsAuth { auth, .. }) => auth
                .instructions()
                .map(String::from)
                .unwrap_or_else(|| format!("{} authenticated successfully", flow.display_name)),
            Ok(crate::extensions::EnsureReadyOutcome::NeedsSetup { instructions, .. }) => {
                instructions
            }
            Err(e) => {
                tracing::warn!(
                    extension = %flow.extension_name,
                    error = %e,
                    "Auto-activation after OAuth failed"
                );
                format!(
                    "{} authenticated successfully. Activation failed: {}. Try activating manually.",
                    flow.display_name, e
                )
            }
        }
    } else if success {
        format!("{} authenticated successfully", flow.display_name)
    } else {
        message
    };

    // Broadcast event to notify the web UI
    let extension_name = flow.extension_name.clone();
    if let Some(ref sse) = flow.sse_manager {
        sse.broadcast_for_user(
            &flow.user_id,
            AppEvent::OnboardingState {
                extension_name: flow.extension_name,
                state: if success {
                    crate::channels::web::types::OnboardingStateDto::Ready
                } else {
                    crate::channels::web::types::OnboardingStateDto::Failed
                },
                request_id: None,
                message: Some(final_message.clone()),
                instructions: None,
                auth_url: None,
                setup_url: None,
                onboarding: None,
                thread_id: None,
            },
        );
    }

    if success {
        match crate::bridge::resolve_engine_auth_callback(&flow.user_id, &flow.secret_name).await {
            Ok(crate::bridge::AuthCallbackContinuation::ResolveGateExternal {
                channel,
                thread_scope,
                request_id,
            }) => {
                if let Some(tx) = state.msg_tx.read().await.as_ref().cloned() {
                    let callback =
                        crate::agent::submission::Submission::ExternalCallback { request_id };
                    match serde_json::to_string(&callback) {
                        Ok(content) => {
                            let msg = web_incoming_message(
                                &channel,
                                &flow.user_id,
                                content,
                                thread_scope.as_deref(),
                            );
                            if let Err(e) = tx.send(msg).await {
                                tracing::warn!(
                                    extension = %extension_name,
                                    user_id = %flow.user_id,
                                    error = %e,
                                    "Failed to resolve pending engine auth gate after OAuth callback"
                                );
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                extension = %extension_name,
                                user_id = %flow.user_id,
                                error = %e,
                                "Failed to serialize external callback submission"
                            );
                        }
                    }
                }
            }
            Ok(crate::bridge::AuthCallbackContinuation::ReplayMessage {
                channel,
                thread_scope,
                content,
            }) => {
                if let Some(tx) = state.msg_tx.read().await.as_ref().cloned() {
                    let msg = web_incoming_message(
                        &channel,
                        &flow.user_id,
                        content,
                        thread_scope.as_deref(),
                    );
                    if let Err(e) = tx.send(msg).await {
                        tracing::warn!(
                            extension = %extension_name,
                            user_id = %flow.user_id,
                            error = %e,
                            "Failed to replay pending engine auth request after OAuth callback"
                        );
                    }
                }
            }
            Ok(crate::bridge::AuthCallbackContinuation::None) => {}
            Err(e) => {
                tracing::warn!(
                    extension = %extension_name,
                    user_id = %flow.user_id,
                    error = %e,
                    "Failed to resume pending engine auth gate after OAuth callback"
                );
            }
        }
    }

    let html = oauth::landing_html(&flow.display_name, success);
    axum::response::Html(html).into_response()
}

/// Webhook endpoint for receiving relay events from channel-relay.
///
/// PUBLIC route — authenticated via HMAC signature (X-Relay-Signature header).
pub(crate) async fn relay_events_handler(
    State(state): State<Arc<GatewayState>>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let ext_mgr = match state.extension_manager.as_ref() {
        Some(mgr) => mgr,
        None => {
            return (StatusCode::SERVICE_UNAVAILABLE, "not ready").into_response();
        }
    };

    let signing_secret = match ext_mgr.relay_signing_secret() {
        Some(s) => s,
        None => {
            return (StatusCode::SERVICE_UNAVAILABLE, "relay not configured").into_response();
        }
    };

    // Verify signature
    let signature = match headers
        .get("x-relay-signature")
        .and_then(|v| v.to_str().ok())
    {
        Some(s) => s.to_string(),
        None => {
            return (StatusCode::UNAUTHORIZED, "missing signature").into_response();
        }
    };

    let timestamp = match headers
        .get("x-relay-timestamp")
        .and_then(|v| v.to_str().ok())
    {
        Some(t) => t.to_string(),
        None => {
            return (StatusCode::UNAUTHORIZED, "missing timestamp").into_response();
        }
    };

    // Check timestamp freshness (5 min window)
    let ts: i64 = match timestamp.parse() {
        Ok(t) => t,
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "malformed timestamp").into_response();
        }
    };
    let now = chrono::Utc::now().timestamp();
    if (now - ts).abs() > 300 {
        return (StatusCode::UNAUTHORIZED, "stale timestamp").into_response();
    }

    // Verify HMAC: sha256(secret, timestamp + "." + body)
    if !crate::channels::relay::webhook::verify_relay_signature(
        &signing_secret,
        &timestamp,
        &body,
        &signature,
    ) {
        return (StatusCode::UNAUTHORIZED, "invalid signature").into_response();
    }

    // Parse event
    let event: crate::channels::relay::client::ChannelEvent = match serde_json::from_slice(&body) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(error = %e, "relay callback invalid JSON");
            return (StatusCode::BAD_REQUEST, "invalid JSON").into_response();
        }
    };

    // Push to relay channel
    let event_tx_guard = ext_mgr.relay_event_tx();
    let event_tx = event_tx_guard.lock().await;
    match event_tx.as_ref() {
        Some(tx) => {
            if let Err(e) = tx.try_send(event) {
                tracing::warn!(error = %e, "relay event channel full or closed");
                return (StatusCode::SERVICE_UNAVAILABLE, "event queue full").into_response();
            }
        }
        None => {
            return (StatusCode::SERVICE_UNAVAILABLE, "relay channel not active").into_response();
        }
    }

    Json(serde_json::json!({"ok": true})).into_response()
}

/// OAuth callback for Slack via channel-relay.
///
/// This is a PUBLIC route (no Bearer token required) because channel-relay
/// redirects the user's browser here after Slack OAuth completes.
/// Query params: `provider`, `team_id`.
pub(crate) async fn slack_relay_oauth_callback_handler(
    State(state): State<Arc<GatewayState>>,
    headers: HeaderMap,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    // Rate limit
    let ip = rate_limit_key_from_headers(&headers);
    if !state.oauth_rate_limiter.check(&ip) {
        return axum::response::Html(
            "<html><body style='font-family: system-ui; text-align: center; padding: 60px;'>\
             <h2>Too Many Requests</h2>\
             <p>Please try again later.</p>\
             </body></html>"
                .to_string(),
        )
        .into_response();
    }

    // Validate team_id format: empty or T followed by alphanumeric (max 20 chars)
    let team_id = params.get("team_id").cloned().unwrap_or_default();
    if !team_id.is_empty() {
        let valid_team_id = team_id.len() <= 21
            && team_id.starts_with('T')
            && team_id[1..].chars().all(|c| c.is_ascii_alphanumeric());
        if !valid_team_id {
            return axum::response::Html(
                "<html><body style='font-family: system-ui; text-align: center; padding: 60px;'>\
                 <h2>Error</h2><p>Invalid callback parameters.</p></body></html>"
                    .to_string(),
            )
            .into_response();
        }
    }

    // Validate provider: must be "slack" (only supported provider)
    let provider = params
        .get("provider")
        .cloned()
        .unwrap_or_else(|| "slack".into());
    if provider != "slack" {
        return axum::response::Html(
            "<html><body style='font-family: system-ui; text-align: center; padding: 60px;'>\
             <h2>Error</h2><p>Invalid callback parameters.</p></body></html>"
                .to_string(),
        )
        .into_response();
    }

    let ext_mgr = match state.extension_manager.as_ref() {
        Some(mgr) => mgr,
        None => {
            return axum::response::Html(
                "<html><body style='font-family: system-ui; text-align: center; padding: 60px;'>\
                 <h2>Error</h2><p>Extension manager not available.</p></body></html>"
                    .to_string(),
            )
            .into_response();
        }
    };

    // Validate CSRF state parameter
    let state_param = match params.get("state") {
        Some(s) if !s.is_empty() && s.len() <= 128 => s.clone(),
        _ => {
            return axum::response::Html(
                "<html><body style='font-family: system-ui; text-align: center; padding: 60px;'>\
                 <h2>Error</h2><p>Invalid or expired authorization.</p></body></html>"
                    .to_string(),
            )
            .into_response();
        }
    };

    let relay_names = extension_name_candidates(DEFAULT_RELAY_NAME);
    let relay_extension_name = relay_names[0].clone();
    let mut nonce_consumed = false;
    let mut last_lookup_error = None;
    for relay_name in &relay_names {
        let state_key = format!("relay:{relay_name}:oauth_state");
        match ext_mgr
            .secrets()
            .consume_if_matches(&state.owner_id, &state_key, &state_param)
            .await
        {
            Ok(SecretConsumeResult::Matched) => {
                nonce_consumed = true;
                break;
            }
            Ok(SecretConsumeResult::Mismatched) => {
                return axum::response::Html(
                    "<html><body style='font-family: system-ui; text-align: center; padding: 60px;'>\
                     <h2>Error</h2><p>Invalid or expired authorization.</p></body></html>"
                        .to_string(),
                )
                .into_response();
            }
            Ok(SecretConsumeResult::NotFound) => {}
            Err(e) => {
                last_lookup_error = Some((state_key, e.to_string()));
            }
        }
    }
    if !nonce_consumed {
        let attempted_state_keys = relay_names
            .iter()
            .map(|relay_name| format!("relay:{relay_name}:oauth_state"))
            .collect::<Vec<_>>();
        let (state_key, error) = match last_lookup_error {
            Some((state_key, error)) => (Some(state_key), error),
            None => (
                None,
                "stored nonce not found under any relay state key".to_string(),
            ),
        };
        tracing::warn!(
            owner_id = %state.owner_id,
            state_key = ?state_key,
            attempted_state_keys = ?attempted_state_keys,
            state = %redact_oauth_state_for_logs(&state_param),
            error = %error,
            "relay OAuth callback: failed to retrieve stored nonce"
        );
        return axum::response::Html(
            "<html><body style='font-family: system-ui; text-align: center; padding: 60px;'>\
             <h2>Error</h2><p>Invalid or expired authorization.</p></body></html>"
                .to_string(),
        )
        .into_response();
    }

    let result: Result<(), String> = async {
        let store = state.store.as_ref().ok_or_else(|| {
            "Relay activation requires persistent settings storage; no-db mode is unsupported."
                .to_string()
        })?;

        // Store team_id in settings
        let team_id_key = format!("relay:{}:team_id", relay_extension_name);
        tracing::info!(
            relay = %relay_extension_name,
            owner_id = %state.owner_id,
            team_id_key = %team_id_key,
            "relay OAuth callback: storing team_id in settings"
        );
        store
            .set_setting(&state.owner_id, &team_id_key, &serde_json::json!(team_id))
            .await
            .map_err(|e| {
                tracing::error!(
                    relay = %relay_extension_name,
                    owner_id = %state.owner_id,
                    error = %e,
                    "relay OAuth callback: failed to persist team_id to settings store"
                );
                format!("Failed to persist relay team_id: {e}")
            })?;

        // Activate the relay channel
        tracing::info!(
            relay = %relay_extension_name,
            owner_id = %state.owner_id,
            "relay OAuth callback: activating relay channel"
        );
        ext_mgr
            .activate_stored_relay(&relay_extension_name, &state.owner_id)
            .await
            .map_err(|e| format!("Failed to activate relay channel: {}", e))?;

        Ok(())
    }
    .await;

    let (success, message) = match &result {
        Ok(()) => (true, "Slack connected successfully!".to_string()),
        Err(e) => {
            tracing::error!(error = %e, "Slack relay OAuth callback failed");
            (
                false,
                "Connection failed. Check server logs for details.".to_string(),
            )
        }
    };

    // Broadcast event to notify the web UI.
    state.sse.broadcast(AppEvent::OnboardingState {
        extension_name: relay_extension_name.clone(),
        state: if success {
            crate::channels::web::types::OnboardingStateDto::Ready
        } else {
            crate::channels::web::types::OnboardingStateDto::Failed
        },
        request_id: None,
        message: Some(message.clone()),
        instructions: None,
        auth_url: None,
        setup_url: None,
        onboarding: None,
        thread_id: None,
    });

    if success {
        axum::response::Html(
            "<html><body style='font-family: system-ui; text-align: center; padding: 60px;'>\
             <h2>Slack Connected!</h2>\
             <p>You can close this tab and return to IronClaw.</p>\
             <script>window.close()</script>\
             </body></html>"
                .to_string(),
        )
        .into_response()
    } else {
        axum::response::Html(format!(
            "<html><body style='font-family: system-ui; text-align: center; padding: 60px;'>\
             <h2>Connection Failed</h2>\
             <p>{}</p>\
             </body></html>",
            message
        ))
        .into_response()
    }
}
