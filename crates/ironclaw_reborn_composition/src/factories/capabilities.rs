//! Capability host factory.
//!
//! `ironclaw_capabilities` is not yet in the workspace. The host crate is
//! deferred from #2999 in the Reborn landing plan until the
//! extensions/processes dependencies are stable. Until it lands, Production
//! fails closed here so partial Reborn islands can never be exposed to
//! channels or routes.

use crate::{RebornBuildError, RebornBuildInput, RebornProductionServices};

pub(crate) fn build(
    input: &RebornBuildInput,
    _services: &mut RebornProductionServices,
) -> Result<(), RebornBuildError> {
    super::gate_substrate(input, "ironclaw_capabilities")
}
