//! Prompt write-safety policy hook factory.
//!
//! Tracked by #3019 (open as of 2026-04-29; no implementing PR in
//! flight). Guards writes to files that can be injected into future
//! model prompts (AGENTS.md, USER.md, IDENTITY.md, SOUL.md,
//! HEARTBEAT.md, and similar). Production cannot run without this
//! hook because the existing protection in `src/workspace/mod.rs`
//! would no longer apply once Reborn services own the filesystem
//! path. Gate fails closed until the Reborn host-mediated hook ships.

use crate::{RebornBuildError, RebornBuildInput, RebornProductionServices};

pub(crate) fn build(
    input: &RebornBuildInput,
    _services: &mut RebornProductionServices,
) -> Result<(), RebornBuildError> {
    super::gate_substrate(input, "prompt_write_safety_policy")
}
