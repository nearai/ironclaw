use ironclaw_host_api::ids::ApprovalRequestId;
use ironclaw_host_api::turn::TurnGateRef;

use super::{ApprovalInteractionRejectionKind, approval_rejected};
use crate::error::ProductSurfaceFailure;

const APPROVAL_GATE_PREFIX: &str = "gate:approval-";

pub fn is_approval_gate_ref(gate_ref_str: &str) -> bool {
    gate_ref_str.starts_with(APPROVAL_GATE_PREFIX)
}

pub fn approval_gate_ref(
    request_id: ApprovalRequestId,
) -> Result<TurnGateRef, ProductSurfaceFailure> {
    TurnGateRef::new(format!("{APPROVAL_GATE_PREFIX}{request_id}"))
        .map_err(|_| approval_rejected(ApprovalInteractionRejectionKind::InvalidGateRef))
}

pub fn approval_request_id_from_gate_ref(
    gate_ref: &TurnGateRef,
) -> Result<ApprovalRequestId, ProductSurfaceFailure> {
    let Some(value) = gate_ref.as_str().strip_prefix(APPROVAL_GATE_PREFIX) else {
        return Err(approval_rejected(
            ApprovalInteractionRejectionKind::InvalidGateRef,
        ));
    };
    ApprovalRequestId::parse(value)
        .map_err(|_| approval_rejected(ApprovalInteractionRejectionKind::InvalidGateRef))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_approval_gate_ref_accepts_only_typed_approval_prefix() {
        let typed = approval_gate_ref(ApprovalRequestId::new()).expect("approval gate");
        let generic = TurnGateRef::new("gate:approve-slack").expect("generic gate");
        let adjacent = TurnGateRef::new("gate:approvalish-test").expect("adjacent gate");

        assert!(is_approval_gate_ref(typed.as_str()));
        assert!(!is_approval_gate_ref(generic.as_str()));
        assert!(!is_approval_gate_ref(adjacent.as_str()));
    }
}
