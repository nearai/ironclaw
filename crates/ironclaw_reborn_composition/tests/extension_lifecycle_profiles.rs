#![cfg(feature = "libsql")]

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use chrono::Utc;
use ironclaw_host_api::{
    AuditMode, CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, DeploymentMode,
    EffectKind, ExecutionContext, ExtensionId, FilesystemBackendKind, GrantConstraints, MountView,
    NetworkMode, NetworkPolicy, PackageId, Principal, ProcessBackendKind, RuntimeKind,
    RuntimeProfile, SecretMode, TrustClass, UserId,
    runtime_policy::{ApprovalPolicy, EffectiveRuntimePolicy},
};
use ironclaw_host_runtime::{
    CapabilitySurfacePolicy, CommandExecutionOutput, CommandExecutionRequest, RuntimeProcessError,
    SandboxCommandTransport, SchedulerTurnRunWakeNotifier, SurfaceKind, TenantSandboxProcessPort,
    TurnRunExecutor, TurnRunExecutorError, TurnRunScheduler, TurnRunSchedulerConfig,
    TurnRunSchedulerHandle, VisibleCapabilityRequest,
};
use ironclaw_reborn_composition::{
    RebornBuildInput, RebornCompositionProfile, RebornReadinessState, RebornRuntimeProcessBinding,
    build_reborn_services,
};
use ironclaw_secrets::SecretMaterial;
use ironclaw_trust::{
    AdminConfig, AdminEntry, AuthorityCeiling, EffectiveTrustClass, HostTrustAssignment,
    HostTrustPolicy, TrustDecision, TrustProvenance,
};
use ironclaw_turns::{
    InMemoryTurnStateStore,
    runner::{ClaimedTurnRun, TurnRunTransitionPort},
};

#[tokio::test]
async fn production_libsql_services_expose_extension_lifecycle_tools() {
    let dir = tempfile::tempdir().unwrap();
    let db = libsql_db_at(dir.path().join("reborn.db")).await;
    let (notifier, handle) = live_wake_notifier();

    let services = build_reborn_services(
        RebornBuildInput::libsql(
            RebornCompositionProfile::Production,
            "test-owner",
            db,
            dir.path().join("events.db").to_string_lossy(),
            None,
            test_master_key(),
        )
        .with_production_trust_policy(production_trust_policy())
        .with_runtime_policy(production_runtime_policy())
        .with_turn_run_wake_notifier(notifier)
        .with_runtime_process_binding(test_sandbox_process_binding())
        .with_required_runtime_backends([RuntimeKind::FirstParty])
        .require_runtime_http_egress(),
    )
    .await
    .unwrap();

    let surface = services
        .host_runtime
        .as_ref()
        .expect("production must expose host runtime")
        .visible_capabilities(extension_lifecycle_visible_request())
        .await
        .unwrap();
    let visible_ids = surface
        .capabilities
        .iter()
        .map(|capability| capability.descriptor.id.as_str())
        .collect::<Vec<_>>();

    handle.shutdown().await;

    assert_eq!(
        services.readiness.state,
        RebornReadinessState::ProductionValidated
    );
    assert!(visible_ids.contains(&"builtin.extension_search"));
    assert!(visible_ids.contains(&"builtin.extension_install"));
    assert!(visible_ids.contains(&"builtin.extension_activate"));
    assert!(visible_ids.contains(&"builtin.extension_remove"));
}

fn extension_lifecycle_visible_request() -> VisibleCapabilityRequest {
    let lifecycle_effects = vec![
        EffectKind::DispatchCapability,
        EffectKind::ReadFilesystem,
        EffectKind::WriteFilesystem,
    ];
    let grants = CapabilitySet {
        grants: [
            "builtin.extension_search",
            "builtin.extension_install",
            "builtin.extension_activate",
            "builtin.extension_remove",
        ]
        .into_iter()
        .map(|capability| test_capability_grant(capability, lifecycle_effects.clone()))
        .collect(),
    };
    let context = ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::FirstParty,
        TrustClass::UserTrusted,
        grants,
        MountView::default(),
    )
    .unwrap();

    let mut provider_trust = BTreeMap::new();
    provider_trust.insert(
        ExtensionId::new("builtin").unwrap(),
        TrustDecision {
            effective_trust: EffectiveTrustClass::user_trusted(),
            authority_ceiling: AuthorityCeiling {
                allowed_effects: lifecycle_effects,
                max_resource_ceiling: None,
            },
            provenance: TrustProvenance::AdminConfig,
            evaluated_at: Utc::now(),
        },
    );

    VisibleCapabilityRequest::new(context, SurfaceKind::new("agent_loop").unwrap())
        .with_policy(CapabilitySurfacePolicy::allow_all())
        .with_provider_trust(provider_trust)
}

fn test_capability_grant(capability: &str, allowed_effects: Vec<EffectKind>) -> CapabilityGrant {
    CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability: CapabilityId::new(capability).unwrap(),
        grantee: Principal::Extension(ExtensionId::new("caller").unwrap()),
        issued_by: Principal::HostRuntime,
        constraints: GrantConstraints {
            allowed_effects,
            mounts: MountView::default(),
            network: NetworkPolicy::default(),
            secrets: Vec::new(),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
    }
}

fn production_trust_policy() -> Arc<HostTrustPolicy> {
    Arc::new(
        HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries([
            AdminEntry::for_admin(
                PackageId::new("reborn-test").unwrap(),
                HostTrustAssignment::first_party(),
                vec![EffectKind::DispatchCapability],
                None,
            ),
            AdminEntry::for_local_manifest(
                PackageId::new("builtin").unwrap(),
                "/system/extensions/builtin/manifest.toml".to_string(),
                None,
                HostTrustAssignment::first_party(),
                vec![
                    EffectKind::DispatchCapability,
                    EffectKind::ReadFilesystem,
                    EffectKind::WriteFilesystem,
                ],
                None,
            ),
        ]))])
        .unwrap(),
    )
}

fn production_runtime_policy() -> EffectiveRuntimePolicy {
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::HostedMultiTenant,
        requested_profile: RuntimeProfile::HostedDev,
        resolved_profile: RuntimeProfile::HostedDev,
        filesystem_backend: FilesystemBackendKind::TenantWorkspace,
        process_backend: ProcessBackendKind::TenantSandbox,
        network_mode: NetworkMode::Allowlist,
        secret_mode: SecretMode::TenantBroker,
        approval_policy: ApprovalPolicy::AskDestructive,
        audit_mode: AuditMode::Standard,
    }
}

fn test_sandbox_process_binding() -> RebornRuntimeProcessBinding {
    RebornRuntimeProcessBinding::tenant_sandbox(Arc::new(TenantSandboxProcessPort::new(Arc::new(
        NoopSandboxTransport,
    ))))
}

struct NoopSandboxTransport;

#[async_trait::async_trait]
impl SandboxCommandTransport for NoopSandboxTransport {
    async fn run_command(
        &self,
        _request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        Ok(CommandExecutionOutput {
            output: String::new(),
            saved_output: None,
            exit_code: 0,
            sandboxed: true,
            duration: Duration::from_millis(1),
        })
    }
}

fn live_wake_notifier() -> (Arc<SchedulerTurnRunWakeNotifier>, TurnRunSchedulerHandle) {
    let transitions: Arc<dyn TurnRunTransitionPort> = Arc::new(InMemoryTurnStateStore::default());
    let executor: Arc<dyn TurnRunExecutor> = Arc::new(NoopTurnRunExecutor);
    let handle =
        TurnRunScheduler::new(transitions, executor, TurnRunSchedulerConfig::default()).start();
    (handle.wake_notifier(), handle)
}

struct NoopTurnRunExecutor;

#[async_trait::async_trait]
impl TurnRunExecutor for NoopTurnRunExecutor {
    async fn execute_claimed_run(
        &self,
        _claimed: ClaimedTurnRun,
        _transitions: Arc<dyn TurnRunTransitionPort>,
    ) -> Result<(), TurnRunExecutorError> {
        Ok(())
    }
}

async fn libsql_db_at(path: impl AsRef<std::path::Path>) -> Arc<libsql::Database> {
    Arc::new(
        libsql::Builder::new_local(path.as_ref())
            .build()
            .await
            .unwrap(),
    )
}

fn test_master_key() -> SecretMaterial {
    SecretMaterial::from("01234567890123456789012345678901")
}
