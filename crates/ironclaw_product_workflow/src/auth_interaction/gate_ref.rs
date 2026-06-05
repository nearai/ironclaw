use ironclaw_turns::GateRef;

const AUTH_GATE_REF: &str = "gate:auth";
const AUTH_GATE_PREFIX: &str = "gate:auth-";
const HOOK_AUTH_GATE_PREFIX: &str = "gate:hook-auth-";

pub fn is_auth_gate_ref(gate_ref: &GateRef) -> bool {
    let value = gate_ref.as_str();
    value == AUTH_GATE_REF
        || value.starts_with(AUTH_GATE_PREFIX)
        || value.starts_with(HOOK_AUTH_GATE_PREFIX)
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
