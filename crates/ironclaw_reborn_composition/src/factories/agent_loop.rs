//! Reference `AgentLoopHost` facade factory.
//!
//! Tracked by #3016 (open as of 2026-04-29). Carries scoped turn /
//! checkpoint / event / transcript services that must be shared with
//! the [`turns`] coordinator (acceptance test #6 in #3026). Production
//! fails closed here until the issue ships.
//!
//! Builds against PR #3095 (`feat(reborn): add host runtime contract
//! facade`, open) — the same precursor the `TurnCoordinator` factory
//! consumes. That facade pins the upper-layer API so this factory
//! does not need to reach into dispatcher / runtime / process
//! internals directly.

use crate::{RebornBuildError, RebornBuildInput, RebornProductionServices};

pub(crate) fn build(
    input: &RebornBuildInput,
    _services: &mut RebornProductionServices,
) -> Result<(), RebornBuildError> {
    super::gate_substrate(input, "agent_loop_host")
}
