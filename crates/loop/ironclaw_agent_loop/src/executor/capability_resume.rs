use ironclaw_host_api::{
    ids::{ApprovalRequestId, CorrelationId},
    result_meta::ResumeToken,
    turn::LoopGateRef,
};
use ironclaw_loop_contracts::{
    AuthResumeApprovalIdentity, CapabilityApprovalResume, CapabilityAuthResume,
    CapabilityCallCandidate, CapabilityResumeToken,
};

use crate::state::LoopExecutionState;

pub(super) fn clear_matching_pending_auth_resume(
    state: &mut LoopExecutionState,
    call: &CapabilityCallCandidate,
) {
    if state
        .pending_auth_resume
        .as_ref()
        .is_some_and(|resume| resume.activity_id == call.activity_id)
    {
        state.pending_auth_resume = None;
    }
}

pub(super) fn clear_matching_pending_external_tool_resume(
    state: &mut LoopExecutionState,
    call: &CapabilityCallCandidate,
) {
    if state
        .pending_external_tool_resume
        .as_ref()
        .is_some_and(|resume| resume.activity_id == call.activity_id)
    {
        state.pending_external_tool_resume = None;
    }
}

pub(super) fn clear_matching_pending_approval_resume(
    state: &mut LoopExecutionState,
    call: &CapabilityCallCandidate,
) {
    if state
        .pending_approval_resume
        .as_ref()
        .is_some_and(|resume| resume.activity_id == call.activity_id)
    {
        state.pending_approval_resume = None;
    }
}

fn auth_resume_for_gate(
    gate_ref: &LoopGateRef,
    mut auth_resume: Option<CapabilityAuthResume>,
    prior_approval: Option<&CapabilityApprovalResume>,
) -> Option<CapabilityAuthResume> {
    let Some(prior_approval) = prior_approval else {
        return auth_resume;
    };

    let prior_identity = || AuthResumeApprovalIdentity {
        approval_request_id: prior_approval.approval_request_id,
        correlation_id: prior_approval.correlation_id,
    };

    match auth_resume.as_mut() {
        Some(resume) => {
            resume.resume_token = Some(prior_approval.resume_token.clone());
            resume.prior_approval.get_or_insert_with(prior_identity);
            auth_resume
        }
        None => Some(CapabilityAuthResume::resolved(
            gate_ref.clone(),
            prior_approval.resume_token.clone(),
            Some(prior_identity()),
        )),
    }
}

/// Reconstruct the byte-stable approval identity from the deterministic
/// `gate:approval-{id}` routing ref, so the fingerprinted approval lease claimed
/// on resume is identical to the pre-flip one.
fn approval_request_id_from_loop_gate_ref(gate_ref: &LoopGateRef) -> Option<ApprovalRequestId> {
    gate_ref
        .as_str()
        .strip_prefix("gate:approval-")
        // silent-ok: an invalid approval identifier makes this resume
        // reconstruction inapplicable; the caller safely re-prompts.
        .and_then(|id| ApprovalRequestId::parse(id).ok())
}

/// Reconstruct the loop-facing approval resume from the gate waypoint: the resume
/// token echoed back, the byte-stable approval id from the routing ref, the
/// call's own input ref (advisory — the host reconstitutes the authoritative one
/// from its replay store on resume), and a fresh correlation id (observability
/// only; not in the idempotency key or lease).
pub(super) fn approval_resume_from_gate(
    gate_ref: &LoopGateRef,
    resume_token: Option<&ResumeToken>,
    call: &CapabilityCallCandidate,
) -> Option<CapabilityApprovalResume> {
    // silent-ok: invalid or absent resume tokens cannot reconstruct an approval
    // resume; the caller safely re-prompts instead.
    let resume_token = CapabilityResumeToken::new(resume_token?.as_str()).ok()?;
    let approval_request_id = approval_request_id_from_loop_gate_ref(gate_ref)?;
    Some(CapabilityApprovalResume {
        approval_request_id,
        resume_token,
        correlation_id: CorrelationId::new(),
        input_ref: call.input_ref.clone(),
    })
}

/// Reconstruct the loop-facing auth resume from the gate waypoint's token, then
/// fold in any prior-approval identity (kept on the wire this slice; its host-side
/// move is deferred to §5.3 Stage 2a-ii).
pub(super) fn auth_resume_from_gate(
    gate_ref: &LoopGateRef,
    resume_token: Option<&ResumeToken>,
    prior_approval: Option<&CapabilityApprovalResume>,
) -> Option<CapabilityAuthResume> {
    let base = resume_token
        .and_then(|token| CapabilityResumeToken::new(token.as_str()).ok())
        .map(|resume_token| CapabilityAuthResume::resolved(gate_ref.clone(), resume_token, None));
    auth_resume_for_gate(gate_ref, base, prior_approval)
}
