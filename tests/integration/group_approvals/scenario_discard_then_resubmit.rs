//! Part 2b-i (#5467 lane): first int-tier coverage of `discard_pending`,
//! driven directly against the group's real wired `FilesystemApprovalRequestStore`
//! (via `approval_requests_store()`) rather than through a live `submit_turn`
//! rollback (no harness fault-injection seam exists for that race). Expected
//! to PASS today -- documents already-correct #5234 behavior, pins nothing.

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

    // Non-vacuity: discard must hide the record from get(), independent of
    // the reuse-block assertion below.
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
