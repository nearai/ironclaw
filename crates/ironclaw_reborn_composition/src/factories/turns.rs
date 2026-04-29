//! Kernel `TurnCoordinator` factory.
//!
//! Tracked by #3013. Owns thread/turn admission, one-active-run enforcement,
//! durable blocked-state coordination, cancellation orchestration, and
//! redacted progress events. Production cannot enforce one-active-run
//! semantics without it, so this gate fails closed until the issue ships.

use crate::{RebornBuildError, RebornBuildInput, RebornProductionServices};

pub(crate) fn build(
    input: &RebornBuildInput,
    _services: &mut RebornProductionServices,
) -> Result<(), RebornBuildError> {
    super::gate_substrate(input, "turn_coordinator")
}
