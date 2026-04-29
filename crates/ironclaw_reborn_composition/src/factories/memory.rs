//! Reborn memory document/search/version services factory.
//!
//! No `ironclaw_memory` substrate crate yet. The full memory composition
//! (document, search, version, layer, profile, seed services) is required
//! for cutover-readiness per #3026. Until the crate lands, Production fails
//! closed here.

use crate::{RebornBuildError, RebornBuildInput, RebornProductionServices};

pub(crate) fn build(
    input: &RebornBuildInput,
    _services: &mut RebornProductionServices,
) -> Result<(), RebornBuildError> {
    super::gate_substrate(input, "ironclaw_memory")
}
