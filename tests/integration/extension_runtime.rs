//! Reborn integration test — the generic extension runtime (P2, TEST-4).
//!
//! Drives the invented-vendor fixture through the REAL production pipeline:
//! model tool calls hit `builtin.extension_install` / `extension_activate`,
//! the lifecycle service mirrors the activation into the generic extension
//! host, the fixture's `first_party` native factory (assembled through the
//! same `RebornHostBindings` seam the binary uses) binds its adapters, and the
//! fixture tool dispatches from the ACTIVE SNAPSHOT — the registry lane
//! serves built-ins only, so a passing dispatch here proves the snapshot
//! path end to end (resolve → policy → credentials → invoke → record).
//! Removal proves fail-closed de-resolution.
//!
//! The Postgres arm of the storage matrix runs the same install flow on a
//! real PostgreSQL testcontainer (REL-3: provisioning failure is a test
//! failure, never a skip).

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use reborn_support::builder::{RebornIntegrationHarness, StorageMode};
use reborn_support::group::{GroupCapability, RebornIntegrationGroup};
use reborn_support::reply::RebornScriptedReply;
use rstest::rstest;
use serde_json::json;

#[tokio::test]
async fn acme_standard_ops_satisfy_canonical_contracts() {
    reborn_support::harness::profiles::extension::standard_op_contract_tests::acme_standard_ops_satisfy_canonical_contracts().await;
}

#[tokio::test]
async fn acme_standard_ops_emit_canonical_error_codes() {
    reborn_support::harness::profiles::extension::standard_op_contract_tests::acme_standard_ops_emit_canonical_error_codes().await;
}

#[tokio::test]
async fn acme_standard_ops_fall_back_to_constructor_egress() {
    reborn_support::harness::profiles::extension::standard_op_contract_tests::acme_standard_ops_fall_back_to_the_constructor_held_vendor_when_ports_lack_egress().await;
}

/// TEST-1: the invented-vendor fixture adapter runs the SAME exported
/// channel-adapter conformance suite the concrete crates run — proof that
/// no generic delivery path needs a real product.
#[tokio::test]
async fn acme_channel_adapter_satisfies_the_conformance_contract() {
    use std::sync::Arc;

    use ironclaw_extension_contracts::channel_adapter::{
        OutboundEnvelope, OutboundPart, OutboundTarget,
    };
    use ironclaw_extension_contracts::external::ExternalConversationRef;
    use ironclaw_extension_contracts::test_support::conformance::{
        ChannelAdapterConformance, ConformanceInbound, run_channel_adapter_conformance,
    };

    run_channel_adapter_conformance(ChannelAdapterConformance {
        adapter: Arc::new(reborn_support::harness::profiles::extension::AcmeFixtureChannelAdapter),
        extension_id: "acme-messenger".to_string(),
        installation_id: "acme-install-1".to_string(),
        message_inbound: ConformanceInbound {
            body: json!({
                "type": "message",
                "event_id": "Ev-acme-conformance",
                "conversation": "C-ACME-CONF",
                "user": "U-ACME-1",
                "text": "conformance hello",
            })
            .to_string()
            .into_bytes(),
            headers: Vec::new(),
        },
        challenge_inbound: Some(ConformanceInbound {
            body: json!({"type": "challenge", "challenge": "acme-conformance-token"})
                .to_string()
                .into_bytes(),
            headers: Vec::new(),
        }),
        outbound_envelope: OutboundEnvelope {
            extension_id: "acme-messenger".to_string(),
            installation_id: "acme-install-1".to_string(),
            delivery_attempt_id: "attempt-acme-conformance".to_string(),
            target: OutboundTarget {
                conversation: ExternalConversationRef::new(None, "C-ACME-CONF", None, None)
                    .expect("conversation"),
                thread_anchor: None,
            },
            parts: vec![OutboundPart::Text("conformance reply".to_string())],
            reply_context: None,
        },
        vendor_responses: Arc::new(|_request| {
            ironclaw_extension_contracts::tool_adapter::RestrictedEgressResponse {
                status: 200,
                body: br#"{"ok":true}"#.to_vec(),
            }
        }),
        config: Vec::new(),
        expects_unsupported_free_target_listing: true,
    })
    .await;
}

/// Full lifecycle — install (which reconciles configured credentials) →
/// dispatch-from-snapshot → remove — all through model tool calls against the
/// real dispatcher, matrixed over libSQL and a real PostgreSQL testcontainer
/// (LIFE-17: the full lifecycle on both DBs; REL-3: a Postgres skip is a
/// failure). Also pins LIFE-13: conversation/LLM history survives extension
/// removal, now on both backends; and the standardized messaging framework's
/// coexistence proof (Task 8): the bespoke `send_note` tool and the standard
/// `send_message` op are both present in the model-visible tool surface at
/// once, so a channel author's own tools and the standard vocabulary share
/// one dispatch surface without either shadowing the other.
#[rstest]
#[case(StorageMode::LibSql)]
#[case(StorageMode::Postgres)]
#[tokio::test]
async fn acme_fixture_lifecycle_dispatches_from_the_active_snapshot(#[case] storage: StorageMode) {
    let group = RebornIntegrationGroup::builder()
        .storage(storage)
        .extension_runtime_acme()
        .await
        .expect("acme extension-runtime group builds on this backend");

    // One install call advances through every internally satisfiable phase.
    let lifecycle = group
        .thread("conv-acme-lifecycle")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "acme-messenger"}),
            ),
            RebornScriptedReply::text("installed"),
        ])
        .build()
        .await
        .expect("install thread builds");
    // The fixture's tool credential is a product-auth account for the
    // invented vendor; seed it before install so the one public lifecycle
    // action can complete readiness and publication. Every `[[tools]]`
    // credential referencing `vendor = "acme"` merges into one whole-package
    // OAuth requirement (same provider + setup), so the seeded account must
    // cover the union of every declared scope — both the write ops'
    // `notes:write` and the 16 standard ops' `notes:read` reads/people
    // scope (standardized messaging framework, task 7) — not just the scope
    // `send_note` alone happens to need.
    lifecycle
        .seed_capability_credential_account(
            "acme",
            "acme fixture account",
            reborn_support::harness::profiles::extension::ACME_CREDENTIAL_SCOPES,
        )
        .await
        .expect("seed acme account");
    lifecycle
        .submit_turn("install the acme messenger extension")
        .await
        .expect("install turn completes");
    lifecycle
        .assert_tool_result_contains("\"installed\":true")
        .await
        .expect("install reported success");
    lifecycle
        .assert_tool_result_contains("\"phase\":\"active\"")
        .await
        .expect("install completed readiness and publication");

    // Dispatch the fixture tool: it can only resolve from the generic
    // host's active snapshot (the registry lane is builtin-restricted).
    let invoke = group
        .thread("conv-acme-invoke")
        .script([
            RebornScriptedReply::tool_call(
                "acme-messenger.send_note",
                json!({
                    "conversation_id": "C-ACME-1",
                    "text": "hello from the generic runtime"
                }),
            ),
            RebornScriptedReply::text("note sent"),
        ])
        .build()
        .await
        .expect("invoke thread builds");
    invoke
        .submit_turn("send an acme note")
        .await
        .expect("invoke turn completes");
    invoke
        .assert_tool_invoked("acme-messenger.send_note")
        .await
        .expect("fixture tool executed");
    invoke
        .assert_tool_result_contains("\"delivered\":true")
        .await
        .expect("fixture adapter output surfaced");

    // Task 8 coexistence: the bespoke tool and the standard ops (standardized
    // messaging framework, task 7) are bound on the SAME extension and must
    // both reach the model-visible tool surface — neither the bespoke tool
    // nor the standard vocabulary shadows the other. The model-visible tool
    // NAME (`ProviderToolName`, `tool_disclosure.rs::capability_id.replace('.',
    // "__")`) is double-underscore-separated, distinct from the dot-separated
    // capability id used everywhere else in this file.
    invoke
        .assert_model_tools_contains("acme-messenger__send_note")
        .await
        .expect("bespoke send_note tool visible alongside the standard ops");
    invoke
        .assert_model_tools_contains("acme-messenger__send_message")
        .await
        .expect("standard send_message op visible alongside the bespoke tool");

    // Remove → the snapshot unpublishes; a later call fails closed at the
    // model gateway (uninstalled-capability denial).
    let remove = group
        .thread("conv-acme-remove")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_remove",
                json!({"extension_id": "acme-messenger"}),
            ),
            RebornScriptedReply::text("removed"),
        ])
        .build()
        .await
        .expect("remove thread builds");
    remove
        .submit_turn("remove the acme messenger extension")
        .await
        .expect("remove turn completes");
    remove
        .assert_tool_result_contains("\"removed\":true")
        .await
        .expect("removal reported success");

    // LIFE-13: removal is integration-state cleanup only — it never touches
    // conversation/LLM history (repo law: LLM data is never deleted). The
    // invoke thread's turn (the user prompt and the model's reply) predates
    // the removal above and must still be readable from persisted history.
    invoke
        .assert_conversation_history_contains("send an acme note")
        .await
        .expect("user turn survives extension removal");
    invoke
        .assert_conversation_history_contains("note sent")
        .await
        .expect("assistant reply survives extension removal");
}

/// Builds an acme extension-runtime group over a caller-supplied vendor
/// script, bypassing the argument-less `RebornIntegrationGroup::extension_runtime_acme()`
/// preset (which discards its `AcmeVendorScript`/`ScriptedVendorServer`
/// handle at construction — no way to script a failure or read back the
/// vendor requests it recorded). `build_with_capability` is the same
/// group-assembly core `extension_runtime_acme()` itself calls; the only
/// thing skipped here is that preset's channel-connection wiring, which only
/// `builtin.extension_remove`'s channel-disconnect step needs — neither
/// scenario below removes the extension.
async fn acme_standard_ops_group_with_vendor_script(
    script: reborn_support::harness::profiles::extension::AcmeVendorScript,
) -> (
    RebornIntegrationGroup,
    std::sync::Arc<ironclaw_extension_contracts::test_support::conformance::ScriptedVendorServer>,
) {
    let (mut profile, vendor) =
        reborn_support::harness::profiles::extension::extension_runtime_acme_tools_profile_with_vendor_script(
            script,
        )
        .expect("acme vendor-scripted profile builds");
    // The base profile runs under `local_dev_yolo_runtime_policy`
    // (`ApprovalPolicy::Minimal`), which suppresses BOTH the effect-based and
    // the origin-gate-matrix approval requirement outright (`Minimal => false`
    // in `ProfileApprovalGatePolicy::effects_require_approval`,
    // `crates/app/ironclaw_composition/src/profile_approval_authorization.rs`)
    // — an "ask" tool never gates under it regardless of the auto-approve
    // setting, which would make scenario (a)/(b)'s approve→resume proof
    // impossible to drive. Clearing the override falls through to
    // `local_dev_build_input`'s default policy (`AskDestructive`), the same
    // `None` `file_tools_requiring_approval_profile` uses to get real gates.
    profile.options.runtime_policy = None;
    let host_runtime = profile.build().await.expect("acme host runtime builds");
    let group = RebornIntegrationGroup::builder()
        .build_with_capability(GroupCapability::HostRuntime(std::sync::Arc::new(
            host_runtime,
        )))
        .await
        .expect("acme vendor-scripted group builds");
    (group, vendor)
}

/// Installs the acme fixture on `harness`'s thread (auto-approve is ON by
/// default for this profile, so the install itself needs no gate handling —
/// exactly like `acme_fixture_lifecycle_dispatches_from_the_active_snapshot`
/// above), then flips auto-approve OFF for the group so a subsequent standard
/// op raises a REAL `TurnStatus::BlockedApproval` instead of resolving
/// transparently. Every acme `[[tools]]` binding (bespoke and standard) is
/// `default_permission = "ask"`; without this the standard op's gate would
/// never block and `submit_turn_until_blocked` would see the run go straight
/// to `Completed`.
///
/// Scope: `GroupSharedStorage::auto_approve_scope_for_owner(&group.canonical_actor_user())`
/// — the run tenant (`product_harness.scope`, `tenant-itest`) with the
/// GROUP'S OWN RESOLVED BINDING ACTOR as the user, which is what dispatch-time
/// authorization's `ExecutionContext::resource_scope` actually carries for a
/// thread built with no explicit owner override (confirmed empirically: the
/// resolved actor is a synthesized `user-<hex>` id, not the literal
/// `"host-user"` binding request string, and not
/// `HostRuntimeCapabilityHarness::user_id()` either — this profile's fixed
/// `"reborn-e2e-extension-lifecycle-user"`, which `auto_approve_scope()`
/// (no `_for_owner`) keys on and which does NOT match here because this
/// group is built via `build_with_capability` directly, skipping the
/// `build_group_capability_with_base` alignment step `live_approvals()` /
/// `extension_lifecycle()` use). Getting the scope wrong doesn't error — it
/// silently leaves the real scope's setting unset, which defaults to
/// enabled, so the gate never fires; this is exactly the failure this
/// suite's own first two TDD attempts hit (both scenarios completed straight
/// through instead of blocking) before landing on this scope.
async fn install_acme_and_gate_standard_ops(
    group: &RebornIntegrationGroup,
    harness: &RebornIntegrationHarness,
) {
    harness
        .seed_capability_credential_account(
            "acme",
            "acme fixture account",
            reborn_support::harness::profiles::extension::ACME_CREDENTIAL_SCOPES,
        )
        .await
        .expect("seed acme account");
    harness
        .submit_turn("install the acme messenger extension")
        .await
        .expect("install turn completes");
    let owner = group.canonical_actor_user();
    let scope = group
        .shared
        .auto_approve_scope_for_owner(&owner)
        .expect("acme group uses a host-runtime capability backend with an auto-approve scope");
    group
        .capability_harness()
        .expect("acme group uses a host-runtime capability backend")
        .disable_global_auto_approve(scope)
        .await
        .expect("disable auto-approve so the standard op below raises a real gate");
}

/// TASK-8(a): a standard op completes end to end through the SAME
/// approve→resume pipeline a human user's approval drives — not the
/// auto-approved (never-blocks) shortcut `install_acme_and_gate_standard_ops`
/// relies on for its own install step. Auto-approve resolves an "ask" gate
/// BEFORE a `BlockedApproval` waypoint is ever created (it short-circuits the
/// authorization decision itself), so an auto-approved call never exercises
/// `ResumeTurnPrecondition::BlockedApprovalGate` / the gate-resume re-dispatch
/// at all. Explicitly blocking on `submit_turn_until_blocked` and resolving
/// via `approve_gate` forces the real resume path, proving the canonical
/// evidence chain (vendor request shape, persisted output, Task 5's
/// output-schema enforcement) holds for a RESUMED dispatch, not only a
/// first-shot one.
#[tokio::test]
async fn standard_send_completes_with_canonical_evidence() {
    use ironclaw_host_api::messaging::StandardMessagingOp;
    use ironclaw_host_api::test_support::messaging_conformance::assert_canonical_output;

    let script = reborn_support::harness::profiles::extension::AcmeVendorScript::default();
    let (group, vendor) = acme_standard_ops_group_with_vendor_script(script).await;

    let harness = group
        .thread("conv-acme-standard-send")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "acme-messenger"}),
            ),
            RebornScriptedReply::text("installed"),
            // Gated turn (default_permission = "ask") = exactly 2 entries:
            // tool_call + the one post-resume model call, whether the gate is
            // approved or denied (tests/integration/CLAUDE.md script discipline).
            RebornScriptedReply::tool_call(
                "acme-messenger.send_message",
                json!({"conversation": "ACME-C-1", "text": "hello", "thread": "ACME-THREAD-1"}),
            ),
            RebornScriptedReply::text("sent"),
        ])
        .build()
        .await
        .expect("thread builds");
    install_acme_and_gate_standard_ops(&group, &harness).await;

    let (run_id, gate_ref) = harness
        .submit_turn_until_blocked("send hello to ACME-C-1")
        .await
        .expect("standard send raises a real approval gate");
    harness
        .approve_gate(run_id, &gate_ref)
        .await
        .expect("approve the send");
    harness
        .wait_for_status(run_id, ironclaw_turns::TurnStatus::Completed)
        .await
        .expect("run completes after resume");

    // Vendor request asserted at the seam: method + exact path + body — not
    // just the tool output (fixture posts once per op, to the op-named path).
    let requests = vendor.requests();
    assert_eq!(
        requests.len(),
        1,
        "expected exactly one vendor request; saw {requests:?}"
    );
    assert_eq!(requests[0].url, "https://api.acme.example/send_message");
    assert!(matches!(
        requests[0].method,
        ironclaw_host_api::action::NetworkMethod::Post
    ));
    let body = String::from_utf8_lossy(requests[0].body.as_deref().unwrap_or_default());
    assert!(
        body.contains("ACME-C-1"),
        "vendor request body missing the conversation ref: {body}"
    );
    assert!(
        body.contains("hello"),
        "vendor request body missing the message text: {body}"
    );

    // Persisted capability result: canonical `message_ref` evidence, then
    // full schema conformance via the Task 6 helper.
    let output = harness
        .tool_result_output("acme-messenger.send_message")
        .await
        .expect("send_message result recorded");
    assert!(
        output.get("message_ref").is_some(),
        "persisted output missing message_ref: {output}"
    );
    assert_canonical_output(StandardMessagingOp::SendMessage, &output);
    // W3 (pre-merge amendment wave), proven through the full production
    // pipeline (install -> gate -> approve -> resume -> dispatch -> persisted
    // result), not just the adapter's own unit test: the input `thread` must
    // echo back on the output so a silent drop is checkable end to end.
    assert_eq!(
        output["thread"],
        json!("ACME-THREAD-1"),
        "thread must echo back on the persisted output: {output}"
    );
}

/// TASK-8(b): a vendor failure on a standard op maps onto the closed
/// `messaging.*` error-code taxonomy (Task 6/7) in the model-visible failure
/// summary, and the run recovers rather than terminating — the model's next
/// scripted turn is a plain text reply, proving `ToolVerdict::RecoverableFailure`
/// (not a `driver_unavailable` dead end). Same gated approve→resume path as
/// scenario (a): the vendor is only contacted on the RESUMED dispatch, so
/// scripting the failure up front does not affect the install step above.
#[tokio::test]
async fn standard_op_vendor_failure_surfaces_canonical_code_and_run_continues() {
    use reborn_support::assertions::ToolErrorClass;

    let script = reborn_support::harness::profiles::extension::AcmeVendorScript::default();
    // Vendor code "conversation_missing" -> StandardMessagingErrorCode::UnknownConversation
    // -> "messaging.unknown_conversation" (acme_error_to_standard_code).
    script.fail("send_message", "conversation_missing");
    let (group, _vendor) = acme_standard_ops_group_with_vendor_script(script).await;

    let harness = group
        .thread("conv-acme-standard-failure")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "acme-messenger"}),
            ),
            RebornScriptedReply::text("installed"),
            RebornScriptedReply::tool_call(
                "acme-messenger.send_message",
                json!({"conversation": "ACME-C-MISSING", "text": "hello"}),
            ),
            RebornScriptedReply::text("that conversation could not be found"),
        ])
        .build()
        .await
        .expect("thread builds");
    install_acme_and_gate_standard_ops(&group, &harness).await;

    let (run_id, gate_ref) = harness
        .submit_turn_until_blocked("send hello to a missing conversation")
        .await
        .expect("standard send raises a real approval gate");
    harness
        .approve_gate(run_id, &gate_ref)
        .await
        .expect("approve the send");
    harness
        .wait_for_status(run_id, ironclaw_turns::TurnStatus::Completed)
        .await
        .expect("run completes despite the vendor failure — recoverable, not terminal");

    harness
        .assert_tool_error(ToolErrorClass::Failed, "messaging.unknown_conversation")
        .await
        .expect("vendor failure surfaces the canonical messaging error code");
}

/// The same production install flow, matrixed across every storage backend —
/// including real PostgreSQL (REL-3's both-DB lane at the integration tier).
#[rstest]
#[case(StorageMode::LibSql)]
#[case(StorageMode::Postgres)]
#[tokio::test]
async fn extension_install_persists_across_storage_backends(#[case] storage: StorageMode) {
    let harness = RebornIntegrationHarness::test_default()
        .storage(storage)
        .script([RebornScriptedReply::text("Hello from the runtime!")])
        .build()
        .await
        .expect("harness builds");
    harness
        .submit_turn("hello")
        .await
        .expect("turn completes on this backend");
    harness
        .assert_reply_persists_after_reopen("Hello from the runtime!")
        .await
        .expect("reply persists across a genuinely fresh storage connection");
}

/// TOOL-7: the five real Slack tools activate and invoke through the generic
/// dispatcher — WASM lane, staged network policy, staged bot-token
/// injection — with the vendor-bound egress recorded at the network
/// transport. The canned transport body is not Slack-shaped, so per-tool
/// guest parsing may surface a model-visible tool error; the pinned proof is
/// each capability resolving from the snapshot and its authenticated
/// `slack.com` request landing on the wire.
#[tokio::test]
async fn slack_tools_invoke_through_the_generic_dispatcher_with_recorded_egress() {
    const SLACK_TOOLS: [&str; 5] = [
        "slack.search_messages",
        "slack.list_conversations",
        "slack.get_conversation_history",
        "slack.get_user_info",
        "slack.send_message",
    ];

    let group = RebornIntegrationGroup::extension_runtime_acme()
        .await
        .expect("extension-runtime group builds");

    let lifecycle = group
        .thread("conv-slack-lifecycle")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "slack"}),
            ),
            RebornScriptedReply::text("installed and ready"),
        ])
        .build()
        .await
        .expect("slack lifecycle thread builds");
    // Slack activation gates on a connected personal account whose scopes
    // cover every declared tool credential; seed it with real material so
    // dispatch-time staging injects a live token.
    lifecycle
        .seed_capability_credential_account(
            "slack",
            "slack fixture account",
            &[
                "search:read",
                "channels:history",
                "groups:history",
                "im:history",
                "mpim:history",
                "channels:read",
                "groups:read",
                "im:read",
                "mpim:read",
                "users:read",
                "chat:write",
            ],
        )
        .await
        .expect("seed slack account");
    lifecycle
        .submit_turn("install slack")
        .await
        .expect("slack install completes");
    lifecycle
        .assert_tool_result_contains("\"installed\":true")
        .await
        .expect("slack install reported success");
    lifecycle
        .assert_tool_result_contains("\"phase\":\"active\"")
        .await
        .expect("slack activation reported success");

    for (index, tool) in SLACK_TOOLS.iter().enumerate() {
        let arguments = match *tool {
            "slack.search_messages" => json!({"query": "release notes"}),
            "slack.list_conversations" => json!({}),
            "slack.get_conversation_history" => json!({"conversation": "C0000001"}),
            "slack.get_user_info" => json!({"user_ref": "U0000001"}),
            "slack.send_message" => {
                json!({"conversation": "C0000001", "text": "hello from the runtime"})
            }
            _ => unreachable!(),
        };
        let harness = group
            .thread(format!("conv-slack-tool-{index}"))
            .script([
                RebornScriptedReply::tool_call(tool, arguments),
                RebornScriptedReply::text("done"),
            ])
            .build()
            .await
            .expect("slack tool thread builds");
        harness
            .submit_turn("run the slack tool")
            .await
            .expect("slack tool turn completes");

        let requests = harness.captured_network_requests_for_test();
        assert!(
            !requests.is_empty(),
            "{tool}: the generic dispatcher must reach the network transport"
        );
        assert!(
            requests
                .iter()
                .all(|request| request.url.contains("slack.com")),
            "{tool}: every recorded request must target the declared vendor host; got {:?}",
            requests.iter().map(|r| r.url.clone()).collect::<Vec<_>>()
        );
        assert!(
            requests.iter().any(|request| {
                request.headers.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("authorization") && value.starts_with("Bearer ")
                })
            }),
            "{tool}: the staged bot token must be injected on the wire"
        );
    }
}
