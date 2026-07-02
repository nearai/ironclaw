//! E-AUTHGATE seam smoke test: a capability whose credential account resolves
//! to `AuthRequired` raises a real `TurnStatus::BlockedAuth` gate.
//!
//! Drives the production auth path end-to-end: scripted `github.*` tool call →
//! real credential-account injection (`FixedRuntimeCredentialAccountResolver`
//! returns `AuthRequired`) → `CapabilityObligationError::AuthRequired` → the
//! agent loop blocks the run at `BlockedAuth` with a `gate:auth-` ref. Nothing
//! is faked except the model at the vendor-SDK seam.

#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
mod support;

use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;

#[tokio::test]
async fn github_capability_with_auth_required_credential_raises_blocked_auth_gate() {
    let group = RebornIntegrationGroup::live_auth_gate()
        .await
        .expect("auth-gate group builds");
    let harness = group
        .thread("conv-auth-gate")
        .script([RebornScriptedReply::tool_call(
            "github.create_issue",
            serde_json::json!({"owner": "o", "repo": "r", "title": "t", "body": "b"}),
        )])
        .build()
        .await
        .expect("thread builds");

    let (_run_id, _gate_ref) = harness
        .submit_turn_until_auth_blocked("file an issue")
        .await
        .expect("run blocks on an auth gate");
    // `submit_turn_until_auth_blocked` already validates the `gate:auth-` prefix
    // and returns `Err` otherwise, so the `.expect` above is the real failure
    // point — no redundant assert needed here.
}
