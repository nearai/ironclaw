fn capability_provider_contracts() -> ironclaw_extensions::HostApiContractRegistry {
    let mut contracts = ironclaw_extensions::HostApiContractRegistry::new();
    contracts
        .register(std::sync::Arc::new(
            ironclaw_extensions::CapabilityProviderHostApiContract::new()
                .expect("capability provider contract"),
        ))
        .expect("register capability provider contract");
    contracts
}
use super::*;
use async_trait::async_trait;
use ironclaw_extensions::InstallationOwner;
use ironclaw_extensions::{
    ExtensionActivationState, ExtensionHealthSnapshot, ExtensionInstallation,
    ExtensionInstallationError, ExtensionInstallationId, ExtensionInstallationStore,
    ExtensionManifest, ExtensionManifestRecord, ExtensionPackage, ExtensionRegistry,
    FilesystemExtensionInstallationStore, ManifestSource,
};
use ironclaw_filesystem::{DiskFilesystem, InMemoryBackend};
use ironclaw_host_api::{
    ActivityId, ExtensionId, HostPath, HostPortCatalog, MountAlias, MountGrant, MountPermissions,
    MountView, Resolution, TenantId, UserId, VirtualPath,
};
use ironclaw_product_workflow::{
    EXTENSION_ACTIVATE_CAPABILITY, EXTENSION_INSTALL_CAPABILITY, EXTENSION_REMOVE_CAPABILITY,
    OPERATOR_SERVICE_LIFECYCLE_OPERATION, ProductCapabilityDescriptor, RebornOperatorToolCatalog,
    RebornOperatorToolInfo,
};
use std::time::Duration;

use crate::extension_host::extension_lifecycle::RebornLocalExtensionManagementPort;
use crate::extension_host::host_api_contracts::product_extension_host_api_contract_registry;

#[tokio::test]
async fn operator_tool_catalog_reads_shared_registry_updates() {
    let registry = Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
    let synthetic_provider = outbound_delivery_synthetic_provider().expect("synthetic provider id");
    // No owner source: every registry tool is tenant-visible (the
    // assembly-without-extension-management case).
    let catalog = ActiveRegistryOperatorToolCatalog::new(
        Arc::clone(&registry),
        vec![
            outbound_delivery_target_set_operator_tool_info(synthetic_provider.clone())
                .expect("synthetic tool info"),
        ],
        None,
    );
    let caller = UserId::new("caller").expect("caller id");

    assert!(
        catalog
            .list_operator_tools(&caller)
            .await
            .iter()
            .any(|tool| {
                tool.capability_id.as_str()
                    == crate::outbound::OUTBOUND_DELIVERY_TARGET_SET_CAPABILITY_ID
                    && tool.provider == synthetic_provider
            }),
        "synthetic outbound delivery capability must use the Settings > Tools provider key"
    );

    registry
        .insert(test_extension_package("dynamic-tools", "echo"))
        .expect("insert dynamic extension");

    let tools = catalog.list_operator_tools(&caller).await;

    assert!(
        tools
            .iter()
            .any(|tool| tool.capability_id.as_str() == "dynamic-tools.echo"),
        "catalog must read the shared registry at list time so lifecycle updates are visible"
    );
}

/// #5459 P1 leak fix: the settings/tools catalog is read by any
/// authenticated member, so it MUST hide another user's private tool. With
/// an owner source wired, `list_operator_tools(bob)` excludes alice's
/// private capability while `list_operator_tools(alice)` includes it; a
/// tenant-shared tool is visible to both. This is the caller-level pin for
/// the confirmed enumeration/metadata-disclosure blocker.
#[tokio::test]
async fn operator_tool_catalog_hides_foreign_private_tools() {
    use crate::extension_host::available_extensions::AvailableExtensionCatalog;
    use ironclaw_extensions::{ExtensionLifecycleService, ExtensionManifestRef};
    use tokio::sync::Mutex;

    fn manifest_record(ext: &str, capability: &str) -> ExtensionManifestRecord {
        let toml = format!(
            "schema_version = \"reborn.extension_manifest.v2\"\n\
             id = \"{ext}\"\nname = \"{ext}\"\nversion = \"0.1.0\"\n\
             description = \"test\"\ntrust = \"third_party\"\n\n\
             [runtime]\nkind = \"wasm\"\nmodule = \"wasm/{ext}.wasm\"\n\n\
             [[host_api]]\nid = \"ironclaw.capability_provider/v1\"\n\
             section = \"capability_provider.tools\"\n\n\
             [capability_provider.tools]\n\n\
             [[capability_provider.tools.capabilities]]\nid = \"{ext}.{capability}\"\ndescription = \"{capability}\"\n\
             effects = [\"network\"]\ndefault_permission = \"ask\"\nvisibility = \"model\"\n\
             input_schema_ref = \"schemas/{capability}.input.json\"\n\
             output_schema_ref = \"schemas/{capability}.output.json\"\n"
        );
        ExtensionManifestRecord::from_toml(
            toml,
            ManifestSource::HostBundled,
            &HostPortCatalog::empty(),
            None,
            &product_extension_host_api_contract_registry().expect("contracts"),
        )
        .expect("manifest record")
    }

    let operator = UserId::new("operator").expect("operator id");
    let alice = UserId::new("alice").expect("alice id");
    let bob = UserId::new("bob").expect("bob id");

    // Store: alice privately owns `market-data`; `hacker-news` is tenant-shared.
    // Wrapped so the test can inject an owner-read failure (#5525 review).
    let store = Arc::new(OwnerReadFailingStore::new().await);
    for (ext, capability, owner) in [
        (
            "market-data",
            "snp500",
            InstallationOwner::user(alice.clone()),
        ),
        ("hacker-news", "top_stories", InstallationOwner::Tenant),
    ] {
        let ext_id = ExtensionId::new(ext).expect("ext id");
        store
            .upsert_manifest_and_installation(
                manifest_record(ext, capability),
                ExtensionInstallation::new(
                    ExtensionInstallationId::new(ext).expect("installation id"),
                    ext_id.clone(),
                    ExtensionActivationState::Enabled,
                    ExtensionManifestRef::new(ext_id, None),
                    Vec::new(),
                    Utc::now(),
                    owner,
                )
                .expect("installation"),
            )
            .await
            .expect("upsert manifest + installation");
    }
    let installation_store: Arc<dyn ExtensionInstallationStore> = store.clone();

    // Registry the catalog reads: both extensions' capabilities are
    // published, plus one anomalous capability with NO installation row.
    let registry = Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
    registry
        .insert(test_extension_package("market-data", "snp500"))
        .expect("insert market-data");
    registry
        .insert(test_extension_package("hacker-news", "top_stories"))
        .expect("insert hacker-news");
    registry
        .insert(test_extension_package("orphan-tool", "probe"))
        .expect("insert orphan-tool");

    let trust_policy = Arc::new(
        ironclaw_trust::HostTrustPolicy::new(vec![Box::new(ironclaw_trust::AdminConfig::new())])
            .expect("trust policy"),
    );
    let port = Arc::new(RebornLocalExtensionManagementPort::new(
        Arc::new(DiskFilesystem::new()),
        AvailableExtensionCatalog::from_packages(Vec::new()),
        installation_store,
        Arc::new(Mutex::new(ExtensionLifecycleService::new(
            ExtensionRegistry::new(),
        ))),
        crate::extension_host::extension_lifecycle::ActiveExtensionPublisher::new(
            Arc::clone(&registry),
            trust_policy,
            Arc::new(ironclaw_trust::InvalidationBus::new()),
        ),
        None,
        operator,
    ));

    let catalog = ActiveRegistryOperatorToolCatalog::new(registry, Vec::new(), Some(port));

    let ids_for = |tools: Vec<RebornOperatorToolInfo>| {
        tools
            .into_iter()
            .map(|t| t.capability_id.as_str().to_string())
            .collect::<Vec<_>>()
    };
    let bob_ids = ids_for(catalog.list_operator_tools(&bob).await);
    assert!(
        bob_ids.contains(&"hacker-news.top_stories".to_string()),
        "tenant-shared tool must be visible to every member: {bob_ids:?}"
    );
    assert!(
        !bob_ids.contains(&"market-data.snp500".to_string()),
        "alice's PRIVATE tool must not appear in bob's settings/tools catalog: {bob_ids:?}"
    );
    assert!(
        !bob_ids.contains(&"orphan-tool.probe".to_string()),
        "an installable capability without an owner row must fail closed: {bob_ids:?}"
    );

    let alice_ids = ids_for(catalog.list_operator_tools(&alice).await);
    assert!(
        alice_ids.contains(&"market-data.snp500".to_string())
            && alice_ids.contains(&"hacker-news.top_stories".to_string()),
        "the owner sees her own private tool plus shared tools: {alice_ids:?}"
    );
    assert!(
        !alice_ids.contains(&"orphan-tool.probe".to_string()),
        "the owner-row fail-closed default applies to every caller: {alice_ids:?}"
    );

    // #5525 review: when the owner map cannot be read at all, the
    // owner-aware assembly must hide every install-backed registry tool
    // (fail closed) instead of treating the empty map as all-shared.
    store
        .fail_list_installations
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let degraded_ids = ids_for(catalog.list_operator_tools(&bob).await);
    assert!(
        degraded_ids.is_empty(),
        "unreadable owner data must hide install-backed registry tools: {degraded_ids:?}"
    );

    // The next healthy read recovers the shared surface.
    let recovered_ids = ids_for(catalog.list_operator_tools(&bob).await);
    assert!(
        recovered_ids.contains(&"hacker-news.top_stories".to_string())
            && !recovered_ids.contains(&"market-data.snp500".to_string()),
        "a healthy re-read restores shared visibility only: {recovered_ids:?}"
    );
}

/// Store wrapper that fails `list_installations` once when armed —
/// injects the owner-read failure the settings catalog must fail closed
/// on (#5525 review).
struct OwnerReadFailingStore {
    inner: FilesystemExtensionInstallationStore,
    fail_list_installations: std::sync::atomic::AtomicBool,
}

impl OwnerReadFailingStore {
    async fn new() -> Self {
        Self {
            inner: filesystem_installation_store().await,
            fail_list_installations: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

async fn filesystem_installation_store() -> FilesystemExtensionInstallationStore {
    FilesystemExtensionInstallationStore::load_at(
        Arc::new(InMemoryBackend::new()),
        VirtualPath::new("/system/extensions/.installations/test").expect("valid root"),
        HostPortCatalog::empty(),
        ironclaw_extensions::HostApiContractRegistry::new(),
    )
    .await
    .expect("filesystem store")
}

#[async_trait]
impl ExtensionInstallationStore for OwnerReadFailingStore {
    async fn list_manifests(
        &self,
    ) -> Result<Vec<ExtensionManifestRecord>, ExtensionInstallationError> {
        self.inner.list_manifests().await
    }

    async fn get_manifest(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<Option<ExtensionManifestRecord>, ExtensionInstallationError> {
        self.inner.get_manifest(extension_id).await
    }

    async fn upsert_manifest(
        &self,
        manifest: ExtensionManifestRecord,
    ) -> Result<(), ExtensionInstallationError> {
        self.inner.upsert_manifest(manifest).await
    }

    async fn upsert_manifest_and_installation(
        &self,
        manifest: ExtensionManifestRecord,
        installation: ExtensionInstallation,
    ) -> Result<(), ExtensionInstallationError> {
        self.inner
            .upsert_manifest_and_installation(manifest, installation)
            .await
    }

    async fn list_installations(
        &self,
    ) -> Result<Vec<ExtensionInstallation>, ExtensionInstallationError> {
        if self
            .fail_list_installations
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(ExtensionInstallationError::InvalidInstallation {
                reason: "injected owner read failure".to_string(),
            });
        }
        self.inner.list_installations().await
    }

    async fn list_enabled_installations(
        &self,
    ) -> Result<Vec<ExtensionInstallation>, ExtensionInstallationError> {
        self.inner.list_enabled_installations().await
    }

    async fn get_installation(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<Option<ExtensionInstallation>, ExtensionInstallationError> {
        self.inner.get_installation(installation_id).await
    }

    async fn upsert_installation(
        &self,
        installation: ExtensionInstallation,
    ) -> Result<(), ExtensionInstallationError> {
        self.inner.upsert_installation(installation).await
    }

    async fn set_activation_state(
        &self,
        installation_id: &ExtensionInstallationId,
        state: ExtensionActivationState,
    ) -> Result<(), ExtensionInstallationError> {
        self.inner
            .set_activation_state(installation_id, state)
            .await
    }

    async fn delete_installation(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<(), ExtensionInstallationError> {
        self.inner.delete_installation(installation_id).await
    }

    async fn delete_manifest(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<(), ExtensionInstallationError> {
        self.inner.delete_manifest(extension_id).await
    }

    async fn update_health(
        &self,
        installation_id: &ExtensionInstallationId,
        health: ExtensionHealthSnapshot,
    ) -> Result<(), ExtensionInstallationError> {
        self.inner.update_health(installation_id, health).await
    }
}

#[tokio::test]
async fn build_webui_services_wires_lifecycle_owner_identity() {
    let dir = tempfile::tempdir().expect("tempdir");
    let input = crate::RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input("runtime-owner", dir.path().join("local-dev"))
            .with_runtime_policy(
                crate::local_dev_runtime_policy().expect("local-dev policy resolves"),
            ),
    )
    .with_identity(crate::RebornRuntimeIdentity {
        tenant_id: "tenant-alpha".to_string(),
        agent_id: "agent-alpha".to_string(),
        source_binding_id: "webui-test-source".to_string(),
        reply_target_binding_id: "webui-test-reply".to_string(),
    });
    let runtime = crate::build_reborn_runtime(input)
        .await
        .expect("runtime builds");
    let bundle = build_webui_services(&runtime, None).expect("webui services build");

    let error = OPERATOR_SERVICE_LIFECYCLE_OPERATION
        .execute_on(
            bundle.api.as_ref(),
            caller("bob"),
            ironclaw_product_workflow::RebornOperatorServiceLifecycleRequest {
                action: ironclaw_product_workflow::RebornOperatorServiceLifecycleAction::Status,
            },
        )
        .await
        .expect_err("non-owner caller is rejected before lifecycle dispatch");

    assert_eq!(error.code, RebornServicesErrorCode::Forbidden);
    assert_eq!(error.status_code, 403);
}

#[tokio::test]
async fn product_surface_extension_lifecycle_remove_succeeds_after_activation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let input = crate::RebornRuntimeInput::from_build_input(
        crate::deployment::local_dev_build_input(
            "product-surface-extension-owner",
            dir.path().join("local-dev"),
        )
        .with_runtime_policy(crate::local_dev_runtime_policy().expect("local-dev policy resolves")),
    )
    .with_identity(crate::RebornRuntimeIdentity {
        tenant_id: "tenant-alpha".to_string(),
        agent_id: "agent-alpha".to_string(),
        source_binding_id: "webui-test-source".to_string(),
        reply_target_binding_id: "webui-test-reply".to_string(),
    });
    let runtime = crate::build_reborn_runtime(input)
        .await
        .expect("runtime builds");
    let bundle = build_webui_services(&runtime, None).expect("webui services build");
    let caller = caller("product-surface-extension-owner");

    let absent_remove = invoke_lifecycle_product_capability(
        &bundle,
        caller.clone(),
        EXTENSION_REMOVE_CAPABILITY,
        serde_json::json!({"extension_id": "web-access"}),
    )
    .await
    .expect("already-absent remove resolves");
    assert_success(absent_remove, "already-absent remove");

    let install = invoke_lifecycle_product_capability(
        &bundle,
        caller.clone(),
        EXTENSION_INSTALL_CAPABILITY,
        serde_json::json!({"extension_id": "web-access"}),
    )
    .await
    .expect("install resolves");
    assert_success(install, "install");

    let activate = invoke_lifecycle_product_capability(
        &bundle,
        caller.clone(),
        EXTENSION_ACTIVATE_CAPABILITY,
        serde_json::json!({"extension_id": "web-access"}),
    )
    .await
    .expect("activate resolves");
    assert_success(activate, "activate");

    let remove = invoke_lifecycle_product_capability(
        &bundle,
        caller,
        EXTENSION_REMOVE_CAPABILITY,
        serde_json::json!({"extension_id": "web-access"}),
    )
    .await
    .expect("remove resolves");
    assert_success(remove, "remove");

    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn readiness_operator_status_service_generates_timestamp_per_call() {
    let service = ReadinessOperatorStatusService::new(RebornReadiness::disabled());

    let first = service
        .status(caller("runtime-owner"))
        .await
        .expect("first status response");
    tokio::time::sleep(Duration::from_millis(1)).await;
    let second = service
        .status(caller("runtime-owner"))
        .await
        .expect("second status response");

    assert_ne!(
        first.generated_at, second.generated_at,
        "status generated_at must be refreshed for each operator status request"
    );
}

#[tokio::test]
async fn readiness_operator_status_includes_stable_readiness_diagnostics() {
    let service = ReadinessOperatorStatusService::new(RebornReadiness::disabled());

    let response = service
        .status(caller("runtime-owner"))
        .await
        .expect("status response");

    assert_eq!(response.overall, RebornOperatorStatusState::Blocked);
    let readiness_check = response
        .checks
        .iter()
        .find(|check| check.id == "readiness_composition_profile")
        .expect("readiness diagnostic check");
    assert_eq!(readiness_check.status, RebornOperatorStatusState::Blocked);
    assert_eq!(
        readiness_check.severity,
        RebornOperatorStatusSeverity::Critical
    );
    assert!(
        readiness_check.summary.contains("reason=disabled"),
        "summary should use stable redacted readiness vocabulary: {}",
        readiness_check.summary
    );
}

#[tokio::test]
async fn readiness_operator_status_keeps_info_diagnostics_ready() {
    let service = ReadinessOperatorStatusService::new(RebornReadiness {
        profile: crate::RebornCompositionProfile::Production,
        state: crate::RebornReadinessState::ProductionValidated,
        facades: crate::RebornFacadeReadiness {
            host_runtime: true,
            turn_coordinator: true,
            product_auth: true,
        },
        workers: crate::RebornWorkerReadiness {
            turn_runner: true,
            trigger_poller: true,
        },
        diagnostics: vec![RebornReadinessDiagnostic {
            profile: crate::RebornCompositionProfile::Production,
            component: crate::RebornReadinessDiagnosticComponent::RuntimeHttpEgress,
            reason: crate::RebornReadinessDiagnosticReason::Unverified,
            status: RebornReadinessDiagnosticStatus::Info,
            blocks_production: false,
        }],
    });

    let response = service
        .status(caller("runtime-owner"))
        .await
        .expect("status response");

    assert_eq!(response.overall, RebornOperatorStatusState::Ready);
    let readiness_check = response
        .checks
        .iter()
        .find(|check| check.id == "readiness_runtime_http_egress")
        .expect("readiness info diagnostic check");
    assert_eq!(readiness_check.status, RebornOperatorStatusState::Ready);
    assert_eq!(readiness_check.severity, RebornOperatorStatusSeverity::Info);
}

#[tokio::test]
async fn skills_product_facade_surfaces_shared_auto_activate_learned_flag() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");
    std::fs::create_dir_all(&storage_root).expect("storage root");

    let mut filesystem = DiskFilesystem::new();
    filesystem
        .mount_local(
            VirtualPath::new("/projects").expect("valid virtual path"),
            HostPath::from_path_buf(storage_root.clone()),
        )
        .expect("mount storage root");
    let filesystem: Arc<dyn ironclaw_filesystem::RootFilesystem> = Arc::new(filesystem);
    let skill_management = Arc::new(RebornLocalSkillManagementPort::new_with_mount_resolver(
        UserId::new("runtime-owner").expect("user"),
        filesystem,
        Arc::new(scoped_skill_mounts),
    ));
    // Share the flag the way production composition does: the activation
    // selector holds the same `Arc`, so a toggle here must be observable on
    // that handle (that is the whole point of the live master switch).
    let flag = Arc::new(AtomicBool::new(true));
    let facade = LocalSkillsProductFacade::new(skill_management, Some(Arc::clone(&flag)));
    let owner = caller("runtime-owner");

    let listed = facade.list_skills(owner.clone()).await.expect("list");
    assert!(
        listed.auto_activate_learned,
        "default master switch must report on"
    );

    flag.store(false, Ordering::Relaxed);
    assert!(
        !flag.load(Ordering::Relaxed),
        "test setup must flip the shared selector flag to false"
    );
    let listed = facade.list_skills(owner.clone()).await.expect("list");
    assert!(
        !listed.auto_activate_learned,
        "list must report the master switch as off after disabling"
    );

    flag.store(true, Ordering::Relaxed);
    assert!(
        flag.load(Ordering::Relaxed),
        "test setup must flip the shared selector flag back to true"
    );
    let listed = facade.list_skills(owner).await.expect("list");
    assert!(
        listed.auto_activate_learned,
        "list must report the master switch as on after re-enabling"
    );
}

#[tokio::test]
async fn skills_product_facade_defaults_auto_activate_learned_when_no_selector_is_wired() {
    // Production assembly mounts the read facade but wires no local-dev
    // flag-reading selector. The list still renders with a sane default
    // rather than erroring; writes go through the first-party capability.
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");
    std::fs::create_dir_all(&storage_root).expect("storage root");

    let mut filesystem = DiskFilesystem::new();
    filesystem
        .mount_local(
            VirtualPath::new("/projects").expect("valid virtual path"),
            HostPath::from_path_buf(storage_root.clone()),
        )
        .expect("mount storage root");
    let filesystem: Arc<dyn ironclaw_filesystem::RootFilesystem> = Arc::new(filesystem);
    let skill_management = Arc::new(RebornLocalSkillManagementPort::new_with_mount_resolver(
        UserId::new("runtime-owner").expect("user"),
        filesystem,
        Arc::new(scoped_skill_mounts),
    ));
    let facade = LocalSkillsProductFacade::new(skill_management, None);
    let owner = caller("runtime-owner");

    let listed = facade.list_skills(owner).await.expect("list");
    assert!(
        listed.auto_activate_learned,
        "list defaults to on when no selector flag is wired"
    );
}

#[tokio::test]
async fn skills_product_facade_hides_owner_user_skills_from_other_callers() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");
    std::fs::create_dir_all(&storage_root).expect("storage root");
    std::fs::create_dir_all(storage_root.join("system/skills/system-helper"))
        .expect("system skill dir");
    std::fs::write(
        storage_root.join("system/skills/system-helper/SKILL.md"),
        skill_content("system-helper", "system skill"),
    )
    .expect("system skill");

    let mut filesystem = DiskFilesystem::new();
    filesystem
        .mount_local(
            VirtualPath::new("/projects").expect("valid virtual path"),
            HostPath::from_path_buf(storage_root.clone()),
        )
        .expect("mount storage root");
    let filesystem: Arc<dyn ironclaw_filesystem::RootFilesystem> = Arc::new(filesystem);
    let skill_management = Arc::new(RebornLocalSkillManagementPort::new_with_mount_resolver(
        UserId::new("runtime-owner").expect("user"),
        filesystem,
        Arc::new(scoped_skill_mounts),
    ));
    let facade = LocalSkillsProductFacade::new(
        Arc::clone(&skill_management),
        Some(Arc::new(AtomicBool::new(true))),
    );
    let owner = caller("runtime-owner");
    let bob = caller("bob");
    let other_tenant_owner = caller_in_tenant("tenant-beta", "runtime-owner");

    skill_management
        .install_for_scope(
            caller_skill_scope(owner.clone()),
            Some("shared-name"),
            &skill_content("shared-name", "alice skill"),
        )
        .await
        .expect("owner installs skill");

    let owner_skills = facade
        .list_skills(owner)
        .await
        .expect("owner lists skills")
        .skills;
    assert!(owner_skills.iter().any(|skill| skill.name == "shared-name"));
    let bob_skills = facade
        .list_skills(bob.clone())
        .await
        .expect("bob lists skills")
        .skills;
    assert!(!bob_skills.iter().any(|skill| skill.name == "shared-name"));
    assert!(bob_skills.iter().any(|skill| skill.name == "system-helper"));
    let other_tenant_skills = facade
        .list_skills(other_tenant_owner.clone())
        .await
        .expect("same user id in another tenant lists skills")
        .skills;
    assert!(
        !other_tenant_skills
            .iter()
            .any(|skill| skill.name == "shared-name")
    );

    let bob_read = facade
        .read_skill_content(bob.clone(), "shared-name".to_string())
        .await
        .expect_err("bob must not read the owner skill root");
    assert_eq!(bob_read.status_code, 404);
    let other_tenant_read = facade
        .read_skill_content(other_tenant_owner.clone(), "shared-name".to_string())
        .await
        .expect_err("same user id in another tenant must not read the owner skill root");
    assert_eq!(other_tenant_read.status_code, 404);

    skill_management
        .install_for_scope(
            caller_skill_scope(bob.clone()),
            Some("bob-skill"),
            &skill_content("bob-skill", "bob skill"),
        )
        .await
        .expect("bob installs own skill");
    let bob_content = facade
        .read_skill_content(bob.clone(), "bob-skill".to_string())
        .await
        .expect("bob reads own skill");
    assert!(bob_content.content.contains("bob skill"));
    let owner_cannot_read_bob = facade
        .read_skill_content(caller("runtime-owner"), "bob-skill".to_string())
        .await
        .expect_err("owner must not read bob skill root");
    assert_eq!(owner_cannot_read_bob.status_code, 404);

    assert!(
        storage_root
            .join("tenants/tenant-alpha/users/runtime-owner/skills/shared-name/SKILL.md")
            .exists()
    );
    assert!(
        storage_root
            .join("tenants/tenant-alpha/users/bob/skills/bob-skill/SKILL.md")
            .exists()
    );
}

fn caller(user_id: &str) -> WebUiAuthenticatedCaller {
    caller_in_tenant("tenant-alpha", user_id)
}

async fn invoke_lifecycle_product_capability(
    bundle: &RebornWebuiBundle,
    caller: WebUiAuthenticatedCaller,
    capability: ProductCapabilityDescriptor,
    input: serde_json::Value,
) -> Result<Resolution, RebornServicesError> {
    capability
        .invoke_on(bundle.api.as_ref(), caller, input, ActivityId::new())
        .await
}

fn assert_success(resolution: Resolution, label: &str) {
    match resolution {
        Resolution::Done(outcome) if outcome.verdict.is_success() => {}
        other => panic!("{label} failed: {other:?}"),
    }
}

fn test_extension_package(extension_id: &str, capability_name: &str) -> ExtensionPackage {
    let manifest_toml = format!(
        r#"
schema_version = "reborn.extension_manifest.v2"
id = "{extension_id}"
name = "{extension_id}"
version = "0.1.0"
description = "test extension"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/{extension_id}.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "{extension_id}.{capability_name}"
description = "{capability_name}"
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/{capability_name}.input.json"
output_schema_ref = "schemas/{capability_name}.output.json"
"#
    );
    let manifest = ExtensionManifest::parse(
        &manifest_toml,
        ManifestSource::HostBundled,
        &HostPortCatalog::empty(),
        &capability_provider_contracts(),
    )
    .expect("manifest parses");
    ExtensionPackage::from_manifest(
        manifest,
        VirtualPath::new(format!("/system/extensions/{extension_id}")).expect("root"),
    )
    .expect("package builds")
}

fn caller_in_tenant(tenant_id: &str, user_id: &str) -> WebUiAuthenticatedCaller {
    WebUiAuthenticatedCaller::new(
        TenantId::new(tenant_id).expect("tenant"),
        UserId::new(user_id).expect("user"),
        None,
        None,
    )
}

fn scoped_skill_mounts(
    scope: &ResourceScope,
) -> Result<MountView, ironclaw_host_api::HostApiError> {
    let user_skills = format!(
        "/projects/tenants/{}/users/{}/skills",
        scope.tenant_id.as_str(),
        scope.user_id.as_str()
    );
    MountView::new(vec![
        MountGrant::new(
            MountAlias::new("/skills")?,
            VirtualPath::new(user_skills)?,
            MountPermissions::read_write_list_delete(),
        ),
        MountGrant::new(
            MountAlias::new("/system/skills")?,
            VirtualPath::new("/projects/system/skills")?,
            MountPermissions::read_only(),
        ),
    ])
}

fn skill_content(name: &str, description: &str) -> String {
    format!("---\nname: {name}\ndescription: {description}\n---\nUse this skill.\n")
}
