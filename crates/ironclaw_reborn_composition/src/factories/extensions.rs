//! Extension registry factory.
//!
//! `ironclaw_extensions` is merged. The composition root wires an empty
//! [`ExtensionRegistry`] under every non-disabled profile. Production
//! discovery against a real `RootFilesystem` lives in a follow-up that
//! co-lands with the trust-class policy engine (#3012/#3043) so that
//! manifest trust assignment is gated by host policy rather than by
//! self-declared manifest fields.

use std::sync::Arc;

use ironclaw_extensions::ExtensionRegistry;

use crate::{RebornBuildError, RebornBuildInput, RebornProductionServices};

pub(crate) fn build(
    _input: &RebornBuildInput,
    services: &mut RebornProductionServices,
) -> Result<(), RebornBuildError> {
    services.extension_registry = Some(Arc::new(ExtensionRegistry::new()));
    Ok(())
}
