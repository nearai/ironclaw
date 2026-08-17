//! A revoked linked session surfaces `AuthRequired` and parks the run;
//! re-linking resumes the SAME run and the parked capability re-dispatches.
//!
//! This is the lifecycle half of "the session died": the durable credential
//! account behind the link is flipped to `Revoked` (the shape an external
//! revocation takes — the user removed the device in the vendor's own settings),
//! and the assertion is that the next linked-account call does not silently
//! fail or run with the dead credential, but parks at `TurnStatus::BlockedAuth`
//! with a live gate the user can answer by linking again.
//!
//! Asserted at three seams: the parked run's gate state, the adapter's own call
//! log (no dispatch while revoked), and the resumed run's terminal status plus
//! a fresh adapter invocation after the re-link.

use serde_json::json;

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::harness::profiles::device_link::{
    LINKED_VENDOR_ID, LINKED_WHOAMI_CAPABILITY_ID, LinkedFixtureHandles,
};
use super::reborn_support::reply::RebornScriptedReply;

pub async fn run(
    group: &RebornIntegrationGroup,
    handles: &LinkedFixtureHandles,
) -> HarnessResult<()> {
    let thread = group
        .thread("conv-device-link-revoked")
        .script([
            RebornScriptedReply::tool_call(LINKED_WHOAMI_CAPABILITY_ID, json!({})),
            RebornScriptedReply::text("relinked and read back"),
        ])
        .build()
        .await?;

    // Link, then model an external revocation of that grant.
    thread
        .seed_capability_credential_account(LINKED_VENDOR_ID, "revocation subject", &[])
        .await?;
    thread
        .revoke_capability_credential_accounts(LINKED_VENDOR_ID)
        .await?;

    let baseline = handles.tools.calls().len();
    let (run_id, gate_ref) = thread
        .submit_turn_until_auth_blocked("who am I on telegram?")
        .await?;
    if handles.tools.calls().len() != baseline {
        return Err(
            "a revoked linked session must park before the adapter is reached, never dispatch"
                .into(),
        );
    }

    // Re-link: a fresh credential account under the same run scope, then resume
    // the parked run so the capability re-dispatches for real.
    thread
        .resolve_auth_gate_for_provider(run_id, &gate_ref, LINKED_VENDOR_ID, "relinked account")
        .await?;
    thread
        .wait_for_status(run_id, ironclaw_turns::TurnStatus::Completed)
        .await?;
    if handles.tools.calls().len() <= baseline {
        return Err("re-linking must let the parked linked-account call dispatch".into());
    }
    thread
        .assert_tool_invoked(LINKED_WHOAMI_CAPABILITY_ID)
        .await?;
    Ok(())
}
