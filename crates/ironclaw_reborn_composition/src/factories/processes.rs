//! Process host factory.
//!
//! `ironclaw_processes` is in flight on PR #3017. Until it merges, Production
//! fails closed here. When the substrate lands, this module replaces the
//! gate with a real builder that shares its process / result / output stores
//! with the capability host (composition rule from #3026 acceptance test 6).

use crate::{RebornBuildError, RebornBuildInput, RebornProductionServices};

pub(crate) fn build(
    input: &RebornBuildInput,
    _services: &mut RebornProductionServices,
) -> Result<(), RebornBuildError> {
    super::gate_substrate(input, "ironclaw_processes")
}
