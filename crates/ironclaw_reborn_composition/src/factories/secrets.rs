//! Reborn typed secret store factory.
//!
//! `ironclaw_secrets` is merged with an in-memory reference backend. The
//! composition root wires [`ironclaw_secrets::InMemorySecretStore`] under
//! every non-Disabled profile so credential injection has a usable
//! `SecretStore` handle today.
//!
//! Durable backends (filesystem-encrypted, PG/libSQL-backed with the
//! AES-256-GCM master key sourced from the OS keychain) are deferred to a
//! later substrate PR. Until those exist, any profile that requires the
//! full graph (Production *or* MigrationDryRun) returns
//! [`crate::RebornBuildError::SubstrateNotImplemented`] with service
//! `durable_secret_store` — the in-memory backend is fine for
//! `LocalDev` only, per issue #3026 acceptance criterion #5 ("missing
//! required production services fail startup with sanitized actionable
//! diagnostics") and the contract that a dry run validates the
//! production-ready shape.
//!
//! Issue #3026 acceptance criterion #9 ("settings reference secret
//! handles only; raw secret material is never persisted in settings or
//! diagnostics") is enforced at the type level: `RebornSettings` carries
//! no `SecretMaterial`-bearing fields, and the test
//! `reborn_settings_carry_no_secret_material` in `src/config/reborn.rs`
//! locks the contract in.

use std::sync::Arc;

use ironclaw_secrets::InMemorySecretStore;

use crate::{RebornBuildError, RebornBuildInput, RebornProductionServices};

pub(crate) fn build(
    input: &RebornBuildInput,
    services: &mut RebornProductionServices,
) -> Result<(), RebornBuildError> {
    services.secret_store = Some(Arc::new(InMemorySecretStore::new()));

    if input.profile.requires_full_graph() {
        // In-memory secret material is fine for LocalDev only.
        // Production and MigrationDryRun both require a durable
        // backend so material survives restart and is recoverable from
        // a known durable source — and so a dry run validates the
        // production-ready secret store, not a dev-leaning one. The
        // factory for that backend lands in a follow-up PR.
        return Err(RebornBuildError::SubstrateNotImplemented {
            service: "durable_secret_store",
        });
    }

    Ok(())
}
