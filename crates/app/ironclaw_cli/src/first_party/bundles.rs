//! Converts the concrete `ironclaw_extension_support` package inventory
//! into extension-host's neutral [`FirstPartyPackageBundle`]s (extension-runtime
//! DEL-7). This is the one production spot allowed to name the concrete
//! inventory; composition consumes the neutral bundles as opaque data.

use ironclaw_extension_host::{
    FirstPartyPackageAsset, FirstPartyPackageBundle, FirstPartyPackageOnboarding,
};
use ironclaw_extension_support::is_gsuite_extension_id;
use ironclaw_extension_support::packages::{PackageAssetContent, bundled_packages};
use ironclaw_host_api::ids::ExtensionId;

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
    let mut bundles: Vec<FirstPartyPackageBundle> = bundled_packages()
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
        .collect();
    // The web-push channel package carries a crate (adapter/codec/provider),
    // so its manifest is embedded crate-locally and bundled here — the binary
    // is the one place that links concrete package crates, and adding it to
    // `ironclaw_extension_support::packages` would grow the §11.2.7
    // cross-crate include inventory instead.
    bundles.push(FirstPartyPackageBundle {
        id: ironclaw_web_push::WEB_PUSH_EXTENSION_ID.to_string(),
        display_name: "Web UI".to_string(),
        manifest_toml: ironclaw_web_push_extension::MANIFEST.to_string(),
        assets: vec![FirstPartyPackageAsset {
            path: "manifest.toml".to_string(),
            bytes: ironclaw_web_push_extension::MANIFEST.as_bytes().to_vec(),
        }],
        onboarding: None,
        oauth_setup: None,
        trust_effects: None,
        search_aliases: Vec::new(),
    });
    bundles
}
