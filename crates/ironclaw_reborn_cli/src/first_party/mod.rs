//! Binary-assembled first-party capability wiring (extension-runtime DEL-7).
//!
//! The CLI is the one generic-side crate allowed to link
//! `ironclaw_first_party_extensions`; it converts the concrete package inventory
//! into composition's neutral bundle set, supplies the concrete GSuite /
//! web-access capability handler registrars, and injects the Google-account
//! credential visibility policy. Composition receives all of this as input and
//! never names a concrete first-party extension.
//!
//! This module lives OUTSIDE `src/runtime/` because the CLI runtime tree is
//! banned from `use ironclaw_host_runtime::` (a dependency-boundary test); the
//! host-runtime / host-api / auth types these builders touch are reached through
//! `ironclaw_reborn_composition` facade re-exports.

use std::sync::Arc;

use anyhow::{Result, bail};
use ironclaw_reborn_composition::{
    FirstPartyHandlerRegistrar, FirstPartyPackageBundle, RuntimeCredentialAccountVisibilityPolicy,
};

mod bundles;
mod gsuite;
mod web_access;

pub(crate) use bundles::bundled_first_party_bundles;

/// The binary-assembled first-party capability handler registrars composition
/// installs into the shared registry.
pub(crate) fn bundled_first_party_registrars() -> Vec<Arc<dyn FirstPartyHandlerRegistrar>> {
    vec![
        Arc::new(gsuite::GsuiteFirstPartyRegistrar),
        Arc::new(web_access::WebAccessFirstPartyRegistrar),
    ]
}

/// The Google-account credential visibility policy composition injects on its
/// product-auth services (fail-closed default applies when absent).
pub(crate) fn first_party_credential_account_visibility_policy()
-> Arc<dyn RuntimeCredentialAccountVisibilityPolicy> {
    Arc::new(gsuite::GsuiteRuntimeCredentialAccountVisibilityPolicy)
}

/// Assert the neutral bundle set is non-empty at assembly time, so an
/// accidentally-empty inventory (which compiles) cannot silently drop every
/// first-party extension from the catalog.
pub(crate) fn assert_first_party_bundles_present(
    bundles: &[FirstPartyPackageBundle],
) -> Result<()> {
    if bundles.is_empty() {
        bail!(
            "the binary must inject the first-party package inventory; an empty bundle set silently \
             removes every first-party extension, trust grant, and vendor auth recipe"
        );
    }
    Ok(())
}
