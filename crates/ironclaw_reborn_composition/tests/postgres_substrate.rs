#![cfg(feature = "postgres")]

use std::sync::Arc;

use deadpool_postgres::tokio_postgres;
use ironclaw_host_api::{
    AuditMode, DeploymentMode, FilesystemBackendKind, NetworkMode, ProcessBackendKind,
    RuntimeProfile, SecretMode,
    runtime_policy::{ApprovalPolicy, EffectiveRuntimePolicy},
};
use ironclaw_host_runtime::{CapabilitySurfaceVersion, ProductionWiringConfig};
use ironclaw_reborn_composition::{
    PostgresProductionSubstrateConfig, RebornCompositionError,
    build_postgres_production_host_runtime_services,
};
use ironclaw_reborn_event_store::RebornEventStoreConfig;
use ironclaw_turns::{TurnRunWake, TurnRunWakeNotifier, TurnRunWakeNotifyError};
use secrecy::SecretString;
use tokio::sync::Mutex;

static SECRETS_MASTER_KEY_ENV_LOCK: Mutex<()> = Mutex::const_new(());

struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var_os(key);
        // SAFETY: tests serialize process-env mutation with
        // SECRETS_MASTER_KEY_ENV_LOCK and restore the prior value on drop.
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        // SAFETY: EnvVarGuard is only constructed while
        // SECRETS_MASTER_KEY_ENV_LOCK is held by this test module.
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}

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
            runtime_policy: production_runtime_policy(),
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
async fn postgres_substrate_builder_rejects_invalid_secret_master_key() {
    let Some((_container, pool, database_url)) = postgres_pool_or_skip().await else {
        return;
    };

    let result =
        build_postgres_production_host_runtime_services(PostgresProductionSubstrateConfig {
            pool,
            event_store: RebornEventStoreConfig::Postgres {
                url: SecretString::from(database_url),
            },
            secret_master_key: Some(SecretString::from("too-short")),
            trust_policy: Arc::new(ironclaw_trust::HostTrustPolicy::fail_closed()),
            runtime_policy: production_runtime_policy(),
            turn_run_wake_notifier: Arc::new(RecordingSchedulerWakeNotifier),
            surface_version: CapabilitySurfaceVersion::new("test-surface").unwrap(),
        })
        .await;

    assert!(matches!(
        result,
        Err(RebornCompositionError::Secret(
            ironclaw_secrets::SecretError::InvalidMasterKey
        ))
    ));
}

#[tokio::test]
async fn postgres_substrate_builder_rejects_weak_env_secret_master_key() {
    let _guard = SECRETS_MASTER_KEY_ENV_LOCK.lock().await;
    let _env = EnvVarGuard::set(
        ironclaw_secrets::keychain::SECRETS_MASTER_KEY_ENV,
        "correct horse battery staple pad!!",
    );
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
            runtime_policy: production_runtime_policy(),
            turn_run_wake_notifier: Arc::new(RecordingSchedulerWakeNotifier),
            surface_version: CapabilitySurfaceVersion::new("test-surface").unwrap(),
        })
        .await;

    assert!(matches!(
        result,
        Err(RebornCompositionError::Secret(
            ironclaw_secrets::SecretError::InvalidMasterKey
        ))
    ));
}

fn production_runtime_policy() -> EffectiveRuntimePolicy {
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::LocalSingleUser,
        requested_profile: RuntimeProfile::LocalDev,
        resolved_profile: RuntimeProfile::LocalDev,
        filesystem_backend: FilesystemBackendKind::HostWorkspace,
        process_backend: ProcessBackendKind::LocalHost,
        network_mode: NetworkMode::DirectLogged,
        secret_mode: SecretMode::ScrubbedEnv,
        approval_policy: ApprovalPolicy::AskDestructive,
        audit_mode: AuditMode::LocalMinimal,
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
