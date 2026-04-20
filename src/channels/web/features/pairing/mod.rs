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
//! through [`ExtensionName::new`] at the handler boundary — which rejects
//! path-traversal / control / mixed-script / oversized values with 400 *at
//! the boundary* instead of silently canonicalizing into a lookup that
//! would mismatch anyway — then discards the typed value and carries the
//! pre-fold lowercased `String` through the handler. The validator's
//! canonical form folds `-` into `_`, but the pairing store keys off the
//! *un-folded* lowercased name (see `crate::pairing::normalize_channel_name`
//! in `src/pairing/mod.rs`). WASM channels like `slack-relay` (see
//! `src/channels/wasm/setup.rs` and `crate::channels::relay::DEFAULT_RELAY_NAME`)
//! store hyphenated rows, and querying `slack_relay` would miss them —
//! returning empty lists / failing approvals silently. When the wider
//! codebase harmonizes `ExtensionName` with WASM-channel naming, this
//! discard-and-keep-the-raw-string dance can go away.
//!
//! The [`AppEvent::OnboardingState.extension_name`] and
//! [`dispatch_onboarding_ready_followup`] call sites both take
//! [`ExtensionName`], so those re-wrap via [`ExtensionName::from_trusted`]
//! at the call site — the value has already passed validation, just with
//! the hyphen-fold axis stripped off.
//!
//! # Why lowercasing happens before `ExtensionName::new`
//!
//! Pairing storage and webhook routes are keyed by lowercase channel
//! names. A mixed-case URL path must resolve to the same backend row as
//! the corresponding webhook, so we `to_ascii_lowercase()` *before*
//! running validation — the validator would reject uppercase input
//! outright, and callers would otherwise need to know that ahead of time.

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

/// Validate an untrusted URL-path channel segment and return the lowercased
/// (but **un-canonicalized**) form used as the pairing persistence key.
///
/// [`ExtensionName::new`] runs for its rejection semantics (path traversal,
/// invalid chars, oversize, edge/consecutive underscores) but its returned
/// value is discarded because it folds `-` into `_` — see the module
/// docstring for why the store keys off the un-folded name.
fn parse_channel(channel: String) -> Result<String, (StatusCode, String)> {
    let lowered = channel.to_ascii_lowercase();
    ExtensionName::new(&lowered).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid channel name: {e}"),
        )
    })?;
    Ok(lowered)
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
        store.list_pending(&channel).await.map_err(|e| {
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
        channel,
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
    let flow = crate::pairing::PairingCodeChallenge::new(&channel);
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
    // Bind to the authenticated user. `from_trusted` is appropriate: user.user_id
    // came from the auth layer (DB-sourced). Role is irrelevant for approval —
    // only the id is recorded on the pairing row.
    let owner_id = crate::ownership::UserId::from_trusted(
        user.user_id.clone(),
        crate::ownership::UserRole::from_db_role(&user.role),
    );
    let approval = match store.approve(&channel, &code, &owner_id).await {
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
            .complete_pairing_approval(&channel, &approval.external_id)
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
                extension_name: ExtensionName::from_trusted(channel.clone()),
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
            extension_name: ExtensionName::from_trusted(channel.clone()),
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
        let extension_name = ExtensionName::from_trusted(channel);
        dispatch_onboarding_ready_followup(&state, &user.user_id, thread_id, &extension_name)
            .await?;
    }

    Ok(Json(ActionResponse::ok("Pairing approved.".to_string())))
}

#[cfg(test)]
mod tests {
    //! `parse_channel` is the boundary that turns an untrusted URL-path
    //! segment into the lowercased pairing key. Every pairing handler
    //! calls it as the first line, so an error here is what triggers the
    //! 400 that the PR #2665 review (Copilot) asked to lock in. These
    //! tests pin four contracts:
    //!
    //! 1. Accept the names pairing actually uses (lowercase, snake_case).
    //! 2. Lowercase mixed-case URL paths.
    //! 3. **Preserve hyphens** — `ExtensionName::new` canonicalizes `-`
    //!    to `_`, but the pairing store keys off the un-folded name, so
    //!    `slack-relay` (a real live WASM channel) must stay addressable.
    //! 4. Reject shapes that can't correspond to a real channel (path
    //!    traversal, invalid charset, edge/consecutive underscores,
    //!    oversize) with `StatusCode::BAD_REQUEST`.
    //!
    //! If `ExtensionName`'s rules grow, or if `parse_channel`'s return
    //! type is re-typed into an `ExtensionName`, this test module is the
    //! first place the regression will surface.
    use super::*;

    #[test]
    fn parse_channel_accepts_lowercase_snake_case() {
        let parsed = parse_channel("telegram".to_string()).expect("lowercase name must validate");
        assert_eq!(parsed, "telegram");

        let parsed =
            parse_channel("slack_relay".to_string()).expect("snake_case name must validate");
        assert_eq!(parsed, "slack_relay");
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
        assert_eq!(parsed, "telegram");
    }

    #[test]
    fn parse_channel_preserves_hyphens_for_slack_relay() {
        // Regression for the PR #2665 Copilot review: a previous revision of
        // `parse_channel` returned `ExtensionName::new(...)` directly, which
        // canonicalizes `-` into `_`. But the live WASM channel name (see
        // `crate::channels::relay::DEFAULT_RELAY_NAME` and
        // `src/channels/wasm/setup.rs`) is `slack-relay` — stored and keyed
        // hyphenated in the pairing store. Folding to `slack_relay` would
        // silently miss every real pairing row. This test pins the un-folded
        // form so the regression can't reoccur without a visible signal.
        let parsed = parse_channel("slack-relay".to_string())
            .expect("slack-relay must validate and retain hyphens");
        assert_eq!(parsed, "slack-relay");

        // Case-sensitivity still folds through `to_ascii_lowercase()`.
        let parsed = parse_channel("SLACK-RELAY".to_string())
            .expect("uppercase slack-relay must validate and retain hyphens");
        assert_eq!(parsed, "slack-relay");
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
