//! C-SYNTH outbound seam: the `outbound_target_tools` group surfaces the two
//! local-dev synthetic `outbound_delivery_*` capabilities, dispatched through
//! the REAL production synthetic-capability wrap over an injected
//! `FakeOutboundPreferencesFacade` at the production-wired facade trait seam.
//!
//! Covers the reachable model-visible routes: `targets_list` happy path (its
//! only reachable route — every facade error is `driver_unavailable`);
//! `target_set` happy path; settings-`Deny` → `Failed{policy_denied}`; facade
//! `NotFound` → `Failed{invalid_input}`; approval gate `Ask` → approve applies
//! the preference / deny leaves it unchanged.
//!
//! Read-back through the SAME facade double (`recorded_set_target_ids`) proves
//! a `Completed`/applied outcome actually reached the facade seam — a no-op set
//! that still fabricated a success payload would leave it empty.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
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

    let output = harness
        .tool_result_output("builtin.outbound_delivery_targets_list")
        .await
        .expect("targets_list recorded a capability result");
    let targets = output["targets"]
        .as_array()
        .expect("targets_list output carries a `targets` array");
    // #6520: the fake facade mirrors production's always-present host-owned
    // WebApp destination, so the inventory is the two seeded Slack targets
    // plus web_app.
    assert_eq!(
        targets.len(),
        3,
        "expected the two seeded targets plus the host-owned web_app; saw {output}"
    );
    let target_ids: Vec<&str> = targets
        .iter()
        .map(|target| {
            target["target"]["target_id"]
                .as_str()
                .expect("each target carries a string target_id")
        })
        .collect();
    assert!(
        target_ids.contains(&KNOWN_TARGET_ID),
        "expected {KNOWN_TARGET_ID:?} in the returned targets; saw {target_ids:?}"
    );
    assert!(
        target_ids.contains(&"slack:channel:beta"),
        "expected the second seeded target in the returned targets; saw {target_ids:?}"
    );
    assert!(
        target_ids.contains(&"builtin:web_app"),
        "expected the host-owned web_app destination in the returned targets; saw {target_ids:?}"
    );
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
    // Assert the model-visible payload itself, not just the facade double's
    // log — proves the preference round-tripped through the capability's own
    // serialized response, not merely that the facade was called.
    let output = harness
        .tool_result_output("builtin.outbound_delivery_target_set")
        .await
        .expect("target_set recorded a capability result");
    assert_eq!(
        output["final_reply_target"]["target_id"],
        serde_json::json!(KNOWN_TARGET_ID),
        "tool result must echo back the applied target id; saw {output}"
    );
    assert_eq!(
        output["final_reply_target_status"],
        serde_json::json!("available"),
        "tool result must report the applied target as available; saw {output}"
    );
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

#[test]
fn target_set_disabled_by_settings_routes_to_policy_denied() {
    run_async_test_with_stack(
        "target_set_disabled_by_settings_routes_to_policy_denied",
        || async {
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
            // A policy-denied dispatch must short-circuit before ever reaching the
            // facade set seam — proves the deny happened at the settings-decision gate,
            // not merely that the model observed a policy_denied error string.
            let facade = group
                .capability_harness()
                .expect("outbound_target_tools always uses HostRuntime")
                .outbound_preferences_facade_for_test()
                .expect("outbound_target_tools always wires a facade double");
            assert!(
                facade.recorded_set_target_ids().is_empty(),
                "a policy-denied target_set must not reach the facade set seam"
            );
        },
    );
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

    // Read the post-resume persisted tool result — proves the resumed dispatch
    // actually reached the model, not merely that the run reached `Completed`
    // (which a silently-dropped resume could also produce).
    harness
        .assert_tool_result_contains(KNOWN_TARGET_ID)
        .await
        .expect("post-resume tool result must reflect the approved target");

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

    // A bare `Completed` also matches a silent no-op/vanish bug. Pin the
    // gate-declined failure summary directly: `short_circuit_denied_resume`
    // surfaces this as a fixed host-authored planner summary, NOT the
    // `capability_denied_summary`/`capability_failed_summary` prefix wrapper
    // (those apply only when a capability itself returns Denied/Failed).
    // Mirrors the analogous assertion in `reborn_integration_auth_gate.rs`.
    harness
        .assert_tool_error_summary_contains("approval gate denied by user")
        .await
        .expect("a denied approval gate surfaces a model-visible gate-declined failure");

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

fn run_async_test_with_stack<F, Fut>(name: &'static str, test: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + 'static,
{
    let handle = std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio test runtime")
                .block_on(test());
        })
        .expect("spawn stack-sized test thread");
    if let Err(panic) = handle.join() {
        std::panic::resume_unwind(panic);
    }
}
