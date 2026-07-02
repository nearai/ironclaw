//! E-AUTHGATE seam smoke test: a capability whose credential account resolves
//! to `AuthRequired` raises a real `TurnStatus::BlockedAuth` gate.
//!
//! Drives the production auth path end-to-end: scripted `github.*` tool call →
//! real credential-account injection (`FixedRuntimeCredentialAccountResolver`
//! returns `AuthRequired`) → `CapabilityObligationError::AuthRequired` → the
//! agent loop blocks the run at `BlockedAuth` with a `gate:auth-` ref. Nothing
//! is faked except the model at the vendor-SDK seam.
//!
//! ## #5416 Phase 3 tie-in
//!
//! This is also the load-bearing e2e coverage for the Phase 3 connection-state
//! description fold (`ironclaw_loop_support::capability_port::describe_with_access`).
//! `FixedRuntimeCredentialAccountResolver::account_configured` returning
//! `Ok(false)` for the `github` provider is exactly the Phase 2 signal that
//! downgrades `VisibleCapabilityAccess` to `NeedsAuth` for every `github.*`
//! capability on this harness's surface — so the real production chain
//! (`HostRuntimeLoopCapabilityPort::visible_capabilities` →
//! `LlmProviderModelGateway` → the real `ironclaw_llm` decorator chain) should
//! ship a model-visible tool description carrying the not-connected marker for
//! `github.create_issue`.
//!
//! This test was chosen over extending
//! `tests/reborn_group_extensions/scenario_search_ready_message_requires_credential.rs`
//! (the scenario named in the original Phase 3 plan): that scenario's group
//! (`RebornIntegrationGroup::extension_lifecycle`) seeds a generic
//! `CredentialOwnership::UserReusable` "google"/"github"/"notion" account at
//! harness construction (`seed_extension_lifecycle_credentials`) that a
//! `UserReusable` account satisfies for ANY requester extension of the same
//! provider — so `account_configured` resolves `true` for `gmail.*` capabilities
//! there regardless of the scenario's `Enabled`-but-uncredentialed extension
//! activation-state manipulation. That scenario's `"availability":"needs_auth"`
//! JSON field comes from a different, still-unconverged signal
//! (`ironclaw_product_workflow`'s per-installation credential-binding check —
//! see `docs/plans/2026-07-01-reborn-google-connection-state-5416.md`), not from
//! `VisibleCapabilityAccess`/`CapabilityCredentialPresence`. Extending it would
//! not actually exercise a `NeedsAuth` capability on the model's tool surface.
//! This `live_auth_gate` harness is the one that genuinely drives the
//! `CapabilityCredentialPresence` → `VisibleCapabilityAccess::NeedsAuth` path our
//! fold depends on, through the same production wiring
//! (`HostRuntimeServices::build_host_runtime`) real Reborn instances use.

#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
mod support;

use ironclaw_loop_support::CAPABILITY_NEEDS_AUTH_DESCRIPTION_MARKER;
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

    // #5416 Phase 3: the real decorator chain must have shipped the
    // `github.create_issue` tool definition to the model with the fixed
    // not-connected marker folded into its description — proving the
    // NeedsAuth→description fold reaches the actual model-visible tool list,
    // not just the host-side descriptor view.
    let captured = harness.captured_tool_definitions();
    let first_call_tools = captured
        .first()
        .expect("the scripted turn made at least one tool-bearing model call");
    let create_issue_tool = first_call_tools
        .iter()
        .find(|tool| tool.name.contains("create_issue"))
        .expect("github.create_issue is on the model-visible tool list");
    assert!(
        create_issue_tool
            .description
            .contains(CAPABILITY_NEEDS_AUTH_DESCRIPTION_MARKER),
        "github.create_issue's model-visible description must carry the \
         not-connected marker when its credential account is unconfigured; got: {:?}",
        create_issue_tool.description
    );
}
