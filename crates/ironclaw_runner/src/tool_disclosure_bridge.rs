//! Typed identity surface for the host-owned progressive-disclosure bridges.
//!
//! Production composition uses this module to reserve the same capability ids
//! that the runner synthesizes. The definitions and string literals remain
//! owned by the private disclosure implementation.

use ironclaw_host_api::CapabilityId;

/// The canonical synthetic bridge capability ids used by the runner.
pub fn bridge_capability_ids() -> impl Iterator<Item = CapabilityId> {
    crate::tool_disclosure::bridge_capability_ids()
}

#[cfg(test)]
mod tests {
    use super::bridge_capability_ids;

    #[test]
    fn bridge_capability_ids_exposes_exact_synthetic_bridge_set() {
        let capability_ids: Vec<String> = bridge_capability_ids()
            .map(|capability_id| capability_id.into_string())
            .collect();

        assert_eq!(
            capability_ids,
            vec![
                "ironclaw.tool_search".to_string(),
                "ironclaw.tool_describe".to_string(),
                "ironclaw.tool_call".to_string(),
            ]
        );
    }
}
