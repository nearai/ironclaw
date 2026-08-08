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

pub(crate) const COMPACT_GOOGLE_CAPABILITIES_ENABLED_ENV: &str =
    "IRONCLAW_COMPACT_GOOGLE_CAPABILITIES_ENABLED";

/// The compact Google capability ids, one per line in
/// `compact_google_capability_ids.txt` — the single source of truth shared
/// with the live-qa benchmark classifier
/// (`scripts/reborn_webui_v2_live_qa/run_live_qa.py`), so the runtime bundle
/// filter and the benchmark metric classifier cannot drift apart.
const COMPACT_GOOGLE_CAPABILITY_IDS_SOURCE: &str =
    include_str!("compact_google_capability_ids.txt");

fn compact_google_capability_ids() -> Vec<&'static str> {
    COMPACT_GOOGLE_CAPABILITY_IDS_SOURCE
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect()
}

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
pub(crate) fn bundled_first_party_bundles() -> anyhow::Result<Vec<FirstPartyPackageBundle>> {
    let compact_google_enabled = compact_google_capabilities_enabled_from_value(
        std::env::var(COMPACT_GOOGLE_CAPABILITIES_ENABLED_ENV)
            .ok()
            .as_deref(),
    );
    bundled_packages()
        .into_iter()
        .map(|bundle| {
            let is_gsuite = is_gsuite_extension_id(&ExtensionId::new(bundle.id)?);
            let manifest_toml = if is_gsuite {
                google_manifest_for_compact_capabilities(
                    &bundle.manifest_toml,
                    compact_google_enabled,
                )?
            } else {
                bundle.manifest_toml.into_owned()
            };
            let assets = bundle
                .assets
                .into_iter()
                .map(|asset| {
                    let PackageAssetContent::Bytes(bytes) = asset.content;
                    let bytes = if asset.path == "manifest.toml" {
                        manifest_toml.as_bytes().to_vec()
                    } else {
                        bytes
                    };
                    FirstPartyPackageAsset {
                        path: asset.path,
                        bytes,
                    }
                })
                .collect();
            let search_aliases = if is_gsuite {
                GSUITE_SEARCH_ALIASES
                    .iter()
                    .map(|alias| (*alias).to_string())
                    .collect()
            } else {
                Vec::new()
            };
            Ok(FirstPartyPackageBundle {
                id: bundle.id.to_string(),
                display_name: bundle.display_name.to_string(),
                manifest_toml,
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
            })
        })
        .collect()
}

fn compact_google_capabilities_enabled_from_value(value: Option<&str>) -> bool {
    !value.is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        )
    })
}

fn google_manifest_for_compact_capabilities(
    manifest_toml: &str,
    enabled: bool,
) -> anyhow::Result<String> {
    if enabled {
        return Ok(manifest_toml.to_string());
    }

    let mut manifest: toml::Value = toml::from_str(manifest_toml)?;
    if let Some(tools) = manifest
        .get_mut("tools")
        .and_then(toml::Value::as_array_mut)
    {
        let compact_ids = compact_google_capability_ids();
        tools.retain(|tool| {
            tool.get("id")
                .and_then(toml::Value::as_str)
                .is_none_or(|id| !compact_ids.contains(&id))
        });
    }
    Ok(toml::to_string(&manifest)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_google_flag_defaults_on_and_recognizes_false_values() {
        assert!(compact_google_capabilities_enabled_from_value(None));
        assert!(compact_google_capabilities_enabled_from_value(Some("true")));
        for value in ["0", "false", "NO", " off "] {
            assert!(!compact_google_capabilities_enabled_from_value(Some(value)));
        }
    }

    #[test]
    fn disabled_compact_google_capabilities_preserve_other_tools() {
        let manifest = r#"
            [[tools]]
            id = "gmail.list_messages"

            [[tools]]
            id = "gmail.fetch_message_summaries"
        "#;
        let filtered = google_manifest_for_compact_capabilities(manifest, false).unwrap();
        let parsed: toml::Value = toml::from_str(&filtered).unwrap();
        let ids = parsed["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool["id"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, ["gmail.list_messages"]);
    }
}
