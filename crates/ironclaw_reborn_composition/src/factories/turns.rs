//! Kernel `TurnCoordinator` factory.
//!
//! Tracked by #3013 (open as of 2026-04-29). Owns thread/turn
//! admission, one-active-run enforcement, durable blocked-state
//! coordination, cancellation orchestration, and redacted progress
//! events. Production cannot enforce one-active-run semantics without
//! it, so this gate fails closed until the issue ships.
//!
//! The implementing PR builds against PR #3095
//! (`feat(reborn): add host runtime contract facade`, open) — that
//! facade provides the stable upper-layer API the coordinator wires
//! against without depending on dispatcher / runtime / process /
//! network internals directly.

use crate::{RebornBuildError, RebornBuildInput, RebornProductionServices};

pub(crate) fn build(
    input: &RebornBuildInput,
    _services: &mut RebornProductionServices,
) -> Result<(), RebornBuildError> {
    super::gate_substrate(input, "turn_coordinator")
}
