#![cfg(feature = "libsql")]

use std::{sync::Arc, time::Duration};

use ironclaw_host_api::{
    AuditMode, DeploymentMode, FilesystemBackendKind, NetworkMode, ProcessBackendKind,
    RuntimeProfile, SecretMode,
    runtime_policy::{ApprovalPolicy, EffectiveRuntimePolicy},
};
use ironclaw_host_runtime::{
    CapabilitySurfaceVersion, CommandExecutionOutput, CommandExecutionRequest,
    ProductionWiringConfig, RuntimeProcessError, SandboxCommandTransport,
};
use ironclaw_reborn_composition::{
    LibSqlProductionSubstrateConfig, RebornCompositionError, RebornProductionRuntimePolicy,
    build_libsql_production_host_runtime_services,
};
use ironclaw_reborn_event_store::RebornEventStoreConfig;
use ironclaw_turns::{TurnRunWake, TurnRunWakeNotifier, TurnRunWakeNotifyError};
use secrecy::SecretString;
use tempfile::tempdir;

#[tokio::test]
async fn libsql_substrate_builder_wires_production_components_without_local_only_seams() {
    let dir = tempdir().unwrap();
    let state_db_path = dir.path().join("state.db");
    let events_db_path = dir.path().join("events.db");
    let database = Arc::new(
        libsql::Builder::new_local(state_db_path.display().to_string())
            .build()
            .await
            .unwrap(),
    );

    let services = build_libsql_production_host_runtime_services(LibSqlProductionSubstrateConfig {
        database,
        event_store: RebornEventStoreConfig::Libsql {
            path_or_url: events_db_path.display().to_string(),
            auth_token: None,
        },
        secret_master_key: Some(SecretString::from("01234567890123456789012345678901")),
        trust_policy: Arc::new(ironclaw_trust::HostTrustPolicy::fail_closed()),
        runtime_policy: RebornProductionRuntimePolicy::with_tenant_sandbox_process_port(
            production_runtime_policy(),
            sandbox_process_port(),
        )
        .unwrap(),
        turn_run_wake_notifier: Arc::new(RecordingSchedulerWakeNotifier),
        surface_version: CapabilitySurfaceVersion::new("test-surface").unwrap(),
    })
    .await
    .unwrap();

    let production_config = ProductionWiringConfig::new([]).require_runtime_http_egress();
    services
        .validate_production_wiring(&production_config)
        .expect("substrate-only production wiring should not use fake seams");
}

#[tokio::test]
async fn libsql_substrate_builder_rejects_missing_secret_master_key() {
    let dir = tempdir().unwrap();
    let state_db_path = dir.path().join("state.db");
    let events_db_path = dir.path().join("events.db");
    let database = Arc::new(
        libsql::Builder::new_local(state_db_path.display().to_string())
            .build()
            .await
            .unwrap(),
    );

    let result = build_libsql_production_host_runtime_services(LibSqlProductionSubstrateConfig {
        database,
        event_store: RebornEventStoreConfig::Libsql {
            path_or_url: events_db_path.display().to_string(),
            auth_token: None,
        },
        secret_master_key: None,
        trust_policy: Arc::new(ironclaw_trust::HostTrustPolicy::fail_closed()),
        runtime_policy: RebornProductionRuntimePolicy::with_tenant_sandbox_process_port(
            production_runtime_policy(),
            sandbox_process_port(),
        )
        .unwrap(),
        turn_run_wake_notifier: Arc::new(RecordingSchedulerWakeNotifier),
        surface_version: CapabilitySurfaceVersion::new("test-surface").unwrap(),
    })
    .await;

    assert!(matches!(
        result,
        Err(RebornCompositionError::MissingSecretMasterKey)
    ));
}

#[test]
fn production_runtime_policy_requires_tenant_sandbox_process_port() {
    let result = RebornProductionRuntimePolicy::without_process_port(production_runtime_policy());

    assert!(matches!(
        result,
        Err(RebornCompositionError::MissingTenantSandboxProcessPort)
    ));
}

#[test]
fn production_runtime_policy_rejects_unexpected_tenant_sandbox_process_port() {
    let mut policy = production_runtime_policy();
    policy.process_backend = ProcessBackendKind::None;

    let result = RebornProductionRuntimePolicy::with_tenant_sandbox_process_port(
        policy,
        sandbox_process_port(),
    );

    assert!(matches!(
        result,
        Err(RebornCompositionError::UnexpectedTenantSandboxProcessPort {
            process_backend: ProcessBackendKind::None
        })
    ));
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

fn sandbox_process_port() -> Arc<ironclaw_host_runtime::TenantSandboxProcessPort> {
    Arc::new(ironclaw_host_runtime::TenantSandboxProcessPort::new(
        Arc::new(RecordingSandboxTransport),
    ))
}

#[derive(Debug)]
struct RecordingSandboxTransport;

#[async_trait::async_trait]
impl SandboxCommandTransport for RecordingSandboxTransport {
    async fn run_command(
        &self,
        _request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        Ok(CommandExecutionOutput {
            output: String::new(),
            exit_code: 0,
            sandboxed: true,
            duration: Duration::ZERO,
        })
    }
}

#[derive(Debug)]
struct RecordingSchedulerWakeNotifier;

impl TurnRunWakeNotifier for RecordingSchedulerWakeNotifier {
    fn notify_queued_run(&self, _wake: TurnRunWake) -> Result<(), TurnRunWakeNotifyError> {
        Ok(())
    }
}
