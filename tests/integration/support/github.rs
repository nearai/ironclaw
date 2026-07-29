use std::path::{Path, PathBuf};

use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ExtensionRegistry, ManifestSource};
use ironclaw_host_api::{
    CapabilityId, EffectKind, ExtensionId, NetworkPolicy, NetworkScheme, NetworkTargetPattern,
    SecretHandle, VirtualPath,
};
use ironclaw_host_runtime::{default_host_api_contract_registry, default_host_port_catalog};

type GithubSupportResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub fn capability_ids() -> GithubSupportResult<Vec<CapabilityId>> {
    Ok(extension_registry()?
        .capabilities()
        .map(|descriptor| descriptor.id.clone())
        .collect())
}

pub fn effect_kinds() -> Vec<EffectKind> {
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
    Ok(vec![SecretHandle::new("github_runtime_token")?])
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
    let mut registry = ExtensionRegistry::new();
    registry.insert(extension_package()?)?;
    Ok(registry)
}

/// The parsed github `ExtensionPackage` alone (no registry wrapper); C-JOURNEY
/// feeds it into `publish_bundled_extension_for_test` for github.* dispatch
/// without a scripted install/setup handshake.
pub fn extension_package() -> GithubSupportResult<ExtensionPackage> {
    // Parse through the single record entry point (the bundled assets are
    // manifest v3 documents since the first-party rewrite).
    let record = ironclaw_extensions::ExtensionManifestRecord::from_toml(
        std::fs::read_to_string(asset_root().join("manifest.toml"))?,
        ManifestSource::HostBundled,
        &default_host_port_catalog()?,
        None,
        &default_host_api_contract_registry()?,
    )?;
    let manifest = ExtensionManifest::try_from(record.manifest().clone())?;
    Ok(ExtensionPackage::from_manifest(
        manifest,
        VirtualPath::new("/system/extensions/github")?,
    )?)
}

pub fn asset_root() -> PathBuf {
    repo_root().join("crates/ironclaw_first_party_extensions/assets/github")
}

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}
