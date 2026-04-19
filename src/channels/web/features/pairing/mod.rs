//! Channel pairing approval.
//!
//! Owns the pairing-code approval flow for WASM channels (Telegram, Slack
//! relay, etc.). The admin dashboard lists pending requests via
//! `GET /api/pairing/{channel}`, and any authenticated user can self-claim
//! a request by submitting the code from their DM via
//! `POST /api/pairing/{channel}/approve` — that's the "self-service" wire
//! the pairing flow is designed around.
//!
//! # Identity boundary
//!
//! The `{channel}` URL path is untrusted input. The slice validates it
//! through [`ExtensionName::new`] at the handler boundary, which rejects
//! path-traversal / control / mixed-script / oversized values with 400 *at
//! the boundary* instead of silently canonicalizing into a lookup that
//! would mismatch anyway. Every downstream API takes `&str`, so the
//! typed value is squeezed back to a string slice via `.as_str()` at the
//! call site — that's still a net win because the construction is audited
//! in exactly one place.
//!
//! # Why lowercasing happens before `ExtensionName::new`
//!
//! Pairing storage and webhook routes are keyed by lowercase channel
//! names. A mixed-case URL path must resolve to the same backend row as
//! the corresponding webhook, so we `to_ascii_lowercase()` *before*
//! constructing the [`ExtensionName`] — the validator would reject
//! uppercase input outright, and callers would otherwise need to know
//! that ahead of time.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use ironclaw_common::ExtensionName;

use crate::channels::web::auth::{AdminUser, AuthenticatedUser};
use crate::channels::web::platform::engine_dispatch::{
    dispatch_engine_external_callback, dispatch_onboarding_ready_followup,
};
use crate::channels::web::platform::state::GatewayState;
use crate::channels::web::types::{
    ActionResponse, AppEvent, OnboardingStateDto, PairingApproveRequest, PairingListResponse,
    PairingRequestInfo,
};

fn parse_channel(channel: String) -> Result<ExtensionName, (StatusCode, String)> {
    ExtensionName::new(channel.to_ascii_lowercase()).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid channel name: {e}"),
        )
    })
}

/// `GET /api/pairing/{channel}` — admin-only list of pending pairing requests.
pub(crate) async fn pairing_list_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(_user): AdminUser,
    Path(channel): Path<String>,
) -> Result<Json<PairingListResponse>, (StatusCode, String)> {
    let channel = parse_channel(channel)?;
    let store = state.pairing_store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Pairing store not available".to_string(),
    ))?;
    let requests: Vec<crate::db::PairingRequestRecord> =
        store.list_pending(channel.as_str()).await.map_err(|e| {
            tracing::warn!(error = %e, "pairing list failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal error listing pairing requests".to_string(),
            )
        })?;

    let infos = requests
        .into_iter()
        .map(|r| PairingRequestInfo {
            code: r.code,
            sender_id: r.external_id,
            meta: None,
            created_at: r.created_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(PairingListResponse {
        channel: channel.into_inner(),
        requests: infos,
    }))
}

/// `POST /api/pairing/{channel}/approve` — authenticated user self-claims a
/// pairing code. Uses `AuthenticatedUser` (not `AdminUser`) because pairing
/// is self-service: the user who received the code in their DM claims it
/// for their own account.
pub(crate) async fn pairing_approve_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(channel): Path<String>,
    Json(req): Json<PairingApproveRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    let channel = parse_channel(channel)?;
    let flow = crate::pairing::PairingCodeChallenge::new(channel.as_str());
    let Some(code) =
        crate::code_challenge::CodeChallengeFlow::normalize_submission(&flow, &req.code)
    else {
        return Ok(Json(ActionResponse::fail(
            "Pairing code is required.".to_string(),
        )));
    };

    let store = state.pairing_store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Pairing store not available".to_string(),
    ))?;
    let owner_id = crate::ownership::OwnerId::from(user.user_id.clone());
    let approval = match store.approve(channel.as_str(), &code, &owner_id).await {
        Ok(approval) => approval,
        Err(crate::error::DatabaseError::NotFound { .. }) => {
            return Ok(Json(ActionResponse::fail(
                "Invalid or expired pairing code.".to_string(),
            )));
        }
        Err(e) => {
            tracing::debug!(error = %e, "pairing approval failed");
            return Ok(Json(ActionResponse::fail(
                "Internal error processing approval.".to_string(),
            )));
        }
    };

    // Propagate owner binding to the running channel.
    let propagation_failed = if let Some(ext_mgr) = state.extension_manager.as_ref() {
        match ext_mgr
            .complete_pairing_approval(channel.as_str(), &approval.external_id)
            .await
        // dispatch-exempt: runtime channel mutation; pairing tool migration tracked as follow-up
        {
            Ok(()) => false,
            Err(e) => {
                tracing::warn!(
                    channel = %channel,
                    error = %e,
                    "Failed to propagate owner binding to running channel"
                );
                true
            }
        }
    } else {
        false
    };

    if propagation_failed {
        if let Err(error) = store.revert_approval(&approval).await {
            tracing::warn!(
                channel = %channel,
                error = %error,
                "Failed to revert pairing approval after runtime propagation failure"
            );
        }
        let message = "Pairing was approved, but the running channel could not be updated. Please retry or restart the channel.".to_string();
        state.sse.broadcast_for_user(
            &user.user_id,
            AppEvent::OnboardingState {
                extension_name: channel.as_str().to_string(),
                state: OnboardingStateDto::Failed,
                request_id: None,
                message: Some(message.clone()),
                instructions: None,
                auth_url: None,
                setup_url: None,
                onboarding: None,
                thread_id: req.thread_id.clone(),
            },
        );
        return Ok(Json(ActionResponse::fail(message)));
    }

    // Notify the frontend so it can dismiss the pairing card.
    state.sse.broadcast_for_user(
        &user.user_id,
        AppEvent::OnboardingState {
            extension_name: channel.as_str().to_string(),
            state: OnboardingStateDto::Ready,
            request_id: None,
            message: Some("Pairing approved.".to_string()),
            instructions: None,
            auth_url: None,
            setup_url: None,
            onboarding: None,
            thread_id: req.thread_id.clone(),
        },
    );

    if let (Some(request_id), Some(thread_id)) =
        (req.request_id.as_deref(), req.thread_id.as_deref())
    {
        dispatch_engine_external_callback(&state, &user.user_id, thread_id, request_id).await?;
    } else if let Some(thread_id) = req.thread_id.as_deref() {
        dispatch_onboarding_ready_followup(&state, &user.user_id, thread_id, &channel).await?;
    }

    Ok(Json(ActionResponse::ok("Pairing approved.".to_string())))
}

#[cfg(test)]
mod tests {
    //! `parse_channel` is the boundary that turns an untrusted URL-path
    //! segment into a validated [`ExtensionName`]. Every pairing handler
    //! calls it as the first line, so an error here is what triggers the
    //! 400 that the PR #2665 review (Copilot) asked to lock in. These
    //! tests pin both sides of the contract: accept the names pairing
    //! actually uses, reject the shapes that can't correspond to a real
    //! extension (path traversal, invalid charset, edge underscores,
    //! oversize). If `ExtensionName`'s rules grow, this test module is
    //! the first place the stricter behavior will surface.
    use super::*;

    #[test]
    fn parse_channel_accepts_lowercase_snake_case() {
        let parsed = parse_channel("telegram".to_string()).expect("lowercase name must validate");
        assert_eq!(parsed.as_str(), "telegram");

        let parsed =
            parse_channel("slack_relay".to_string()).expect("snake_case name must validate");
        assert_eq!(parsed.as_str(), "slack_relay");
    }

    #[test]
    fn parse_channel_lowercases_mixed_case_input() {
        // The handler's pre-validation `to_ascii_lowercase()` is what lets
        // mixed-case URL paths resolve to the same pairing row as the
        // corresponding webhook. `ExtensionName::new` would reject the raw
        // uppercase input, so losing this step regresses to 400 on every
        // dashboard-entered channel name — exactly the silent-drop regression
        // this test guards against.
        let parsed = parse_channel("Telegram".to_string()).expect("mixed case must lowercase");
        assert_eq!(parsed.as_str(), "telegram");
    }

    #[test]
    fn parse_channel_rejects_empty_with_bad_request() {
        let (status, _msg) = parse_channel(String::new()).expect_err("empty must fail");
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn parse_channel_rejects_path_traversal_with_bad_request() {
        let (status, _msg) =
            parse_channel("../bad".to_string()).expect_err("path traversal must fail");
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn parse_channel_rejects_invalid_chars_with_bad_request() {
        // Dot is the canonical injection-shaped separator the old
        // `sanitize_extension_name` used to strip silently.
        let (status, _msg) = parse_channel("bad.name".to_string()).expect_err("dot must fail");
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (status, _msg) =
            parse_channel("bad name".to_string()).expect_err("whitespace inside must fail");
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn parse_channel_rejects_consecutive_underscores_with_bad_request() {
        let (status, _msg) =
            parse_channel("bad__name".to_string()).expect_err("consecutive _ must fail");
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn parse_channel_rejects_edge_underscores_with_bad_request() {
        let (status, _msg) = parse_channel("_leading".to_string()).expect_err("leading _");
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (status, _msg) = parse_channel("trailing_".to_string()).expect_err("trailing _");
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn parse_channel_rejects_too_long_with_bad_request() {
        let long = "a".repeat(ironclaw_common::MAX_NAME_LEN + 1);
        let (status, _msg) = parse_channel(long).expect_err("over length must fail");
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}
