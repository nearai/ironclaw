const AUTH_GATE_REF: &str = "gate:auth";
const AUTH_GATE_PREFIX: &str = "gate:auth-";
const HOOK_AUTH_GATE_PREFIX: &str = "gate:hook-auth-";

pub fn is_auth_gate_ref(gate_ref_str: &str) -> bool {
    gate_ref_str == AUTH_GATE_REF
        || gate_ref_str.starts_with(AUTH_GATE_PREFIX)
        || gate_ref_str.starts_with(HOOK_AUTH_GATE_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_auth_gate_ref_matches_only_auth_gate_shapes() {
        assert!(is_auth_gate_ref("gate:auth"));
        assert!(is_auth_gate_ref("gate:auth-oauth"));
        assert!(is_auth_gate_ref("gate:hook-auth-oauth"));
        assert!(!is_auth_gate_ref("gate:approval-123"));
        assert!(!is_auth_gate_ref("gate:other-auth"));
    }
}
