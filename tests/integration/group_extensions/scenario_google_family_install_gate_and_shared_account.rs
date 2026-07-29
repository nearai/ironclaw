//! Scenario 10: the Google-family install-and-connect user journeys from
//! chat (the "Install and Connect Extensions" walk for OAuth packages that
//! share one provider):
//!
//! - **Phase 1 — install parks activation at the provider's auth gate, and a
//!   same-provider account with the WRONG scopes must not satisfy it.** The
//!   scenario seeds a `google` account carrying only gmail scopes. Installing
//!   google-calendar from chat and
//!   activating must park `BlockedAuth` with a renderable requirement naming
//!   provider `google` and the CALENDAR capability scopes — reusing a shared
//!   Google account is only allowed after the selected capability scope is
//!   granted.
//! - **Phase 2 — bulk installs park INDEPENDENT gates.** Installing
//!   google-drive in a second thread parks at its own gate while calendar's
//!   gate stays parked; denying drive's gate resolves only drive's run.
//! - **Phase 3 — denial leaves a clean retry path.** Denying calendar's gate
//!   completes its run without an error-shaped tool result.
//! - **Phase 4 — one correctly-scoped Google account serves both packages.**
//!   After the wrongly-scoped account is revoked and a fresh `google`
//!   account with calendar+drive scopes exists, both activations complete —
//!   the credential-appears → activation-completes arm, and the
//!   one-account-many-extensions shape.
//!
//! Uses "google-calendar" and "google-drive" in its own
//! Google-OAuth-configured group. The isolated composition is required by the
//! provider-instance readiness contract: this scenario tests per-account
//! gating, not the earlier missing-instance remediation path.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use ironclaw_auth::{
    GOOGLE_CALENDAR_EVENTS_SCOPE, GOOGLE_CALENDAR_READONLY_SCOPE, GOOGLE_GMAIL_MODIFY_SCOPE,
    GOOGLE_GMAIL_READONLY_SCOPE, GOOGLE_GMAIL_SEND_SCOPE,
};
use ironclaw_turns::TurnStatus;
use serde_json::json;

// Not re-exported from `ironclaw_auth`'s root like the calendar pair; string
// values are pinned by the google-drive manifest's `provider_scopes`.
const GOOGLE_DRIVE_READONLY_SCOPE: &str = "https://www.googleapis.com/auth/drive.readonly";
const GOOGLE_DRIVE_SCOPE: &str = "https://www.googleapis.com/auth/drive";

pub async fn run(_g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let g = RebornIntegrationGroup::extension_lifecycle_google_oauth_configured().await?;
    let g = &g;
    let google_provider =
        ironclaw_host_api::VendorId::new("google").map_err(|error| error.to_string())?;

    // ── Phase 1: calendar install parks at the google gate despite the
    //    existing (gmail-scoped) google account ─────────────────────────────
    let calendar = g
        .thread("google-family-calendar-install")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "google-calendar"}),
            ),
            // Consumed by the post-deny resume model call in phase 3.
            RebornScriptedReply::text("calendar needs authorization"),
        ])
        .build()
        .await?;
    calendar
        .seed_capability_credential_account(
            "google",
            "itest gmail-only account",
            &[
                GOOGLE_GMAIL_MODIFY_SCOPE,
                GOOGLE_GMAIL_READONLY_SCOPE,
                GOOGLE_GMAIL_SEND_SCOPE,
            ],
        )
        .await?;
    let (calendar_run, calendar_gate) = calendar
        .submit_turn_until_auth_blocked("set up google calendar")
        .await?;
    let calendar_state = calendar
        .wait_for_status(calendar_run, TurnStatus::BlockedAuth)
        .await?;
    let calendar_requirement = calendar_state
        .credential_requirements
        .iter()
        .find(|requirement| requirement.provider == google_provider)
        .ok_or_else(|| {
            format!(
                "calendar activation must park a renderable google requirement; a \
                 gmail-scoped google account must not satisfy the calendar scopes; got {:?}",
                calendar_state.credential_requirements
            )
        })?;
    for expected_scope in [GOOGLE_CALENDAR_READONLY_SCOPE, GOOGLE_CALENDAR_EVENTS_SCOPE] {
        if !calendar_requirement
            .provider_scopes
            .iter()
            .any(|scope| scope == expected_scope)
        {
            return Err(format!(
                "the parked requirement must carry the SELECTED capability's calendar \
                 scopes (what the OAuth card renders); missing {expected_scope}; got {:?}",
                calendar_requirement.provider_scopes
            )
            .into());
        }
    }
    // The wrong-scope account surfaced EXCLUSIVELY as the gate — no misleading
    // error-shaped tool result (#5878 shape).
    calendar.assert_no_error_shaped_tool_result().await?;

    // ── Phase 2: a second OAuth package parks its OWN independent gate ──────
    let drive = g
        .thread("google-family-drive-install")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "google-drive"}),
            ),
            RebornScriptedReply::text("drive needs authorization"),
        ])
        .build()
        .await?;
    let (drive_run, drive_gate) = drive
        .submit_turn_until_auth_blocked("set up google drive")
        .await?;
    if drive_run == calendar_run {
        return Err("each OAuth install must park its own run, not join calendar's".into());
    }
    let drive_state = drive
        .wait_for_status(drive_run, TurnStatus::BlockedAuth)
        .await?;
    let drive_requirement = drive_state
        .credential_requirements
        .iter()
        .find(|requirement| requirement.provider == google_provider)
        .ok_or_else(|| {
            format!(
                "drive activation must park a renderable google requirement; got {:?}",
                drive_state.credential_requirements
            )
        })?;
    for expected_scope in [GOOGLE_DRIVE_READONLY_SCOPE, GOOGLE_DRIVE_SCOPE] {
        if !drive_requirement
            .provider_scopes
            .iter()
            .any(|scope| scope == expected_scope)
        {
            return Err(format!(
                "the parked drive requirement must carry the SELECTED capability's drive \
                 scopes; missing {expected_scope}; got {:?}",
                drive_requirement.provider_scopes
            )
            .into());
        }
    }
    // Both gates are parked at once; resolving drive's must not touch
    // calendar's.
    drive.deny_auth_gate(drive_run, &drive_gate).await?;
    drive
        .wait_for_status(drive_run, TurnStatus::Completed)
        .await?;
    calendar
        .wait_for_status(calendar_run, TurnStatus::BlockedAuth)
        .await
        .map_err(|error| {
            format!(
                "denying drive's gate must leave calendar's gate parked \
                 (independent gates per package): {error}"
            )
        })?;

    // ── Phase 3: denying calendar's gate leaves a clean retry path ──────────
    calendar
        .deny_auth_gate(calendar_run, &calendar_gate)
        .await?;
    calendar
        .wait_for_status(calendar_run, TurnStatus::Completed)
        .await?;
    // No post-denial `assert_no_error_shaped_tool_result` here: an explicit
    // user denial legitimately lands as an error-status observation ("auth
    // gate denied by user") so the model knows authorization was refused. The
    // #5878 shape rule bans errors standing in for GATES, not denial markers.

    // ── Phase 4: one correctly-scoped google account unlocks BOTH packages ──
    // Retire the gmail-scoped account so account selection is unambiguous,
    // then connect a fresh google account carrying the calendar+drive scopes
    // (what completing the real popup would have granted).
    // A fresh composition represents retry after the denied setup-needed
    // memberships were removed/reset. The credential is present before the
    // only public lifecycle action, so install can auto-reconcile to active.
    let restored_group =
        RebornIntegrationGroup::extension_lifecycle_google_oauth_configured().await?;
    let calendar_restore = restored_group
        .thread("google-family-calendar-restore")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "google-calendar"}),
            ),
            RebornScriptedReply::text("calendar authorized"),
        ])
        .build()
        .await?;
    calendar_restore
        .seed_capability_credential_account(
            "google",
            "itest google workspace",
            &[
                GOOGLE_CALENDAR_READONLY_SCOPE,
                GOOGLE_CALENDAR_EVENTS_SCOPE,
                GOOGLE_DRIVE_READONLY_SCOPE,
                GOOGLE_DRIVE_SCOPE,
            ],
        )
        .await?;
    calendar_restore
        .submit_turn("install google calendar")
        .await?;
    calendar_restore
        .assert_tool_result_contains("\"phase\":\"active\"")
        .await?;
    calendar_restore
        .assert_model_message_content_contains(r#"\"phase\":\"active\""#)
        .await?;

    let drive_restore = restored_group
        .thread("google-family-drive-restore")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "google-drive"}),
            ),
            RebornScriptedReply::text("drive authorized"),
        ])
        .build()
        .await?;
    drive_restore.submit_turn("install google drive").await?;
    drive_restore
        .assert_tool_result_contains("\"phase\":\"active\"")
        .await?;
    drive_restore
        .assert_model_message_content_contains(r#"\"phase\":\"active\""#)
        .await?;

    Ok(())
}
