//! Module-owned factories for Reborn substrate crates.
//!
//! Each submodule is responsible for one substrate. A factory either:
//!
//! 1. Builds the substrate (when the crate is in the workspace), populating
//!    the matching field on [`RebornProductionServices`], or
//! 2. Gates on profile and surfaces
//!    [`RebornBuildError::SubstrateNotImplemented`] when the substrate has
//!    not yet merged and the selected profile requires it.
//!
//! This split lets the composition root land before every substrate crate is
//! merged. As each cutover-blocker PR lands, its factory swaps from a gate
//! to a real builder without changing the call sites in `lib.rs`.

pub(crate) mod agent_loop;
pub(crate) mod auth;
pub(crate) mod capabilities;
pub(crate) mod dispatcher;
pub(crate) mod events;
pub(crate) mod extensions;
pub(crate) mod filesystem;
pub(crate) mod memory;
pub(crate) mod network;
pub(crate) mod processes;
pub(crate) mod prompt_safety;
pub(crate) mod resources;
pub(crate) mod run_state;
pub(crate) mod secrets;
pub(crate) mod trust;
pub(crate) mod turns;

use crate::{RebornBuildError, RebornBuildInput};

/// Helper used by gate factories: surfaces `SubstrateNotImplemented` only when
/// the selected profile requires the full graph. Under
/// [`crate::RebornProfile::Disabled`] the caller short-circuits before reaching
/// here; under `LocalDev` the missing substrate is tolerated.
pub(crate) fn gate_substrate(
    input: &RebornBuildInput,
    service: &'static str,
) -> Result<(), RebornBuildError> {
    if input.profile.requires_full_graph() {
        return Err(RebornBuildError::SubstrateNotImplemented { service });
    }
    Ok(())
}
