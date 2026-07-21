//! Provider-instance readiness map ("config set" + restart early-fail),
//! two-phase scenario:
//!
//! **Phase 1** (unconfigured instance): activating a google-family extension
//! via the model tool path on an instance with NO Google OAuth backend
//! configured at all (this harness never wires one — the build-time gap, not
//! a per-account credential problem) must fail EARLY at the lifecycle tool
//! call with a model-visible remediation naming `config set google.client_id`,
//! run continuing to `Completed` — not park an unresolvable `BlockedAuth`
//! gate with zero remediation text.
//!
//! **Phase 2** (configured instance — the "config set" + restart arm): once
//! a Google OAuth backend IS configured, the same readiness check must NOT
//! fire — activation falls through to the ordinary per-account `BlockedAuth`
//! credential gate, matching `scenario_extension_activation_reauth_gate.rs`'s
//! shape. A real `config set` + service restart is a new process, not a live
//! toggle, so Phase 2 builds a genuinely SEPARATE
//! `RebornIntegrationGroup::extension_lifecycle_google_oauth_configured()`
//! composition rather than reusing `g` — the honest analog of "restart".
//! Phase 2 pins the no-false-positive contract: the readiness check must not
//! consume the ordinary per-account gate, so a configured instance with no
//! credential account still falls through to a real, renderable `BlockedAuth`
//! gate.
//!
//! Uses "google-calendar" for Phase 1 (not "gmail"): the readiness-map
//! chokepoint gates the "google" PROVIDER build-time-wide on `g` (not a
//! specific package), so no google-family extension can ever activate
//! normally on `g` regardless of a later per-account credential seed —
//! `scenario_uninstalled_tool_call_denied_until_activated` (formerly the
//! "gmail on `g`" scenario) now runs on its OWN isolated,
//! Google-OAuth-configured group for exactly this reason, so no ordering
//! dependency between it and this scenario remains. Phase 2 uses "gmail"
//! freely in its own, independent composition, where the readiness map has
//! no "google" entry at all.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use serde_json::json;

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    // Both phases always run, independent of each other's outcome: they pin
    // two different halves of the contract (early-fail, and no false
    // positive), so an early return via `?` would let the first failure mask
    // whether the second still holds. Running both names which one broke.
    let phase_1 = phase_1_unconfigured_instance_fails_early(g).await;
    let phase_2 = phase_2_configured_instance_falls_through_to_normal_gate().await;
    match (phase_1, phase_2) {
        (Ok(()), Ok(())) => Ok(()),
        (phase_1, phase_2) => Err(format!(
            "phase 1 (unconfigured instance): {}; phase 2 (configured instance): {}",
            phase_1
                .err()
                .map_or_else(|| "ok".to_string(), |e| e.to_string()),
            phase_2
                .err()
                .map_or_else(|| "ok".to_string(), |e| e.to_string()),
        )
        .into()),
    }
}

/// The exact string a host-authored remediation collapses to when it is routed
/// through the untrusted `SafeSummary` channel — the #6299 failure signature.
/// Asserted ABSENT everywhere below: the regression produced a completed run
/// with a plausible-looking error, so "an error was surfaced" is NOT a
/// discriminating assertion.
const DEGRADED_PLACEHOLDER: &str = "capability summary unavailable";

async fn phase_1_unconfigured_instance_fails_early(
    g: &RebornIntegrationGroup,
) -> HarnessResult<()> {
    let activator = g
        .thread("google-calendar-instance-not-configured")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "google-calendar"}),
            ),
            RebornScriptedReply::tool_call(
                "builtin.extension_activate",
                json!({"extension_id": "google-calendar"}),
            ),
            // Consumed by the post-Failed-result recovery model call — proof
            // the run continues past activation instead of parking on a gate
            // (RED today: this entry is never reached, `submit_turn` below
            // times out waiting for `Completed` with last status `BlockedAuth`).
            RebornScriptedReply::text("google-calendar needs instance configuration"),
        ])
        .build()
        .await?;

    // `submit_turn` waits for `TurnStatus::Completed` and fails fast on any
    // OTHER terminal status (a host-error regression) or times out with the
    // last-seen status on a non-terminal park — the single call proves both
    // "not parked BlockedAuth" and "not terminally failed by a host error".
    activator
        .submit_turn("install and activate google-calendar")
        .await?;

    // The readiness check's remediation text must reach the model verbatim
    // on the diagnostic-detail channel, landing inside the persisted
    // `ToolResultReference` envelope's `model_observation.detail`.
    activator
        .assert_conversation_history_contains("config set google.client_id")
        .await
        .map_err(|error| {
            format!(
                "activation on an unconfigured instance must surface a model-visible \
                 'config set google.client_id' remediation as a Failed tool result: {error}"
            )
        })?;

    // The other half of the assertion, and the one that catches #6299: the
    // remediation must not have DEGRADED. Without this, a run that collapsed
    // every host-authored string to the placeholder still passes any
    // "completed" or "an error was surfaced" check.
    activator
        .assert_conversation_history_lacks(DEGRADED_PLACEHOLDER)
        .await
        .map_err(|error| {
            format!(
                "host-authored remediation degraded to the safe-summary placeholder \
                 instead of reaching the model intact (#6299 regression): {error}"
            )
        })?;

    // The full multi-step guidance must arrive, not just the first line — the
    // restart step is what makes the remediation actionable.
    activator
        .assert_conversation_history_contains("ironclaw service restart")
        .await
        .map_err(|error| {
            format!("remediation must name the apply step, not just the config keys: {error}")
        })?;
    Ok(())
}

async fn phase_2_configured_instance_falls_through_to_normal_gate() -> HarnessResult<()> {
    // Fresh composition, Google OAuth backend registered — the "operator ran
    // config set and restarted" state, not a toggle on Phase 1's group.
    let configured_group =
        RebornIntegrationGroup::extension_lifecycle_google_oauth_configured().await?;
    let activator = configured_group
        .thread("gmail-instance-configured")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "gmail"}),
            ),
            RebornScriptedReply::tool_call(
                "builtin.extension_activate",
                json!({"extension_id": "gmail"}),
            ),
            RebornScriptedReply::text("gmail needs an account"),
        ])
        .build()
        .await?;

    let (run_id, gate_ref) = activator
        .submit_turn_until_auth_blocked("install and activate gmail")
        .await?;
    let state = activator
        .wait_for_status(run_id, ironclaw_turns::TurnStatus::BlockedAuth)
        .await?;
    // The readiness check must not consume the ordinary per-account gate: a
    // configured instance with no credential account still needs a real,
    // renderable auth gate (#6043 shape), the same contract
    // `scenario_extension_activation_reauth_gate` pins.
    if state.credential_requirements.is_empty() {
        return Err(
            "activation on a CONFIGURED instance with no credential account must still open \
             a real, renderable auth gate — the readiness check must not false-positive here"
                .into(),
        );
    }
    activator.assert_no_error_shaped_tool_result().await?;
    activator.deny_auth_gate(run_id, &gate_ref).await?;
    activator
        .wait_for_status(run_id, ironclaw_turns::TurnStatus::Completed)
        .await?;
    activator
        .assert_conversation_history_contains("gmail needs an account")
        .await
        .map_err(|error| {
            format!(
                "after denying the ordinary per-account gate the run must continue to the \
                 scripted recovery reply, not merely reach Completed: {error}"
            )
        })?;
    Ok(())
}
