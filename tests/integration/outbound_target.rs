//! C-SYNTH outbound seam: the `outbound_target_tools` group surfaces the
//! local-dev synthetic `outbound_delivery_targets_list` and
//! `notification_channels_set` capabilities, dispatched through the REAL
//! production synthetic-capability wrap over an injected
//! `FakeOutboundPreferencesService` at the production-wired service trait seam.
//!
//! Covers the reachable model-visible routes: `targets_list` happy path (its
//! only reachable route — every service error is `driver_unavailable`);
//! `notification_channels_set` full-replace happy path; settings-`Deny` →
//! `Failed{policy_denied}`; foreign/overflow ids → `Failed{invalid_input}`;
//! approval gate `Ask` → approve applies the set / deny leaves it unchanged.
//!
//! The `policy_denied` and gate-`deny` arms were retargeted here from a
//! since-retired sibling preference-setting capability (which owned the only
//! coverage of those two routes) when it was deleted; that capability's other
//! three arms were dropped as duplicates of the `notification_channels_set`
//! cases beside them.
//!
//! Read-back through the SAME service double (`recorded_notification_channel_ids`)
//! proves a `Completed`/applied outcome actually reached the service seam — a
//! no-op set that still fabricated a success payload would leave it empty.

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
    // The host-owned `builtin:web_app` destination was retired with the
    // run-scoped routing stack: "keep it in the app" is now the ABSENCE of a
    // delivery call, not an addressable target. The fake facade mirrors
    // production, so the inventory is exactly the two seeded Slack targets.
    assert_eq!(
        targets.len(),
        2,
        "expected exactly the two seeded targets and no host-owned pseudo-target; saw {output}"
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
        !target_ids.contains(&"builtin:web_app"),
        "the retired host-owned web_app pseudo-target must not be addressable; saw {target_ids:?}"
    );
}

#[test]
fn notification_channels_set_disabled_by_settings_routes_to_policy_denied() {
    run_async_test_with_stack(
        "notification_channels_set_disabled_by_settings_routes_to_policy_denied",
        || async {
            let group = RebornIntegrationGroup::outbound_target_tools()
                .await
                .expect("outbound-target-tools group builds");
            let harness = group
                .thread("conv-notification-channels-denied")
                .script([
                    RebornScriptedReply::tool_call(
                        "builtin.notification_channels_set",
                        serde_json::json!({ "target_ids": [KNOWN_TARGET_ID] }),
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
                .disable_notification_channels_set_tool(
                    harness.binding.tenant_id.clone(),
                    harness.binding.actor_user_id.clone(),
                )
                .await
                .expect("tool override persists");

            harness
                .submit_turn("send my replies to slack dm alpha")
                .await
                .expect("turn completes despite the policy-denied notification_channels_set");

            harness
                .assert_tool_invoked("builtin.notification_channels_set")
                .await
                .expect(
                    "notification_channels_set dispatched through the synthetic-capability port",
                );
            harness
                .assert_tool_error(ToolErrorClass::Failed, "policy_denied")
                .await
                .expect("a disabled tool surfaces as Failed(PolicyDenied)");
            // A policy-denied dispatch must short-circuit before ever reaching the
            // service set seam — proves the deny happened at the settings-decision gate,
            // not merely that the model observed a policy_denied error string.
            let service = group
                .capability_harness()
                .expect("outbound_target_tools always uses HostRuntime")
                .outbound_preferences_service_for_test()
                .expect("outbound_target_tools always wires a service double");
            assert!(
                service.recorded_notification_channel_ids().is_empty(),
                "a policy-denied notification_channels_set must not reach the service set seam"
            );
        },
    );
}

#[tokio::test]
async fn notification_channels_set_approval_gate_deny_leaves_channels_unchanged() {
    let group = RebornIntegrationGroup::outbound_target_tools()
        .await
        .expect("outbound-target-tools group builds");
    let harness = group
        .thread("conv-notification-channels-gate-deny")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.notification_channels_set",
                serde_json::json!({ "target_ids": [KNOWN_TARGET_ID] }),
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
        .submit_turn_until_blocked("notify me in slack dm alpha")
        .await
        .expect("notification_channels_set raises a BlockedApproval gate");
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

    // A denied gate must short-circuit BEFORE the service set — the preference is
    // never applied.
    let service = group
        .capability_harness()
        .expect("outbound_target_tools always uses HostRuntime")
        .outbound_preferences_service_for_test()
        .expect("outbound_target_tools always wires a service double");
    assert!(
        service.recorded_notification_channel_ids().is_empty(),
        "a denied notification_channels_set must not reach the service set seam"
    );
}

#[tokio::test]
async fn notification_channels_set_replaces_and_reads_back() {
    let group = RebornIntegrationGroup::outbound_target_tools()
        .await
        .expect("outbound-target-tools group builds");
    let service = group
        .capability_harness()
        .expect("outbound_target_tools always uses HostRuntime")
        .outbound_preferences_service_for_test()
        .expect("outbound_target_tools always wires a service double");
    let harness = group
        .thread("conv-notification-channels-set")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.notification_channels_set",
                serde_json::json!({ "target_ids": [KNOWN_TARGET_ID, "slack:channel:beta"] }),
            ),
            RebornScriptedReply::text("notification channels updated"),
            RebornScriptedReply::tool_call(
                "builtin.notification_channels_set",
                serde_json::json!({ "target_ids": [] }),
            ),
            RebornScriptedReply::text("notifications cleared"),
        ])
        .build()
        .await
        .expect("thread builds");

    harness
        .submit_turn("notify me in slack dm alpha and channel beta")
        .await
        .expect("first turn completes");
    harness
        .assert_tool_invoked("builtin.notification_channels_set")
        .await
        .expect("notification_channels_set dispatched through the synthetic-capability port");
    // Assert the model-visible payload itself (not just the service double's
    // log) — proves both channels round-tripped through the capability's own
    // serialized response.
    let output = harness
        .tool_result_output("builtin.notification_channels_set")
        .await
        .expect("notification_channels_set recorded a capability result");
    let channels = output["channels"]
        .as_array()
        .expect("notification_channels_set output carries a `channels` array");
    let channel_ids: Vec<&str> = channels
        .iter()
        .map(|channel| {
            channel["target_id"]
                .as_str()
                .expect("each channel carries a string target_id")
        })
        .collect();
    assert_eq!(
        channel_ids,
        vec![KNOWN_TARGET_ID, "slack:channel:beta"],
        "tool result must echo back both applied channels; saw {output}"
    );
    assert!(
        channels.iter().all(
            |channel| channel["status"] == serde_json::json!("available")
                && channel["option"].is_object()
        ),
        "every applied channel must be available with a resolved option; saw {output}"
    );
    // Read-back through the SAME service double proves a `Completed`/applied
    // outcome actually reached the service seam — a no-op set that still
    // fabricated a success payload would leave this empty.
    assert_eq!(
        service.recorded_notification_channel_ids(),
        vec![
            KNOWN_TARGET_ID.to_string(),
            "slack:channel:beta".to_string()
        ],
        "the applied set must reach the service seam exactly once"
    );

    harness
        .submit_turn("actually clear my notification channels")
        .await
        .expect("second turn completes");
    harness
        .assert_tool_invoked("builtin.notification_channels_set")
        .await
        .expect("second notification_channels_set dispatched");
    let output = harness
        .tool_result_output("builtin.notification_channels_set")
        .await
        .expect("second notification_channels_set recorded a capability result");
    assert_eq!(
        output["channels"],
        serde_json::json!([]),
        "clearing with an empty list must read back empty; saw {output}"
    );
    assert!(
        service.recorded_notification_channel_ids().is_empty(),
        "the empty set must reach the service seam and clear the recorded state"
    );
}

#[tokio::test]
async fn notification_channels_set_rejects_foreign_and_overflow_ids() {
    let group = RebornIntegrationGroup::outbound_target_tools()
        .await
        .expect("outbound-target-tools group builds");
    let service = group
        .capability_harness()
        .expect("outbound_target_tools always uses HostRuntime")
        .outbound_preferences_service_for_test()
        .expect("outbound_target_tools always wires a service double");
    let harness = group
        .thread("conv-notification-channels-set-rejects")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.notification_channels_set",
                serde_json::json!({ "target_ids": [UNKNOWN_TARGET_ID] }),
            ),
            RebornScriptedReply::text("that channel isn't available"),
            RebornScriptedReply::tool_call(
                "builtin.notification_channels_set",
                serde_json::json!({
                    "target_ids": (0..=ironclaw_assistant::NOTIFICATION_CHANNELS_SET_MAX_ITEMS)
                        .map(|index| format!("slack:overflow:{index}"))
                        .collect::<Vec<_>>()
                }),
            ),
            RebornScriptedReply::text("that's too many channels"),
        ])
        .build()
        .await
        .expect("thread builds");

    harness
        .submit_turn("notify me somewhere that doesn't exist")
        .await
        .expect("turn completes despite the rejected foreign id");
    harness
        .assert_tool_invoked("builtin.notification_channels_set")
        .await
        .expect("notification_channels_set dispatched through the synthetic-capability port");
    harness
        .assert_tool_error(ToolErrorClass::Failed, "input_encode")
        .await
        .expect("an id outside the registry surfaces as Failed(InvalidInput)");
    assert!(
        service.recorded_notification_channel_ids().is_empty(),
        "a rejected foreign id must not reach the service set seam"
    );

    let baseline = harness.history_len().await.expect("history length");
    harness
        .submit_turn("notify me in nine different places")
        .await
        .expect("turn completes despite the rejected overflow list");
    harness
        .assert_tool_invoked("builtin.notification_channels_set")
        .await
        .expect("second notification_channels_set dispatched");
    harness
        .assert_tool_error_since(baseline, ToolErrorClass::Failed, "input_encode")
        .await
        .expect("more than the cap surfaces as Failed(InvalidInput)");
    assert!(
        service.recorded_notification_channel_ids().is_empty(),
        "a rejected overflow list must not reach the service set seam"
    );
}

#[tokio::test]
async fn notification_channels_set_approval_gate_approve_applies_channels() {
    // `builtin.notification_channels_set` is `PermissionMode::Ask`, so with
    // auto-approve off it must raise a real `BlockedApproval` gate and the
    // approve-resume must successfully claim a lease and apply the set —
    // proves the full gate-raise/replay-payload/approve/claim-lease dance
    // works for this capability (see `NotificationChannelsSetHandler`'s doc
    // comment in `runtime/local_dev/notification_channels_set.rs`). This does NOT exercise the
    // `builtin_capability_policy.toml` grant this task added: the harness's
    // `approve_gate` mints lease terms from its own test-only
    // `HostRuntimeCapabilityHarness::lease_approval_for` fixture, never from
    // `PolicyApprovalLeaseTermsProvider`/`BuiltinCapabilityPolicy`. The grant
    // IS required for the SEPARATE "Always Allow" persistent-approval path
    // (`PolicyApprovalLeaseTermsProvider::persistent_approval_allowed`,
    // reached only through `ApprovalInteractionService`'s always-allow
    // resolution, which this harness path never calls) — pinned by
    // `runtime::approval::tests::notification_channels_set_allows_persistent_approval`
    // in `crates/app/ironclaw_composition/src/runtime/approval.rs`.
    let group = RebornIntegrationGroup::outbound_target_tools()
        .await
        .expect("outbound-target-tools group builds");
    let harness = group
        .thread("conv-notification-channels-gate-approve")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.notification_channels_set",
                serde_json::json!({ "target_ids": [KNOWN_TARGET_ID] }),
            ),
            RebornScriptedReply::text("updated after approval"),
        ])
        .build()
        .await
        .expect("thread builds");
    harness
        .disable_auto_approve()
        .await
        .expect("auto-approve disabled");

    let (run_id, gate_ref) = harness
        .submit_turn_until_blocked("notify me in slack dm alpha")
        .await
        .expect("notification_channels_set raises a BlockedApproval gate");

    // §5.3 Stage 0: the local-dev synthetic approval producer persists a durable
    // `GateRecord` AT THE RAISE (`notification_channels_set.rs`'s
    // `gate_record_store.save`), keyed by the canonical
    // `GateRef::for_approval_request` — while the loop carries the routing
    // `gate:approval-{id}` ref. Both encodings must agree or a host-persisted
    // approval gate is unfindable by the product read model. Asserted BEFORE the
    // approve so a resume-path write cannot mask a missing raise-path save.
    // (Re-added here when the since-retired sibling capability — whose
    // crate-tier test owned the only coverage of this seam — was deleted; this
    // capability performs the same gate-raise dance.)
    {
        let capability = group
            .capability_harness()
            .expect("outbound_target_tools always uses HostRuntime");
        // Recovers the approval id from the routing ref via the same
        // `approval_request_id_from_gate_ref` the product read model uses.
        let (recovered_id, record_scope) = capability
            .approval_request_scope_for_test(&gate_ref)
            .expect("read model recovers the approval request id from the routing ref");
        let record_key = ironclaw_host_api::ids::GateRef::for_approval_request(recovered_id);
        let persisted = capability
            .gate_record_store()
            .expect("harness wires a durable gate-record store")
            .load(&record_scope, record_key)
            .await
            .expect("gate record load succeeds")
            .expect("the raise must have persisted a durable gate record");
        assert!(
            matches!(
                persisted,
                ironclaw_host_api::gate_record::GateRecord::Approval { .. }
            ),
            "persisted gate record is an approval record, got {persisted:?}"
        );
    }

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
        .expect("post-resume tool result must reflect the approved channel");

    let service = group
        .capability_harness()
        .expect("outbound_target_tools always uses HostRuntime")
        .outbound_preferences_service_for_test()
        .expect("outbound_target_tools always wires a service double");
    assert_eq!(
        service.recorded_notification_channel_ids(),
        vec![KNOWN_TARGET_ID.to_string()],
        "the approved channel set must reach the service seam after resume"
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
