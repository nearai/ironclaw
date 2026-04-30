//! Trust-class policy engine factory.
//!
//! `ironclaw_trust` is merged. The composition root wires
//! [`ironclaw_trust::HostTrustPolicy`] under every non-Disabled profile —
//! authorization, approvals, dispatcher, and the extension registry all
//! consume the resulting `TrustDecision` when they run.
//!
//! The wired policy starts with no `PolicySource` chain entries. An empty
//! chain returns the default decision (Sandbox / UserTrusted) for every
//! manifest, which is the safe fail-closed answer until the typed
//! settings layer selects bundled / admin / signed sources. That overlay
//! lands with the second composition phase
//! (`reborn.trust_policy.backend` in issue #3026's config-model section).
//!
//! The `extension_registry` ↔ `trust_policy` coupling rule in
//! [`crate::RebornProductionServices::validate`] (rule 6) makes the
//! pairing a build-time guarantee — a registry without a host trust
//! ceiling fails before traffic is served.

use std::sync::Arc;

use ironclaw_trust::HostTrustPolicy;

use crate::{RebornBuildError, RebornBuildInput, RebornProductionServices};

pub(crate) fn build(
    _input: &RebornBuildInput,
    services: &mut RebornProductionServices,
) -> Result<(), RebornBuildError> {
    let policy = Arc::new(HostTrustPolicy::empty());
    services.trust_policy = Some(policy);
    Ok(())
}
