use ironclaw_turns::{GateRef, ReplyTargetBindingRef, SourceBindingRef};

use super::{AuthInteractionRejectionKind, auth_rejected};
use crate::binding_ref::{
    DEFAULT_BINDING_REF_RAW_MAX_BYTES, bounded_reply_target_binding_ref, bounded_source_binding_ref,
};
use crate::error::ProductWorkflowError;

const AUTH_GATE_REF: &str = "gate:auth";
const AUTH_GATE_PREFIX: &str = "gate:auth-";
const HOOK_AUTH_GATE_PREFIX: &str = "gate:hook-auth-";

pub fn is_auth_gate_ref(gate_ref: &GateRef) -> bool {
    let value = gate_ref.as_str();
    value == AUTH_GATE_REF
        || value.starts_with(AUTH_GATE_PREFIX)
        || value.starts_with(HOOK_AUTH_GATE_PREFIX)
}

pub(crate) fn auth_source_binding_ref(
    binding_id: &str,
) -> Result<SourceBindingRef, ProductWorkflowError> {
    bounded_source_binding_ref(
        "auth-interaction-src",
        binding_id,
        DEFAULT_BINDING_REF_RAW_MAX_BYTES,
    )
    .map_err(|_| auth_rejected(AuthInteractionRejectionKind::InvalidBindingRef))
}

pub(crate) fn auth_reply_binding_ref(
    binding_id: &str,
) -> Result<ReplyTargetBindingRef, ProductWorkflowError> {
    bounded_reply_target_binding_ref(
        "auth-interaction-reply",
        binding_id,
        DEFAULT_BINDING_REF_RAW_MAX_BYTES,
    )
    .map_err(|_| auth_rejected(AuthInteractionRejectionKind::InvalidBindingRef))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_auth_gate_ref_matches_only_auth_gate_shapes() {
        assert!(is_auth_gate_ref(&GateRef::new("gate:auth").unwrap()));
        assert!(is_auth_gate_ref(&GateRef::new("gate:auth-oauth").unwrap()));
        assert!(is_auth_gate_ref(
            &GateRef::new("gate:hook-auth-oauth").unwrap()
        ));
        assert!(!is_auth_gate_ref(
            &GateRef::new("gate:approval-123").unwrap()
        ));
        assert!(!is_auth_gate_ref(&GateRef::new("gate:other-auth").unwrap()));
    }
}
