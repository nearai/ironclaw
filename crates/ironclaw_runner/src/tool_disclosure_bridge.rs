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
