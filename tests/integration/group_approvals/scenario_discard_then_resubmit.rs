//! Part 2b-i (#5467 lane): int-tier coverage of `discard_pending`'s
//! reuse-blocking tombstone semantics through the harness's real, on-disk
//! `FilesystemApprovalRequestStore` instance. No int-tier test drove
//! `discard_pending` at all before this scenario
//! (`grep -rl discard tests/integration/` only matched a doubles-file doc
//! comment, zero test bodies).
//!
//! The only production caller of `discard_pending`
//! (`crates/ironclaw_capabilities/src/host.rs`'s rollback-on-partial-failure
//! path: `save_pending` succeeds, the immediately-following
//! `run_state.block_approval`/`block_auth` fails) fires on a crash-window
//! race that a normal scripted `submit_turn` can't naturally trigger without
//! new fault-injection harness surface (see the lane plan's option (a) vs
//! (b) discussion). This scenario takes the cheaper, still-genuine fallback:
//! drive `save_pending` -> `discard_pending` -> `save_pending` (same id)
//! directly against the group's real wired store -- reached via
//! `HostRuntimeCapabilityHarness::approval_requests_store()`, mirroring how
//! `local_dev_approval_test_parts()` exposes the same store elsewhere --
//! rather than a hand-built `InMemoryApprovalRequestStore`. This proves the
//! invariant through the actual production store instance the harness wired
//! (on-disk `FilesystemApprovalRequestStore`, regardless of the group's
//! `StorageMode`; see `scenario_approval_request_persists_after_reopen.rs`'s
//! C-DURABLE note), without needing to force the rollback race end-to-end.
//!
//! This test is expected to PASS today -- it documents already-correct
//! filesystem-parity behavior shipped in #5234, it does not pin a
//! currently-broken invariant (contrast with the crate-tier fix in this same
//! lane, which pins a fix to `InMemoryApprovalRequestStore`).

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use ironclaw_host_api::{
    Action, ApprovalRequest, ApprovalRequestId, CapabilityId, CorrelationId, ExtensionId,
    InvocationId, Principal, ResourceEstimate, ResourceScope,
};
use ironclaw_run_state::RunStateError;

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let capability_harness = g
        .capability_harness()
        .ok_or("live_approvals always uses HostRuntime")?;
    let approval_requests = capability_harness
        .approval_requests_store()
        .ok_or("live_approvals always wires a local-dev approval store")?;

    let scope =
        ResourceScope::local_default(capability_harness.user_id().clone(), InvocationId::new())?;
    let request_id = ApprovalRequestId::new();
    let first = ApprovalRequest {
        id: request_id,
        correlation_id: CorrelationId::new(),
        requested_by: Principal::Extension(ExtensionId::new("caller")?),
        action: Box::new(Action::Dispatch {
            capability: CapabilityId::new("builtin.write_file")?,
            estimated_resources: ResourceEstimate::default(),
        }),
        invocation_fingerprint: None,
        reason: "discard-then-resubmit coverage (#5467 lane)".to_string(),
        reusable_scope: None,
    };

    approval_requests.save_pending(scope.clone(), first).await?;
    approval_requests
        .discard_pending(&scope, request_id)
        .await?;

    // Non-vacuity: discard already hides the record from get()/records_for_scope()
    // even under the old delete-based behavior, so assert that separately from
    // the reuse-blocking invariant below (the actual point of this test).
    if approval_requests.get(&scope, request_id).await?.is_some() {
        return Err("discarded record must not be readable via get()".into());
    }

    let second = ApprovalRequest {
        id: request_id,
        correlation_id: CorrelationId::new(),
        requested_by: Principal::Extension(ExtensionId::new("caller")?),
        action: Box::new(Action::Dispatch {
            capability: CapabilityId::new("builtin.write_file")?,
            estimated_resources: ResourceEstimate::default(),
        }),
        invocation_fingerprint: None,
        reason: "reuse attempt after discard".to_string(),
        reusable_scope: None,
    };
    let err = approval_requests
        .save_pending(scope, second)
        .await
        .err()
        .ok_or("expected save_pending to fail closed on a discarded request id")?;
    if !matches!(
        err,
        RunStateError::ApprovalRequestAlreadyExists { request_id: id } if id == request_id
    ) {
        return Err(format!("expected ApprovalRequestAlreadyExists, got {err:?}").into());
    }

    Ok(())
}
