//! Extension registry factory.
//!
//! `ironclaw_extensions` is merged, and the trust-class policy engine
//! it gates against landed via PR #3043 (issue #3012, merged
//! 2026-04-29). The composition root wires an empty
//! [`ExtensionRegistry`] under every non-disabled profile and pairs it
//! with `factories::trust`'s `HostTrustPolicy::empty()`.
//!
//! Production discovery against a real `RootFilesystem` is the
//! remaining additive work: the registry needs a typed-settings
//! overlay that selects bundled / admin / signed sources for the trust
//! policy chain, then populates the registry against those. The
//! `extension_registry` ↔ `trust_policy` coupling rule (validate rule
//! 6) keeps the pairing a build-time guarantee.

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
