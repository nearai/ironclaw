//! Per-trigger delivery routing fails closed at the int tier: a
//! `builtin.trigger_create` carrying a `delivery_target_id` is rejected as
//! invalid input on a host with NO outbound delivery target providers
//! registered (this harness), and nothing is persisted.
//!
//! The ACCEPT path (a host that can resolve outbound targets) is covered at
//! the dispatch tier with a validating hook
//! (`builtin_trigger_create_with_delivery_target_persists_it_when_host_validates`,
//! `crates/ironclaw_host_runtime/tests/first_party_builtin_tools.rs`) and at
//! the composition tier against the real Slack provider + registry
//! (`slack_delivery.rs` driver tests,
//! `factory.rs::trigger_delivery_target_validation_resolves_through_the_outbound_registry`).
//! This harness deliberately wires no outbound-target provider — wiring one
//! here would be test-only wiring for a path production wires via
//! `build_slack_host_beta_mounts` — so the int tier owns the fail-closed arm.

use super::ironclaw_support::group::{HarnessResult, IronClawIntegrationGroup};
use super::ironclaw_support::reply::IronClawScriptedReply;
use ironclaw_turns::TurnStatus;
use serde_json::json;

/// Distinctive so a name collision with other scenarios' triggers is not a
/// concern (this group shares one trigger repository).
const ROUTED_TRIGGER_NAME: &str = "delivery-target-fail-closed-should-not-exist";

pub async fn run(g: &IronClawIntegrationGroup) -> HarnessResult<()> {
    let h = g
        .thread("conv-trigger-delivery-target-fail-closed")
        .script([
            IronClawScriptedReply::tool_call(
                "builtin.trigger_create",
                json!({
                    "name": ROUTED_TRIGGER_NAME,
                    "prompt": "summarize the day",
                    "schedule": {"kind": "once", "at": "2999-01-01T00:00:00", "timezone": "UTC"},
                    "delivery_target_id": "slack:personal-dm:T123:someone",
                }),
            ),
            IronClawScriptedReply::text("that delivery target is not available here"),
            IronClawScriptedReply::tool_call("builtin.trigger_list", json!({})),
            IronClawScriptedReply::text("listed"),
        ])
        .build()
        .await?;

    // Turn 1: the routed create dispatches, fails as recoverable invalid
    // input (not a terminal host error), and the run completes.
    let run_id = h
        .submit_turn("create a routed reminder for someone else's DM")
        .await?;
    h.assert_tool_invoked("builtin.trigger_create").await?;
    h.wait_for_status(run_id, TurnStatus::Completed).await?;

    // Turn 2: nothing was persisted — the routed trigger is absent from a
    // genuine trigger_list read over the same shared repository.
    h.submit_turn("list my triggers").await?;
    let listed = h.tool_result_output("builtin.trigger_list").await?;
    let triggers = listed["triggers"]
        .as_array()
        .ok_or("trigger_list output missing triggers array")?;
    if triggers
        .iter()
        .any(|t| t["name"] == json!(ROUTED_TRIGGER_NAME))
    {
        return Err(format!(
            "expected the routed trigger to be rejected fail-closed, but trigger_list \
             returned {listed}"
        )
        .into());
    }
    Ok(())
}
