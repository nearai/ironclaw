//! Link → linked tool call → unlink, end to end.
//!
//! Seams asserted, in order:
//!
//! 1. **Deployment configuration** — the MTProto application identity lands
//!    through the production administrator-configuration capability, so the
//!    linked-device surface is configured rather than fail-closed.
//! 2. **Activation** — installing the package publishes an active snapshot
//!    whose bindings satisfy `check_binding` for all three declared surfaces
//!    (channel + device-link auth + fifteen `standard_op` tools); the linked
//!    tools reach the model-visible tool surface.
//! 3. **Dispatch** — a scripted `telegram.whoami` call reaches
//!    `ToolAdapter::invoke` through `CapabilityHost` and its canonical output
//!    survives standard-op output validation back to the model.
//! 4. **Unlink** — revoking the linked credential account makes the next
//!    identical call park on an auth gate instead of dispatching, so the tool
//!    surface genuinely depends on a live link.

use serde_json::json;

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::harness::profiles::device_link::{
    LINKED_ADMIN_GROUP_ID, LINKED_EXTENSION_ID, LINKED_VENDOR_ID, LINKED_WHOAMI_CAPABILITY_ID,
    LinkedFixtureHandles, linked_admin_configuration_values,
};
use super::reborn_support::reply::RebornScriptedReply;

/// The composition owner the linked-account profile names — administrator
/// configuration is deployment-scoped and writes as this identity, not as the
/// capability executor.
const OPERATOR_USER_ID: &str = "reborn-e2e-extension-lifecycle-tools";

pub async fn run(
    group: &RebornIntegrationGroup,
    handles: &LinkedFixtureHandles,
) -> HarnessResult<()> {
    // 1. Deployment configuration, through the production capability.
    super::admin_config::replace_admin_configuration(
        group,
        OPERATOR_USER_ID,
        LINKED_ADMIN_GROUP_ID,
        0,
        linked_admin_configuration_values(),
    )
    .await?;

    // 2. Install → activate through the real lifecycle tool. The linked tools
    //    carry a `vendor = "telegram"` credential requirement, so the link
    //    (a real credential account minted through the production manual-token
    //    flow) is seeded first, exactly as the acme fixture seeds its own.
    let lifecycle = group
        .thread("conv-device-link-install")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": LINKED_EXTENSION_ID}),
            ),
            RebornScriptedReply::text("linked account ready"),
        ])
        .build()
        .await?;
    lifecycle
        .seed_capability_credential_account(LINKED_VENDOR_ID, "linked telegram account", &[])
        .await?;
    lifecycle.submit_turn("link my telegram account").await?;
    lifecycle
        .assert_tool_result_contains("\"installed\":true")
        .await?;
    lifecycle
        .assert_tool_result_contains("\"phase\":\"active\"")
        .await?;

    // 3. Dispatch a linked-account read from the published snapshot.
    let invoke = group
        .thread("conv-device-link-call")
        .script([
            RebornScriptedReply::tool_call(LINKED_WHOAMI_CAPABILITY_ID, json!({})),
            RebornScriptedReply::text("that is your linked account"),
        ])
        .build()
        .await?;
    invoke
        .seed_capability_credential_account(LINKED_VENDOR_ID, "linked telegram account", &[])
        .await?;
    invoke.submit_turn("who am I on telegram?").await?;
    invoke
        .assert_tool_invoked(LINKED_WHOAMI_CAPABILITY_ID)
        .await?;
    invoke
        .assert_tool_result_contains("Linked fixture account")
        .await?;
    // The adapter seam itself: exactly one linked-account invocation, and it
    // carried the run's own dispatching actor rather than a fixed fallback.
    let calls = handles.tools.calls();
    let whoami_calls: Vec<_> = calls
        .iter()
        .filter(|(capability_id, _)| capability_id == LINKED_WHOAMI_CAPABILITY_ID)
        .collect();
    if whoami_calls.len() != 1 {
        return Err(format!(
            "exactly one linked-account whoami must reach the adapter, saw {whoami_calls:?}"
        )
        .into());
    }
    if whoami_calls[0].1.is_empty() {
        return Err("the linked-account dispatch carried no resolved caller".into());
    }

    // 4. Unlink: revoke the linked credential account. The identical call must
    //    now park on an auth gate rather than reaching the adapter again.
    let baseline = handles.tools.calls().len();
    let unlinked = group
        .thread("conv-device-link-unlinked")
        .script([
            RebornScriptedReply::tool_call(LINKED_WHOAMI_CAPABILITY_ID, json!({})),
            RebornScriptedReply::text("you need to link telegram first"),
        ])
        .build()
        .await?;
    unlinked
        .revoke_capability_credential_accounts(LINKED_VENDOR_ID)
        .await?;
    let (run_id, gate_ref) = unlinked
        .submit_turn_until_auth_blocked("who am I on telegram?")
        .await?;
    if handles.tools.calls().len() != baseline {
        return Err("an unlinked caller's call must never reach the linked-account adapter".into());
    }
    // Leave the run resolved rather than parked, so the shared runtime carries
    // no dangling gate into the next scenario.
    unlinked.deny_auth_gate(run_id, &gate_ref).await?;
    unlinked
        .wait_for_status(run_id, ironclaw_turns::TurnStatus::Completed)
        .await?;
    Ok(())
}
