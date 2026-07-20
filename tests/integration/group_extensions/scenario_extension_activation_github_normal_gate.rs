//! Negative case for the provider-instance readiness map: a provider with no
//! instance-config requirement (github has no entry in the readiness map —
//! only providers with an OPERATOR-level OAuth backend need one) must not
//! false-positive on the readiness check — activating it must still raise the
//! NORMAL per-account `BlockedAuth` gate. Pins CURRENT behavior; must stay
//! green after the readiness-map implementation lands.
//!
//! Uses "github", not telegram: telegram is feature-available here, but
//! empirically (verified against this exact harness) its activation resolves
//! through a SEPARATE `ExtensionAccountSetupRegistry`/pairing mechanism that
//! needs a live `AccountConnectionStatusSource` this bare harness never
//! mounts (`telegram/telegram_host_beta.rs`'s `connect()` call is a
//! production/serve-time wiring step) — so an unseeded activation here hits
//! a pre-existing, unrelated "host unavailable" error instead of the
//! per-account credential gate this test targets, and would misrepresent the
//! contract under test. github resolves through the SAME generic
//! product-auth credential-account mechanism as google/notion (the
//! mechanism `scenario_extension_activation_reauth_gate` already proves
//! raises a real `BlockedAuth` gate for an unsatisfied requirement), so it
//! is the honest in-catalog stand-in for a readiness-map-absent provider.
//! Runs on github's EXISTING install from Scenario 1 (never activated,
//! never credentialed there) — no fresh install, mirroring how Scenario 7
//! activates the same pre-existing install.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let activator = g
        .thread("github-normal-auth-gate")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_activate",
                serde_json::json!({"extension_id": "github"}),
            ),
            RebornScriptedReply::text("github needs a credential"),
        ])
        .build()
        .await?;

    let (run_id, gate_ref) = activator
        .submit_turn_until_auth_blocked("set up github")
        .await?;
    let state = activator
        .wait_for_status(run_id, ironclaw_turns::TurnStatus::BlockedAuth)
        .await?;
    if state.credential_requirements.is_empty() {
        return Err(
            "github activation must open a real, renderable auth gate (populated \
             credential_requirements), not an unsubmittable empty gate"
                .into(),
        );
    }
    // No config-set-shaped Failed tool result — a provider with no
    // instance-config entry must land on the ordinary auth gate, never the
    // readiness-check's diagnostic path.
    activator.assert_no_error_shaped_tool_result().await?;

    activator.deny_auth_gate(run_id, &gate_ref).await?;
    activator
        .wait_for_status(run_id, ironclaw_turns::TurnStatus::Completed)
        .await?;
    Ok(())
}
