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
use crate::channels::web::platform::legacy_auth::{
    clear_auth_mode, clear_session_auth_mode_for_thread,
};
use crate::channels::web::platform::state::{GatewayState, rate_limit_key_from_headers};
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

/// Failure category for the OAuth callback handler. The text is embedded in
/// `tracing::warn!` so a user-reported correlation ID can be mapped to the
/// actual cause without having to re-enable verbose tracing on the gateway.
#[derive(Copy, Clone)]
enum OauthCallbackFailure {
    /// `?error=...` returned by the provider (e.g. user denied consent).
    ProviderError,
    /// `state` param is missing or empty.
    MissingState,
    /// `code` param is missing or empty.
    MissingCode,
    /// `state` couldn't be decoded — likely a tampered or stale token.
    MalformedState,
    /// No matching pending flow found for the decoded state.
    UnknownState,
    /// Pending flow exceeded `OAUTH_FLOW_EXPIRY` before the callback fired.
    Expired,
    /// Token exchange (provider or proxy) returned a non-2xx response.
    Exchange,
    /// `extension_manager` is unavailable on the gateway state.
    NoExtensionManager,
}

impl OauthCallbackFailure {
    fn as_str(self) -> &'static str {
        match self {
            Self::ProviderError => "provider_error",
            Self::MissingState => "missing_state",
            Self::MissingCode => "missing_code",
            Self::MalformedState => "malformed_state",
            Self::UnknownState => "unknown_state",
            Self::Expired => "expired",
            Self::Exchange => "exchange_failed",
            Self::NoExtensionManager => "no_extension_manager",
        }
    }
}

/// Generate a short correlation ID from an arbitrary seed plus the current
/// monotonic-ish timestamp. Emitted on every `tracing::warn!` call in the
/// OAuth callback failure paths so an operator can map a timestamp /
/// category / correlation cluster to a single log line.
///
/// The seed is whatever caller-side string best identifies the failure —
/// in practice the raw `state` query parameter (when present) or the
/// `flow.extension_name` for post-state-resolution failures. The seed is
/// hashed (SHA-256) before any hex output, so it is never logged or
/// exposed verbatim through the returned ID; callers that want the seed
/// itself in logs must pass it separately through
/// [`redact_oauth_state_for_logs`].
///
/// Currently logs-only: the user-facing landing page rendered by
/// [`oauth_error_page`] uses [`crate::auth::oauth::landing_html`], which has
/// a fixed failure subtitle and does not embed the correlation. Plumbing
/// the ID into the HTML is a follow-up — see `landing_html` for the
/// fixed-subtitle wiring.
///
/// 8 hex characters — collision-resistant within a single tracing window
/// (operators grep recent logs), short enough to read aloud over phone.
fn oauth_failure_correlation_id(seed: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut hasher = Sha256::new();
    hasher.update(seed.as_bytes());
    hasher.update(nanos.to_le_bytes());
    let digest = hasher.finalize();
    let mut s = String::with_capacity(8);
    for byte in digest.iter().take(4) {
        use std::fmt::Write as _;
        let _ = write!(&mut s, "{byte:02x}");
    }
    s
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
    for byte in digest.iter().take(6) {
        use std::fmt::Write as _;
        let _ = write!(&mut short_hash, "{byte:02x}");
    }
    format!("sha256:{short_hash}:len={}", state.len())
}

/// Resolve the IronClaw user who initiated a Slack relay OAuth flow.
///
/// `auth_channel_relay` (in `extensions/manager.rs`) stores the
/// initiating user_id under `relay:{name}:oauth_user`, namespaced by
/// the *gateway owner* secret-store user (`owner_id`). The callback
/// handler reads it back here so the completion toast can be
/// broadcast to the actual tenant who started the flow rather than
/// the gateway owner — in multi-tenant deployments those differ, and
/// broadcasting to `owner_id` would deliver the success/failure
/// toast to the wrong browser tab.
///
/// Falls back to `owner_id` when the secret is missing
/// (single-tenant deployments or flows started before the field was
/// stored). When `multi_tenant_mode` is true, the fallback emits a
/// WARN: in a multi-tenant deployment a missing initiator secret
/// means we have no way to recover the original tenant, and quietly
/// routing the toast to `owner_id` reintroduces the same misroute
/// the rest of this PR closes. The completion still proceeds —
/// the `oauth_user` value is also used to seed the pairing identity,
/// and aborting the callback would leave a dangling Slack install —
/// but the WARN surfaces the lost-initiator case for operators.
///
/// Returns the resolved user_id as a `String` so the caller owns the
/// value past the secret store's lifetime.
///
/// This helper is the canonical broadcast-target resolver. The
/// pairing-identity creation path also uses it so the toast and the
/// new identity always agree on which user just completed the flow.
async fn resolve_relay_oauth_user(
    secrets: &(dyn crate::secrets::SecretsStore + Send + Sync),
    owner_id: &str,
    extension_name: &str,
    multi_tenant_mode: bool,
) -> String {
    let user_key = format!("relay:{extension_name}:oauth_user");
    match secrets.get_decrypted(owner_id, &user_key).await.ok() {
        Some(s) => s.expose().to_string(),
        None => {
            if multi_tenant_mode {
                tracing::warn!(
                    extension = %extension_name,
                    owner_id = %owner_id,
                    "relay OAuth callback: missing `relay:{{ext}}:oauth_user` secret in \
                     multi-tenant mode — falling back to gateway owner_id for the \
                     completion broadcast. The original initiator is unrecoverable \
                     (likely a flow started before the field was stored, or a \
                     premature secret delete). Toast may reach the wrong tab."
                );
            }
            owner_id.to_string()
        }
    }
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

    // Check for error from OAuth provider (e.g., user denied consent).
    //
    // The pending flow was inserted into `ext_mgr.pending_oauth_flows()` when
    // the user started the OAuth dance. On provider error we must remove it
    // here — otherwise it lingers until `OAUTH_FLOW_EXPIRY` (5 min) and any
    // subsequent auth attempt for the same (extension, user) pair has to
    // dedupe against a ghost entry.
    //
    // We also auto-cancel the engine pending auth gate for the same user so
    // the user's conversation doesn't sit blocked waiting for an OAuth
    // resume that will never arrive (#3320 — the user reported being unable
    // to continue the conversation after a failed Gmail OAuth from
    // Telegram, even after `/clear`).
    if let Some(error) = params.get("error") {
        let description = params
            .get("error_description")
            .cloned()
            .unwrap_or_else(|| error.clone());
        let state_seed = params
            .get("state")
            .cloned()
            .unwrap_or_else(|| "no-state".to_string());
        let correlation = oauth_failure_correlation_id(&state_seed);

        // Pull the full flow out so we can mirror the failure handling of
        // the exchange-failure and expiry paths: SSE `OnboardingState::Failed`
        // for the UI, legacy-v1 session `pending_auth` clear, and
        // credential-scoped engine gate clear. Without all three, an
        // `?error=access_denied` from the provider would leave the auth
        // card spinning and a legacy v1 session intercepting the next
        // user message as a token (Copilot review on PR #3381).
        let removed_flow = if let Some(state_param) = params.get("state")
            && !state_param.is_empty()
            && let Ok(decoded) = oauth::decode_hosted_oauth_state(state_param)
            && let Some(ext_mgr) = state.extension_manager.as_ref()
        {
            ext_mgr
                .pending_oauth_flows()
                .write()
                .await
                .remove(&decoded.flow_id)
        } else {
            None
        };

        tracing::warn!(
            category = OauthCallbackFailure::ProviderError.as_str(),
            correlation = %correlation,
            error = %error,
            description = %description,
            user_id = removed_flow
                .as_ref()
                .map(|f| f.user_id.as_str())
                .unwrap_or("<unknown>"),
            "OAuth callback received provider error"
        );

        if let Some(ref flow) = removed_flow {
            // Notify the UI so the auth card stops spinning and shows the
            // error instead. Same shape as the expiry branch.
            if let Some(ref sse) = flow.sse_manager {
                sse.broadcast_for_user(
                    &flow.user_id,
                    AppEvent::OnboardingState {
                        extension_name: flow.extension_name.clone(),
                        state: crate::channels::web::types::OnboardingStateDto::Failed,
                        request_id: None,
                        message: Some(description.clone()),
                        instructions: None,
                        auth_url: None,
                        setup_url: None,
                        onboarding: None,
                        thread_id: None,
                    },
                );
            }
            // Discard the pending engine auth gate that was waiting on
            // *this* OAuth flow (matched by credential name). Without this
            // the engine sits paused forever (#3320). Scoping by
            // credential keeps the cleanup from nuking unrelated auth
            // gates the user may have open in parallel — see PR review
            // on #3381. The bridge helper takes the credential as `&str`
            // and parses it backend-side, so the web layer keeps its
            // string-typed boundary intact.
            crate::bridge::clear_engine_pending_auth_for_credential(
                &flow.user_id,
                &flow.secret_name,
            )
            .await;
            // Legacy v1 session cleanup: drop `pending_auth` so the next
            // user message goes through to the LLM rather than being
            // intercepted as a token. Use `clear_session_auth_mode_for_thread`
            // (not the broader `clear_auth_mode`) because the latter would
            // re-call the unscoped engine cleanup and undo the credential
            // scoping above.
            let _ = clear_session_auth_mode_for_thread(&state, &flow.user_id, None).await;
        }

        return oauth_error_page(&description);
    }

    let state_param = match params.get("state") {
        Some(s) if !s.is_empty() => s.clone(),
        _ => {
            let correlation = oauth_failure_correlation_id("missing-state");
            tracing::warn!(
                category = OauthCallbackFailure::MissingState.as_str(),
                correlation = %correlation,
                "OAuth callback missing or empty state parameter"
            );
            return oauth_error_page("IronClaw");
        }
    };

    let code = match params.get("code") {
        Some(c) if !c.is_empty() => c.clone(),
        _ => {
            let correlation = oauth_failure_correlation_id(&state_param);
            tracing::warn!(
                category = OauthCallbackFailure::MissingCode.as_str(),
                correlation = %correlation,
                state = %redact_oauth_state_for_logs(&state_param),
                "OAuth callback missing or empty code parameter"
            );
            return oauth_error_page("IronClaw");
        }
    };

    // Look up the pending flow by CSRF state (atomic remove prevents replay)
    let ext_mgr = match state.extension_manager.as_ref() {
        Some(mgr) => mgr,
        None => {
            let correlation = oauth_failure_correlation_id(&state_param);
            tracing::warn!(
                category = OauthCallbackFailure::NoExtensionManager.as_str(),
                correlation = %correlation,
                "OAuth callback fired but extension manager is not configured"
            );
            return oauth_error_page("IronClaw");
        }
    };

    let decoded_state = match oauth::decode_hosted_oauth_state(&state_param) {
        Ok(decoded) => decoded,
        Err(error) => {
            let redacted_state = redact_oauth_state_for_logs(&state_param);
            let correlation = oauth_failure_correlation_id(&state_param);
            tracing::warn!(
                category = OauthCallbackFailure::MalformedState.as_str(),
                correlation = %correlation,
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
            let correlation = oauth_failure_correlation_id(&state_param);
            tracing::warn!(
                category = OauthCallbackFailure::UnknownState.as_str(),
                correlation = %correlation,
                state = %redacted_state,
                lookup_key = %redacted_lookup_key,
                "OAuth callback received with unknown or expired state"
            );
            return oauth_error_page("IronClaw");
        }
    };

    // Check flow expiry (5 minutes, matching TCP listener timeout)
    if flow.created_at.elapsed() > oauth::OAUTH_FLOW_EXPIRY {
        let correlation = oauth_failure_correlation_id(flow.extension_name.as_str());
        tracing::warn!(
            category = OauthCallbackFailure::Expired.as_str(),
            correlation = %correlation,
            extension = %flow.extension_name,
            user_id = %flow.user_id,
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
        // Expiry is a terminal failure path just like provider-error and
        // exchange-failure: discard the engine pending auth gate so the
        // conversation isn't blocked on a callback that will never arrive
        // (#3320). Scoped to this flow's credential so unrelated auth
        // gates the user has open in parallel survive — review feedback
        // on #3381. Use `clear_session_auth_mode_for_thread` for the
        // legacy v1 cleanup; the broader `clear_auth_mode` helper would
        // re-call `clear_engine_pending_auth(user, None)` and undo the
        // credential scoping we just applied.
        crate::bridge::clear_engine_pending_auth_for_credential(&flow.user_id, &flow.secret_name)
            .await;
        let _ = clear_session_auth_mode_for_thread(&state, &flow.user_id, None).await;
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
            // Token exchange or downstream persistence failed. Tag the log
            // line with a correlation ID so a user-reported "I saw 400" can
            // be mapped to this exact failure without re-enabling verbose
            // tracing on the gateway. Then auto-cancel the engine pending
            // auth gate for this user so subsequent messages aren't blocked
            // waiting for an OAuth resume that will never arrive (#3320).
            //
            // Scope the cleanup to the failed flow's credential so an
            // unrelated pending auth gate (e.g. Slack/MCP open in another
            // thread) survives — review feedback on #3381.
            let correlation = oauth_failure_correlation_id(flow.extension_name.as_str());
            tracing::warn!(
                category = OauthCallbackFailure::Exchange.as_str(),
                correlation = %correlation,
                extension = %flow.extension_name,
                user_id = %flow.user_id,
                error = %e,
                "OAuth failed via gateway callback"
            );
            crate::bridge::clear_engine_pending_auth_for_credential(
                &flow.user_id,
                &flow.secret_name,
            )
            .await;
        }
    }

    // Clear legacy session auth mode regardless of outcome so the next
    // user message goes through to the LLM instead of being intercepted
    // as a token.
    //
    // Engine-gate handling is scoped per failure mode and done earlier in
    // each branch — provider-error, expiry, and exchange-failure each
    // call `clear_engine_pending_auth_for_credential` at their own
    // failure site (#3320 + PR #3381 review). The successful-callback
    // path intentionally leaves the gate intact so the `ExternalCallback`
    // resume below can resolve it and replay the paused action
    // (preserving the paused_lease). This is why we use the legacy-v1-only
    // `clear_session_auth_mode_for_thread` here instead of the broader
    // `clear_auth_mode` helper — `clear_auth_mode` would also discard
    // the engine gate on the *success* path and undo the resume.
    let _ = clear_session_auth_mode_for_thread(&state, &flow.user_id, None).await;

    // After successful OAuth, auto-activate the extension so it moves
    // from "Installed (Authenticate)" → "Active" without a second click.
    // OAuth success is independent of activation — tokens are already stored.
    // Report auth as successful and attempt activation as a bonus step.
    let final_message = if success && flow.auto_activate_extension {
        match ext_mgr
            .ensure_extension_ready(
                flow.extension_name.as_str(),
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
        // Half-2 of #3133, two-pronged auto-resume:
        //
        // 1. Wake any Tier 0 / Tier 1 inline-await VMs parked on this
        //    credential. The CodeAct VM (mission's child thread, in
        //    the bug-shape #3133 reported) keeps its full state across
        //    the OAuth round-trip; on Approved it retries the original
        //    action and continues without unwinding.
        // 2. Auto-resume any paused background missions whose
        //    `paused_gate` matches this credential. This handles the
        //    case where the mission's child thread already finished
        //    (Tier 0 unwind, or Tier 1 hit MaxIterations) before
        //    OAuth completed.
        // Both are best-effort — failures are logged inside the bridge
        // helpers and never block the OAuth landing page.
        let inline_woken =
            crate::bridge::resolve_inline_gates_for_credential(&flow.user_id, &flow.secret_name)
                .await;
        let _ =
            crate::bridge::resume_paused_missions_for_credential(&flow.user_id, &flow.secret_name)
                .await;

        // #3533: when the inline-await path already woke a parked
        // waiter, the engine is already retrying the action that was
        // blocked on this credential. Sending an `ExternalCallback`
        // submission down the agent loop in addition to that is
        // redundant and causes "thread already running" races against
        // the in-flight retry (and re-dispatches `tool_install` a
        // second time when the mock LLM pattern-matches the user
        // turn). Skip the external-callback re-entry in that case;
        // it stays in place for paths where no inline waiter exists
        // (mission's child thread already finished, or Tier 0 unwound
        // before OAuth landed).
        let skip_external_callback = inline_woken > 0;

        if !skip_external_callback {
            match crate::bridge::resolve_engine_auth_callback(&flow.user_id, &flow.secret_name)
                .await
            {
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

    // Resolve the IronClaw user who initiated this OAuth flow BEFORE the
    // result block so the completion toast can be routed to that tenant
    // even on the failure path.
    let oauth_user = resolve_relay_oauth_user(
        ext_mgr.secrets().as_ref(),
        &state.owner_id,
        &relay_extension_name,
        state.multi_tenant_mode,
    )
    .await;

    // Delete the temporary `relay:{ext}:oauth_user` secret unconditionally
    // once we've captured its value — regardless of whether the result
    // block below short-circuits, whether `pairing_store` is wired up,
    // or whether identity pairing later fails. Leaving it behind would
    // let a subsequent OAuth callback for the same extension read a
    // stale initiating user and misroute the toast (Copilot review on
    // PR #3390 commit d247bde8). The previous cleanup site inside the
    // `if let Some(pairing_store)` branch was unreachable on three of
    // those failure paths.
    let oauth_user_key = format!("relay:{}:oauth_user", relay_extension_name);
    let _ = ext_mgr
        .secrets()
        .delete(&state.owner_id, &oauth_user_key)
        .await;

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

        // Activate the relay channel first — this creates the relay client and
        // verifies the connection is usable.
        tracing::info!(
            relay = %relay_extension_name,
            owner_id = %state.owner_id,
            "relay OAuth callback: activating relay channel"
        );
        ext_mgr
            .activate_stored_relay(&relay_extension_name, &state.owner_id)
            .await
            .map_err(|e| format!("Failed to activate relay channel: {}", e))?;

        // Create channel identity pairing: Slack authed_user_id → IronClaw user.
        // Fetch authed_user_id from the relay's connections API (server-side,
        // not from the redirect URL which could be tampered).
        if let Some(pairing_store) = ext_mgr.pairing_store() {
            let relay_config = ext_mgr
                .relay_config()
                .map_err(|e| format!("Relay config not available: {e}"))?;
            let effective_url = ext_mgr
                .effective_relay_url(&relay_extension_name)
                .await
                .unwrap_or_else(|| relay_config.url.clone());
            let client = crate::channels::relay::RelayClient::new(
                effective_url,
                relay_config.api_key.clone(),
                relay_config.request_timeout_secs,
            )
            .map_err(|e| format!("Failed to create relay client: {e}"))?;

            let connections = client
                .list_connections("")
                .await
                .map_err(|e| format!("Failed to fetch relay connections: {e}"))?;
            let authed_user_id = connections
                .iter()
                .find(|c| c.team_id == team_id)
                .and_then(|c| c.authed_user_id.clone())
                .ok_or_else(|| {
                    "No connection with authed_user_id found for this team".to_string()
                })?;

            // The outer `oauth_user` (resolved by `resolve_relay_oauth_user`
            // above the result block) is the IronClaw user this OAuth flow
            // belongs to. Reuse it here so the pairing identity and the
            // completion toast always agree on the target tenant. The
            // temporary `relay:{ext}:oauth_user` secret was already
            // deleted unconditionally above the result block, so we
            // never reach this branch with a stale credential lingering.
            let user_record = if let Some(ref db) = state.store {
                db.get_user(&oauth_user).await.ok().flatten()
            } else {
                None
            };
            let Some(ref record) = user_record else {
                return Err(format!(
                    "OAuth user '{oauth_user}' not found — cannot create relay identity"
                ));
            };
            if record.status != "active" {
                return Err(format!(
                    "OAuth user '{oauth_user}' is not active (status: {})",
                    record.status
                ));
            }
            let role = match record.role.as_str() {
                "owner" => crate::ownership::UserRole::Owner,
                "admin" => crate::ownership::UserRole::Admin,
                _ => crate::ownership::UserRole::Regular,
            };
            let Ok(user_id) = crate::ownership::UserId::new(&oauth_user, role) else {
                return Err(format!(
                    "OAuth user '{oauth_user}' has invalid user_id format"
                ));
            };
            // Scope external_id to workspace: "team_id:slack_user_id"
            let scoped_external_id = format!("{}:{}", team_id, authed_user_id);
            pairing_store
                .create_identity(
                    crate::channels::relay::channel::DEFAULT_RELAY_NAME,
                    &scoped_external_id,
                    &user_id,
                )
                .await
                .map_err(|e| format!("Failed to create relay identity: {e}"))?;
        }

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

    // Broadcast event to notify the web UI. Scope to the resolved
    // OAuth flow user (`oauth_user`) rather than `state.owner_id` —
    // in multi-tenant mode a non-owner can complete a Slack OAuth
    // flow, and the completion toast must reach THAT user's open
    // browser tab, not the gateway owner's. Broadcasting to
    // `state.owner_id` was a cross-tenant misroute even after the
    // global-broadcast leak was closed.
    let onboarding_event = AppEvent::OnboardingState {
        extension_name: ironclaw_common::ExtensionName::from_trusted(relay_extension_name.clone()),
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
    };
    state.sse.broadcast_for_user(&oauth_user, onboarding_event); // projection-exempt: channel-lifecycle, slack relay onboarding state

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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{
        Json, Router,
        extract::State,
        http::StatusCode,
        routing::{get, post},
    };

    use crate::auth::oauth;
    use crate::channels::relay::DEFAULT_RELAY_NAME;

    use crate::channels::web::features::oauth::{
        oauth_callback_handler, slack_relay_oauth_callback_handler,
    };

    use crate::channels::web::platform::state::GatewayState;

    use crate::channels::web::sse::SseManager;
    use crate::channels::web::test_helpers::{
        test_ext_mgr, test_gateway_state, test_secrets_store,
    };

    use crate::testing::credentials::TEST_GATEWAY_CRYPTO_KEY;

    /// `auth_channel_relay` stores the initiating user_id under
    /// `relay:{ext}:oauth_user` namespaced by the GATEWAY OWNER's
    /// secret-store user. The callback must read it back from the same
    /// namespace and return it — broadcasting the completion toast to
    /// `state.owner_id` instead would deliver the success/failure
    /// notification to the wrong tenant in multi-tenant deployments.
    #[tokio::test]
    async fn resolve_relay_oauth_user_returns_stored_value() {
        use crate::secrets::CreateSecretParams;

        let secrets = test_secrets_store();
        let owner_id = "gateway-owner";
        let extension_name = DEFAULT_RELAY_NAME;
        let initiating_user = "alice"; // deliberately != owner_id

        secrets
            .create(
                owner_id,
                CreateSecretParams::new(
                    format!("relay:{extension_name}:oauth_user"),
                    initiating_user,
                ),
            )
            .await
            .expect("seed oauth_user secret"); // safety: cfg(test) fixture

        // Multi-tenant flag is irrelevant when the secret IS present —
        // the resolved value comes from the stored initiator regardless.
        let resolved = super::resolve_relay_oauth_user(
            secrets.as_ref(),
            owner_id,
            extension_name,
            true, // multi_tenant_mode
        )
        .await;
        assert_eq!(
            resolved, initiating_user,
            "callback must broadcast to the initiating user, not the gateway owner"
        );
    }

    /// Falls back to `owner_id` when the secret was never written —
    /// covers single-tenant deployments and pre-multi-tenant flows.
    /// In single-tenant mode the fallback is silent: owner == only user,
    /// so routing the toast there is correct, not a misroute.
    #[tokio::test]
    async fn resolve_relay_oauth_user_falls_back_to_owner_id() {
        let secrets = test_secrets_store();
        let owner_id = "single-tenant-owner";

        let resolved = super::resolve_relay_oauth_user(
            secrets.as_ref(),
            owner_id,
            DEFAULT_RELAY_NAME,
            false, // multi_tenant_mode
        )
        .await;
        assert_eq!(
            resolved, owner_id,
            "missing secret must fall back to owner_id, not panic or empty-string"
        );
    }

    /// In multi-tenant mode a missing initiator secret means the
    /// original tenant is unrecoverable. The function still falls back
    /// to `owner_id` so the OAuth flow doesn't strand a half-installed
    /// Slack relay, but the WARN documented in `resolve_relay_oauth_user`
    /// is the operator-visible signal that a toast may have reached the
    /// wrong tab. We can't easily assert on tracing output without
    /// pulling in `tracing-test`, so this test pins the behavioral
    /// contract: even with `multi_tenant_mode=true`, the function does
    /// not panic, return empty-string, or block the callback.
    #[tokio::test]
    async fn resolve_relay_oauth_user_multitenant_fallback_returns_owner_id() {
        let secrets = test_secrets_store();
        let owner_id = "gateway-owner";

        let resolved = super::resolve_relay_oauth_user(
            secrets.as_ref(),
            owner_id,
            DEFAULT_RELAY_NAME,
            true, // multi_tenant_mode
        )
        .await;
        assert_eq!(
            resolved, owner_id,
            "multi-tenant fallback must still return owner_id (and emit the WARN); \
             returning empty would crash the broadcast call site"
        );
    }

    fn test_oauth_router(state: Arc<GatewayState>) -> Router {
        Router::new()
            .route("/oauth/callback", get(oauth_callback_handler))
            .with_state(state)
    }

    #[derive(Clone, Debug)]
    struct RecordedOauthProxyRequest {
        authorization: Option<String>,
        form: std::collections::HashMap<String, String>,
    }

    #[derive(Clone)]
    struct MockOauthProxyState {
        requests: Arc<tokio::sync::Mutex<Vec<RecordedOauthProxyRequest>>>,
    }

    struct MockOauthProxyServer {
        addr: std::net::SocketAddr,
        requests: Arc<tokio::sync::Mutex<Vec<RecordedOauthProxyRequest>>>,
        shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
        server_task: Option<tokio::task::JoinHandle<()>>,
    }

    impl MockOauthProxyServer {
        async fn start() -> Self {
            async fn exchange_handler(
                State(state): State<MockOauthProxyState>,
                headers: axum::http::HeaderMap,
                axum::Form(form): axum::Form<std::collections::HashMap<String, String>>,
            ) -> Json<serde_json::Value> {
                state.requests.lock().await.push(RecordedOauthProxyRequest {
                    authorization: headers
                        .get(axum::http::header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_string),
                    form,
                });
                Json(serde_json::json!({
                    "access_token": "proxy-access-token",
                    "refresh_token": "proxy-refresh-token",
                    "expires_in": 7200
                }))
            }

            let requests = Arc::new(tokio::sync::Mutex::new(Vec::new()));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind mock oauth proxy");
            let addr = listener.local_addr().expect("mock oauth proxy addr");
            let app = Router::new()
                .route("/oauth/exchange", post(exchange_handler))
                .with_state(MockOauthProxyState {
                    requests: Arc::clone(&requests),
                });
            let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
            let server_task = tokio::spawn(async move {
                let _ = axum::serve(listener, app)
                    .with_graceful_shutdown(async {
                        let _ = shutdown_rx.await;
                    })
                    .await;
            });

            Self {
                addr,
                requests,
                shutdown_tx: Some(shutdown_tx),
                server_task: Some(server_task),
            }
        }

        fn base_url(&self) -> String {
            format!("http://{}", self.addr)
        }

        async fn requests(&self) -> Vec<RecordedOauthProxyRequest> {
            self.requests.lock().await.clone()
        }

        async fn shutdown(mut self) {
            if let Some(tx) = self.shutdown_tx.take() {
                let _ = tx.send(());
            }
            if let Some(task) = self.server_task.take() {
                let _ = task.await;
            }
        }
    }

    impl Drop for MockOauthProxyServer {
        fn drop(&mut self) {
            if let Some(tx) = self.shutdown_tx.take() {
                let _ = tx.send(());
            }
            if let Some(task) = self.server_task.take() {
                task.abort();
            }
        }
    }

    struct EnvVarGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            // SAFETY: Tests use lock_env() to serialize environment access.
            unsafe {
                if let Some(ref value) = self.original {
                    std::env::set_var(self.key, value);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }
    }

    fn set_env_var(key: &'static str, value: Option<&str>) -> EnvVarGuard {
        let original = std::env::var(key).ok();
        // SAFETY: Tests use lock_env() to serialize environment access.
        unsafe {
            if let Some(value) = value {
                std::env::set_var(key, value);
            } else {
                std::env::remove_var(key);
            }
        }
        EnvVarGuard { key, original }
    }

    fn fresh_pending_oauth_flow(
        secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync>,
        sse_manager: Option<Arc<SseManager>>,
        oauth_proxy_auth_token: Option<String>,
    ) -> crate::auth::oauth::PendingOAuthFlow {
        crate::auth::oauth::PendingOAuthFlow {
            extension_name: ironclaw_common::ExtensionName::new("test_tool").unwrap(),
            display_name: "Test Tool".to_string(),
            token_url: "https://example.com/token".to_string(),
            client_id: "client123".to_string(),
            client_secret: None,
            redirect_uri: "https://example.com/oauth/callback".to_string(),
            code_verifier: Some("test-code-verifier".to_string()),
            access_token_field: "access_token".to_string(),
            secret_name: "test_token".to_string(),
            provider: Some("google".to_string()),
            validation_endpoint: None,
            scopes: vec!["email".to_string()],
            user_id: "test".to_string(),
            secrets,
            sse_manager,
            gateway_token: oauth_proxy_auth_token,
            token_exchange_extra_params: std::collections::HashMap::new(),
            client_id_secret_name: None,
            client_secret_secret_name: None,
            client_secret_expires_at: None,
            created_at: std::time::Instant::now(),
            auto_activate_extension: true,
        }
    }

    fn expired_flow_created_at() -> std::time::Instant {
        // Panics on systems whose monotonic clock started less than
        // `OAUTH_FLOW_EXPIRY + 1s` ago. That only happens on a just-booted
        // CI host, and on that path we want a loud failure rather than a
        // silent test skip that quietly loses coverage of the expired-flow
        // branch — see the Gemini review on PR #2706.
        std::time::Instant::now()
            .checked_sub(oauth::OAUTH_FLOW_EXPIRY + std::time::Duration::from_secs(1))
            .expect("monotonic clock must have run long enough for expired_flow_created_at") // safety: cfg(test) fixture
    }

    #[tokio::test]
    async fn test_oauth_callback_missing_params() {
        use axum::body::Body;
        use tower::ServiceExt;

        let state = test_gateway_state(None);
        let app = test_oauth_router(state);

        let req = axum::http::Request::builder()
            .uri("/oauth/callback")
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        assert!(html.contains("Authorization Failed"));
    }

    #[tokio::test]
    async fn test_oauth_callback_error_from_provider() {
        use axum::body::Body;
        use tower::ServiceExt;

        let state = test_gateway_state(None);
        let app = test_oauth_router(state);

        let req = axum::http::Request::builder()
            .uri("/oauth/callback?error=access_denied&error_description=access_denied")
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        assert!(html.contains("Authorization Failed"));
    }

    /// Regression test for #3320: when the OAuth provider returns an error,
    /// the gateway must remove the pending flow associated with the inbound
    /// `state` so a subsequent auth attempt isn't blocked by a ghost entry
    /// (the entry would otherwise live until `OAUTH_FLOW_EXPIRY`).
    ///
    /// The previous code path did `.remove(&decoded.flow_id)` but threw
    /// the value away, so we never knew the flow's `user_id` and could not
    /// auto-cancel the matching engine pending auth gate. After the fix
    /// we capture the user_id; the engine-gate side of the cleanup is
    /// covered by integration tests since it requires bringing up the
    /// engine state.
    #[tokio::test]
    async fn test_oauth_callback_provider_error_drains_pending_flow() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync> =
            Arc::new(crate::secrets::InMemorySecretsStore::new(Arc::new(
                crate::secrets::SecretsCrypto::new(secrecy::SecretString::from(
                    TEST_GATEWAY_CRYPTO_KEY.to_string(),
                ))
                .expect("crypto"),
            )));
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets.clone());

        // Insert a pending flow keyed by a known `flow_id`. Encode an
        // OAuth `state` value that decodes to this flow_id so the
        // error-branch cleanup actually exercises the lookup, not the
        // outer "no state param" path.
        let flow_id = "drain-me-flow-id";
        let flow = fresh_pending_oauth_flow(secrets.clone(), None, None);
        ext_mgr
            .pending_oauth_flows()
            .write()
            .await
            .insert(flow_id.to_string(), flow);

        let state_param = crate::auth::oauth::encode_hosted_oauth_state(flow_id, None);

        let state = test_gateway_state(Some(Arc::clone(&ext_mgr)));
        let app = test_oauth_router(Arc::clone(&state));

        let req = axum::http::Request::builder()
            .uri(format!(
                "/oauth/callback?error=access_denied&error_description=user_denied&state={state_param}"
            ))
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        // Pending flow must be drained on provider error so a stale
        // `state` can't satisfy a later attempt.
        let flows = ext_mgr.pending_oauth_flows().read().await;
        assert!(
            !flows.contains_key(flow_id),
            "pending OAuth flow should be drained on provider error"
        );
    }

    /// Regression for Copilot review on PR #3381: the provider-error
    /// branch must mirror the exchange-failure / expiry branches —
    /// broadcast `OnboardingState::Failed` so the UI auth card stops
    /// spinning, not just drain the pending flow and return an error
    /// page. Without the SSE emit the legacy v1 web UI gets no signal
    /// that the OAuth dance has terminated.
    #[tokio::test]
    async fn test_oauth_callback_provider_error_broadcasts_onboarding_failed() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets = test_secrets_store();
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(Arc::clone(&secrets));
        let sse_mgr = Arc::new(SseManager::new());
        let mut receiver = sse_mgr.sender().subscribe();
        let flow = fresh_pending_oauth_flow(Arc::clone(&secrets), Some(Arc::clone(&sse_mgr)), None);
        let flow_id = "provider-error-broadcast-flow";
        ext_mgr
            .pending_oauth_flows()
            .write()
            .await
            .insert(flow_id.to_string(), flow);

        let state_param = crate::auth::oauth::encode_hosted_oauth_state(flow_id, None);
        let state = test_gateway_state(Some(Arc::clone(&ext_mgr)));
        let app = test_oauth_router(state);

        let req = axum::http::Request::builder()
            .uri(format!(
                "/oauth/callback?error=access_denied&error_description=user_denied&state={}",
                urlencoding::encode(&state_param)
            ))
            .body(Body::empty())
            .expect("request");
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        match receiver.recv().await.expect("onboarding_state event").event {
            crate::channels::web::types::AppEvent::OnboardingState {
                extension_name,
                state,
                message,
                ..
            } => {
                assert_eq!(extension_name, "test_tool");
                assert_eq!(
                    state,
                    crate::channels::web::types::OnboardingStateDto::Failed
                );
                assert_eq!(message.as_deref(), Some("user_denied"));
            }
            event => panic!("expected OnboardingState event, got {event:?}"),
        }
    }

    #[tokio::test]
    async fn test_oauth_callback_unknown_state() {
        use axum::body::Body;
        use tower::ServiceExt;

        // Build an ExtensionManager so the handler can look up flows
        let secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync> =
            Arc::new(crate::secrets::InMemorySecretsStore::new(Arc::new(
                crate::secrets::SecretsCrypto::new(secrecy::SecretString::from(
                    TEST_GATEWAY_CRYPTO_KEY.to_string(),
                ))
                .expect("crypto"),
            )));
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets.clone());

        let state = test_gateway_state(Some(ext_mgr));
        let app = test_oauth_router(state);

        let req = axum::http::Request::builder()
            .uri("/oauth/callback?code=test_code&state=unknown_state_value")
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        assert!(html.contains("Authorization Failed"));
    }

    #[tokio::test]
    async fn test_oauth_callback_expired_flow() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync> =
            Arc::new(crate::secrets::InMemorySecretsStore::new(Arc::new(
                crate::secrets::SecretsCrypto::new(secrecy::SecretString::from(
                    TEST_GATEWAY_CRYPTO_KEY.to_string(),
                ))
                .expect("crypto"),
            )));
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets.clone());
        let created_at = expired_flow_created_at();

        // Insert an expired flow.
        let flow = crate::auth::oauth::PendingOAuthFlow {
            extension_name: ironclaw_common::ExtensionName::new("test_tool").unwrap(),
            display_name: "Test Tool".to_string(),
            token_url: "https://example.com/token".to_string(),
            client_id: "client123".to_string(),
            client_secret: None,
            redirect_uri: "https://example.com/oauth/callback".to_string(),
            code_verifier: None,
            access_token_field: "access_token".to_string(),
            secret_name: "test_token".to_string(),
            provider: None,
            validation_endpoint: None,
            scopes: vec![],
            user_id: "test".to_string(),
            secrets,
            sse_manager: None,
            gateway_token: None,
            token_exchange_extra_params: std::collections::HashMap::new(),
            client_id_secret_name: None,
            client_secret_secret_name: None,
            client_secret_expires_at: None,
            created_at,
            auto_activate_extension: true,
        };

        ext_mgr
            .pending_oauth_flows()
            .write()
            .await
            .insert("expired_state".to_string(), flow);

        let state = test_gateway_state(Some(ext_mgr));
        let app = test_oauth_router(state);

        let req = axum::http::Request::builder()
            .uri("/oauth/callback?code=test_code&state=expired_state")
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        // Expired flow → error landing page
        assert!(html.contains("Authorization Failed"));
    }

    #[tokio::test]
    async fn test_oauth_callback_expired_flow_broadcasts_auth_completed_failure() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync> =
            Arc::new(crate::secrets::InMemorySecretsStore::new(Arc::new(
                crate::secrets::SecretsCrypto::new(secrecy::SecretString::from(
                    TEST_GATEWAY_CRYPTO_KEY.to_string(),
                ))
                .expect("crypto"),
            )));
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets.clone());

        let sse_mgr = Arc::new(SseManager::new());
        let mut receiver = sse_mgr.sender().subscribe();
        let created_at = expired_flow_created_at();
        let flow = crate::auth::oauth::PendingOAuthFlow {
            extension_name: ironclaw_common::ExtensionName::new("test_tool").unwrap(),
            display_name: "Test Tool".to_string(),
            token_url: "https://example.com/token".to_string(),
            client_id: "client123".to_string(),
            client_secret: None,
            redirect_uri: "https://example.com/oauth/callback".to_string(),
            code_verifier: None,
            access_token_field: "access_token".to_string(),
            secret_name: "test_token".to_string(),
            provider: None,
            validation_endpoint: None,
            scopes: vec![],
            user_id: "test".to_string(),
            secrets,
            sse_manager: Some(sse_mgr),
            gateway_token: None,
            token_exchange_extra_params: std::collections::HashMap::new(),
            client_id_secret_name: None,
            client_secret_secret_name: None,
            client_secret_expires_at: None,
            created_at,
            auto_activate_extension: true,
        };

        ext_mgr
            .pending_oauth_flows()
            .write()
            .await
            .insert("expired_state".to_string(), flow);

        let state = test_gateway_state(Some(ext_mgr));
        let app = test_oauth_router(state);

        let req = axum::http::Request::builder()
            .uri("/oauth/callback?code=test_code&state=expired_state")
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        match receiver.recv().await.expect("onboarding_state event").event {
            crate::channels::web::types::AppEvent::OnboardingState {
                extension_name,
                state,
                message,
                ..
            } => {
                assert_eq!(extension_name, "test_tool");
                assert_eq!(
                    state,
                    crate::channels::web::types::OnboardingStateDto::Failed
                );
                assert_eq!(
                    message.as_deref(),
                    Some("OAuth flow expired. Please try again.")
                );
            }
            event => panic!("expected OnboardingState event, got {event:?}"),
        }
    }

    #[tokio::test]
    async fn test_oauth_callback_no_extension_manager() {
        use axum::body::Body;
        use tower::ServiceExt;

        // No extension manager set → graceful error
        let state = test_gateway_state(None);
        let app = test_oauth_router(state);

        let req = axum::http::Request::builder()
            .uri("/oauth/callback?code=test_code&state=some_state")
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        assert!(html.contains("Authorization Failed"));
    }

    #[tokio::test]
    async fn test_oauth_callback_strips_instance_prefix() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync> =
            Arc::new(crate::secrets::InMemorySecretsStore::new(Arc::new(
                crate::secrets::SecretsCrypto::new(secrecy::SecretString::from(
                    TEST_GATEWAY_CRYPTO_KEY.to_string(),
                ))
                .expect("crypto"),
            )));
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets.clone());

        // Insert a flow keyed by raw nonce "test_nonce" (without instance prefix).
        // Use an expired flow so the handler exits before attempting a real HTTP
        // token exchange — we only need to verify that the instance prefix was
        // stripped and the flow was found by the raw nonce.
        let created_at = expired_flow_created_at();
        let flow = crate::auth::oauth::PendingOAuthFlow {
            extension_name: ironclaw_common::ExtensionName::new("test_tool").unwrap(),
            display_name: "Test Tool".to_string(),
            token_url: "https://example.com/token".to_string(),
            client_id: "client123".to_string(),
            client_secret: None,
            redirect_uri: "https://example.com/oauth/callback".to_string(),
            code_verifier: None,
            access_token_field: "access_token".to_string(),
            secret_name: "test_token".to_string(),
            provider: None,
            validation_endpoint: None,
            scopes: vec![],
            user_id: "test".to_string(),
            secrets,
            sse_manager: None,
            gateway_token: None,
            token_exchange_extra_params: std::collections::HashMap::new(),
            client_id_secret_name: None,
            client_secret_secret_name: None,
            client_secret_expires_at: None,
            // Expired — handler will reject after lookup (no network I/O)
            created_at,
            auto_activate_extension: true,
        };

        ext_mgr
            .pending_oauth_flows()
            .write()
            .await
            .insert("test_nonce".to_string(), flow);

        let state = test_gateway_state(Some(ext_mgr.clone()));
        let app = test_oauth_router(state);

        // Send callback with instance prefix: "myinstance:test_nonce"
        // The handler should strip "myinstance:" and find the flow keyed by "test_nonce"
        let req = axum::http::Request::builder()
            .uri("/oauth/callback?code=fake_code&state=myinstance:test_nonce")
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);

        // The flow was found (stripped prefix matched) but is expired, so the
        // handler returns an error landing page. The flow being consumed from
        // the registry (checked below) proves the prefix was stripped correctly.
        assert!(
            html.contains("Authorization Failed"),
            "Expected error page, html was: {}",
            &html[..html.len().min(500)]
        );

        // Verify the flow was consumed (removed from registry)
        assert!(
            ext_mgr
                .pending_oauth_flows()
                .read()
                .await
                .get("test_nonce")
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_oauth_callback_accepts_versioned_hosted_state() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync> =
            Arc::new(crate::secrets::InMemorySecretsStore::new(Arc::new(
                crate::secrets::SecretsCrypto::new(secrecy::SecretString::from(
                    TEST_GATEWAY_CRYPTO_KEY.to_string(),
                ))
                .expect("crypto"),
            )));
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets.clone());

        let created_at = expired_flow_created_at();
        let flow = crate::auth::oauth::PendingOAuthFlow {
            extension_name: ironclaw_common::ExtensionName::new("test_tool").unwrap(),
            display_name: "Test Tool".to_string(),
            token_url: "https://example.com/token".to_string(),
            client_id: "client123".to_string(),
            client_secret: None,
            redirect_uri: "https://example.com/oauth/callback".to_string(),
            code_verifier: None,
            access_token_field: "access_token".to_string(),
            secret_name: "test_token".to_string(),
            provider: None,
            validation_endpoint: None,
            scopes: vec![],
            user_id: "test".to_string(),
            secrets,
            sse_manager: None,
            gateway_token: None,
            token_exchange_extra_params: std::collections::HashMap::new(),
            client_id_secret_name: None,
            client_secret_secret_name: None,
            client_secret_expires_at: None,
            created_at,
            auto_activate_extension: true,
        };

        ext_mgr
            .pending_oauth_flows()
            .write()
            .await
            .insert("test_nonce".to_string(), flow);

        let state = test_gateway_state(Some(ext_mgr.clone()));
        let app = test_oauth_router(state);
        let versioned_state =
            crate::auth::oauth::encode_hosted_oauth_state("test_nonce", Some("myinstance"));

        let req = axum::http::Request::builder()
            .uri(format!(
                "/oauth/callback?code=fake_code&state={}",
                urlencoding::encode(&versioned_state)
            ))
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        assert!(html.contains("Authorization Failed"));
        assert!(
            ext_mgr
                .pending_oauth_flows()
                .read()
                .await
                .get("test_nonce")
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_oauth_callback_accepts_versioned_hosted_state_without_instance_name() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync> =
            Arc::new(crate::secrets::InMemorySecretsStore::new(Arc::new(
                crate::secrets::SecretsCrypto::new(secrecy::SecretString::from(
                    TEST_GATEWAY_CRYPTO_KEY.to_string(),
                ))
                .expect("crypto"),
            )));
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets.clone());

        let created_at = expired_flow_created_at();
        let flow = crate::auth::oauth::PendingOAuthFlow {
            extension_name: ironclaw_common::ExtensionName::new("test_tool").unwrap(),
            display_name: "Test Tool".to_string(),
            token_url: "https://example.com/token".to_string(),
            client_id: "client123".to_string(),
            client_secret: None,
            redirect_uri: "https://example.com/oauth/callback".to_string(),
            code_verifier: None,
            access_token_field: "access_token".to_string(),
            secret_name: "test_token".to_string(),
            provider: None,
            validation_endpoint: None,
            scopes: vec![],
            user_id: "test".to_string(),
            secrets,
            sse_manager: None,
            gateway_token: None,
            token_exchange_extra_params: std::collections::HashMap::new(),
            client_id_secret_name: None,
            client_secret_secret_name: None,
            client_secret_expires_at: None,
            created_at,
            auto_activate_extension: true,
        };

        ext_mgr
            .pending_oauth_flows()
            .write()
            .await
            .insert("test_nonce".to_string(), flow);

        let state = test_gateway_state(Some(ext_mgr.clone()));
        let app = test_oauth_router(state);
        let versioned_state = crate::auth::oauth::encode_hosted_oauth_state("test_nonce", None);

        let req = axum::http::Request::builder()
            .uri(format!(
                "/oauth/callback?code=fake_code&state={}",
                urlencoding::encode(&versioned_state)
            ))
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        assert!(html.contains("Authorization Failed"));
        assert!(
            ext_mgr
                .pending_oauth_flows()
                .read()
                .await
                .get("test_nonce")
                .is_none()
        );
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn test_oauth_callback_happy_path_with_gateway_token_fallback() {
        use axum::body::Body;
        use tower::ServiceExt;

        let proxy = MockOauthProxyServer::start().await;
        // Keep the process-wide env locked for the full callback so the handler
        // sees a stable proxy URL/token configuration throughout the test.
        let _env_guard = crate::config::helpers::lock_env();
        let _exchange_url_guard =
            set_env_var("IRONCLAW_OAUTH_EXCHANGE_URL", Some(&proxy.base_url()));
        let _proxy_auth_guard = set_env_var("IRONCLAW_OAUTH_PROXY_AUTH_TOKEN", None);
        let _gateway_token_guard = set_env_var("GATEWAY_AUTH_TOKEN", Some("gateway-test-token"));

        let secrets = test_secrets_store();
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(Arc::clone(&secrets));
        let sse_mgr = Arc::new(SseManager::new());
        let mut receiver = sse_mgr.sender().subscribe();
        let flow = fresh_pending_oauth_flow(
            Arc::clone(&secrets),
            Some(Arc::clone(&sse_mgr)),
            crate::auth::oauth::oauth_proxy_auth_token(),
        );

        ext_mgr
            .pending_oauth_flows()
            .write()
            .await
            .insert("test_nonce".to_string(), flow);

        let state = test_gateway_state(Some(ext_mgr.clone()));
        let app = test_oauth_router(state);
        let versioned_state =
            crate::auth::oauth::encode_hosted_oauth_state("test_nonce", Some("myinstance"));

        let req = axum::http::Request::builder()
            .uri(format!(
                "/oauth/callback?code=fake_code&state={}",
                urlencoding::encode(&versioned_state)
            ))
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        assert!(html.contains("Test Tool Connected"));

        let requests = proxy.requests().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].authorization.as_deref(),
            Some("Bearer gateway-test-token")
        );
        assert_eq!(
            requests[0].form.get("code").map(String::as_str),
            Some("fake_code")
        );
        assert_eq!(
            requests[0].form.get("code_verifier").map(String::as_str),
            Some("test-code-verifier")
        );

        let access_token = secrets
            .get_decrypted("test", "test_token")
            .await
            .expect("access token stored");
        assert_eq!(access_token.expose(), "proxy-access-token");

        let refresh_token = secrets
            .get_decrypted("test", "test_token_refresh_token")
            .await
            .expect("refresh token stored");
        assert_eq!(refresh_token.expose(), "proxy-refresh-token");

        match receiver.recv().await.expect("onboarding_state event").event {
            crate::channels::web::types::AppEvent::OnboardingState {
                extension_name,
                state,
                ..
            } => {
                assert_eq!(extension_name, "test_tool");
                assert_eq!(
                    state,
                    crate::channels::web::types::OnboardingStateDto::Ready
                );
            }
            event => panic!("expected OnboardingState event, got {event:?}"),
        }

        proxy.shutdown().await;
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn test_oauth_callback_happy_path_with_dedicated_proxy_auth_token() {
        use axum::body::Body;
        use tower::ServiceExt;

        let proxy = MockOauthProxyServer::start().await;
        // Keep the process-wide env locked for the full callback so the handler
        // sees a stable proxy URL/token configuration throughout the test.
        let _env_guard = crate::config::helpers::lock_env();
        let _exchange_url_guard =
            set_env_var("IRONCLAW_OAUTH_EXCHANGE_URL", Some(&proxy.base_url()));
        let _proxy_auth_guard = set_env_var(
            "IRONCLAW_OAUTH_PROXY_AUTH_TOKEN",
            Some("shared-oauth-proxy-secret"),
        );
        let _gateway_token_guard = set_env_var("GATEWAY_AUTH_TOKEN", None);

        let secrets = test_secrets_store();
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(Arc::clone(&secrets));
        let sse_mgr = Arc::new(SseManager::new());
        let mut receiver = sse_mgr.sender().subscribe();
        let flow = fresh_pending_oauth_flow(
            Arc::clone(&secrets),
            Some(Arc::clone(&sse_mgr)),
            crate::auth::oauth::oauth_proxy_auth_token(),
        );

        ext_mgr
            .pending_oauth_flows()
            .write()
            .await
            .insert("test_nonce".to_string(), flow);

        let state = test_gateway_state(Some(ext_mgr.clone()));
        let app = test_oauth_router(state);
        let versioned_state = crate::auth::oauth::encode_hosted_oauth_state("test_nonce", None);

        let req = axum::http::Request::builder()
            .uri(format!(
                "/oauth/callback?code=fake_code&state={}",
                urlencoding::encode(&versioned_state)
            ))
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        assert!(html.contains("Test Tool Connected"));

        let requests = proxy.requests().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].authorization.as_deref(),
            Some("Bearer shared-oauth-proxy-secret")
        );
        assert_eq!(
            requests[0].form.get("code").map(String::as_str),
            Some("fake_code")
        );
        assert_eq!(
            requests[0].form.get("code_verifier").map(String::as_str),
            Some("test-code-verifier")
        );

        let access_token = secrets
            .get_decrypted("test", "test_token")
            .await
            .expect("access token stored");
        assert_eq!(access_token.expose(), "proxy-access-token");

        let refresh_token = secrets
            .get_decrypted("test", "test_token_refresh_token")
            .await
            .expect("refresh token stored");
        assert_eq!(refresh_token.expose(), "proxy-refresh-token");

        match receiver.recv().await.expect("onboarding_state event").event {
            crate::channels::web::types::AppEvent::OnboardingState {
                extension_name,
                state,
                ..
            } => {
                assert_eq!(extension_name, "test_tool");
                assert_eq!(
                    state,
                    crate::channels::web::types::OnboardingStateDto::Ready
                );
            }
            event => panic!("expected OnboardingState event, got {event:?}"),
        }

        proxy.shutdown().await;
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn test_oauth_callback_happy_path_without_auto_activation() {
        use axum::body::Body;
        use tower::ServiceExt;

        let proxy = MockOauthProxyServer::start().await;
        let _env_guard = crate::config::helpers::lock_env();
        let _exchange_url_guard =
            set_env_var("IRONCLAW_OAUTH_EXCHANGE_URL", Some(&proxy.base_url()));
        let _proxy_auth_guard = set_env_var("IRONCLAW_OAUTH_PROXY_AUTH_TOKEN", None);
        let _gateway_token_guard = set_env_var("GATEWAY_AUTH_TOKEN", Some("gateway-test-token"));

        let secrets = test_secrets_store();
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(Arc::clone(&secrets));
        let sse_mgr = Arc::new(SseManager::new());
        let mut receiver = sse_mgr.sender().subscribe();
        let mut flow = fresh_pending_oauth_flow(
            Arc::clone(&secrets),
            Some(Arc::clone(&sse_mgr)),
            crate::auth::oauth::oauth_proxy_auth_token(),
        );
        flow.auto_activate_extension = false;

        ext_mgr
            .pending_oauth_flows()
            .write()
            .await
            .insert("test_nonce".to_string(), flow);

        let state = test_gateway_state(Some(ext_mgr.clone()));
        let app = test_oauth_router(state);
        let versioned_state =
            crate::auth::oauth::encode_hosted_oauth_state("test_nonce", Some("myinstance"));

        let req = axum::http::Request::builder()
            .uri(format!(
                "/oauth/callback?code=fake_code&state={}",
                urlencoding::encode(&versioned_state)
            ))
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        match receiver.recv().await.expect("onboarding_state event").event {
            crate::channels::web::types::AppEvent::OnboardingState {
                extension_name,
                state,
                message,
                ..
            } => {
                assert_eq!(extension_name, "test_tool");
                assert_eq!(
                    state,
                    crate::channels::web::types::OnboardingStateDto::Ready
                );
                assert_eq!(
                    message.as_deref(),
                    Some("Test Tool authenticated successfully")
                );
            }
            event => panic!("expected OnboardingState event, got {event:?}"),
        }

        proxy.shutdown().await;
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn test_oauth_callback_exchange_failure_broadcasts_auth_completed_failure() {
        use axum::body::Body;
        use tower::ServiceExt;

        let _env_guard = crate::config::helpers::lock_env();
        let _exchange_url_guard =
            set_env_var("IRONCLAW_OAUTH_EXCHANGE_URL", Some("http://127.0.0.1:1"));
        let _proxy_auth_guard = set_env_var("IRONCLAW_OAUTH_PROXY_AUTH_TOKEN", None);
        let _gateway_token_guard = set_env_var("GATEWAY_AUTH_TOKEN", Some("gateway-test-token"));

        let secrets = test_secrets_store();
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(Arc::clone(&secrets));
        let sse_mgr = Arc::new(SseManager::new());
        let mut receiver = sse_mgr.sender().subscribe();
        let flow = fresh_pending_oauth_flow(
            Arc::clone(&secrets),
            Some(Arc::clone(&sse_mgr)),
            crate::auth::oauth::oauth_proxy_auth_token(),
        );

        ext_mgr
            .pending_oauth_flows()
            .write()
            .await
            .insert("test_nonce".to_string(), flow);

        let state = test_gateway_state(Some(ext_mgr.clone()));
        let app = test_oauth_router(state);
        let versioned_state =
            crate::auth::oauth::encode_hosted_oauth_state("test_nonce", Some("myinstance"));

        let req = axum::http::Request::builder()
            .uri(format!(
                "/oauth/callback?code=fake_code&state={}",
                urlencoding::encode(&versioned_state)
            ))
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        match receiver.recv().await.expect("onboarding_state event").event {
            crate::channels::web::types::AppEvent::OnboardingState {
                extension_name,
                state,
                message,
                ..
            } => {
                assert_eq!(extension_name, "test_tool");
                assert_eq!(
                    state,
                    crate::channels::web::types::OnboardingStateDto::Failed
                );
                assert!(
                    message
                        .as_deref()
                        .unwrap_or_default()
                        .contains("authentication failed")
                );
            }
            event => panic!("expected OnboardingState event, got {event:?}"),
        }
    }

    fn test_relay_oauth_router(state: Arc<GatewayState>) -> Router {
        Router::new()
            .route(
                "/oauth/slack/callback",
                get(slack_relay_oauth_callback_handler),
            )
            .with_state(state)
    }

    #[tokio::test]
    async fn test_relay_oauth_callback_missing_state_param() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets = test_secrets_store();
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets.clone());
        let state = test_gateway_state(Some(ext_mgr));
        let app = test_relay_oauth_router(state);

        // Callback without state param should be rejected
        let req = axum::http::Request::builder()
            .uri("/oauth/slack/callback?team_id=T123&provider=slack")
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        assert!(
            html.contains("Invalid or expired authorization"),
            "Expected CSRF error, got: {}",
            &html[..html.len().min(300)]
        );
    }

    #[tokio::test]
    async fn test_relay_oauth_callback_wrong_state_param() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets = test_secrets_store();

        // Store a valid nonce
        secrets
            .create(
                "test",
                crate::secrets::CreateSecretParams::new(
                    format!("relay:{}:oauth_state", DEFAULT_RELAY_NAME),
                    "correct-nonce-value",
                ),
            )
            .await
            .expect("store nonce");

        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets.clone());
        let state = test_gateway_state(Some(ext_mgr));
        let app = test_relay_oauth_router(state);

        // Callback with wrong state param
        let req = axum::http::Request::builder()
            .uri("/oauth/slack/callback?team_id=T123&provider=slack&state=wrong-nonce")
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        assert!(
            html.contains("Invalid or expired authorization"),
            "Expected CSRF error for wrong nonce, got: {}",
            &html[..html.len().min(300)]
        );

        let state_key = format!("relay:{}:oauth_state", DEFAULT_RELAY_NAME);
        let exists = secrets.exists("test", &state_key).await.unwrap_or(false);
        assert!(exists, "Wrong nonce must not consume the stored CSRF nonce");
    }

    #[tokio::test]
    async fn test_relay_oauth_callback_correct_canonical_state_proceeds() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets = test_secrets_store();
        let nonce = "valid-test-nonce-12345";
        let relay_name = crate::extensions::naming::canonicalize_extension_name(DEFAULT_RELAY_NAME)
            .expect("canonical relay name");

        // Store the correct nonce under the canonical extension name used by
        // install/auth/activate flows (`slack_relay`).
        secrets
            .create(
                "test",
                crate::secrets::CreateSecretParams::new(
                    format!("relay:{}:oauth_state", relay_name),
                    nonce,
                ),
            )
            .await
            .expect("store nonce");

        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets.clone());
        let state = test_gateway_state(Some(ext_mgr));
        let app = test_relay_oauth_router(state);

        // Callback with correct state param — will pass CSRF check
        // but may fail downstream (no real relay service) — that's OK,
        // we just verify it doesn't return a CSRF error.
        let req = axum::http::Request::builder()
            .uri(format!(
                "/oauth/slack/callback?team_id=T123&provider=slack&state={}",
                nonce
            ))
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        // Should NOT contain the CSRF error message
        assert!(
            !html.contains("Invalid or expired authorization"),
            "Should have passed CSRF check, got: {}",
            &html[..html.len().min(300)]
        );

        // Verify the nonce was consumed (deleted)
        let state_key = format!("relay:{}:oauth_state", relay_name);
        let exists = secrets.exists("test", &state_key).await.unwrap_or(true);
        assert!(!exists, "CSRF nonce should be deleted after use");
    }

    #[tokio::test]
    async fn test_relay_oauth_callback_legacy_state_proceeds_and_is_consumed() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets = test_secrets_store();
        let nonce = "legacy-test-nonce-12345";
        let relay_name = crate::extensions::naming::canonicalize_extension_name(DEFAULT_RELAY_NAME)
            .expect("canonical relay name");
        let legacy_relay_name = crate::extensions::naming::legacy_extension_alias(&relay_name)
            .expect("legacy relay alias");

        secrets
            .create(
                "test",
                crate::secrets::CreateSecretParams::new(
                    format!("relay:{}:oauth_state", legacy_relay_name),
                    nonce,
                ),
            )
            .await
            .expect("store legacy nonce");

        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets.clone());
        let state = test_gateway_state(Some(ext_mgr));
        let app = test_relay_oauth_router(state);

        let req = axum::http::Request::builder()
            .uri(format!(
                "/oauth/slack/callback?team_id=T123&provider=slack&state={}",
                nonce
            ))
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        assert!(
            !html.contains("Invalid or expired authorization"),
            "Should have passed CSRF check with legacy nonce, got: {}",
            &html[..html.len().min(300)]
        );

        let state_key = format!("relay:{}:oauth_state", legacy_relay_name);
        let exists = secrets.exists("test", &state_key).await.unwrap_or(true);
        assert!(!exists, "Legacy CSRF nonce should be deleted after use");
    }

    /// Regression: the `relay:{ext}:oauth_user` secret stores the
    /// IronClaw user who initiated the OAuth flow. The callback must
    /// consume it unconditionally once the value is read, regardless of
    /// whether downstream activation, identity-pairing, or a missing
    /// `pairing_store` causes the result block to short-circuit. Leaving
    /// the secret behind would let a subsequent OAuth callback for the
    /// same extension read a stale initiating user and misroute the
    /// completion toast.
    ///
    /// The test fixture has no real relay service and no `pairing_store`
    /// wired up, so the result block errors out after the CSRF check
    /// passes — exactly the failure path Copilot flagged on PR #3390
    /// (comment id 3211833864) where the previous in-`if-let` cleanup
    /// site was unreachable.
    #[tokio::test]
    async fn test_relay_oauth_callback_consumes_oauth_user_secret_on_failure_path() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets = test_secrets_store();
        let nonce = "test-nonce-oauth-user-cleanup";
        let relay_name = crate::extensions::naming::canonicalize_extension_name(DEFAULT_RELAY_NAME)
            .expect("canonical relay name");

        secrets
            .create(
                "test",
                crate::secrets::CreateSecretParams::new(
                    format!("relay:{}:oauth_state", relay_name),
                    nonce,
                ),
            )
            .await
            .expect("store nonce"); // safety: cfg(test) fixture
        secrets
            .create(
                "test",
                crate::secrets::CreateSecretParams::new(
                    format!("relay:{}:oauth_user", relay_name),
                    "alice", // initiating user, deliberately != owner_id
                ),
            )
            .await
            .expect("store oauth_user secret"); // safety: cfg(test) fixture

        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets.clone());
        let state = test_gateway_state(Some(ext_mgr));
        let app = test_relay_oauth_router(state);

        let req = axum::http::Request::builder()
            .uri(format!(
                "/oauth/slack/callback?team_id=T123&provider=slack&state={}",
                nonce
            ))
            .body(Body::empty())
            .expect("request");

        let _resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        // We don't assert on the response body — the result block almost
        // certainly errors (no real relay backend), and that's the point:
        // the cleanup must still run on the failure path.

        let oauth_user_key = format!("relay:{}:oauth_user", relay_name);
        let exists = secrets
            .exists("test", &oauth_user_key)
            .await
            .unwrap_or(true);
        assert!(
            !exists,
            "relay:{{ext}}:oauth_user must be deleted unconditionally — \
             leaving it behind on the failure path lets a subsequent \
             OAuth callback misroute the completion toast"
        );
    }

    #[tokio::test]
    async fn test_relay_oauth_callback_nonce_under_different_user_fails() {
        // why: In hosted mode, the DB user's UUID differs from the gateway
        //      owner_id. If the nonce is stored under the DB user's scope,
        //      the callback handler (which uses owner_id) cannot find it.
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets = test_secrets_store();
        let nonce = "nonce-stored-under-wrong-user";

        // given: nonce stored under a DB user UUID, NOT the gateway owner ("test")
        secrets
            .create(
                "b50a4a66-ba1b-439c-907b-cc6b371871b0",
                crate::secrets::CreateSecretParams::new(
                    format!("relay:{}:oauth_state", DEFAULT_RELAY_NAME),
                    nonce,
                ),
            )
            .await
            .expect("store nonce");

        // ext_mgr.user_id = "test", gateway owner_id = "test"
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets);
        let state = test_gateway_state(Some(ext_mgr));
        let app = test_relay_oauth_router(state);

        // when: callback arrives with the correct nonce value
        let req = axum::http::Request::builder()
            .uri(format!(
                "/oauth/slack/callback?team_id=T123&provider=slack&state={}",
                nonce
            ))
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");

        // then: fails because nonce is under a different user scope
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        assert!(
            html.contains("Invalid or expired authorization"),
            "Nonce stored under wrong user scope should fail lookup, got: {}",
            &html[..html.len().min(300)]
        );
    }
}
