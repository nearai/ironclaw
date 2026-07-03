//! C-SYNTH outbound seam: the `outbound_target_tools` group surfaces the two
//! local-dev synthetic `outbound_delivery_*` capabilities, and scripted tool
//! calls dispatch through the REAL production synthetic-capability wrap
//! (`wrap_local_dev_synthetic_capabilities` + `outbound_delivery_capabilities`)
//! over an injected `FakeOutboundPreferencesFacade` at the production-wired
//! facade trait seam.
//!
//! Covers the reachable model-visible (kind-A) routes the C-SYNTH spike
//! enumerated for these capabilities:
//! - `targets_list` happy path (its only reachable route — every facade error is
//!   kind-B `driver_unavailable`, so only the happy path is pinned).
//! - `target_set` happy path (settings decision `Allow` via default-ON
//!   auto-approve → facade succeeds).
//! - `target_set` settings-`Deny` → `Failed{policy_denied}` (a `Disabled` tool
//!   override, `outbound_delivery.rs:184`).
//! - `target_set` facade `NotFound` → `Failed{invalid_input}` (unknown target,
//!   `outbound_delivery.rs:212`).
//! - `target_set` approval gate: `Ask` (auto-approve disabled) → real
//!   `BlockedApproval` gate → approve → resume applies the preference; deny →
//!   resume leaves the preference unchanged.
//!
//! Read-back through the SAME facade double (`recorded_set_target_ids`) proves a
//! `Completed`/applied outcome actually reached the facade seam — a no-op set
//! that still fabricated a success payload would leave it empty.

#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
mod support;

use reborn_support::assertions::ToolErrorClass;
use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;

const KNOWN_TARGET_ID: &str = "slack:dm:alpha";
const UNKNOWN_TARGET_ID: &str = "slack:unknown:zzz";

#[tokio::test]
async fn targets_list_capability_dispatches_and_returns_targets() {
    let group = RebornIntegrationGroup::outbound_target_tools()
        .await
        .expect("outbound-target-tools group builds");
    let harness = group
        .thread("conv-outbound-list")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.outbound_delivery_targets_list",
                serde_json::json!({}),
            ),
            RebornScriptedReply::text("here are your delivery targets"),
        ])
        .build()
        .await
        .expect("thread builds");

    harness
        .submit_turn("list my delivery targets")
        .await
        .expect("turn completes");

    harness
        .assert_tool_invoked("builtin.outbound_delivery_targets_list")
        .await
        .expect("targets_list dispatched through the synthetic-capability port");
    harness
        .assert_tool_result_contains(KNOWN_TARGET_ID)
        .await
        .expect("targets_list returned the seeded target inventory");
}

#[tokio::test]
async fn target_set_capability_applies_preference_through_facade() {
    let group = RebornIntegrationGroup::outbound_target_tools()
        .await
        .expect("outbound-target-tools group builds");
    let harness = group
        .thread("conv-outbound-set")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.outbound_delivery_target_set",
                serde_json::json!({ "target_id": KNOWN_TARGET_ID }),
            ),
            RebornScriptedReply::text("updated your delivery target"),
        ])
        .build()
        .await
        .expect("thread builds");

    harness
        .submit_turn("send my replies to slack dm alpha")
        .await
        .expect("turn completes");

    harness
        .assert_tool_invoked("builtin.outbound_delivery_target_set")
        .await
        .expect("target_set dispatched through the synthetic-capability port");
    // Read-back through the SAME facade double: a no-op set that still fabricated
    // a success payload would leave `recorded_set_target_ids` empty.
    let facade = group
        .capability_harness()
        .expect("outbound_target_tools always uses HostRuntime")
        .outbound_preferences_facade_for_test()
        .expect("outbound_target_tools always wires a facade double");
    assert_eq!(
        facade.recorded_set_target_ids(),
        vec![KNOWN_TARGET_ID.to_string()],
        "the applied preference must reach the facade set seam exactly once"
    );
}

#[tokio::test]
async fn target_set_unknown_target_routes_to_invalid_input() {
    let group = RebornIntegrationGroup::outbound_target_tools()
        .await
        .expect("outbound-target-tools group builds");
    let harness = group
        .thread("conv-outbound-set-notfound")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.outbound_delivery_target_set",
                serde_json::json!({ "target_id": UNKNOWN_TARGET_ID }),
            ),
            RebornScriptedReply::text("that target isn't available"),
        ])
        .build()
        .await
        .expect("thread builds");

    harness
        .submit_turn("send my replies to an unknown target")
        .await
        .expect("turn completes despite the rejected target_set");

    harness
        .assert_tool_invoked("builtin.outbound_delivery_target_set")
        .await
        .expect("target_set dispatched through the synthetic-capability port");
    harness
        .assert_tool_error(ToolErrorClass::Failed, "invalid_input")
        .await
        .expect("an unknown target surfaces as Failed(InvalidInput)");
}

#[tokio::test]
async fn target_set_disabled_by_settings_routes_to_policy_denied() {
    let group = RebornIntegrationGroup::outbound_target_tools()
        .await
        .expect("outbound-target-tools group builds");
    let harness = group
        .thread("conv-outbound-set-denied")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.outbound_delivery_target_set",
                serde_json::json!({ "target_id": KNOWN_TARGET_ID }),
            ),
            RebornScriptedReply::text("that tool is disabled"),
        ])
        .build()
        .await
        .expect("thread builds");

    // Persist a `Disabled` per-tool override for the run's effective dispatch
    // user (the thread binding actor), driving the settings decision to Deny.
    group
        .capability_harness()
        .expect("outbound_target_tools always uses HostRuntime")
        .disable_outbound_target_set_tool(
            harness.binding.tenant_id.clone(),
            harness.binding.actor_user_id.clone(),
        )
        .await
        .expect("tool override persists");

    harness
        .submit_turn("send my replies to slack dm alpha")
        .await
        .expect("turn completes despite the policy-denied target_set");

    harness
        .assert_tool_invoked("builtin.outbound_delivery_target_set")
        .await
        .expect("target_set dispatched through the synthetic-capability port");
    harness
        .assert_tool_error(ToolErrorClass::Failed, "policy_denied")
        .await
        .expect("a disabled tool surfaces as Failed(PolicyDenied)");
}

#[tokio::test]
async fn target_set_approval_gate_approve_applies_preference() {
    let group = RebornIntegrationGroup::outbound_target_tools()
        .await
        .expect("outbound-target-tools group builds");
    let harness = group
        .thread("conv-outbound-gate-approve")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.outbound_delivery_target_set",
                serde_json::json!({ "target_id": KNOWN_TARGET_ID }),
            ),
            RebornScriptedReply::text("updated after approval"),
        ])
        .build()
        .await
        .expect("thread builds");
    // Disable auto-approve so the `Ask`-mode target_set raises a real gate.
    harness
        .disable_auto_approve()
        .await
        .expect("auto-approve disabled");

    let (run_id, gate_ref) = harness
        .submit_turn_until_blocked("send my replies to slack dm alpha")
        .await
        .expect("target_set raises a BlockedApproval gate");
    harness
        .approve_gate(run_id, &gate_ref)
        .await
        .expect("gate approved");
    harness
        .wait_for_status(run_id, ironclaw_turns::TurnStatus::Completed)
        .await
        .expect("run resumes to Completed after approval");

    let facade = group
        .capability_harness()
        .expect("outbound_target_tools always uses HostRuntime")
        .outbound_preferences_facade_for_test()
        .expect("outbound_target_tools always wires a facade double");
    assert_eq!(
        facade.recorded_set_target_ids(),
        vec![KNOWN_TARGET_ID.to_string()],
        "the approved preference must reach the facade set seam after resume"
    );
}

#[tokio::test]
async fn target_set_approval_gate_deny_leaves_preference_unchanged() {
    let group = RebornIntegrationGroup::outbound_target_tools()
        .await
        .expect("outbound-target-tools group builds");
    let harness = group
        .thread("conv-outbound-gate-deny")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.outbound_delivery_target_set",
                serde_json::json!({ "target_id": KNOWN_TARGET_ID }),
            ),
            RebornScriptedReply::text("okay, leaving it as-is"),
        ])
        .build()
        .await
        .expect("thread builds");
    harness
        .disable_auto_approve()
        .await
        .expect("auto-approve disabled");

    let (run_id, gate_ref) = harness
        .submit_turn_until_blocked("send my replies to slack dm alpha")
        .await
        .expect("target_set raises a BlockedApproval gate");
    harness
        .deny_gate(run_id, &gate_ref)
        .await
        .expect("gate denied");
    harness
        .wait_for_status(run_id, ironclaw_turns::TurnStatus::Completed)
        .await
        .expect("run resumes to Completed after denial");

    // A denied gate must short-circuit BEFORE the facade set — the preference is
    // never applied.
    let facade = group
        .capability_harness()
        .expect("outbound_target_tools always uses HostRuntime")
        .outbound_preferences_facade_for_test()
        .expect("outbound_target_tools always wires a facade double");
    assert!(
        facade.recorded_set_target_ids().is_empty(),
        "a denied target_set must not reach the facade set seam"
    );
}
