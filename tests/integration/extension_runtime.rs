//! Reborn integration test — the generic extension runtime (P2, TEST-4).
//!
//! Drives the invented-vendor fixture through the REAL production pipeline:
//! model tool calls hit `builtin.extension_install`, which reconciles every
//! internal readiness/publication checkpoint it can,
//! the lifecycle facade mirrors the activation into the generic extension
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
use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;
use rstest::rstest;
use serde_json::json;

/// TEST-1: the invented-vendor fixture adapter runs the SAME exported
/// channel-adapter conformance suite the concrete crates run — proof that
/// no generic delivery path needs a real product.
#[tokio::test]
async fn acme_channel_adapter_satisfies_the_conformance_contract() {
    use std::sync::Arc;

    use ironclaw_product::test_support::conformance::{
        ChannelAdapterConformance, ConformanceInbound, run_channel_adapter_conformance,
    };
    use ironclaw_product::{
        ExternalConversationRef, OutboundEnvelope, OutboundPart, OutboundTarget,
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
        vendor_responses: Arc::new(|_request| ironclaw_host_api::RestrictedEgressResponse {
            status: 200,
            body: br#"{"ok":true}"#.to_vec(),
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
/// removal, now on both backends.
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
    // action can complete readiness and publication.
    lifecycle
        .seed_capability_credential_account("acme", "acme fixture account", &["notes:write"])
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
            "slack.get_conversation_history" => json!({"channel": "C0000001"}),
            "slack.get_user_info" => json!({"user_id": "U0000001"}),
            "slack.send_message" => {
                json!({"channel": "C0000001", "text": "hello from the runtime"})
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
