//! Shared `#[cfg(test)]` fixtures for the registered-extension-store test
//! suites (`extension_lifecycle_registered_store_tests`,
//! `registered_extension_store_blast_radius_tests`, and
//! `extension_lifecycle::tests`), which independently re-implemented the
//! same mount/reboot/seed boilerplate. Hoisted here so a change to that
//! boilerplate (e.g. a new required mount) lands once.

use std::sync::Arc;

use ironclaw_extensions::{
    ExtensionActivationState, ExtensionInstallation, ExtensionInstallationId,
    ExtensionInstallationStore, ExtensionLifecycleService, ExtensionManifestRecord,
    ExtensionManifestRef, ExtensionRegistry, InstallationOwner, ManifestHash, ManifestSource,
    SharedExtensionRegistry,
};
use ironclaw_filesystem::LocalFilesystem;
use ironclaw_host_api::{
    ExtensionId, HostPath, TenantId, UserId, VirtualPath, sha256_digest_token,
};
use ironclaw_trust::{AdminConfig, HostTrustPolicy, InvalidationBus};
use tokio::sync::Mutex;

use crate::extension_host::extension_lifecycle::ActiveExtensionPublisher;

/// A `LocalFilesystem` with `/system/extensions` mounted onto
/// `storage_root/system/extensions`, creating the directory first — the
/// mount every registered-store test needs before writing/reading manifests.
/// Returned unwrapped (not `Arc<dyn RootFilesystem>`) so callers that need to
/// wrap it further (e.g. injecting a failing `list_dir`) still can.
pub(crate) fn mounted_local_filesystem(storage_root: &std::path::Path) -> LocalFilesystem {
    std::fs::create_dir_all(storage_root.join("system/extensions")).expect("storage root"); // safety: test-only fixture setup.
    let mut local_filesystem = LocalFilesystem::new();
    local_filesystem
        .mount_local(
            VirtualPath::new("/system/extensions").expect("valid virtual path"), // safety: test-only fixture setup.
            HostPath::from_path_buf(storage_root.join("system/extensions")),
        )
        .expect("mount system extensions"); // safety: test-only fixture setup.
    local_filesystem
}

/// A fresh in-memory lifecycle/active-registry/trust-policy stack — the
/// "reboot" the boot-restore tests rebuild over the SAME durable filesystem
/// and installation store, simulating a process restart.
pub(crate) struct BootFixture {
    pub(crate) lifecycle_service: Arc<Mutex<ExtensionLifecycleService>>,
    pub(crate) active_registry: Arc<SharedExtensionRegistry>,
    pub(crate) active_extensions: ActiveExtensionPublisher,
}

pub(crate) fn fresh_boot_fixture() -> BootFixture {
    let lifecycle_service = Arc::new(Mutex::new(ExtensionLifecycleService::new(
        ExtensionRegistry::new(),
    )));
    let active_registry = Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
    let trust_policy =
        Arc::new(HostTrustPolicy::new(vec![Box::new(AdminConfig::new())]).expect("trust policy")); // safety: test-only fixture setup.
    let active_extensions = ActiveExtensionPublisher::new(
        Arc::clone(&active_registry),
        Arc::clone(&trust_policy),
        Arc::new(InvalidationBus::new()),
    );
    BootFixture {
        lifecycle_service,
        active_registry,
        active_extensions,
    }
}

/// Seeds a `UserRegistered` manifest record + installation row into `store`,
/// mirroring what a real owner-aware install persists (manifest hash on
/// both records, so restore's clean-path hash validation matches). Passing
/// `manifest_hash: None` computes it from `manifest_toml`; pass `Some(..)`
/// when the on-disk manifest is meant to later drift from the seeded row.
/// Returns the extension id and the hash actually stored.
pub(crate) async fn seed_registered_installation(
    installation_store: &Arc<dyn ExtensionInstallationStore>,
    manifest_toml: &str,
    tenant_id: &TenantId,
    owner: &UserId,
    extension_id_str: &str,
    manifest_hash: Option<ManifestHash>,
) -> (ExtensionId, ManifestHash) {
    let host_ports = ironclaw_host_runtime::default_host_port_catalog().expect("host port catalog"); // safety: test-only fixture setup.
    let contracts =
        ironclaw_host_runtime::default_host_api_contract_registry().expect("host API contracts"); // safety: test-only fixture setup.
    let manifest_hash = manifest_hash.unwrap_or_else(|| {
        ManifestHash::new(sha256_digest_token(manifest_toml.as_bytes()))
            .expect("valid manifest hash") // safety: test-only fixture setup.
    });
    let manifest_record = ExtensionManifestRecord::from_toml_with_contracts(
        manifest_toml,
        ManifestSource::UserRegistered {
            tenant_id: tenant_id.clone(),
            owner: owner.clone(),
        },
        &host_ports,
        Some(manifest_hash.clone()),
        &contracts,
    )
    .expect("registered manifest record"); // safety: test-only fixture setup.
    let extension_id = ExtensionId::new(extension_id_str).expect("valid extension id"); // safety: test-only fixture setup.
    let value = toml::from_str::<toml::Value>(manifest_toml).expect("manifest TOML"); // safety: test-only fixture setup.
    let url = value["runtime"]["url"].as_str().expect("hosted MCP URL"); // safety: test-only fixture setup.
    let expected_extension_id =
        crate::extension_host::registered_extension_store::HostedMcpExtensionId::mint(
            tenant_id, owner, url, "",
        )
        .expect("mint hosted MCP id") // safety: test-only fixture setup.
        .into_extension_id();
    let installation = ExtensionInstallation::new(
        ExtensionInstallationId::new(extension_id_str).expect("valid installation id"), // safety: test-only fixture setup.
        extension_id.clone(),
        ExtensionActivationState::Enabled,
        ExtensionManifestRef::new(extension_id.clone(), Some(manifest_hash.clone())),
        Vec::new(),
        chrono::Utc::now(),
        InstallationOwner::user(owner.clone()),
    )
    .expect("registered installation"); // safety: test-only fixture setup.
    installation_store
        .upsert_manifest(manifest_record)
        .await
        .expect("seed registered manifest record"); // safety: test-only fixture setup.
    installation_store
        .upsert_installation(installation)
        .await
        .expect("seed registered installation"); // safety: test-only fixture setup.
    (expected_extension_id, manifest_hash)
}

/// Mint a hosted MCP id directly from `(tenant, owner, url)`, for fixtures
/// that must know the id BEFORE writing manifest content (the reverse of
/// `seed_registered_installation`, which reads a URL back out of an
/// already-written manifest). R1 gates every registered-store read on
/// `HostedMcpExtensionId::parse` succeeding and matching this mint, so a live
/// (non-restore) fixture's on-disk descriptor, directory name, and
/// installation row must all be keyed by this id, not a bare literal.
pub(crate) fn minted_extension_id(tenant: &TenantId, owner: &UserId, url: &str) -> ExtensionId {
    crate::extension_host::registered_extension_store::HostedMcpExtensionId::mint(
        tenant, owner, url, "",
    )
    .expect("mint hosted MCP id") // safety: test-only fixture minting.
    .into_extension_id()
}
