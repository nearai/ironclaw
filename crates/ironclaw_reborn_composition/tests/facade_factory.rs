#[cfg(feature = "libsql")]
use std::collections::BTreeMap;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use std::sync::Arc;

#[cfg(any(feature = "libsql", feature = "postgres"))]
use chrono::Utc;
#[cfg(feature = "postgres")]
use deadpool_postgres::tokio_postgres;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_host_api::{
    AgentId, AuditMode, DeploymentMode, EffectKind, FilesystemBackendKind, NetworkMode, PackageId,
    ProcessBackendKind, ProjectId, RuntimeProfile, SecretMode, TenantId, ThreadId, UserId,
    runtime_policy::{ApprovalPolicy, EffectiveRuntimePolicy},
};
#[cfg(feature = "libsql")]
use ironclaw_host_api::{
    CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, ExecutionContext, ExtensionId,
    GrantConstraints, MountView, NetworkPolicy, Principal, RuntimeKind, TrustClass,
};
#[cfg(feature = "libsql")]
use ironclaw_host_runtime::{CapabilitySurfacePolicy, SurfaceKind, VisibleCapabilityRequest};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_host_runtime::{
    SchedulerTurnRunWakeNotifier, TurnRunExecutor, TurnRunExecutorError, TurnRunScheduler,
    TurnRunSchedulerConfig, TurnRunSchedulerHandle,
};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_reborn::planned_driver_factory::PLANNED_DEFAULT_PROFILE_ID;
#[cfg(all(feature = "postgres", not(feature = "libsql")))]
use ironclaw_reborn_composition::RebornCompositionProfile;
#[cfg(feature = "libsql")]
use ironclaw_reborn_composition::{RebornBuildError, RebornCompositionProfile};
use ironclaw_reborn_composition::{RebornBuildInput, RebornReadinessState, build_reborn_services};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_secrets::SecretMaterial;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_trust::{AdminConfig, AdminEntry, HostTrustAssignment, HostTrustPolicy};
#[cfg(feature = "libsql")]
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_turns::{
    AcceptedMessageRef, IdempotencyKey, InMemoryTurnStateStore, ReplyTargetBindingRef,
    RunProfileRequest, SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse, TurnActor,
    TurnScope,
    runner::{ClaimedTurnRun, TurnRunTransitionPort},
};

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn test_master_key() -> SecretMaterial {
    SecretMaterial::from("01234567890123456789012345678901")
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
struct NoopTurnRunExecutor;

#[cfg(any(feature = "libsql", feature = "postgres"))]
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

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn production_trust_policy() -> Arc<HostTrustPolicy> {
    Arc::new(
        HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries([
            AdminEntry::for_admin(
                PackageId::new("reborn-test").unwrap(),
                HostTrustAssignment::first_party(),
                vec![EffectKind::DispatchCapability],
                None,
            ),
        ]))])
        .unwrap(),
    )
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
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

#[cfg(feature = "libsql")]
fn network_denied_runtime_policy() -> EffectiveRuntimePolicy {
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::LocalSingleUser,
        requested_profile: RuntimeProfile::SecureDefault,
        resolved_profile: RuntimeProfile::SecureDefault,
        filesystem_backend: FilesystemBackendKind::ScopedVirtual,
        process_backend: ProcessBackendKind::None,
        network_mode: NetworkMode::Deny,
        secret_mode: SecretMode::BrokeredHandles,
        approval_policy: ApprovalPolicy::AskAlways,
        audit_mode: AuditMode::LocalMinimal,
    }
}

#[cfg(feature = "libsql")]
fn local_dev_builtin_visible_request() -> VisibleCapabilityRequest {
    let grants = CapabilitySet {
        grants: vec![
            local_dev_grant("builtin.echo", vec![EffectKind::DispatchCapability]),
            local_dev_grant(
                "builtin.http",
                vec![EffectKind::DispatchCapability, EffectKind::Network],
            ),
        ],
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
                allowed_effects: vec![EffectKind::DispatchCapability, EffectKind::Network],
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

#[cfg(feature = "libsql")]
fn local_dev_grant(capability: &str, allowed_effects: Vec<EffectKind>) -> CapabilityGrant {
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

#[cfg(feature = "libsql")]
fn empty_trust_policy() -> Arc<HostTrustPolicy> {
    Arc::new(HostTrustPolicy::empty())
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn live_wake_notifier() -> (Arc<SchedulerTurnRunWakeNotifier>, TurnRunSchedulerHandle) {
    let transitions: Arc<dyn TurnRunTransitionPort> = Arc::new(InMemoryTurnStateStore::default());
    let executor: Arc<dyn TurnRunExecutor> = Arc::new(NoopTurnRunExecutor);
    let handle =
        TurnRunScheduler::new(transitions, executor, TurnRunSchedulerConfig::default()).start();
    (handle.wake_notifier(), handle)
}

#[cfg(feature = "libsql")]
async fn libsql_db_at(path: impl AsRef<std::path::Path>) -> Arc<libsql::Database> {
    Arc::new(
        libsql::Builder::new_local(path.as_ref())
            .build()
            .await
            .unwrap(),
    )
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn submit_turn_request(thread: &str, idempotency_key: &str) -> SubmitTurnRequest {
    SubmitTurnRequest {
        scope: TurnScope::new(
            TenantId::new("tenant1").unwrap(),
            Some(AgentId::new("agent1").unwrap()),
            Some(ProjectId::new("project1").unwrap()),
            ThreadId::new(thread).unwrap(),
        ),
        actor: TurnActor::new(UserId::new("user1").unwrap()),
        accepted_message_ref: AcceptedMessageRef::new(format!("message-{thread}")).unwrap(),
        source_binding_ref: SourceBindingRef::new("source-web").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web").unwrap(),
        requested_run_profile: Some(RunProfileRequest::new("default").unwrap()),
        idempotency_key: IdempotencyKey::new(idempotency_key).unwrap(),
        received_at: Utc::now(),
    }
}

#[cfg(feature = "postgres")]
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

#[cfg(feature = "postgres")]
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

#[tokio::test]
async fn disabled_returns_empty_services() {
    let services = build_reborn_services(RebornBuildInput::disabled("test-owner"))
        .await
        .unwrap();

    assert!(services.host_runtime.is_none());
    assert!(services.turn_coordinator.is_none());
    assert_eq!(services.readiness.state, RebornReadinessState::Disabled);
}

#[tokio::test]
async fn local_dev_builds_facades_without_production_claim() {
    let dir = tempfile::tempdir().unwrap();
    let services = build_reborn_services(RebornBuildInput::local_dev(
        "test-owner",
        dir.path().to_path_buf(),
    ))
    .await
    .unwrap();

    assert!(services.host_runtime.is_some());
    assert!(services.turn_coordinator.is_some());
    assert_eq!(services.readiness.state, RebornReadinessState::DevOnly);
    assert!(services.readiness.facades.host_runtime);
    assert!(services.readiness.facades.turn_coordinator);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn local_dev_runtime_policy_hides_http_capability() {
    let dir = tempfile::tempdir().unwrap();
    let services = build_reborn_services(
        RebornBuildInput::local_dev("test-owner", dir.path().to_path_buf())
            .with_runtime_policy(network_denied_runtime_policy()),
    )
    .await
    .unwrap();
    let runtime = services
        .host_runtime
        .expect("local dev exposes host runtime");

    let surface = runtime
        .visible_capabilities(local_dev_builtin_visible_request())
        .await
        .unwrap();
    let visible_ids = surface
        .capabilities
        .iter()
        .map(|capability| capability.descriptor.id.as_str())
        .collect::<Vec<_>>();

    assert!(visible_ids.contains(&"builtin.echo"));
    assert!(
        !visible_ids.contains(&"builtin.http"),
        "local-dev facade must forward the supplied runtime policy before visible-surface filtering"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn production_requires_configured_trust_policy() {
    let dir = tempfile::tempdir().unwrap();
    let db = libsql_db_at(dir.path().join("reborn.db")).await;

    let result = build_reborn_services(RebornBuildInput::libsql(
        RebornCompositionProfile::Production,
        "test-owner",
        db,
        dir.path().join("events.db").to_string_lossy(),
        None,
        test_master_key(),
    ))
    .await;

    assert!(matches!(
        result,
        Err(RebornBuildError::MissingProductionTrustPolicy)
    ));
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn production_rejects_empty_trust_policy() {
    let dir = tempfile::tempdir().unwrap();
    let db = libsql_db_at(dir.path().join("reborn.db")).await;
    let (notifier, handle) = live_wake_notifier();

    let result = build_reborn_services(
        RebornBuildInput::libsql(
            RebornCompositionProfile::Production,
            "test-owner",
            db,
            dir.path().join("events.db").to_string_lossy(),
            None,
            test_master_key(),
        )
        .with_production_trust_policy(empty_trust_policy())
        .with_turn_run_wake_notifier(notifier),
    )
    .await;

    handle.shutdown().await;

    assert!(matches!(
        result,
        Err(RebornBuildError::EmptyProductionTrustPolicy)
    ));
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn production_requires_live_turn_wake_notifier() {
    let dir = tempfile::tempdir().unwrap();
    let db = libsql_db_at(dir.path().join("reborn.db")).await;

    let result = build_reborn_services(
        RebornBuildInput::libsql(
            RebornCompositionProfile::Production,
            "test-owner",
            db,
            dir.path().join("events.db").to_string_lossy(),
            None,
            test_master_key(),
        )
        .with_production_trust_policy(production_trust_policy())
        .with_runtime_policy(production_runtime_policy()),
    )
    .await;

    assert!(matches!(
        result,
        Err(RebornBuildError::MissingTurnRunWakeNotifier)
    ));
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn production_requires_runtime_policy() {
    let dir = tempfile::tempdir().unwrap();
    let db = libsql_db_at(dir.path().join("reborn.db")).await;
    let (notifier, handle) = live_wake_notifier();

    let result = build_reborn_services(
        RebornBuildInput::libsql(
            RebornCompositionProfile::Production,
            "test-owner",
            db,
            dir.path().join("events.db").to_string_lossy(),
            None,
            test_master_key(),
        )
        .with_production_trust_policy(production_trust_policy())
        .with_turn_run_wake_notifier(notifier),
    )
    .await;

    handle.shutdown().await;

    assert!(matches!(
        result,
        Err(RebornBuildError::MissingRuntimePolicy)
    ));
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn production_rejects_memory_libsql_event_store() {
    let db = Arc::new(
        libsql::Builder::new_local(":memory:")
            .build()
            .await
            .unwrap(),
    );
    let (notifier, handle) = live_wake_notifier();

    let result = build_reborn_services(
        RebornBuildInput::libsql(
            RebornCompositionProfile::Production,
            "test-owner",
            db,
            ":memory:",
            None,
            test_master_key(),
        )
        .with_production_trust_policy(production_trust_policy())
        .with_runtime_policy(production_runtime_policy())
        .with_turn_run_wake_notifier(notifier),
    )
    .await;

    handle.shutdown().await;

    let error = result.expect_err("production must reject in-memory event store");
    let rendered = error.to_string();
    assert!(!rendered.contains("postgres://"));
    assert!(!rendered.contains("token"));
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn migration_dry_run_validates_libsql_shape() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("reborn.db");
    let db = libsql_db_at(&db_path).await;
    let (notifier, handle) = live_wake_notifier();

    let services = build_reborn_services(
        RebornBuildInput::libsql(
            RebornCompositionProfile::MigrationDryRun,
            "test-owner",
            db,
            db_path.to_string_lossy(),
            None,
            test_master_key(),
        )
        .with_production_trust_policy(production_trust_policy())
        .with_runtime_policy(production_runtime_policy())
        .with_turn_run_wake_notifier(notifier),
    )
    .await
    .unwrap();

    let response = services
        .turn_coordinator
        .as_ref()
        .expect("migration dry-run must expose turn coordinator")
        .submit_turn(submit_turn_request(
            "thread-planned-profile",
            "idem-planned-profile",
        ))
        .await
        .unwrap();
    let SubmitTurnResponse::Accepted {
        resolved_run_profile_id,
        ..
    } = response;
    assert_eq!(resolved_run_profile_id.as_str(), PLANNED_DEFAULT_PROFILE_ID);

    handle.shutdown().await;

    assert_eq!(
        services.readiness.state,
        RebornReadinessState::MigrationDryRunValidated
    );
    assert!(services.host_runtime.is_some());
    assert!(services.turn_coordinator.is_some());
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn migration_dry_run_validates_postgres_planned_turn_profile() {
    let Some((_container, pool, database_url)) = postgres_pool_or_skip().await else {
        return;
    };
    let (notifier, handle) = live_wake_notifier();

    let services = build_reborn_services(
        RebornBuildInput::postgres(
            RebornCompositionProfile::MigrationDryRun,
            "test-owner",
            pool,
            SecretMaterial::from(database_url),
            test_master_key(),
        )
        .with_production_trust_policy(production_trust_policy())
        .with_runtime_policy(production_runtime_policy())
        .with_turn_run_wake_notifier(notifier),
    )
    .await
    .unwrap();

    let response = services
        .turn_coordinator
        .as_ref()
        .expect("migration dry-run must expose turn coordinator")
        .submit_turn(submit_turn_request(
            "thread-postgres-planned-profile",
            "idem-postgres-planned-profile",
        ))
        .await
        .unwrap();
    let SubmitTurnResponse::Accepted {
        resolved_run_profile_id,
        ..
    } = response;
    assert_eq!(resolved_run_profile_id.as_str(), PLANNED_DEFAULT_PROFILE_ID);

    handle.shutdown().await;

    assert_eq!(
        services.readiness.state,
        RebornReadinessState::MigrationDryRunValidated
    );
    assert!(services.host_runtime.is_some());
    assert!(services.turn_coordinator.is_some());
}
