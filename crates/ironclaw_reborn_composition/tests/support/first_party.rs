//! Shared integration-test helper: builds the neutral first-party bundle set
//! from the `ironclaw_first_party_extensions` dev-dependency and the production
//! trust policy over it (extension-runtime DEL-7).
//!
//! Composition no longer links first-party extensions in production, so
//! `builtin_first_party_trust_policy` is gated to composition's own unit tests.
//! Integration tests reach the concrete inventory directly through the
//! dev-dependency here and feed the same neutral bundle set the production
//! binary injects, then build the trust policy with the public
//! `production_first_party_trust_policy`.
//!
//! Included via `#[path = "support/first_party.rs"] mod first_party_support;`.

#![allow(dead_code)]

use ironclaw_first_party_extensions::is_gsuite_extension_id;
use ironclaw_first_party_extensions::packages::{PackageAssetContent, bundled_packages};
use ironclaw_reborn_composition::{
    ExtensionId, FirstPartyPackageAsset, FirstPartyPackageBundle, FirstPartyPackageOnboarding,
    production_first_party_trust_policy,
};
use ironclaw_trust::HostTrustPolicy;

const GSUITE_SEARCH_ALIASES: &[&str] = &[
    "google",
    "gsuite",
    "g suite",
    "workspace",
    "google workspace",
];

/// The neutral first-party bundle set — the same conversion the production
/// binary performs, mirrored here for tests.
pub(crate) fn test_first_party_bundles() -> Vec<FirstPartyPackageBundle> {
    bundled_packages()
        .into_iter()
        .map(|bundle| {
            let assets = bundle
                .assets
                .into_iter()
                .map(|asset| {
                    let PackageAssetContent::Bytes(bytes) = asset.content;
                    FirstPartyPackageAsset {
                        path: asset.path,
                        bytes,
                    }
                })
                .collect();
            let search_aliases = if ExtensionId::new(bundle.id)
                .map(|id| is_gsuite_extension_id(&id))
                .unwrap_or(false)
            {
                GSUITE_SEARCH_ALIASES
                    .iter()
                    .map(|alias| (*alias).to_string())
                    .collect()
            } else {
                Vec::new()
            };
            FirstPartyPackageBundle {
                id: bundle.id.to_string(),
                display_name: bundle.display_name.to_string(),
                manifest_toml: bundle.manifest_toml.into_owned(),
                assets,
                onboarding: bundle.onboarding.map(|copy| FirstPartyPackageOnboarding {
                    instructions: copy.instructions,
                    credential_instructions: copy.credential_instructions,
                    setup_url: copy.setup_url,
                    credential_next_step: copy.credential_next_step,
                }),
                // The source `PackageBundle` no longer carries a bespoke
                // `oauth_setup` override (#6520 folded it into the manifest).
                oauth_setup: None,
                trust_effects: bundle.trust_effects,
                search_aliases,
            }
        })
        .collect()
}

/// The production first-party trust policy over the neutral inventory — the
/// integration-test equivalent of the (now `#[cfg(test)]`-only)
/// `builtin_first_party_trust_policy`.
pub(crate) fn test_builtin_first_party_trust_policy() -> HostTrustPolicy {
    production_first_party_trust_policy(&test_first_party_bundles())
        .expect("first-party trust policy")
}
