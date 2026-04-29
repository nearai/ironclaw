//! Reborn network policy / hardened HTTP egress factory.
//!
//! No `ironclaw_network` substrate crate yet. Production cannot serve traffic
//! without a network policy backend, so this gate fails closed. When the
//! crate lands, this module wires the `NetworkPolicyStore` and a hardened
//! egress HTTP client behind it.

use crate::{RebornBuildError, RebornBuildInput, RebornProductionServices};

pub(crate) fn build(
    input: &RebornBuildInput,
    _services: &mut RebornProductionServices,
) -> Result<(), RebornBuildError> {
    super::gate_substrate(input, "ironclaw_network")
}
