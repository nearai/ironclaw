#![cfg(feature = "postgres")]

use std::{sync::Arc, time::Duration};

use deadpool_postgres::tokio_postgres;
use ironclaw_host_api::{
    AuditMode, DeploymentMode, FilesystemBackendKind, NetworkMode, ProcessBackendKind,
    RuntimeProfile, SecretMode,
    runtime_policy::{ApprovalPolicy, EffectiveRuntimePolicy},
};
use ironclaw_host_runtime::{
    CapabilitySurfaceVersion, CommandExecutionOutput, CommandExecutionRequest,
    ProductionWiringConfig, RuntimeProcessError, SandboxCommandTransport,
    VerifiedTenantSandboxProcessPort,
};
use ironclaw_reborn_composition::{
    PostgresProductionSubstrateConfig, RebornCompositionError, RebornProductionRuntimePolicy,
    build_postgres_production_host_runtime_services,
};
use ironclaw_reborn_event_store::RebornEventStoreConfig;
use ironclaw_turns::{TurnRunWake, TurnRunWakeNotifier, TurnRunWakeNotifyError};
use secrecy::SecretString;

#[tokio::test]
async fn postgres_substrate_builder_wires_production_components_without_local_only_seams() {
    let Some((_container, pool, database_url)) = postgres_pool_or_skip().await else {
        return;
    };

    let services =
        build_postgres_production_host_runtime_services(PostgresProductionSubstrateConfig {
            pool,
            event_store: RebornEventStoreConfig::Postgres {
                url: SecretString::from(database_url),
            },
            secret_master_key: Some(SecretString::from("01234567890123456789012345678901")),
            trust_policy: Arc::new(ironclaw_trust::HostTrustPolicy::fail_closed()),
            runtime_policy: RebornProductionRuntimePolicy::with_tenant_sandbox_process_port(
                production_runtime_policy(),
                verified_sandbox_process_port(),
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
        .expect("postgres substrate production wiring should not use fake seams");
}

#[tokio::test]
async fn postgres_substrate_builder_rejects_missing_secret_master_key() {
    let Some((_container, pool, database_url)) = postgres_pool_or_skip().await else {
        return;
    };

    let result =
        build_postgres_production_host_runtime_services(PostgresProductionSubstrateConfig {
            pool,
            event_store: RebornEventStoreConfig::Postgres {
                url: SecretString::from(database_url),
            },
            secret_master_key: None,
            trust_policy: Arc::new(ironclaw_trust::HostTrustPolicy::fail_closed()),
            runtime_policy: RebornProductionRuntimePolicy::with_tenant_sandbox_process_port(
                production_runtime_policy(),
                verified_sandbox_process_port(),
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

fn verified_sandbox_process_port() -> VerifiedTenantSandboxProcessPort {
    VerifiedTenantSandboxProcessPort::assume_verified_transport(Arc::new(RecordingSandboxTransport))
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

async fn postgres_pool_or_skip() -> Option<(
    testcontainers_modules::testcontainers::ContainerAsync<
        testcontainers_modules::postgres::Postgres,
    >,
    deadpool_postgres::Pool,
    String,
)> {
    let (container, database_url) = start_postgres_container().await?;
    let config: tokio_postgres::Config = database_url
        .parse()
        .expect("testcontainer database URL must parse");
    let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    let pool = deadpool_postgres::Pool::builder(manager)
        .max_size(4)
        .build()
        .expect("Postgres pool must build");
    let _connection = pool
        .get()
        .await
        .expect("Postgres testcontainer must accept connections");
    Some((container, pool, database_url))
}

async fn start_postgres_container() -> Option<(
    testcontainers_modules::testcontainers::ContainerAsync<
        testcontainers_modules::postgres::Postgres,
    >,
    String,
)> {
    use testcontainers_modules::testcontainers::{ImageExt, runners::AsyncRunner};

    let image = testcontainers_modules::postgres::Postgres::default()
        .with_db_name("ironclaw_test")
        .with_user("postgres")
        .with_password("postgres")
        .with_tag("16-alpine");

    let container = match image.start().await {
        Ok(container) => container,
        Err(error) => {
            eprintln!(
                "skipping Postgres composition tests: docker/testcontainers unavailable ({error})"
            );
            return None;
        }
    };
    let host = match container.get_host().await {
        Ok(host) => host,
        Err(error) => {
            eprintln!(
                "skipping Postgres composition tests: could not resolve container host ({error})"
            );
            return None;
        }
    };
    let port = match container.get_host_port_ipv4(5432).await {
        Ok(port) => port,
        Err(error) => {
            eprintln!(
                "skipping Postgres composition tests: could not resolve container port ({error})"
            );
            return None;
        }
    };
    Some((
        container,
        format!("postgres://postgres:postgres@{host}:{port}/ironclaw_test"),
    ))
}

#[derive(Debug)]
struct RecordingSchedulerWakeNotifier;

impl TurnRunWakeNotifier for RecordingSchedulerWakeNotifier {
    fn notify_queued_run(&self, _wake: TurnRunWake) -> Result<(), TurnRunWakeNotifyError> {
        Ok(())
    }
}
