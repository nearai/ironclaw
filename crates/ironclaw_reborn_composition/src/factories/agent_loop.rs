//! Reference `AgentLoopHost` facade factory.
//!
//! Tracked by #3016. Carries scoped turn / checkpoint / event / transcript
//! services that must be shared with the [`turns`] coordinator (acceptance
//! test #6 in #3026). Production fails closed here until the issue ships.

use crate::{RebornBuildError, RebornBuildInput, RebornProductionServices};

pub(crate) fn build(
    input: &RebornBuildInput,
    _services: &mut RebornProductionServices,
) -> Result<(), RebornBuildError> {
    super::gate_substrate(input, "agent_loop_host")
}
