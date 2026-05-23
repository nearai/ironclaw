use std::path::{Path, PathBuf};

use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ExtensionRegistry, ManifestSource};
use ironclaw_host_api::{
    CapabilityId, EffectKind, ExtensionId, NetworkPolicy, NetworkScheme, NetworkTargetPattern,
    SecretHandle, VirtualPath,
};
use ironclaw_host_runtime::{default_host_api_contract_registry, default_host_port_catalog};

type GithubSupportResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub fn issue_capability_ids() -> GithubSupportResult<Vec<CapabilityId>> {
    Ok(vec![
        CapabilityId::new("github.search_issues")?,
        CapabilityId::new("github.get_issue")?,
        CapabilityId::new("github.comment_issue")?,
    ])
}

pub fn issue_effect_kinds() -> Vec<EffectKind> {
    vec![
        EffectKind::DispatchCapability,
        EffectKind::Network,
        EffectKind::UseSecret,
        EffectKind::ExternalWrite,
    ]
}

pub fn provider_id() -> GithubSupportResult<ExtensionId> {
    Ok(ExtensionId::new("github")?)
}

pub fn secret_handles() -> GithubSupportResult<Vec<SecretHandle>> {
    Ok(vec![SecretHandle::new("github_token")?])
}

pub fn search_response_body() -> Vec<u8> {
    br#"{"total_count":0,"incomplete_results":false,"items":[]}"#.to_vec()
}

pub fn api_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "api.github.com".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(10_000),
    }
}

pub fn extension_registry() -> GithubSupportResult<ExtensionRegistry> {
    let manifest = ExtensionManifest::parse_with_host_api_contracts(
        &std::fs::read_to_string(asset_root().join("manifest.toml"))?,
        ManifestSource::HostBundled,
        &default_host_port_catalog()?,
        &default_host_api_contract_registry()?,
    )?;
    let package =
        ExtensionPackage::from_manifest(manifest, VirtualPath::new("/system/extensions/github")?)?;
    let mut registry = ExtensionRegistry::new();
    registry.insert(package)?;
    Ok(registry)
}

pub fn asset_root() -> PathBuf {
    repo_root().join("crates/ironclaw_first_party_extensions/assets/github")
}

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}
