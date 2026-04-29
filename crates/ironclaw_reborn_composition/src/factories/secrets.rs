//! Reborn typed secrets repository factory.
//!
//! There is no `ironclaw_secrets` substrate crate yet. The Reborn typed
//! `SecretRepository` (with Postgres / libSQL backends, `SecretHandle`
//! references, and post-secret re-resolution) is required by
//! `Production` so that no raw `SecretMaterial` ever reaches typed settings.
//! The legacy `crate::secrets` module on the binary is intentionally not
//! routed through here — bridging that lives in a follow-up.

use crate::{RebornBuildError, RebornBuildInput, RebornProductionServices};

pub(crate) fn build(
    input: &RebornBuildInput,
    _services: &mut RebornProductionServices,
) -> Result<(), RebornBuildError> {
    super::gate_substrate(input, "ironclaw_secrets")
}
