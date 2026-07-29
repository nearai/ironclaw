use ironclaw_extensions::{ExtensionRuntimeV2, ManifestSource};

use ironclaw_extension_host::{
    AvailableExtensionPackage, parse_imported_manifest, registry_extension_package,
};

use super::catalog::validate_hub_name;
use super::model::{
    GENERIC_TOOL_INPUT_SCHEMA, GENERIC_TOOL_OUTPUT_SCHEMA, IronHubCommandError, IronHubToolEntry,
};

/// Assemble a registry tool package around the manifest the registry published.
///
/// The manifest is authored where `capabilities.json` is authored, so a tool's
/// credentials, its auth recipe, and its account-setup copy arrive as data
/// instead of being reconstructed here from a schema this crate does not own.
/// Every earlier reconstruction lost fields silently — first the credentials,
/// then the `[auth.<vendor>]` recipe they referenced, then the OAuth versus
/// API-key distinction — and each loss surfaced as an install that could not
/// authenticate.
///
/// The manifest also chooses where its own assets live: the wasm and the two
/// generic tool schemas are placed at the paths it declares, so publisher and
/// host never have to agree a filename convention across two repositories.
pub(crate) fn ironhub_tool_package(
    entry: &IronHubToolEntry,
    manifest: Vec<u8>,
    wasm: Vec<u8>,
    capabilities: Vec<u8>,
    reserved_bundled_ids: &[String],
) -> Result<AvailableExtensionPackage, IronHubCommandError> {
    validate_hub_name(&entry.name)?;
    let manifest_toml =
        String::from_utf8(manifest).map_err(|error| IronHubCommandError::Catalog {
            reason: format!(
                "'{}' published a manifest that is not UTF-8: {error}",
                entry.name
            ),
        })?;

    // Parsed here only to learn the paths it declares. `registry_extension_package`
    // parses it again as the authoritative validation; this pass decides nothing.
    let record = parse_imported_manifest(&manifest_toml, ManifestSource::RegistryInstalled)
        .map_err(IronHubCommandError::Product)?;

    let mut files = vec![
        ("manifest.toml".to_string(), manifest_toml.into_bytes()),
        ("legacy/capabilities.json".to_string(), capabilities),
    ];
    if let ExtensionRuntimeV2::Wasm { module } = &record.manifest().runtime {
        files.push((module.clone(), wasm));
    }
    // Registry tools expose one generic invoke capability whose input and output
    // schemas are host-owned constants; the manifest only says where they live.
    for capability in &record.manifest().capabilities {
        files.push((
            capability.input_schema_ref.as_str().to_string(),
            GENERIC_TOOL_INPUT_SCHEMA.to_vec(),
        ));
        if let Some(output_schema_ref) = &capability.output_schema_ref {
            files.push((
                output_schema_ref.as_str().to_string(),
                GENERIC_TOOL_OUTPUT_SCHEMA.to_vec(),
            ));
        }
    }

    let package = registry_extension_package(files, reserved_bundled_ids)
        .map_err(IronHubCommandError::Product)?;

    // The catalog entry names the tool the user asked for; the manifest declares
    // the id it installs as. They are covered by the same signature, so a
    // mismatch is a publishing mistake rather than an attack — but installing
    // under an id the user did not ask for would shadow an unrelated extension,
    // so it fails here rather than resolving in the manifest's favour.
    let installed_id = package.package.id.as_str();
    if installed_id != entry.name {
        return Err(IronHubCommandError::Catalog {
            reason: format!(
                "catalog lists '{}' but its published manifest declares id '{installed_id}'",
                entry.name
            ),
        });
    }

    Ok(package)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{IronHubArtifact, IronHubProvenance};

    /// A real WASI component; `registry_extension_package` rejects core modules.
    fn component() -> Vec<u8> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../ironclaw_first_party_extensions/assets/github/wasm/github_tool.wasm"),
        )
        .expect("github component fixture")
    }

    fn artifact() -> IronHubArtifact {
        IronHubArtifact {
            url: "https://hub.ironclaw.com/t".to_string(),
            size_bytes: 1,
            sha256: "a".repeat(64),
        }
    }

    fn entry_named(name: &str) -> IronHubToolEntry {
        IronHubToolEntry {
            name: name.to_string(),
            version: "0.1.0".to_string(),
            description: "test tool".to_string(),
            provenance: IronHubProvenance::Official,
            wasm: artifact(),
            capabilities: artifact(),
            manifest: Some(artifact()),
        }
    }

    /// Shaped like what `scripts/generate-extension-manifest.py` publishes, down
    /// to the asset paths, so this pins the contract that repository emits.
    fn published_manifest(id: &str, auth: &str) -> Vec<u8> {
        format!(
            r#"schema_version = "reborn.extension_manifest.v3"
id = "{id}"
name = "{id}"
version = "0.1.0"
description = "published tool"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/{id}-tool.wasm"

[[tools]]
origin_gate_matrix = {{ loop_run = "gated_unless_granted", product = "forbidden", automation = "forbidden" }}
id = "{id}.invoke"
description = "published tool"
effects = ["network", "use_secret"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/{id}/invoke.input.v1.json"
output_schema_ref = "schemas/{id}/raw_output.v1.json"

[[tools.credentials]]
handle = "{id}_api_key"
vendor = "{id}"
audience = {{ scheme = "https", host = "api.{id}.com" }}
injection = {{ type = "header", name = "authorization", prefix = "Bearer " }}
{auth}"#
        )
        .into_bytes()
    }

    fn api_key_auth(id: &str, extra: &str) -> String {
        format!(
            r#"
[auth.{id}]
method = "api_key"
display_name = "{id}"
fields = [ {{ handle = "{id}_api_key", label = "API key", secret = true }} ]
{extra}"#
        )
    }

    fn build(entry: &IronHubToolEntry, manifest: Vec<u8>) -> AvailableExtensionPackage {
        ironhub_tool_package(entry, manifest, component(), b"{}".to_vec(), &[])
            .expect("package builds")
    }

    /// The published manifest is what installs, byte for byte. Nothing in the
    /// host rewrites, augments, or regenerates it.
    #[test]
    fn the_published_manifest_is_what_installs() {
        let manifest = published_manifest("attio", &api_key_auth("attio", ""));
        let package = build(&entry_named("attio"), manifest.clone());

        assert_eq!(
            package.manifest_toml,
            String::from_utf8(manifest).expect("utf8")
        );
    }

    /// Assets land where the manifest says, not at a convention this crate
    /// invents — so the publisher can rename its wasm without a host release.
    #[test]
    fn assets_land_at_manifest_declared_paths() {
        let package = build(
            &entry_named("attio"),
            published_manifest("attio", &api_key_auth("attio", "")),
        );

        let paths: Vec<&str> = package
            .assets
            .iter()
            .map(|asset| asset.path.as_str())
            .collect();
        assert!(
            paths.contains(&"wasm/attio-tool.wasm"),
            "wasm should be at the declared path, got {paths:?}"
        );
        assert!(
            paths.contains(&"schemas/attio/invoke.input.v1.json"),
            "input schema should be at the declared path, got {paths:?}"
        );
        assert!(
            paths.contains(&"schemas/attio/raw_output.v1.json"),
            "output schema should be at the declared path, got {paths:?}"
        );
    }

    /// The credential the tool published survives the install. This is the
    /// regression that shipped twice while the manifest was reconstructed here.
    #[test]
    fn published_credentials_reach_the_installed_package() {
        let package = build(
            &entry_named("attio"),
            published_manifest("attio", &api_key_auth("attio", "")),
        );

        let credentials: Vec<_> = package
            .package
            .manifest
            .capabilities
            .iter()
            .flat_map(|capability| capability.runtime_credentials.iter())
            .collect();
        assert_eq!(credentials.len(), 1, "expected one runtime credential");
        assert_eq!(credentials[0].handle.as_str(), "attio_api_key");
    }

    /// The vendor's account-setup copy reaches the user. Without it an installed
    /// tool can say a secret is required but not where to get it, which is what
    /// left users stuck and models inventing setup steps.
    #[test]
    fn published_setup_instructions_reach_the_user() {
        let auth = api_key_auth(
            "attio",
            r#"instructions = "Open Workspace Settings > Developers and create an access token."
setup_url = "https://app.attio.com/settings/developers""#,
        );
        let package = build(&entry_named("attio"), published_manifest("attio", &auth));

        let onboarding = package
            .onboarding_override
            .expect("published instructions should become onboarding copy");
        assert!(
            onboarding
                .instructions
                .contains("Workspace Settings > Developers"),
            "got {:?}",
            onboarding.instructions
        );
        assert_eq!(
            onboarding.setup_url.as_deref(),
            Some("https://app.attio.com/settings/developers")
        );
    }

    /// A tool that publishes no setup copy still installs; it simply has no
    /// onboarding to show.
    #[test]
    fn a_manifest_without_instructions_still_installs() {
        let package = build(
            &entry_named("attio"),
            published_manifest("attio", &api_key_auth("attio", "")),
        );

        assert!(package.onboarding_override.is_none());
    }

    /// A manifest whose id disagrees with the catalog entry would install one
    /// extension under another's name.
    #[test]
    fn a_manifest_id_that_contradicts_the_catalog_is_refused() {
        let error = ironhub_tool_package(
            &entry_named("attio"),
            published_manifest("other-tool", &api_key_auth("other-tool", "")),
            component(),
            b"{}".to_vec(),
            &[],
        )
        .expect_err("mismatched id must not install");

        let message = error.to_string();
        assert!(
            message.contains("attio") && message.contains("other-tool"),
            "error should name both ids, got {message}"
        );
    }

    /// Bytes that are not a manifest fail with the tool named, not with a parse
    /// error from somewhere deeper.
    #[test]
    fn a_malformed_published_manifest_names_the_tool() {
        let error = ironhub_tool_package(
            &entry_named("attio"),
            b"this is not toml".to_vec(),
            component(),
            b"{}".to_vec(),
            &[],
        )
        .expect_err("malformed manifest must not install");

        assert!(
            error.to_string().contains("attio") || error.to_string().contains("manifest"),
            "got {error}"
        );
    }
}
