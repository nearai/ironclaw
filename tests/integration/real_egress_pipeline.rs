//! S1 — real egress pipeline under the harness.
//!
//! Every other `builtin.http` integration test scripts `RecordingRuntimeHttpEgress`,
//! which implements `RuntimeHttpEgress` directly and so bypasses the ENTIRE
//! production security pipeline (network-policy enforcement + leak scan).
//! `.with_real_egress_pipeline()` instead runs the REAL
//! `HostHttpEgressService` (leak scan) over the REAL `PolicyNetworkHttpEgress`
//! (network-policy enforcement, DNS/private-IP checks) — only the wire-level
//! transport (the would-be socket) is a recorder. See
//! `tests/integration/support/harness/assembly.rs`'s
//! `local_dev_host_runtime_with_real_egress_pipeline` for the production call
//! sites this mirrors (`ironclaw_host_runtime::egress::HostHttpEgressService::production`,
//! `ironclaw_network::PolicyNetworkHttpEgress`).

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod ironclaw_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use ironclaw_support::assertions::ToolErrorClass;
use ironclaw_support::builder::IronClawIntegrationHarness;
use ironclaw_support::reply::IronClawScriptedReply;
use serde_json::json;

/// Not in the harness's default `http_test_policy()` allowlist
/// (`api.example.test` only).
const DENIED_URL: &str = "https://evil.example.test/leak";
const ALLOWED_URL: &str = "https://api.example.test/v1/data";
/// AWS access-key fixture matching the real `LeakDetector`'s `aws_access_key`
/// pattern (`AKIA[0-9A-Z]{16}`, `LeakAction::Block`) — the AWS docs' own
/// canonical example key, so this is unambiguously a test fixture, not a
/// live credential.
const SEEDED_AWS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";

/// Real network-policy enforcement (not a scripted `policy_denied` error)
/// denies an out-of-allowlist host before the request ever reaches the
/// wire-level transport. Proves the REAL `PolicyNetworkHttpEgress` runs under
/// the harness, not just a stand-in that always says yes.
#[tokio::test]
async fn real_network_policy_denies_out_of_allowlist_host_before_transport() {
    let h = IronClawIntegrationHarness::test_default()
        .with_real_egress_pipeline()
        .script([
            IronClawScriptedReply::tool_call("builtin.http", json!({"url": DENIED_URL})),
            IronClawScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("fetch the denied url")
        .await
        .expect("turn completes");
    h.assert_tool_error(ToolErrorClass::Denied, "policy_denied")
        .await
        .expect("real policy enforcement surfaced a model-visible Denied tool error");
    h.assert_real_egress_transport_count(0)
        .await
        .expect("denied request must never reach the wire-level transport");
    h.assert_reply_contains("done")
        .await
        .expect("run recovered and finalized");
}

/// Real leak scan (not a scripted error) blocks a response whose body
/// contains a seeded secret, on an ALLOWED host. Proves the request DID clear
/// real network-policy enforcement and reach the transport, and that the real
/// `HostHttpEgressService` response leak scan is what stopped the secret from
/// reaching the model.
#[tokio::test]
async fn real_leak_scan_blocks_response_containing_seeded_secret() {
    let h = IronClawIntegrationHarness::test_default()
        .with_real_egress_response_bodies(
            [format!(r#"{{"note":"{SEEDED_AWS_KEY}"}}"#).into_bytes()],
        )
        .script([
            IronClawScriptedReply::tool_call("builtin.http", json!({"url": ALLOWED_URL})),
            IronClawScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("fetch the allowed url")
        .await
        .expect("turn completes");
    h.assert_real_egress_transport_count(1)
        .await
        .expect("allowed request must clear policy and reach the wire-level transport");
    h.assert_tool_error(ToolErrorClass::Failed, "operation_failed")
        .await
        .expect("real leak scan blocked the secret-bearing response as a Failed tool error");
    h.assert_reply_contains("done")
        .await
        .expect("run recovered and finalized");
}

/// Real-egress mode wires the inert `RecordingProcessPort` like the recording
/// mode: a scripted `builtin.shell` call is recorded by the port and no real
/// OS process is spawned. Distinct from `process_port.rs` (recording-egress
/// runtime) because this pins the SEPARATE real-egress runtime constructor.
#[tokio::test]
async fn real_egress_pipeline_shell_dispatches_through_inert_process_port() {
    let h = IronClawIntegrationHarness::test_default()
        .with_real_egress_pipeline()
        .script([
            IronClawScriptedReply::tool_call(
                "builtin.shell",
                json!({"command": "echo real-egress-probe"}),
            ),
            IronClawScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("run shell").await.expect("turn completes");
    h.assert_shell_command_recorded("real-egress-probe")
        .await
        .expect("command recorded by inert port, no real process spawned");
    h.assert_reply_contains("done")
        .await
        .expect("final reply finalized");
}

/// The wire-level transport recorder honors `response_body_limit` the way the
/// real `ReqwestNetworkTransport` does: an oversized scripted body surfaces
/// as a limit-truncated partial response (`response_bytes` = limit + 1, body
/// cut to the limit), which the real pipeline converts into a truncated tool
/// result — not a full-body success.
#[tokio::test]
async fn real_egress_transport_honors_response_body_limit() {
    let h = IronClawIntegrationHarness::test_default()
        .with_real_egress_response_bodies([vec![b'a'; 2048]])
        .script([
            IronClawScriptedReply::tool_call(
                "builtin.http",
                json!({"url": ALLOWED_URL, "response_body_limit": 1024}),
            ),
            IronClawScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("fetch the big body")
        .await
        .expect("turn completes");
    h.assert_real_egress_transport_count(1)
        .await
        .expect("request cleared policy and reached the wire-level transport");
    h.assert_tool_result_contains("\"response_bytes\":1025")
        .await
        .expect("limit-truncated partial response surfaced (limit + 1 bytes reported)");
    h.assert_reply_contains("done")
        .await
        .expect("final reply finalized");
}
