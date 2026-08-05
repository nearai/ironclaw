use std::collections::{BTreeMap, BTreeSet};

use ironclaw_extension_contracts::runtime::ExtensionAssetPath;
use ironclaw_extensions::{ExtensionRuntimeV2, ManifestSource};

use ironclaw_extension_host::{
    AvailableExtensionPackage, parse_imported_manifest, registry_extension_package,
};

use crate::ironhub::catalog::validate_hub_name;
use crate::ironhub::model::{IronHubCommandError, IronHubToolEntry};

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
/// The manifest also chooses where its own assets live: the wasm and
/// digest-verified tool assets are placed at the paths it declares, so
/// publisher and host never have to agree a filename convention across two
/// repositories.
pub(crate) fn ironhub_tool_package(
    entry: &IronHubToolEntry,
    manifest: Vec<u8>,
    wasm: Vec<u8>,
    capabilities: Vec<u8>,
    schemas: Vec<(String, Vec<u8>)>,
    prompts: Vec<(String, Vec<u8>)>,
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
        validate_manifest_asset_path(module)?;
        files.push((module.clone(), wasm));
    }
    let mut schema_assets = BTreeMap::new();
    for (path, content) in schemas {
        validate_manifest_asset_path(&path)?;
        if schema_assets.insert(path.clone(), content).is_some() {
            return Err(IronHubCommandError::Catalog {
                reason: format!("published schema path '{path}' appears more than once"),
            });
        }
    }
    let mut referenced_schemas = BTreeSet::new();
    for capability in &record.manifest().capabilities {
        let input_path = capability.input_schema_ref.as_str();
        validate_manifest_asset_path(input_path)?;
        referenced_schemas.insert(input_path.to_string());
        if let Some(output_schema_ref) = &capability.output_schema_ref {
            let output_path = output_schema_ref.as_str();
            validate_manifest_asset_path(output_path)?;
            referenced_schemas.insert(output_path.to_string());
        }
    }
    let published_schemas: BTreeSet<_> = schema_assets.keys().cloned().collect();
    if referenced_schemas != published_schemas {
        let missing: Vec<_> = referenced_schemas
            .difference(&published_schemas)
            .cloned()
            .collect();
        let unreferenced: Vec<_> = published_schemas
            .difference(&referenced_schemas)
            .cloned()
            .collect();
        return Err(IronHubCommandError::Catalog {
            reason: format!(
                "published schemas do not match manifest references (missing: {missing:?}, unreferenced: {unreferenced:?})"
            ),
        });
    }
    files.extend(schema_assets);

    let mut prompt_assets = BTreeMap::new();
    for (path, content) in prompts {
        validate_manifest_asset_path(&path)?;
        if prompt_assets.insert(path.clone(), content).is_some() {
            return Err(IronHubCommandError::Catalog {
                reason: format!("published prompt path '{path}' appears more than once"),
            });
        }
    }
    let referenced_prompts: BTreeSet<_> = record
        .manifest()
        .capabilities
        .iter()
        .filter_map(|capability| capability.prompt_doc_ref.as_ref())
        .map(|path| path.as_str().to_string())
        .collect();
    let published_prompts: BTreeSet<_> = prompt_assets.keys().cloned().collect();
    if referenced_prompts != published_prompts {
        let missing: Vec<_> = referenced_prompts
            .difference(&published_prompts)
            .cloned()
            .collect();
        let unreferenced: Vec<_> = published_prompts
            .difference(&referenced_prompts)
            .cloned()
            .collect();
        return Err(IronHubCommandError::Catalog {
            reason: format!(
                "published prompts do not match manifest references (missing: {missing:?}, unreferenced: {unreferenced:?})"
            ),
        });
    }
    files.extend(prompt_assets);

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

fn validate_manifest_asset_path(path: &str) -> Result<(), IronHubCommandError> {
    ExtensionAssetPath::new(path.to_string())
        .map(|_| ())
        .map_err(|error| IronHubCommandError::Catalog {
            reason: format!("published manifest contains an invalid asset path: {error}"),
        })
}

#[cfg(test)]
mod tests {
    use super::super::model::{IronHubArtifact, IronHubProvenance};
    use super::*;

    /// A real WASI component; `registry_extension_package` rejects core modules.
    fn component() -> Vec<u8> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../extensions/packages/github/wasm/github_tool.wasm"),
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
        let schemas = BTreeMap::from([
            (format!("schemas/{name}/invoke.input.v1.json"), artifact()),
            (format!("schemas/{name}/raw_output.v1.json"), artifact()),
        ]);
        IronHubToolEntry {
            name: name.to_string(),
            version: "0.1.0".to_string(),
            description: "test tool".to_string(),
            provenance: IronHubProvenance::Official,
            wasm: artifact(),
            capabilities: artifact(),
            manifest: Some(artifact()),
            schemas,
            prompts: BTreeMap::new(),
        }
    }

    fn published_schemas(id: &str) -> Vec<(String, Vec<u8>)> {
        vec![
            (
                format!("schemas/{id}/invoke.input.v1.json"),
                br#"{"type":"object","required":["action"]}"#.to_vec(),
            ),
            (
                format!("schemas/{id}/raw_output.v1.json"),
                br#"{"description":"Raw JSON output"}"#.to_vec(),
            ),
        ]
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

    fn oauth_auth(id: &str) -> String {
        format!(
            r#"
[auth.{id}]
method = "oauth2_code"
display_name = "{id}"
authorization_endpoint = "https://accounts.{id}.com/oauth/authorize"
token_endpoint = "https://accounts.{id}.com/oauth/token"
scopes = ["read"]
client_credentials = {{ client_id_handle = "{id}_oauth_client_id", client_secret_handle = "{id}_oauth_client_secret" }}
instructions = "Register an OAuth application."
setup_url = "https://accounts.{id}.com/developers/apps"

[auth.{id}.token_response]
access_token = "/access_token""#
        )
    }

    fn build(entry: &IronHubToolEntry, manifest: Vec<u8>) -> AvailableExtensionPackage {
        ironhub_tool_package(
            entry,
            manifest,
            component(),
            b"{}".to_vec(),
            published_schemas(&entry.name),
            Vec::new(),
            &[],
        )
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

    #[test]
    fn published_input_schema_reaches_the_installed_package() {
        let package = build(
            &entry_named("attio"),
            published_manifest("attio", &api_key_auth("attio", "")),
        );
        let schema = package
            .assets
            .iter()
            .find(|asset| asset.path == "schemas/attio/invoke.input.v1.json")
            .expect("input schema asset");

        assert_eq!(
            schema.content,
            ironclaw_extension_host::AvailableExtensionAssetContent::Bytes(
                br#"{"type":"object","required":["action"]}"#.to_vec(),
            ),
            "the model-facing schema must come from the signed package, not a permissive host placeholder",
        );
    }

    #[test]
    fn package_rejects_a_manifest_with_an_unpublished_schema() {
        let mut schemas = published_schemas("attio");
        schemas.pop();
        let error = ironhub_tool_package(
            &entry_named("attio"),
            published_manifest("attio", &api_key_auth("attio", "")),
            component(),
            b"{}".to_vec(),
            schemas,
            Vec::new(),
            &[],
        )
        .expect_err("every manifest schema must be present in the signed catalog");

        assert!(
            error.to_string().contains("missing")
                && error.to_string().contains("raw_output.v1.json"),
            "got {error}"
        );
    }

    #[test]
    fn package_rejects_a_manifest_with_an_unpublished_prompt() {
        let manifest = String::from_utf8(published_manifest(
            "attio",
            &api_key_auth("attio", ""),
        ))
        .expect("manifest UTF-8")
        .replace(
            "output_schema_ref = \"schemas/attio/raw_output.v1.json\"",
            "output_schema_ref = \"schemas/attio/raw_output.v1.json\"\nprompt_doc_ref = \"prompts/attio/invoke.md\"",
        )
        .into_bytes();
        let error = ironhub_tool_package(
            &entry_named("attio"),
            manifest,
            component(),
            b"{}".to_vec(),
            published_schemas("attio"),
            Vec::new(),
            &[],
        )
        .expect_err("every manifest prompt must be present in the signed catalog");

        assert!(
            error.to_string().contains("missing")
                && error.to_string().contains("prompts/attio/invoke.md"),
            "got {error}"
        );
    }

    #[test]
    fn package_rejects_invalid_utf8_and_duplicate_or_unreferenced_schemas() {
        let entry = entry_named("attio");
        let invalid_utf8 = ironhub_tool_package(
            &entry,
            vec![0xff],
            component(),
            b"{}".to_vec(),
            published_schemas("attio"),
            Vec::new(),
            &[],
        )
        .expect_err("manifest must be UTF-8");
        assert!(invalid_utf8.to_string().contains("not UTF-8"));

        let mut duplicate = published_schemas("attio");
        duplicate.push(duplicate[0].clone());
        let duplicate_error = ironhub_tool_package(
            &entry,
            published_manifest("attio", &api_key_auth("attio", "")),
            component(),
            b"{}".to_vec(),
            duplicate,
            Vec::new(),
            &[],
        )
        .expect_err("duplicate schema paths must be rejected");
        assert!(duplicate_error.to_string().contains("more than once"));

        let mut unreferenced = published_schemas("attio");
        unreferenced.push(("schemas/attio/unused.json".to_string(), b"{}".to_vec()));
        let unreferenced_error = ironhub_tool_package(
            &entry,
            published_manifest("attio", &api_key_auth("attio", "")),
            component(),
            b"{}".to_vec(),
            unreferenced,
            Vec::new(),
            &[],
        )
        .expect_err("unreferenced schema paths must be rejected");
        assert!(unreferenced_error.to_string().contains("unreferenced"));
    }

    #[test]
    fn schema_digest_changes_the_pinned_tool_artifact_digest() {
        let original = entry_named("attio");
        let mut changed = original.clone();
        changed
            .schemas
            .get_mut("schemas/attio/invoke.input.v1.json")
            .expect("input schema metadata")
            .sha256 = "b".repeat(64);

        assert_ne!(
            super::super::catalog::tool_artifact_digest(&original),
            super::super::catalog::tool_artifact_digest(&changed),
        );
    }

    #[test]
    fn prompt_digest_changes_the_pinned_tool_artifact_digest() {
        let mut original = entry_named("attio");
        original
            .prompts
            .insert("prompts/attio/invoke.md".to_string(), artifact());
        let mut changed = original.clone();
        changed
            .prompts
            .get_mut("prompts/attio/invoke.md")
            .expect("prompt metadata")
            .sha256 = "b".repeat(64);

        assert_ne!(
            super::super::catalog::tool_artifact_digest(&original),
            super::super::catalog::tool_artifact_digest(&changed),
        );
    }

    #[test]
    fn package_rejects_manifest_asset_paths_outside_the_extension_root() {
        let base = String::from_utf8(published_manifest("attio", &api_key_auth("attio", "")))
            .expect("manifest UTF-8");
        for (label, manifest) in [
            (
                "absolute module",
                base.replace(
                    "module = \"wasm/attio-tool.wasm\"",
                    "module = \"/tmp/attio.wasm\"",
                ),
            ),
            (
                "traversing input schema",
                base.replace(
                    "input_schema_ref = \"schemas/attio/invoke.input.v1.json\"",
                    "input_schema_ref = \"../invoke.input.v1.json\"",
                ),
            ),
            (
                "traversing output schema",
                base.replace(
                    "output_schema_ref = \"schemas/attio/raw_output.v1.json\"",
                    "output_schema_ref = \"schemas/../../raw_output.v1.json\"",
                ),
            ),
        ] {
            let error = ironhub_tool_package(
                &entry_named("attio"),
                manifest.into_bytes(),
                component(),
                b"{}".to_vec(),
                published_schemas("attio"),
                Vec::new(),
                &[],
            )
            .expect_err(label);
            assert!(
                error.to_string().contains("asset path")
                    || error.to_string().contains("path segments"),
                "{label} must fail as an invalid manifest asset path: {error}"
            );
        }
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

    #[test]
    fn published_oauth_setup_instructions_reach_the_user() {
        let package = build(
            &entry_named("attio"),
            published_manifest("attio", &oauth_auth("attio")),
        );

        let onboarding = package
            .onboarding_override
            .expect("OAuth setup metadata should become onboarding copy");
        assert_eq!(onboarding.instructions, "Register an OAuth application.");
        assert_eq!(
            onboarding.setup_url.as_deref(),
            Some("https://accounts.attio.com/developers/apps")
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
            published_schemas("other-tool"),
            Vec::new(),
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
            published_schemas("attio"),
            Vec::new(),
            &[],
        )
        .expect_err("malformed manifest must not install");

        assert!(
            error.to_string().contains("attio") || error.to_string().contains("manifest"),
            "got {error}"
        );
    }
}
