//! Engine-channel dispatch helpers.
//!
//! Thin wrappers that compose a web-origin [`IncomingMessage`] and shove it
//! into the agent-loop `msg_tx` channel. Shared by the chat HTTP handlers
//! (`server.rs`), the extensions setup-submit handler (`server.rs`), the
//! WebSocket approval path (`server.rs`), and the pairing slice
//! (`features/pairing/`) — each of those callers originally lived in
//! `server.rs`, so migrating a slice out of `server.rs` means giving it a
//! shared home that both `features/*` and the still-in-`server.rs` callers
//! can reach. Platform is the only layer visible to both.
//!
//! [`IncomingMessage`]: crate::channels::IncomingMessage

use axum::http::StatusCode;
use ironclaw_common::ExtensionName;
use uuid::Uuid;

use crate::channels::web::platform::state::GatewayState;
use crate::channels::web::util::web_incoming_message;

/// Send a typed [`Submission`] to the agent loop on behalf of the web user.
///
/// Also attaches a human-readable placeholder string so history / log views
/// that look at `IncomingMessage::content` still show something meaningful
/// for structured submissions.
///
/// [`Submission`]: crate::agent::submission::Submission
pub(crate) async fn dispatch_engine_submission(
    state: &GatewayState,
    user_id: &str,
    thread_id: &str,
    submission: crate::agent::submission::Submission,
) -> Result<(), (StatusCode, String)> {
    let tx = {
        let tx_guard = state.msg_tx.read().await;
        tx_guard
            .as_ref()
            .ok_or((
                StatusCode::SERVICE_UNAVAILABLE,
                "Channel not started".to_string(),
            ))?
            .clone()
    };

    let placeholder = match &submission {
        crate::agent::submission::Submission::ExecApproval { .. } => {
            "[structured execution approval]"
        }
        crate::agent::submission::Submission::ExternalCallback { .. } => {
            "[structured external callback]"
        }
        crate::agent::submission::Submission::GateAuthResolution { .. } => {
            "[structured auth gate resolution]"
        }
        _ => "[structured submission]",
    };
    let msg = web_incoming_message("gateway", user_id, placeholder, Some(thread_id))
        .with_structured_submission(submission);

    tx.send(msg).await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Channel closed".to_string(),
        )
    })
}

/// Parse a `request_id` (UUID) and submit an `ExternalCallback` to the
/// engine. The `BAD_REQUEST` on parse failure intentionally short-circuits
/// the wider handler so the caller gets a structured 400 rather than a
/// generic 500.
pub(crate) async fn dispatch_engine_external_callback(
    state: &GatewayState,
    user_id: &str,
    thread_id: &str,
    request_id: &str,
) -> Result<(), (StatusCode, String)> {
    let request_id = Uuid::parse_str(request_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid request_id (expected UUID)".to_string(),
        )
    })?;
    let callback = crate::agent::submission::Submission::ExternalCallback { request_id };
    dispatch_engine_submission(state, user_id, thread_id, callback).await
}

/// Nudge the agent after an onboarding flow reports `ready`, so the
/// reply-to-user step happens without the user having to send another
/// message. Takes a typed [`ExtensionName`] so no caller has to re-run an
/// ad-hoc `sanitize_*` pass before formatting it into the follow-up prompt —
/// the type system is the sanitation contract now.
pub(crate) async fn dispatch_onboarding_ready_followup(
    state: &GatewayState,
    user_id: &str,
    thread_id: &str,
    extension_name: &ExtensionName,
) -> Result<(), (StatusCode, String)> {
    let tx = {
        let tx_guard = state.msg_tx.read().await;
        tx_guard
            .as_ref()
            .ok_or((
                StatusCode::SERVICE_UNAVAILABLE,
                "Channel not started".to_string(),
            ))?
            .clone()
    };

    let content = format!(
        "System event: onboarding for '{extension_name}' is now fully complete and ready. \
Reply to the user with a brief confirmation and any immediately useful next step. \
Do not call install, activate, authenticate, configure, or setup tools again unless the user explicitly asks."
    );
    let msg = web_incoming_message("gateway", user_id, content, Some(thread_id));

    tx.send(msg).await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Channel closed".to_string(),
        )
    })
}
