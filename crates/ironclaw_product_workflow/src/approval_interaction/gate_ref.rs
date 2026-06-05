use ironclaw_host_api::ApprovalRequestId;
use ironclaw_turns::{GateRef, ReplyTargetBindingRef, SourceBindingRef};

use super::{ApprovalInteractionRejectionKind, approval_rejected};
use crate::binding_ref::{
    DEFAULT_BINDING_REF_RAW_MAX_BYTES, bounded_reply_target_binding_ref, bounded_source_binding_ref,
};
use crate::error::ProductWorkflowError;

const APPROVAL_GATE_PREFIX: &str = "gate:approval-";

pub fn is_approval_gate_ref(gate_ref: &GateRef) -> bool {
    gate_ref.as_str().starts_with(APPROVAL_GATE_PREFIX)
}

pub fn approval_gate_ref(request_id: ApprovalRequestId) -> Result<GateRef, ProductWorkflowError> {
    GateRef::new(format!("{APPROVAL_GATE_PREFIX}{request_id}"))
        .map_err(|_| approval_rejected(ApprovalInteractionRejectionKind::InvalidGateRef))
}

pub(super) fn approval_request_id_from_gate_ref(
    gate_ref: &GateRef,
) -> Result<ApprovalRequestId, ProductWorkflowError> {
    let Some(value) = gate_ref.as_str().strip_prefix(APPROVAL_GATE_PREFIX) else {
        return Err(approval_rejected(
            ApprovalInteractionRejectionKind::InvalidGateRef,
        ));
    };
    ApprovalRequestId::parse(value)
        .map_err(|_| approval_rejected(ApprovalInteractionRejectionKind::InvalidGateRef))
}

pub(super) fn approval_source_binding_ref(
    gate_ref: &GateRef,
) -> Result<SourceBindingRef, ProductWorkflowError> {
    bounded_source_binding_ref(
        "approval-src",
        gate_ref.as_str(),
        DEFAULT_BINDING_REF_RAW_MAX_BYTES,
    )
    .map_err(|_| approval_rejected(ApprovalInteractionRejectionKind::InvalidBindingRef))
}

pub(super) fn approval_reply_binding_ref(
    gate_ref: &GateRef,
) -> Result<ReplyTargetBindingRef, ProductWorkflowError> {
    bounded_reply_target_binding_ref(
        "approval-reply",
        gate_ref.as_str(),
        DEFAULT_BINDING_REF_RAW_MAX_BYTES,
    )
    .map_err(|_| approval_rejected(ApprovalInteractionRejectionKind::InvalidBindingRef))
}
