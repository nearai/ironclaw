//! Runtime dispatcher factory.
//!
//! `ironclaw_dispatcher` is in flight on PR #3023, with WASM/Script/MCP lane
//! adapters following on PR #3027 and #3028. The dispatcher must register all
//! adapters under a shared obligation handler before traffic is served, so
//! Production fails closed until those PRs land.

use crate::{RebornBuildError, RebornBuildInput, RebornProductionServices};

pub(crate) fn build(
    input: &RebornBuildInput,
    _services: &mut RebornProductionServices,
) -> Result<(), RebornBuildError> {
    super::gate_substrate(input, "ironclaw_dispatcher")
}
