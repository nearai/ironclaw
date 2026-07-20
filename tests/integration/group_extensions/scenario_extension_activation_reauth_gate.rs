//! Scenario 9 (bucket-3 arms of issue #6105; the #5878 and #6043 failure
//! shapes): ACTIVATION-TIME auth surface. `extension_activate` when the
//! caller's only credential account for the provider is REVOKED (external
//! revocation — the user pulled the grant on the provider side) must open a
//! REAL re-auth gate:
//! - the run parks `BlockedAuth` with `credential_requirements` populated
//!   (provider set — what the Configure card / auth prompt renders, the
//!   #6043 "starts authentication instead of a generic capability error"
//!   discriminator);
//! - NO error-shaped tool result is persisted (#5878's misleading "the
//!   tool input could not be encoded" / "provider unavailable" regression);
//! - a RETRIED activation with the credential still revoked parks at a
//!   fresh re-auth gate — denied once is not satisfied, and the gate is not
//!   one-shot (the lifecycle.md "reconnect must not resume without updated
//!   credentials" arm);
//! - after a reconfigure (fresh credential through the production
//!   manual-token flow) activation completes — the revoked state does not
//!   wedge the machine (#5878's "requires multiple retry attempts" arm).
//!
//! Complements the DISPATCH-TIME 401 → re-auth pin in
//! `tests/integration/auth_gate.rs` (issue #5878 reported the
//! `extension_activate` surface specifically, which that test does not
//! drive). Uses "notion" (installed+removed by scenario 2, so the install
//! here is fresh; no other scenario touches its credential accounts).

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use ironclaw_turns::TurnStatus;
use serde_json::json;

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    // ── Phase 1: activation with only a revoked credential opens the gate ───
    let activator = g
        .thread("notion-reauth-activate")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "notion"}),
            ),
            RebornScriptedReply::tool_call(
                "builtin.extension_activate",
                json!({"extension_id": "notion"}),
            ),
            // Consumed by the post-deny resume model call.
            RebornScriptedReply::text("notion needs reauthorization"),
        ])
        .build()
        .await?;
    // The #5878 repro order: CONNECT first (a real account under the
    // capability dispatch scope, via the production manual-token flow)…
    activator
        .seed_capability_credential_account("notion", "itest notion original", &[])
        .await?;
    // …then the user revokes the grant on the provider side: the account
    // flips to Revoked through the same update_status transition the refresh
    // sweep's terminal invalid_grant path performs.
    activator
        .revoke_capability_credential_accounts("notion")
        .await?;

    let (run_id, gate_ref) = activator
        .submit_turn_until_auth_blocked("set up notion")
        .await?;
    let state = activator
        .wait_for_status(run_id, TurnStatus::BlockedAuth)
        .await?;
    // #6043 discriminator: a REAL auth gate the UI can render — provider
    // populated — not a provider-null unsubmittable gate and not a failure.
    if state.credential_requirements.is_empty() {
        return Err("activation over a revoked credential must populate \
             credential_requirements; an empty list is the unsubmittable-gate shape"
            .into());
    }
    let notion_provider = ironclaw_host_api::RuntimeCredentialAccountProviderId::new("notion")
        .map_err(|error| error.to_string())?;
    if !state
        .credential_requirements
        .iter()
        .any(|requirement| requirement.provider == notion_provider)
    {
        return Err(format!(
            "the re-auth gate must name the notion provider; got {:?}",
            state.credential_requirements
        )
        .into());
    }
    // #5878 discriminator: the revoked credential surfaced EXCLUSIVELY as the
    // gate — no error-shaped tool result of ANY class was persisted
    // (structural observation-status check plus both class prefixes, so a
    // Denied-classed or raw-literal misleading error can't slip past a
    // Failed-prefix-only filter).
    activator
        .assert_no_error_shaped_tool_result()
        .await
        .map_err(|error| {
            format!(
                "activation over a revoked credential must park at the auth gate only, \
                 not persist an error-shaped tool result (#5878 misleading-error shape): {error}"
            )
        })?;

    activator.deny_auth_gate(run_id, &gate_ref).await?;
    activator
        .wait_for_status(run_id, TurnStatus::Completed)
        .await?;

    // ── Phase 2: retry over the STILL-revoked credential gates AGAIN ────────
    // The lifecycle.md authentication-failure arm: a denied re-auth gate must
    // not mark the requirement satisfied, and activation must not resume
    // merely because the user retried — with no new credential, the second
    // attempt parks at a fresh BlockedAuth gate (not one-shot, not a wedge,
    // not a silent success). Without this arm, phase 3's post-seed success
    // cannot distinguish "fresh credential unwedged it" from "the gate only
    // ever fires once".
    let retrier = g
        .thread("notion-reauth-retry")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_activate",
                json!({"extension_id": "notion"}),
            ),
            RebornScriptedReply::text("notion still needs reauthorization"),
        ])
        .build()
        .await?;
    let (retry_run_id, retry_gate_ref) = retrier
        .submit_turn_until_auth_blocked("set up notion again")
        .await?;
    let retry_state = retrier
        .wait_for_status(retry_run_id, TurnStatus::BlockedAuth)
        .await?;
    if !retry_state
        .credential_requirements
        .iter()
        .any(|requirement| requirement.provider == notion_provider)
    {
        return Err(format!(
            "a RETRIED activation over the still-revoked credential must re-open the \
             notion re-auth gate (one-shot gate = the #5878 wedge); got {:?}",
            retry_state.credential_requirements
        )
        .into());
    }
    retrier.assert_no_error_shaped_tool_result().await?;
    retrier
        .deny_auth_gate(retry_run_id, &retry_gate_ref)
        .await?;
    retrier
        .wait_for_status(retry_run_id, TurnStatus::Completed)
        .await?;

    // ── Phase 3: reconfigure (fresh credential) unwedges activation ─────────
    let restorer = g
        .thread("notion-reauth-restore")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_activate",
                json!({"extension_id": "notion"}),
            ),
            RebornScriptedReply::text("notion reauthorized"),
        ])
        .build()
        .await?;
    restorer
        .seed_capability_credential_account("notion", "itest notion reconfigure", &[])
        .await?;
    restorer.submit_turn("activate notion again").await?;
    restorer
        .assert_tool_result_contains("\"activated\":true")
        .await?;

    Ok(())
}
