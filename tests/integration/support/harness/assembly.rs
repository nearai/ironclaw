use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use super::super::{
    extension_surface::BUNDLED_EXTENSION_IDS, github as github_support, harness_web_access,
};
use ironclaw_authorization::GrantAuthorizer;
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_filesystem::{
    BackendCapabilities, BackendId, BackendKind, CompositeRootFilesystem, ContentKind,
    InMemoryBackend, IndexPolicy, LocalFilesystem, MountDescriptor, RootFilesystem, StorageClass,
};
use ironclaw_host_api::{
    CapabilityId, CredentialStageError, EffectKind, ExtensionId, HostPath, MountAlias, MountGrant,
    MountPermissions, MountView, NetworkPolicy, NetworkScheme, NetworkTargetPattern, PackageId,
    SecretHandle, VirtualPath,
};
use ironclaw_host_runtime::{
    BUILTIN_FIRST_PARTY_PROVIDER, CapabilitySurfaceVersion as HostRuntimeCapabilitySurfaceVersion,
    HostRuntime, HostRuntimeServices, RuntimeProcessPort, builtin_first_party_handlers,
    builtin_first_party_package,
};
use ironclaw_network::{PolicyNetworkHttpEgress, ReqwestNetworkTransport};
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_secrets::{InMemorySecretStore, SecretMaterial};
use ironclaw_trust::{AdminConfig, AdminEntry, HostTrustAssignment, HostTrustPolicy};
use ironclaw_wasm::{WitToolHost, WitToolRuntimeConfig};

use super::super::doubles::{
    FixedRuntimeCredentialAccountResolver, GithubHarnessAuthorizer, RecordingNetworkHttpEgress,
    RecordingNetworkHttpTransport, RecordingRuntimeHttpEgress, StaticNetworkResolver,
    StaticSecretStore,
};
use super::HarnessResult;

/// Default capability-io pair: the ephemeral `ProductLiveCapabilityIo` test
/// double, coerced into ONE shared object's two trait-object roles (input
/// resolver + result writer). Every `HostRuntimeCapabilityHarness`
/// constructor that does not opt into `.with_durable_capability_io()`
/// (issue #5838) uses this so both roles keep sharing one underlying object,
/// matching production's `LocalDevCapabilityIo` invariant.
pub(crate) fn default_capability_io_pair() -> (
    Arc<dyn ironclaw_loop_support::LoopCapabilityInputResolver>,
    Arc<dyn ironclaw_loop_support::LoopCapabilityResultWriter>,
) {
    let capability_io = Arc::new(ironclaw_reborn_composition::ProductLiveCapabilityIo::default());
    (capability_io.clone(), capability_io)
}

pub(crate) fn local_dev_host_runtime_with_http_egress(
    storage_root: PathBuf,
    egress: Arc<RecordingRuntimeHttpEgress>,
    process_port: Option<Arc<dyn RuntimeProcessPort>>,
) -> HarnessResult<Arc<dyn HostRuntime>> {
    let mut registry = ExtensionRegistry::new();
    registry.insert(builtin_first_party_package()?)?;
    local_dev_host_runtime_with_registry_and_runtime_http_egress(
        storage_root,
        registry,
        egress,
        process_port,
    )
}

pub(crate) fn host_runtime_storage_roots()
-> HarnessResult<(Arc<tempfile::TempDir>, PathBuf, PathBuf)> {
    let root = Arc::new(tempfile::tempdir()?);
    let storage_root = root.path().join("local-dev");
    let workspace_root = storage_root.join("workspace");
    std::fs::create_dir_all(&workspace_root)?;
    Ok((root, storage_root, workspace_root))
}

pub(crate) fn local_dev_host_runtime_with_registry_and_runtime_http_egress(
    storage_root: PathBuf,
    registry: ExtensionRegistry,
    egress: Arc<RecordingRuntimeHttpEgress>,
    process_port: Option<Arc<dyn RuntimeProcessPort>>,
) -> HarnessResult<Arc<dyn HostRuntime>> {
    let mut services = HostRuntimeServices::new(
        Arc::new(registry),
        local_dev_root_filesystem(storage_root, LocalDevRootMounts::core_builtins())?,
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        HostRuntimeCapabilitySurfaceVersion::new("reborn-app-v1")?,
    )
    .with_secret_store(Arc::new(StaticSecretStore::new(
        SecretHandle::new("github_manual_access")?,
        SecretMaterial::from("ghp_fake_fixture_token"),
    )))
    .with_runtime_credential_account_resolver(Arc::new(FixedRuntimeCredentialAccountResolver {
        result: Ok(SecretHandle::new("github_manual_access")?),
    }))
    .with_first_party_capabilities(Arc::new(builtin_first_party_handlers(Arc::new(
        ironclaw_triggers::InMemoryTriggerRepository::default(),
    ))?))
    .with_first_party_http_egress(egress)
    .with_trust_policy(Arc::new(first_party_trust_policy()?));
    // Inject the recording process port when provided; `None` defaults to
    // `LocalHostProcessPort` (real execution).
    if let Some(port) = process_port {
        services = services.with_runtime_process_port_dyn(port);
    }

    Ok(Arc::new(services.host_runtime_for_local_testing()))
}

pub(crate) fn local_dev_host_runtime_with_registry_and_egress(
    storage_root: PathBuf,
    registry: ExtensionRegistry,
    runtime_http_egress: Arc<RecordingRuntimeHttpEgress>,
    network_egress: Arc<RecordingNetworkHttpEgress>,
    // E-AUTHGATE: `Ok(handle)` resolves the credential account (capability
    // dispatches); `Err(AuthRequired)` raises a `BlockedAuth` gate at dispatch.
    credential_account_result: Result<SecretHandle, CredentialStageError>,
) -> HarnessResult<Arc<dyn HostRuntime>> {
    let services = HostRuntimeServices::new(
        Arc::new(registry),
        local_dev_root_filesystem(storage_root, LocalDevRootMounts::github_assets())?,
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GithubHarnessAuthorizer::new()?),
        ironclaw_processes::ProcessServices::in_memory(),
        HostRuntimeCapabilitySurfaceVersion::new("reborn-app-v1")?,
    )
    .with_secret_store(Arc::new(StaticSecretStore::new(
        SecretHandle::new("github_manual_access")?,
        SecretMaterial::from("ghp_fake_fixture_token"),
    )))
    .with_runtime_credential_account_resolver(Arc::new(FixedRuntimeCredentialAccountResolver {
        result: credential_account_result,
    }))
    .with_first_party_capabilities(Arc::new(builtin_first_party_handlers(Arc::new(
        ironclaw_triggers::InMemoryTriggerRepository::default(),
    ))?))
    .with_runtime_http_egress(runtime_http_egress)
    .with_trust_policy(Arc::new(github_first_party_trust_policy()?))
    .try_with_host_http_egress((*network_egress).clone())
    .map_err(|report| std::io::Error::other(format!("host HTTP egress failed: {report:?}")))?
    .try_with_wasm_runtime(WitToolRuntimeConfig::default(), WitToolHost::deny_all())
    .map_err(|report| std::io::Error::other(format!("WASM runtime failed: {report:?}")))?;

    Ok(Arc::new(services.host_runtime_for_local_testing()))
}

pub(crate) fn local_dev_host_runtime_with_live_http_egress(
    storage_root: PathBuf,
) -> HarnessResult<Arc<dyn HostRuntime>> {
    let mut registry = ExtensionRegistry::new();
    registry.insert(builtin_first_party_package()?)?;

    let services = HostRuntimeServices::new(
        Arc::new(registry),
        local_dev_root_filesystem(storage_root, LocalDevRootMounts::core_builtins())?,
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        HostRuntimeCapabilitySurfaceVersion::new("reborn-app-v1")?,
    )
    .with_secret_store(Arc::new(InMemorySecretStore::new()))
    .with_first_party_capabilities(Arc::new(builtin_first_party_handlers(Arc::new(
        ironclaw_triggers::InMemoryTriggerRepository::default(),
    ))?))
    .try_with_host_http_egress(PolicyNetworkHttpEgress::new(ReqwestNetworkTransport::new(
        Duration::from_secs(2),
    )))
    .map_err(|report| {
        std::io::Error::other(format!(
            "live HTTP egress production wiring failed: {report:?}"
        ))
    })?
    .with_trust_policy(Arc::new(first_party_trust_policy()?));

    Ok(Arc::new(services.host_runtime_for_local_testing()))
}

/// S1 seam: wires the REAL production egress pipeline —
/// `PolicyNetworkHttpEgress` (network-policy enforcement + DNS/private-IP
/// checks) over `HostHttpEgressService` (leak scan) — with only the
/// wire-level transport swapped for `RecordingNetworkHttpTransport`. Mirrors
/// [`local_dev_host_runtime_with_live_http_egress`] exactly, but the
/// transport records instead of making a real network call, and DNS
/// resolution is faked via `StaticNetworkResolver` so the pipeline stays
/// hermetic.
pub(crate) fn local_dev_host_runtime_with_real_egress_pipeline(
    storage_root: PathBuf,
    network_transport: RecordingNetworkHttpTransport,
    process_port: Option<Arc<dyn RuntimeProcessPort>>,
) -> HarnessResult<Arc<dyn HostRuntime>> {
    let mut registry = ExtensionRegistry::new();
    registry.insert(builtin_first_party_package()?)?;

    let mut services = HostRuntimeServices::new(
        Arc::new(registry),
        local_dev_root_filesystem(storage_root, LocalDevRootMounts::core_builtins())?,
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        HostRuntimeCapabilitySurfaceVersion::new("reborn-app-v1")?,
    )
    .with_secret_store(Arc::new(InMemorySecretStore::new()))
    .with_first_party_capabilities(Arc::new(builtin_first_party_handlers(Arc::new(
        ironclaw_triggers::InMemoryTriggerRepository::default(),
    ))?))
    .try_with_host_http_egress(PolicyNetworkHttpEgress::new_with_resolver(
        network_transport,
        StaticNetworkResolver,
    ))
    .map_err(|report| {
        std::io::Error::other(format!(
            "real egress pipeline production wiring failed: {report:?}"
        ))
    })?
    .with_trust_policy(Arc::new(first_party_trust_policy()?));
    // Inject the recording process port when provided; `None` defaults to
    // `LocalHostProcessPort` (real execution).
    if let Some(port) = process_port {
        services = services.with_runtime_process_port_dyn(port);
    }

    Ok(Arc::new(services.host_runtime_for_local_testing()))
}

pub(crate) fn local_dev_root_filesystem(
    storage_root: PathBuf,
    mounts: LocalDevRootMounts,
) -> HarnessResult<Arc<CompositeRootFilesystem>> {
    let mut local = LocalFilesystem::new();
    local.mount_local(
        VirtualPath::new("/projects")?,
        HostPath::from_path_buf(storage_root),
    )?;
    if mounts.github_assets {
        local.mount_local(
            VirtualPath::new("/system/extensions/github")?,
            HostPath::from_path_buf(github_support::asset_root()),
        )?;
    }
    if mounts.web_access_assets {
        local.mount_local(
            VirtualPath::new("/system/extensions/web-access")?,
            HostPath::from_path_buf(harness_web_access::asset_root()),
        )?;
    }

    let local = Arc::new(local);
    let mut root = CompositeRootFilesystem::new();
    root.mount(
        local_dev_mount_descriptor(
            "/projects",
            "local-dev-projects",
            BackendKind::LocalFilesystem,
            StorageClass::FileContent,
            ContentKind::ProjectFile,
            IndexPolicy::NotIndexed,
            BackendCapabilities::bytes_only(),
        )?,
        Arc::clone(&local),
    )?;
    if mounts.github_assets {
        root.mount(
            local_dev_mount_descriptor(
                "/system/extensions/github",
                "local-dev-github-assets",
                BackendKind::LocalFilesystem,
                StorageClass::FileContent,
                ContentKind::ExtensionPackage,
                IndexPolicy::NotIndexed,
                BackendCapabilities::bytes_only(),
            )?,
            Arc::clone(&local),
        )?;
    }
    if mounts.web_access_assets {
        root.mount(
            local_dev_mount_descriptor(
                "/system/extensions/web-access",
                "local-dev-web-access-assets",
                BackendKind::LocalFilesystem,
                StorageClass::FileContent,
                ContentKind::ExtensionPackage,
                IndexPolicy::NotIndexed,
                BackendCapabilities::bytes_only(),
            )?,
            Arc::clone(&local),
        )?;
    }
    if mounts.memory {
        let memory = Arc::new(InMemoryBackend::new());
        root.mount(
            local_dev_mount_descriptor(
                "/memory",
                "local-dev-memory",
                BackendKind::MemoryDocuments,
                StorageClass::StructuredRecords,
                ContentKind::MemoryDocument,
                IndexPolicy::FullTextAndVector,
                memory.capabilities(),
            )?,
            memory,
        )?;
    }
    Ok(Arc::new(root))
}

#[derive(Clone, Copy)]
pub(crate) struct LocalDevRootMounts {
    github_assets: bool,
    web_access_assets: bool,
    memory: bool,
}

impl LocalDevRootMounts {
    pub(crate) fn core_builtins() -> Self {
        Self {
            github_assets: false,
            web_access_assets: false,
            memory: true,
        }
    }

    fn github_assets() -> Self {
        Self {
            github_assets: true,
            web_access_assets: false,
            memory: false,
        }
    }

    pub(crate) fn web_access_assets() -> Self {
        Self {
            github_assets: false,
            web_access_assets: true,
            memory: false,
        }
    }
}

pub(crate) fn local_dev_mount_descriptor(
    virtual_root: &str,
    backend_id: &str,
    backend_kind: BackendKind,
    storage_class: StorageClass,
    content_kind: ContentKind,
    index_policy: IndexPolicy,
    capabilities: BackendCapabilities,
) -> HarnessResult<MountDescriptor> {
    Ok(MountDescriptor {
        virtual_root: VirtualPath::new(virtual_root)?,
        backend_id: BackendId::new(backend_id)?,
        backend_kind,
        storage_class,
        content_kind,
        index_policy,
        capabilities,
    })
}

pub(crate) fn first_party_trust_policy() -> HarnessResult<HostTrustPolicy> {
    Ok(HostTrustPolicy::new(vec![Box::new(
        AdminConfig::with_entries(vec![AdminEntry::for_local_manifest(
            PackageId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            "/system/extensions/builtin/manifest.toml".to_string(),
            None,
            HostTrustAssignment::first_party(),
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::DeleteFilesystem,
                EffectKind::Network,
                EffectKind::SpawnProcess,
                EffectKind::ExecuteCode,
                EffectKind::ExternalWrite,
            ],
            None,
        )]),
    )])?)
}

pub(crate) fn github_first_party_trust_policy() -> HarnessResult<HostTrustPolicy> {
    Ok(HostTrustPolicy::new(vec![Box::new(
        AdminConfig::with_entries(vec![AdminEntry::for_local_manifest(
            PackageId::new("github")?,
            "/system/extensions/github/manifest.toml".to_string(),
            None,
            HostTrustAssignment::first_party(),
            vec![
                EffectKind::DispatchCapability,
                EffectKind::Network,
                EffectKind::UseSecret,
                EffectKind::ExternalWrite,
            ],
            None,
        )]),
    )])?)
}

pub(crate) fn http_test_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "api.example.test".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(10_000),
    }
}

pub(crate) fn wildcard_test_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: None,
            host_pattern: "*".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(1_000_000),
    }
}

/// C-JOURNEY: recursively copy `src` into `dst` (creating `dst` and any
/// intermediate directories). Used to populate a harness's per-test
/// `/system/extensions/<id>` mount with the real bundled-extension asset
/// directory (manifest + wasm module + schemas) so a WASM capability
/// published via `publish_bundled_extension_for_test` is genuinely loadable,
/// not just registered as metadata.
pub(crate) fn copy_dir_recursive(src: &Path, dst: &Path) -> HarnessResult<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &dst_path)?;
        } else {
            std::fs::copy(entry.path(), &dst_path)?;
        }
    }
    Ok(())
}

pub(crate) fn capability_ids_from_strs(ids: &[&str]) -> HarnessResult<Vec<CapabilityId>> {
    ids.iter()
        .map(|id| CapabilityId::new(*id).map_err(Into::into))
        .collect()
}

pub(crate) fn bundled_extension_provider_trust()
-> HarnessResult<Vec<(ExtensionId, Vec<EffectKind>)>> {
    BUNDLED_EXTENSION_IDS
        .iter()
        .map(|id| Ok((ExtensionId::new(*id)?, local_dev_all_effects())))
        .collect()
}

pub(crate) fn local_dev_all_effects() -> Vec<EffectKind> {
    vec![
        EffectKind::DispatchCapability,
        EffectKind::ReadFilesystem,
        EffectKind::WriteFilesystem,
        EffectKind::DeleteFilesystem,
        EffectKind::Network,
        EffectKind::UseSecret,
        EffectKind::SpawnProcess,
        EffectKind::ExecuteCode,
        EffectKind::ExternalWrite,
    ]
}

pub(crate) fn workspace_mounts(permissions: MountPermissions) -> HarnessResult<MountView> {
    Ok(MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace")?,
        VirtualPath::new("/projects/workspace")?,
        permissions,
    )])?)
}

pub(crate) fn memory_mounts(permissions: MountPermissions) -> HarnessResult<MountView> {
    Ok(MountView::new(vec![MountGrant::new(
        MountAlias::new("/memory")?,
        VirtualPath::new("/memory")?,
        permissions,
    )])?)
}

pub(crate) fn skill_mounts() -> HarnessResult<MountView> {
    Ok(MountView::new(vec![
        MountGrant::new(
            MountAlias::new("/skills")?,
            VirtualPath::new("/projects/skills")?,
            MountPermissions::read_write_list_delete(),
        ),
        MountGrant::new(
            MountAlias::new("/system/skills")?,
            VirtualPath::new("/projects/system/skills")?,
            MountPermissions::read_only(),
        ),
    ])?)
}

pub(crate) fn qa_smoke_mounts() -> HarnessResult<MountView> {
    Ok(MountView::new(vec![
        MountGrant::new(
            MountAlias::new("/workspace")?,
            VirtualPath::new("/projects/workspace")?,
            MountPermissions::read_write_list_delete(),
        ),
        MountGrant::new(
            MountAlias::new("/skills")?,
            VirtualPath::new("/projects/skills")?,
            MountPermissions::read_write_list_delete(),
        ),
        MountGrant::new(
            MountAlias::new("/system/skills")?,
            VirtualPath::new("/projects/system/skills")?,
            MountPermissions::read_only(),
        ),
    ])?)
}
