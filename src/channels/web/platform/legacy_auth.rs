//! Legacy v1 thread-level auth-mode shim.
//!
//! Temporary compatibility helpers for browser and WebSocket clients that still
//! use the pre-gate `pending_auth` flow. Remove this module once every web auth
//! prompt is gate-backed and the `/api/chat/auth-token` / `/api/chat/auth-cancel`
//! endpoints are retired.
//!
//! Lives under `platform/` because both the HTTP handlers in `server.rs` and
//! the WebSocket dispatcher in `platform/ws.rs` consume these helpers; moving
//! them here lets both call sites reach the implementation without re-creating
//! the back-edge the boundary check was designed to prevent.

use std::sync::Arc;

use axum::http::StatusCode;
use ironclaw_common::ExtensionName;
use uuid::Uuid;

use crate::channels::web::platform::state::GatewayState;
use crate::channels::web::types::{
    ActionResponse, AppEvent, AuthCancelRequest, AuthTokenRequest, OnboardingStateDto,
};

/// Clear pending auth mode on the active thread for a user (both legacy
/// v1 session-scoped and engine v2 pending-gate).
pub async fn clear_auth_mode(state: &GatewayState, user_id: &str) {
    let _ = clear_auth_mode_for_thread(state, user_id, None).await;
}

/// Clear both the legacy v1 session `pending_auth` and the engine v2
/// pending auth gate. Use this after the user has resolved the credential
/// flow (or explicitly cancelled) and the gate is done with.
pub(crate) async fn clear_auth_mode_for_thread(
    state: &GatewayState,
    user_id: &str,
    thread_id: Option<&str>,
) -> Result<(), (StatusCode, String)> {
    clear_session_auth_mode_for_thread(state, user_id, thread_id).await?;
    crate::bridge::clear_engine_pending_auth(user_id, thread_id).await;
    Ok(())
}

/// Clear ONLY the legacy v1 session-level `pending_auth` state, leaving
/// the engine v2 pending auth gate intact. Used by the OAuth callback:
/// the successful-callback path still needs the engine gate present so
/// the `ExternalCallback` replay can resolve it (and preserve the
/// paused_lease), and the failed-callback path should leave the gate
/// visible so the user can retry from the UI.
pub(crate) async fn clear_session_auth_mode_for_thread(
    state: &GatewayState,
    user_id: &str,
    thread_id: Option<&str>,
) -> Result<(), (StatusCode, String)> {
    if let Some(ref sm) = state.session_manager {
        let session = sm.get_or_create_session(user_id).await;
        let mut sess = session.lock().await;
        let target_thread_id = match thread_id {
            Some(raw) => Some(Uuid::parse_str(raw).map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    "Invalid thread_id (expected UUID)".to_string(),
                )
            })?),
            None => sess.active_thread,
        };
        if let Some(thread_id) = target_thread_id
            && let Some(thread) = sess.threads.get_mut(&thread_id)
        {
            thread.pending_auth = None;
        }
    }
    Ok(())
}

async fn restore_pending_auth_mode(
    session: &Arc<tokio::sync::Mutex<crate::agent::session::Session>>,
    thread_id: Uuid,
    extension_name: &ExtensionName,
) {
    let mut sess = session.lock().await;
    if let Some(thread) = sess.threads.get_mut(&thread_id) {
        thread.enter_auth_mode(extension_name.clone());
    }
}

/// Temporary legacy shim for browser and WebSocket clients that still use the
/// v1 thread-level auth mode. Remove this helper together with
/// `/api/chat/auth-token` once every web auth prompt is gate-backed.
pub(crate) async fn handle_legacy_auth_token_submission(
    state: &GatewayState,
    user_id: &str,
    req: AuthTokenRequest,
) -> Result<ActionResponse, (StatusCode, String)> {
    let token = req.token.trim();
    if token.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "token must not be empty".to_string(),
        ));
    }

    // Temporary web compatibility shim for engine v1 `pending_auth`.
    // Gate-backed auth must go through `/api/chat/gate/resolve`; only prompts
    // without a `request_id` should hit this endpoint.
    let session_manager = state.session_manager.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Session manager unavailable".to_string(),
    ))?;
    let session = session_manager.get_or_create_session(user_id).await;
    let (thread_id, pending_auth) = {
        let mut sess = session.lock().await;
        let target_thread_id = match req.thread_id.as_deref() {
            Some(raw) => Uuid::parse_str(raw).map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    "Invalid thread_id (expected UUID)".to_string(),
                )
            })?,
            None => sess.active_thread.ok_or((
                StatusCode::BAD_REQUEST,
                "thread_id is required when there is no active thread".to_string(),
            ))?,
        };

        let thread = sess
            .threads
            .get_mut(&target_thread_id)
            .ok_or((StatusCode::NOT_FOUND, "Thread not found".to_string()))?;
        let pending_auth = thread.pending_auth.clone().ok_or((
            StatusCode::BAD_REQUEST,
            "No pending authentication request for this thread".to_string(),
        ))?;

        if pending_auth.is_expired() {
            thread.pending_auth = None;
            let message = format!(
                "Authentication for '{}' expired. Please try again.",
                pending_auth.extension_name
            );
            state.sse.broadcast_for_user(
                user_id,
                AppEvent::OnboardingState {
                    extension_name: pending_auth.extension_name.clone(),
                    state: OnboardingStateDto::Failed,
                    request_id: None,
                    message: Some(message.clone()),
                    instructions: None,
                    auth_url: None,
                    setup_url: None,
                    onboarding: None,
                    thread_id: Some(target_thread_id.to_string()),
                },
            );
            return Ok(ActionResponse::fail(message));
        }

        thread.pending_auth = None;
        (target_thread_id, pending_auth)
    };

    let result = if let Some(auth_manager) = state.auth_manager.as_ref() {
        auth_manager
            .submit_auth_token(pending_auth.extension_name.as_str(), token, user_id)
            .await
    } else if let Some(ext_mgr) = state.extension_manager.as_ref() {
        ext_mgr
            .configure_token(pending_auth.extension_name.as_str(), token, user_id)
            .await
    } else {
        restore_pending_auth_mode(&session, thread_id, &pending_auth.extension_name).await;
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Extension manager not available".to_string(),
        ));
    };

    match result {
        Ok(result) if result.activated => {
            state.sse.broadcast_for_user(
                user_id,
                AppEvent::OnboardingState {
                    extension_name: pending_auth.extension_name,
                    state: OnboardingStateDto::Ready,
                    request_id: None,
                    message: Some(result.message.clone()),
                    instructions: None,
                    auth_url: None,
                    setup_url: None,
                    onboarding: None,
                    thread_id: Some(thread_id.to_string()),
                },
            );
            Ok(ActionResponse::ok(result.message))
        }
        Ok(result) => {
            restore_pending_auth_mode(&session, thread_id, &pending_auth.extension_name).await;
            state.sse.broadcast_for_user(
                user_id,
                AppEvent::OnboardingState {
                    extension_name: pending_auth.extension_name,
                    state: OnboardingStateDto::AuthRequired,
                    request_id: None,
                    message: None,
                    instructions: Some(result.message.clone()),
                    auth_url: result.auth_url.clone(),
                    setup_url: None,
                    onboarding: None,
                    thread_id: Some(thread_id.to_string()),
                },
            );
            Ok(ActionResponse::fail(result.message))
        }
        Err(crate::extensions::ExtensionError::ValidationFailed(_)) => {
            let message = "Invalid token. Please try again.".to_string();
            restore_pending_auth_mode(&session, thread_id, &pending_auth.extension_name).await;
            state.sse.broadcast_for_user(
                user_id,
                AppEvent::OnboardingState {
                    extension_name: pending_auth.extension_name,
                    state: OnboardingStateDto::AuthRequired,
                    request_id: None,
                    message: None,
                    instructions: Some(message.clone()),
                    auth_url: None,
                    setup_url: None,
                    onboarding: None,
                    thread_id: Some(thread_id.to_string()),
                },
            );
            Ok(ActionResponse::fail(message))
        }
        Err(error) => {
            restore_pending_auth_mode(&session, thread_id, &pending_auth.extension_name).await;
            let message = error.to_string();
            state.sse.broadcast_for_user(
                user_id,
                AppEvent::OnboardingState {
                    extension_name: pending_auth.extension_name,
                    state: OnboardingStateDto::Failed,
                    request_id: None,
                    message: Some(message.clone()),
                    instructions: None,
                    auth_url: None,
                    setup_url: None,
                    onboarding: None,
                    thread_id: Some(thread_id.to_string()),
                },
            );
            Ok(ActionResponse::fail(message))
        }
    }
}

/// Temporary legacy shim for browser and WebSocket clients that still cancel
/// v1 thread-level auth mode directly. Remove this helper together with
/// `/api/chat/auth-cancel` once the gateway retires the no-request_id path.
pub(crate) async fn handle_legacy_auth_cancel(
    state: &GatewayState,
    user_id: &str,
    req: AuthCancelRequest,
) -> Result<ActionResponse, (StatusCode, String)> {
    clear_auth_mode_for_thread(state, user_id, req.thread_id.as_deref()).await?;
    Ok(ActionResponse::ok("Authentication cancelled."))
}
