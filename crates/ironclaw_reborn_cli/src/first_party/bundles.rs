//! Converts the concrete `ironclaw_first_party_extensions` package inventory
//! into composition's neutral [`FirstPartyPackageBundle`]s (extension-runtime
//! DEL-7). This is the one production spot allowed to name the concrete
//! inventory; composition consumes the neutral bundles as opaque data.

use ironclaw_first_party_extensions::is_gsuite_extension_id;
use ironclaw_first_party_extensions::packages::{PackageAssetContent, bundled_packages};
use ironclaw_reborn_composition::{
    ExtensionId, FirstPartyPackageAsset, FirstPartyPackageBundle, FirstPartyPackageOnboarding,
};

/// The GSuite family's catalog search aliases, folded into the neutral bundle so
/// composition search never special-cases a concrete id.
const GSUITE_SEARCH_ALIASES: &[&str] = &[
    "google",
    "gsuite",
    "g suite",
    "workspace",
    "google workspace",
];

/// Build the neutral first-party bundle set the binary injects onto the build
/// input. Every bundled package is converted; the real inventory must be
/// injected here or first-party extensions silently vanish from the catalog.
pub(crate) fn bundled_first_party_bundles() -> Vec<FirstPartyPackageBundle> {
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
                // #6442×#6520 reconciliation: the source `PackageBundle` no
                // longer carries a bespoke `oauth_setup` override (#6520 folded
                // first-party OAuth setup into the manifest credential
                // requirements); the manifest-derived requirement is authoritative.
                oauth_setup: None,
                trust_effects: bundle.trust_effects,
                search_aliases,
            }
        })
        .collect()
}
