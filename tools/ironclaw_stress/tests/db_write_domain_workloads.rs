use std::{env, error::Error, future::Future, path::PathBuf, sync::Arc};

use chrono::{Duration, TimeZone, Utc};
use ironclaw_auth::{
    AuthChallenge, AuthContinuationRef, AuthFlowKind, AuthFlowStatus, AuthProductScope,
    AuthProviderId, AuthSurface, AuthorizationCodeHash, CredentialAccountLabel,
    CredentialAccountLookupRequest, NewAuthFlow, OAuthAuthorizationCode, OAuthAuthorizationUrl,
    OAuthProviderCallbackRequest, OpaqueStateHash, PkceVerifierHash, PkceVerifierSecret,
    ProviderScope, RebornOAuthCallbackOutcome, RebornOAuthCallbackRequest,
};
use ironclaw_composition::test_support::build_oauth_product_auth_for_test_on_root;
use ironclaw_filesystem::{
    LibSqlRootFilesystem, PostgresRootFilesystem, RootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::{
    ids::{
        AgentId, CapabilityId, InvocationId, ProjectId, ProviderToolName, TenantId, ThreadId,
        UserId,
    },
    mount::{MountGrant, MountPermissions, MountView},
    path::{MountAlias, VirtualPath},
    resource::ResourceScope,
    turn::TurnRunId,
};
use ironclaw_libsql_runtime::LibSqlRuntime;
use ironclaw_stress::db_probe::{
    DbProbeConfig, DbProbeError, DbProbeSummary, DbWriteMeasurement, StatsScope, begin,
    capture_settled, finish, postgres_relation_write_totals, summarize_measurement,
};
use ironclaw_threads::{
    AcceptInboundMessageRequest, AppendCapabilityDisplayPreviewRequest,
    AppendFinalizedAssistantMessageRequest, AppendToolResultReferenceRequest,
    CapabilityDisplayPreviewEnvelope, CapabilityDisplayPreviewEnvelopeInput,
    CapabilityDisplayPreviewStatus, EnsureThreadRequest, FilesystemSessionThreadService,
    FinalizedAssistantMessageByRunRequest, ListThreadsForScopeRequest, MessageContent,
    ProviderToolCallReferenceEnvelope, SessionThreadService, ThreadHistoryRequest, ThreadScope,
    ToolResultSafeSummary,
};
use ironclaw_triggers::{
    ClaimDueFireOutcome, ClaimDueFireRequest, ClearActiveFireRequest, FireAcceptedRequest,
    LibSqlTriggerRepository, PostgresTriggerRepository, TriggerId, TriggerRecord,
    TriggerRepository, TriggerRunHistoryStatus, TriggerSchedule, TriggerSourceKind, TriggerState,
};
use secrecy::SecretString;
use serde::Serialize;
use uuid::Uuid;

#[tokio::test]
async fn trigger_history_pruning_libsql() -> Result<(), Box<dyn Error>> {
    emit(run_libsql(FixedWorkload::TriggerHistoryPruning).await?)?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires IRONCLAW_FILESYSTEM_POSTGRES_URL"]
async fn trigger_history_pruning_postgres() -> Result<(), Box<dyn Error>> {
    emit(run_postgres(FixedWorkload::TriggerHistoryPruning).await?)?;
    Ok(())
}

#[tokio::test]
async fn oauth_callback_durability_libsql() -> Result<(), Box<dyn Error>> {
    emit(run_libsql(FixedWorkload::OauthCallbackDurability).await?)?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires IRONCLAW_FILESYSTEM_POSTGRES_URL"]
async fn oauth_callback_durability_postgres() -> Result<(), Box<dyn Error>> {
    emit(run_postgres(FixedWorkload::OauthCallbackDurability).await?)?;
    Ok(())
}

#[tokio::test]
async fn thread_activity_coalescing_libsql() -> Result<(), Box<dyn Error>> {
    emit(run_libsql(FixedWorkload::ThreadActivityCoalescing).await?)?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires IRONCLAW_FILESYSTEM_POSTGRES_URL"]
async fn thread_activity_coalescing_postgres() -> Result<(), Box<dyn Error>> {
    emit(run_postgres(FixedWorkload::ThreadActivityCoalescing).await?)?;
    Ok(())
}

#[tokio::test]
async fn message_lookup_row_folding_libsql() -> Result<(), Box<dyn Error>> {
    emit(run_libsql(FixedWorkload::MessageLookupRowFolding).await?)?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires IRONCLAW_FILESYSTEM_POSTGRES_URL"]
async fn message_lookup_row_folding_postgres() -> Result<(), Box<dyn Error>> {
    emit(run_postgres(FixedWorkload::MessageLookupRowFolding).await?)?;
    Ok(())
}

#[test]
fn workload_registry_metadata_is_complete() {
    let registry = workload_registry();
    let cursor_entry = registry
        .iter()
        .find(|entry| entry.issue == 7597)
        .expect("outbound cursor removal must stay registered");

    assert_eq!(cursor_entry.workload, None);
    assert_eq!(cursor_entry.coverage, CoverageKind::CompileOnly);
    assert!(cursor_entry.reason.contains("does not verify API absence"));
    assert!(cursor_entry.reason.contains("registry metadata"));

    assert!(
        registry
            .iter()
            .filter(|entry| entry.coverage == CoverageKind::RuntimeMeasurement)
            .all(|entry| entry.workload.is_some())
    );
    for workload in [
        FixedWorkload::TriggerHistoryPruning,
        FixedWorkload::OauthCallbackDurability,
        FixedWorkload::ThreadActivityCoalescing,
        FixedWorkload::MessageLookupRowFolding,
    ] {
        assert_eq!(
            registry
                .iter()
                .filter(|entry| entry.workload == Some(workload))
                .count(),
            1,
            "{workload:?} must occur exactly once"
        );
    }
}

async fn run_libsql(
    workload: FixedWorkload,
) -> Result<FixedWorkloadMeasurement, FixedWorkloadError> {
    let dir = tempfile::tempdir().map_err(|source| {
        FixedWorkloadError::setup("libsql", "create temporary database directory", source)
    })?;
    run_fixed_workload(
        workload,
        WorkloadBackend::LibSql {
            path: dir.path().join("domain-write-measurement.db"),
        },
    )
    .await
}

async fn run_postgres(
    workload: FixedWorkload,
) -> Result<FixedWorkloadMeasurement, FixedWorkloadError> {
    let url = env::var("IRONCLAW_FILESYSTEM_POSTGRES_URL").map_err(|source| {
        FixedWorkloadError::setup("postgres", "read IRONCLAW_FILESYSTEM_POSTGRES_URL", source)
    })?;
    run_fixed_workload(workload, WorkloadBackend::Postgres { url }).await
}

fn emit(measurement: FixedWorkloadMeasurement) -> Result<(), serde_json::Error> {
    println!("{}", serde_json::to_string(&measurement)?);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FixedWorkload {
    TriggerHistoryPruning,
    OauthCallbackDurability,
    ThreadActivityCoalescing,
    MessageLookupRowFolding,
}

impl FixedWorkload {
    const fn name(self) -> &'static str {
        match self {
            Self::TriggerHistoryPruning => "trigger-history-pruning",
            Self::OauthCallbackDurability => "oauth-callback-durability",
            Self::ThreadActivityCoalescing => "thread-activity-coalescing",
            Self::MessageLookupRowFolding => "message-lookup-row-folding",
        }
    }

    const fn changed_tables(self) -> &'static [&'static str] {
        match self {
            Self::TriggerHistoryPruning => &["trigger_records", "trigger_run_history"],
            Self::OauthCallbackDurability
            | Self::ThreadActivityCoalescing
            | Self::MessageLookupRowFolding => &["root_filesystem_entries"],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CoverageKind {
    RuntimeMeasurement,
    CompileOnly,
}

struct WorkloadRegistryEntry {
    issue: u32,
    workload: Option<FixedWorkload>,
    coverage: CoverageKind,
    reason: &'static str,
}

fn workload_registry() -> &'static [WorkloadRegistryEntry] {
    const ENTRIES: &[WorkloadRegistryEntry] = &[
        WorkloadRegistryEntry {
            issue: 7595,
            workload: Some(FixedWorkload::TriggerHistoryPruning),
            coverage: CoverageKind::RuntimeMeasurement,
            reason: "repeated claim, accept, and complete changes trigger history",
        },
        WorkloadRegistryEntry {
            issue: 7597,
            workload: None,
            coverage: CoverageKind::CompileOnly,
            reason: "registry metadata records zero production callers; this does not verify API absence",
        },
        WorkloadRegistryEntry {
            issue: 7604,
            workload: Some(FixedWorkload::OauthCallbackDurability),
            coverage: CoverageKind::RuntimeMeasurement,
            reason: "cross-worker durable OAuth callback state machine",
        },
        WorkloadRegistryEntry {
            issue: 7596,
            workload: Some(FixedWorkload::ThreadActivityCoalescing),
            coverage: CoverageKind::RuntimeMeasurement,
            reason: "burst activity on one durable thread",
        },
        WorkloadRegistryEntry {
            issue: 7605,
            workload: Some(FixedWorkload::MessageLookupRowFolding),
            coverage: CoverageKind::RuntimeMeasurement,
            reason: "message lookup write and read-back matrix",
        },
    ];
    ENTRIES
}

enum WorkloadBackend {
    LibSql { path: PathBuf },
    Postgres { url: String },
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
enum BackendName {
    LibSql,
    Postgres,
}

impl WorkloadBackend {
    const fn name(&self) -> BackendName {
        match self {
            Self::LibSql { .. } => BackendName::LibSql,
            Self::Postgres { .. } => BackendName::Postgres,
        }
    }
}

#[derive(Serialize)]
struct FixedWorkloadMeasurement {
    workload: &'static str,
    backend: BackendName,
    #[serde(flatten)]
    summary: DbProbeSummary,
}

type BoxError = Box<dyn Error + Send + Sync>;

#[derive(Debug, thiserror::Error)]
enum FixedWorkloadError {
    #[error("{backend} workload setup failed while attempting to {operation}: {source}")]
    Setup {
        backend: &'static str,
        operation: &'static str,
        #[source]
        source: BoxError,
    },
    #[error("database probe failed: {source}")]
    Probe {
        #[source]
        source: DbProbeError,
    },
    #[error("workload failed: {source}")]
    Workload {
        #[source]
        source: WorkloadFailure,
    },
    #[error("{primary}; database probe cleanup also failed: {cleanup}")]
    WorkloadCleanup {
        #[source]
        primary: Box<WorkloadFailure>,
        cleanup: Box<DbProbeError>,
    },
    #[error("{primary}; database probe cleanup also failed: {cleanup}")]
    CaptureCleanup {
        #[source]
        primary: Box<DbProbeError>,
        cleanup: Box<DbProbeError>,
    },
    #[error("{backend} workload {workload} produced no writes for required table {table}")]
    NonzeroBaseline {
        backend: &'static str,
        workload: &'static str,
        table: &'static str,
    },
}

impl FixedWorkloadError {
    fn setup(
        backend: &'static str,
        operation: &'static str,
        source: impl Error + Send + Sync + 'static,
    ) -> Self {
        Self::Setup {
            backend,
            operation,
            source: Box::new(source),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{operation}: {source}")]
struct WorkloadFailure {
    operation: &'static str,
    #[source]
    source: BoxError,
}

impl WorkloadFailure {
    fn source(operation: &'static str, source: impl Error + Send + Sync + 'static) -> Self {
        Self {
            operation,
            source: Box::new(source),
        }
    }

    fn message(operation: &'static str, message: impl Into<String>) -> Self {
        Self::source(operation, std::io::Error::other(message.into()))
    }
}

async fn run_fixed_workload(
    workload: FixedWorkload,
    backend: WorkloadBackend,
) -> Result<FixedWorkloadMeasurement, FixedWorkloadError> {
    let backend_name = backend.name();
    match backend {
        WorkloadBackend::LibSql { path } => {
            let database = Arc::new(libsql::Builder::new_local(&path).build().await.map_err(
                |source| FixedWorkloadError::setup("libsql", "open durable database", source),
            )?);
            let runtime = Arc::new(LibSqlRuntime::new(database).map_err(|source| {
                FixedWorkloadError::setup("libsql", "build shared database runtime", source)
            })?);
            let root = Arc::new(LibSqlRootFilesystem::from_runtime(Arc::clone(&runtime)));
            root.run_migrations().await.map_err(|source| {
                FixedWorkloadError::setup("libsql", "run filesystem migrations", source)
            })?;
            let config = DbProbeConfig::libsql(path, false);

            if workload == FixedWorkload::TriggerHistoryPruning {
                let repository = Arc::new(LibSqlTriggerRepository::from_runtime(runtime));
                repository.run_migrations().await.map_err(|source| {
                    FixedWorkloadError::setup("libsql", "run trigger migrations", source)
                })?;
                run_trigger_measurement(repository, root, config, backend_name).await
            } else {
                run_root_measurement(workload, root, config, backend_name).await
            }
        }
        WorkloadBackend::Postgres { url } => {
            let postgres_config = url.parse::<tokio_postgres::Config>().map_err(|source| {
                FixedWorkloadError::setup("postgres", "parse database URL", source)
            })?;
            let manager = deadpool_postgres::Manager::new(postgres_config, tokio_postgres::NoTls);
            let pool = deadpool_postgres::Pool::builder(manager)
                .max_size(8)
                .build()
                .map_err(|source| {
                    FixedWorkloadError::setup("postgres", "build connection pool", source)
                })?;
            let root = Arc::new(PostgresRootFilesystem::new(pool.clone()));
            root.run_migrations().await.map_err(|source| {
                FixedWorkloadError::setup("postgres", "run filesystem migrations", source)
            })?;
            let config = DbProbeConfig::postgres(url, false);

            if workload == FixedWorkload::TriggerHistoryPruning {
                let repository = Arc::new(PostgresTriggerRepository::new(pool));
                repository.run_migrations().await.map_err(|source| {
                    FixedWorkloadError::setup("postgres", "run trigger migrations", source)
                })?;
                run_trigger_measurement(repository, root, config, backend_name).await
            } else {
                drop(pool);
                run_root_measurement(workload, root, config, backend_name).await
            }
        }
    }
}

async fn measure_fixed<F, Fut>(
    workload: FixedWorkload,
    backend: BackendName,
    config: DbProbeConfig,
    operation: F,
) -> Result<FixedWorkloadMeasurement, FixedWorkloadError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<(), WorkloadFailure>>,
{
    let before = begin(&config)
        .await
        .map_err(|source| FixedWorkloadError::Probe { source })?;

    if let Err(primary) = operation().await {
        return match finish(&config).await {
            Ok(()) => Err(FixedWorkloadError::Workload { source: primary }),
            Err(cleanup) => Err(FixedWorkloadError::WorkloadCleanup {
                primary: Box::new(primary),
                cleanup: Box::new(cleanup),
            }),
        };
    }

    let after = match capture_settled(&config).await {
        Ok(after) => after,
        Err(primary) => {
            return match finish(&config).await {
                Ok(()) => Err(FixedWorkloadError::Probe { source: primary }),
                Err(cleanup) => Err(FixedWorkloadError::CaptureCleanup {
                    primary: Box::new(primary),
                    cleanup: Box::new(cleanup),
                }),
            };
        }
    };

    finish(&config)
        .await
        .map_err(|source| FixedWorkloadError::Probe { source })?;

    let summary = summarize_measurement(
        before,
        after,
        None,
        DbWriteMeasurement {
            workload: workload.name().to_string(),
            tool_calls_per_turn: 0,
            idle_observation_seconds: 0,
            reset_stats: config.reset_stats(),
            stats_scope: if config.reset_stats() {
                StatsScope::ExplicitResetCurrentDatabase
            } else {
                StatsScope::SnapshotDeltaCurrentDatabase
            },
        },
    );
    assert_nonzero_baseline(workload, backend, &summary)?;

    Ok(FixedWorkloadMeasurement {
        workload: workload.name(),
        backend,
        summary,
    })
}

fn assert_nonzero_baseline(
    workload: FixedWorkload,
    backend: BackendName,
    summary: &DbProbeSummary,
) -> Result<(), FixedWorkloadError> {
    let postgres_totals = postgres_relation_write_totals(&summary.delta.postgres_table_writes);
    for table in workload.changed_tables() {
        let writes = match backend {
            BackendName::LibSql => summary
                .delta
                .libsql_table_writes
                .iter()
                .find(|row| row.table == *table)
                .map(|row| row.inserts + row.updates + row.deletes)
                .unwrap_or_default(),
            BackendName::Postgres => postgres_totals.get(*table).copied().unwrap_or_default(),
        };
        if writes <= 0 {
            return Err(FixedWorkloadError::NonzeroBaseline {
                backend: backend.as_str(),
                workload: workload.name(),
                table,
            });
        }
    }
    Ok(())
}

impl BackendName {
    const fn as_str(self) -> &'static str {
        match self {
            Self::LibSql => "libsql",
            Self::Postgres => "postgres",
        }
    }
}

async fn run_trigger_measurement<R, F>(
    repository: Arc<R>,
    root: Arc<F>,
    config: DbProbeConfig,
    backend: BackendName,
) -> Result<FixedWorkloadMeasurement, FixedWorkloadError>
where
    R: TriggerRepository + 'static,
    F: RootFilesystem + 'static,
{
    let fixture = seed_trigger(repository.as_ref())
        .await
        .map_err(|source| FixedWorkloadError::Workload { source })?;
    drop(root);
    measure_fixed(
        FixedWorkload::TriggerHistoryPruning,
        backend,
        config,
        move || run_trigger_lifecycle(repository, fixture),
    )
    .await
}

struct TriggerFixture {
    tenant_id: TenantId,
    trigger_id: TriggerId,
    first_fire_slot: chrono::DateTime<Utc>,
}

async fn seed_trigger(
    repository: &impl TriggerRepository,
) -> Result<TriggerFixture, WorkloadFailure> {
    let trigger_id = TriggerId::new();
    let tenant_id = TenantId::new("tenant-db-write-trigger")
        .map_err(|source| WorkloadFailure::source("build trigger tenant", source))?;
    let creator_user_id = UserId::new("user-db-write-trigger")
        .map_err(|source| WorkloadFailure::source("build trigger user", source))?;
    let agent_id = AgentId::new("agent-db-write-trigger")
        .map_err(|source| WorkloadFailure::source("build trigger agent", source))?;
    let project_id = ProjectId::new("project-db-write-trigger")
        .map_err(|source| WorkloadFailure::source("build trigger project", source))?;
    let first_fire_slot = Utc
        .with_ymd_and_hms(2026, 8, 14, 8, 0, 0)
        .single()
        .ok_or_else(|| WorkloadFailure::message("build trigger timestamp", "invalid timestamp"))?;
    let schedule = TriggerSchedule::cron("0 8 * * *")
        .map_err(|source| WorkloadFailure::source("build trigger schedule", source))?;
    repository
        .upsert_trigger(TriggerRecord {
            trigger_id,
            tenant_id: tenant_id.clone(),
            creator_user_id,
            agent_id: Some(agent_id),
            project_id: Some(project_id),
            name: "DB-write trigger history".to_string(),
            source: TriggerSourceKind::Schedule,
            schedule,
            prompt: "measure trigger history writes".to_string(),
            execution_spec: None,
            delivery_target: None,
            state: TriggerState::Scheduled,
            next_run_at: first_fire_slot,
            last_run_at: None,
            last_fired_slot: None,
            last_status: None,
            active_fire_slot: None,
            active_run_ref: None,
            created_at: first_fire_slot - Duration::minutes(1),
        })
        .await
        .map_err(|source| WorkloadFailure::source("seed trigger record", source))?;

    Ok(TriggerFixture {
        tenant_id,
        trigger_id,
        first_fire_slot,
    })
}

async fn run_trigger_lifecycle<R>(
    repository: Arc<R>,
    fixture: TriggerFixture,
) -> Result<(), WorkloadFailure>
where
    R: TriggerRepository + 'static,
{
    let mut fire_slot = fixture.first_fire_slot;
    for _ in 0..16 {
        let claim = repository
            .claim_due_fire(ClaimDueFireRequest {
                tenant_id: fixture.tenant_id.clone(),
                trigger_id: fixture.trigger_id,
                fire_slot,
                now: fire_slot,
            })
            .await
            .map_err(|source| WorkloadFailure::source("claim due trigger fire", source))?;
        if !matches!(claim, ClaimDueFireOutcome::Claimed(_)) {
            return Err(WorkloadFailure::message(
                "claim due trigger fire",
                format!("expected claimed outcome, got {claim:?}"),
            ));
        }

        let run_id_text = Uuid::new_v4().to_string();
        let run_id = TurnRunId::parse(&run_id_text)
            .map_err(|source| WorkloadFailure::source("build trigger run id", source))?;
        let thread_id = ThreadId::new(Uuid::new_v4().to_string())
            .map_err(|source| WorkloadFailure::source("build trigger thread id", source))?;
        let accepted = repository
            .mark_fire_accepted(FireAcceptedRequest {
                tenant_id: fixture.tenant_id.clone(),
                trigger_id: fixture.trigger_id,
                fire_slot,
                run_id,
                thread_id,
                submitted_at: fire_slot + Duration::seconds(1),
            })
            .await
            .map_err(|source| WorkloadFailure::source("accept trigger fire", source))?;
        if accepted.is_none() {
            return Err(WorkloadFailure::message(
                "accept trigger fire",
                "claimed trigger disappeared before acceptance",
            ));
        }

        let completed = repository
            .clear_active_fire(ClearActiveFireRequest {
                tenant_id: fixture.tenant_id.clone(),
                trigger_id: fixture.trigger_id,
                fire_slot,
                run_id,
                status: TriggerRunHistoryStatus::Ok,
            })
            .await
            .map_err(|source| WorkloadFailure::source("complete trigger fire", source))?;
        if completed.is_none() {
            return Err(WorkloadFailure::message(
                "complete trigger fire",
                "accepted trigger disappeared before completion",
            ));
        }

        fire_slot = repository
            .get_trigger(fixture.tenant_id.clone(), fixture.trigger_id)
            .await
            .map_err(|source| WorkloadFailure::source("read completed trigger", source))?
            .ok_or_else(|| {
                WorkloadFailure::message("read completed trigger", "trigger record missing")
            })?
            .next_run_at;
    }

    let history = repository
        .list_trigger_run_history(fixture.tenant_id, fixture.trigger_id, 32)
        .await
        .map_err(|source| WorkloadFailure::source("read trigger run history", source))?;
    if history.len() != 16
        || history
            .iter()
            .any(|entry| entry.status != TriggerRunHistoryStatus::Ok)
    {
        return Err(WorkloadFailure::message(
            "read trigger run history",
            format!("expected 16 completed rows, got {}", history.len()),
        ));
    }
    Ok(())
}

async fn run_root_measurement<F>(
    workload: FixedWorkload,
    root: Arc<F>,
    config: DbProbeConfig,
    backend: BackendName,
) -> Result<FixedWorkloadMeasurement, FixedWorkloadError>
where
    F: RootFilesystem + 'static,
{
    match workload {
        FixedWorkload::OauthCallbackDurability => {
            run_oauth_measurement(root, config, backend).await
        }
        FixedWorkload::ThreadActivityCoalescing => {
            run_thread_activity_measurement(root, config, backend).await
        }
        FixedWorkload::MessageLookupRowFolding => {
            run_message_lookup_measurement(root, config, backend).await
        }
        FixedWorkload::TriggerHistoryPruning => unreachable!("trigger repository dispatched above"),
    }
}

async fn run_oauth_measurement<F>(
    root: Arc<F>,
    config: DbProbeConfig,
    backend: BackendName,
) -> Result<FixedWorkloadMeasurement, FixedWorkloadError>
where
    F: RootFilesystem + 'static,
{
    let first_worker = build_oauth_product_auth_for_test_on_root(Arc::clone(&root)).await;
    let callback_worker = build_oauth_product_auth_for_test_on_root(root).await;
    measure_fixed(
        FixedWorkload::OauthCallbackDurability,
        backend,
        config,
        move || async move {
            let scope = oauth_scope()?;
            let provider = AuthProviderId::new("test-oauth-provider")
                .map_err(|source| WorkloadFailure::source("build OAuth provider", source))?;
            let state_hash = OpaqueStateHash::new(hex64(0xaa))
                .map_err(|source| WorkloadFailure::source("build OAuth state hash", source))?;
            let pkce_hash = PkceVerifierHash::new(hex64(0xbb))
                .map_err(|source| WorkloadFailure::source("build PKCE hash", source))?;
            let code_hash = AuthorizationCodeHash::new(hex64(0xcc)).map_err(|source| {
                WorkloadFailure::source("build authorization code hash", source)
            })?;
            let expires_at = Utc::now() + Duration::minutes(5);

            let flow = first_worker
                .services
                .flow_manager()
                .create_flow(NewAuthFlow {
                    requested_scopes: Vec::new(),
                    id: None,
                    scope: scope.clone(),
                    kind: AuthFlowKind::IntegrationCredential,
                    provider: provider.clone(),
                    challenge: AuthChallenge::OAuthUrl {
                        authorization_url: OAuthAuthorizationUrl::new(
                            "https://accounts.example.com/o/oauth2/auth",
                        )
                        .map_err(|source| {
                            WorkloadFailure::source("build OAuth authorization URL", source)
                        })?,
                        expires_at,
                    },
                    continuation: AuthContinuationRef::SetupOnly,
                    update_binding: None,
                    opaque_state_hash: Some(state_hash.clone()),
                    pkce_verifier_hash: Some(pkce_hash.clone()),
                    expires_at,
                    requester_extension: None,
                })
                .await
                .map_err(|source| WorkloadFailure::source("start durable OAuth flow", source))?;

            let callback = callback_worker
                .services
                .handle_oauth_callback(RebornOAuthCallbackRequest {
                    scope: scope.clone(),
                    flow_id: flow.id,
                    opaque_state_hash: state_hash,
                    outcome: RebornOAuthCallbackOutcome::Authorized {
                        provider_request: OAuthProviderCallbackRequest {
                            provider: provider.clone(),
                            account_label: CredentialAccountLabel::new("DB-write account")
                                .map_err(|source| {
                                    WorkloadFailure::source("build OAuth account label", source)
                                })?,
                            authorization_code: OAuthAuthorizationCode::new(SecretString::from(
                                "db-write-authorization-code".to_string(),
                            ))
                            .map_err(|source| {
                                WorkloadFailure::source("build OAuth authorization code", source)
                            })?,
                            authorization_code_hash: code_hash,
                            pkce_verifier: PkceVerifierSecret::new(SecretString::from(
                                "db-write-pkce-verifier".to_string(),
                            ))
                            .map_err(|source| {
                                WorkloadFailure::source("build PKCE verifier", source)
                            })?,
                            pkce_verifier_hash: pkce_hash,
                            scopes: vec![ProviderScope::new("test.readonly").map_err(
                                |source| {
                                    WorkloadFailure::source("build OAuth provider scope", source)
                                },
                            )?],
                        },
                    },
                })
                .await
                .map_err(|source| {
                    WorkloadFailure::message(
                        "claim and complete OAuth callback",
                        format!("code={:?}, retryable={}", source.code, source.retryable),
                    )
                })?;
            let account_id = callback.credential_account_id.ok_or_else(|| {
                WorkloadFailure::message(
                    "read OAuth callback result",
                    "completed callback omitted credential account id",
                )
            })?;

            let persisted_flow = first_worker
                .services
                .flow_manager()
                .get_flow(&scope, flow.id)
                .await
                .map_err(|source| WorkloadFailure::source("read completed OAuth flow", source))?
                .ok_or_else(|| {
                    WorkloadFailure::message(
                        "read completed OAuth flow",
                        "durable flow disappeared across workers",
                    )
                })?;
            if persisted_flow.status != AuthFlowStatus::Completed {
                return Err(WorkloadFailure::message(
                    "read completed OAuth flow",
                    format!("expected completed status, got {:?}", persisted_flow.status),
                ));
            }

            let account = first_worker
                .services
                .credential_account_service()
                .get_account(CredentialAccountLookupRequest::new(scope, account_id))
                .await
                .map_err(|source| {
                    WorkloadFailure::source("read cross-worker credential account", source)
                })?
                .ok_or_else(|| {
                    WorkloadFailure::message(
                        "read cross-worker credential account",
                        "credential account was not durable",
                    )
                })?;
            if account.id != account_id || account.provider != provider {
                return Err(WorkloadFailure::message(
                    "read cross-worker credential account",
                    "credential account identity did not round trip",
                ));
            }
            Ok(())
        },
    )
    .await
}

fn oauth_scope() -> Result<AuthProductScope, WorkloadFailure> {
    let user_id = UserId::new(format!("test-user-{}", Uuid::new_v4()))
        .map_err(|source| WorkloadFailure::source("build OAuth user", source))?;
    let resource = ResourceScope::local_default(user_id, InvocationId::new())
        .map_err(|source| WorkloadFailure::source("build OAuth resource scope", source))?;
    Ok(AuthProductScope::new(resource, AuthSurface::Callback))
}

fn hex64(fill: u8) -> String {
    format!("{fill:02x}").repeat(32)
}

struct ThreadFixture<F>
where
    F: RootFilesystem,
{
    service: Arc<FilesystemSessionThreadService<F>>,
    scope: ThreadScope,
    thread_id: ThreadId,
}

async fn prepare_thread_fixture<F>(
    root: Arc<F>,
    label: &'static str,
    title: Option<String>,
) -> Result<ThreadFixture<F>, FixedWorkloadError>
where
    F: RootFilesystem + 'static,
{
    let unique_label = format!("{label}-{}", Uuid::new_v4());
    let scope =
        thread_scope(&unique_label).map_err(|source| FixedWorkloadError::Workload { source })?;
    let target = format!(
        "/tenants/{}/users/{}/threads",
        scope.tenant_id.as_str(),
        scope
            .owner_user_id
            .as_ref()
            .expect("fixed workload always has an owner")
            .as_str()
    );
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/threads").map_err(|source| {
            FixedWorkloadError::setup("filesystem", "build threads mount alias", source)
        })?,
        VirtualPath::new(target).map_err(|source| {
            FixedWorkloadError::setup("filesystem", "build threads mount target", source)
        })?,
        MountPermissions::read_write_list_delete(),
    )])
    .map_err(|source| {
        FixedWorkloadError::setup("filesystem", "build threads mount view", source)
    })?;
    let scoped = Arc::new(ScopedFilesystem::with_fixed_view(root, mounts));
    let service = Arc::new(FilesystemSessionThreadService::new(scoped));
    let thread_id = ThreadId::new(Uuid::new_v4().to_string())
        .map_err(|source| FixedWorkloadError::setup("filesystem", "build thread id", source))?;
    service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(thread_id.clone()),
            created_by_actor_id: "db-write-workload".to_string(),
            title,
            metadata_json: None,
        })
        .await
        .map_err(|source| FixedWorkloadError::setup("filesystem", "seed durable thread", source))?;

    Ok(ThreadFixture {
        service,
        scope,
        thread_id,
    })
}

fn thread_scope(label: &str) -> Result<ThreadScope, WorkloadFailure> {
    Ok(ThreadScope {
        tenant_id: TenantId::new(format!("tenant-{label}"))
            .map_err(|source| WorkloadFailure::source("build thread tenant", source))?,
        agent_id: AgentId::new(format!("agent-{label}"))
            .map_err(|source| WorkloadFailure::source("build thread agent", source))?,
        project_id: Some(
            ProjectId::new(format!("project-{label}"))
                .map_err(|source| WorkloadFailure::source("build thread project", source))?,
        ),
        owner_user_id: Some(
            UserId::new(format!("user-{label}"))
                .map_err(|source| WorkloadFailure::source("build thread owner", source))?,
        ),
        mission_id: None,
    })
}

async fn run_thread_activity_measurement<F>(
    root: Arc<F>,
    config: DbProbeConfig,
    backend: BackendName,
) -> Result<FixedWorkloadMeasurement, FixedWorkloadError>
where
    F: RootFilesystem + 'static,
{
    let fixture = prepare_thread_fixture(
        root,
        "db-write-thread-activity",
        Some("Thread activity workload".to_string()),
    )
    .await?;
    measure_fixed(
        FixedWorkload::ThreadActivityCoalescing,
        backend,
        config,
        move || run_thread_activity_burst(fixture),
    )
    .await
}

async fn run_thread_activity_burst<F>(fixture: ThreadFixture<F>) -> Result<(), WorkloadFailure>
where
    F: RootFilesystem + 'static,
{
    for message_index in 0..32 {
        fixture
            .service
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: fixture.scope.clone(),
                thread_id: fixture.thread_id.clone(),
                actor_id: "db-write-workload".to_string(),
                source_binding_id: Some("db-write-thread-activity".to_string()),
                reply_target_binding_id: Some("db-write-thread-activity-reply".to_string()),
                external_event_id: Some(format!("activity-message-{message_index}")),
                content: MessageContent::text(format!(
                    "thread activity burst message {message_index}"
                )),
            })
            .await
            .map_err(|source| WorkloadFailure::source("append thread activity message", source))?;
    }

    let history = fixture
        .service
        .list_thread_history(ThreadHistoryRequest {
            scope: fixture.scope.clone(),
            thread_id: fixture.thread_id.clone(),
        })
        .await
        .map_err(|source| WorkloadFailure::source("read thread activity history", source))?;
    if history.messages.len() != 32 {
        return Err(WorkloadFailure::message(
            "read thread activity history",
            format!("expected 32 messages, got {}", history.messages.len()),
        ));
    }

    let listed = fixture
        .service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: fixture.scope,
            limit: Some(10),
            cursor: None,
        })
        .await
        .map_err(|source| WorkloadFailure::source("read thread activity index", source))?;
    if listed.threads.len() != 1 || listed.threads[0].thread_id != fixture.thread_id {
        return Err(WorkloadFailure::message(
            "read thread activity index",
            "burst thread did not round trip through the thread index",
        ));
    }
    Ok(())
}

async fn run_message_lookup_measurement<F>(
    root: Arc<F>,
    config: DbProbeConfig,
    backend: BackendName,
) -> Result<FixedWorkloadMeasurement, FixedWorkloadError>
where
    F: RootFilesystem + 'static,
{
    let fixture = prepare_thread_fixture(root, "db-write-message-lookup", None).await?;
    measure_fixed(
        FixedWorkload::MessageLookupRowFolding,
        backend,
        config,
        move || run_message_lookup_matrix(fixture),
    )
    .await
}

async fn run_message_lookup_matrix<F>(fixture: ThreadFixture<F>) -> Result<(), WorkloadFailure>
where
    F: RootFilesystem + 'static,
{
    let first_user = fixture
        .service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: fixture.scope.clone(),
            thread_id: fixture.thread_id.clone(),
            actor_id: "db-write-workload".to_string(),
            source_binding_id: Some("db-write-message-lookup".to_string()),
            reply_target_binding_id: Some("db-write-message-lookup-reply".to_string()),
            external_event_id: Some("message-lookup-first-user".to_string()),
            content: MessageContent::text("First durable user message"),
        })
        .await
        .map_err(|source| WorkloadFailure::source("append first user message", source))?;

    let turn_run_id = Uuid::new_v4().to_string();
    let assistant = fixture
        .service
        .append_finalized_assistant_message(AppendFinalizedAssistantMessageRequest {
            scope: fixture.scope.clone(),
            thread_id: fixture.thread_id.clone(),
            turn_run_id: turn_run_id.clone(),
            content: MessageContent::text("Durable assistant response"),
        })
        .await
        .map_err(|source| WorkloadFailure::source("append assistant message", source))?;
    let assistant_read = fixture
        .service
        .finalized_assistant_message_by_run(FinalizedAssistantMessageByRunRequest {
            scope: fixture.scope.clone(),
            thread_id: fixture.thread_id.clone(),
            turn_run_id: turn_run_id.clone(),
        })
        .await
        .map_err(|source| WorkloadFailure::source("read assistant run lookup", source))?
        .ok_or_else(|| {
            WorkloadFailure::message(
                "read assistant run lookup",
                "assistant lookup row did not resolve",
            )
        })?;
    if assistant_read.message_id != assistant.message_id {
        return Err(WorkloadFailure::message(
            "read assistant run lookup",
            "assistant lookup resolved the wrong message",
        ));
    }

    let bare_result_ref = "result:db-write-bare-tool-result".to_string();
    let bare_tool_result = fixture
        .service
        .append_tool_result_reference(AppendToolResultReferenceRequest {
            scope: fixture.scope.clone(),
            thread_id: fixture.thread_id.clone(),
            turn_run_id: turn_run_id.clone(),
            result_ref: bare_result_ref.clone(),
            safe_summary: ToolResultSafeSummary::new("durable bare tool result").map_err(
                |source| WorkloadFailure::message("build bare tool result summary", source),
            )?,
            provider_call: None,
            model_observation: None,
        })
        .await
        .map_err(|source| WorkloadFailure::source("append bare tool result message", source))?;
    let bare_tool_result_read = fixture
        .service
        .append_tool_result_reference(AppendToolResultReferenceRequest {
            scope: fixture.scope.clone(),
            thread_id: fixture.thread_id.clone(),
            turn_run_id: turn_run_id.clone(),
            result_ref: bare_result_ref,
            safe_summary: ToolResultSafeSummary::new("bare duplicate ignored").map_err(
                |source| WorkloadFailure::message("build bare duplicate summary", source),
            )?,
            provider_call: None,
            model_observation: None,
        })
        .await
        .map_err(|source| WorkloadFailure::source("read bare tool result lookup", source))?;
    if bare_tool_result_read.message_id != bare_tool_result.message_id {
        return Err(WorkloadFailure::message(
            "read bare tool result lookup",
            "tool-result lookup did not deduplicate",
        ));
    }

    let result_ref = "result:db-write-message-lookup".to_string();
    let provider_call = provider_call_reference()?;
    let tool_result = fixture
        .service
        .append_tool_result_reference(AppendToolResultReferenceRequest {
            scope: fixture.scope.clone(),
            thread_id: fixture.thread_id.clone(),
            turn_run_id: turn_run_id.clone(),
            result_ref: result_ref.clone(),
            safe_summary: ToolResultSafeSummary::new("durable tool result")
                .map_err(|source| WorkloadFailure::message("build tool result summary", source))?,
            provider_call: Some(provider_call.clone()),
            model_observation: None,
        })
        .await
        .map_err(|source| WorkloadFailure::source("append tool result message", source))?;
    let tool_result_read = fixture
        .service
        .append_tool_result_reference(AppendToolResultReferenceRequest {
            scope: fixture.scope.clone(),
            thread_id: fixture.thread_id.clone(),
            turn_run_id: turn_run_id.clone(),
            result_ref: result_ref.clone(),
            safe_summary: ToolResultSafeSummary::new("duplicate ignored").map_err(|source| {
                WorkloadFailure::message("build duplicate tool result summary", source)
            })?,
            provider_call: Some(provider_call),
            model_observation: None,
        })
        .await
        .map_err(|source| WorkloadFailure::source("read provider-call lookup", source))?;
    if tool_result_read.message_id != tool_result.message_id {
        return Err(WorkloadFailure::message(
            "read provider-call lookup",
            "provider-call lookup did not deduplicate",
        ));
    }

    let invocation_id = InvocationId::new();
    let preview = capability_preview(invocation_id)?;
    let preview_message = fixture
        .service
        .append_capability_display_preview(AppendCapabilityDisplayPreviewRequest {
            scope: fixture.scope.clone(),
            thread_id: fixture.thread_id.clone(),
            turn_run_id: turn_run_id.clone(),
            preview: preview.clone(),
        })
        .await
        .map_err(|source| WorkloadFailure::source("append capability preview", source))?;
    let preview_read = fixture
        .service
        .append_capability_display_preview(AppendCapabilityDisplayPreviewRequest {
            scope: fixture.scope.clone(),
            thread_id: fixture.thread_id.clone(),
            turn_run_id,
            preview,
        })
        .await
        .map_err(|source| WorkloadFailure::source("read capability preview lookup", source))?;
    if preview_read.message_id != preview_message.message_id {
        return Err(WorkloadFailure::message(
            "read capability preview lookup",
            "capability preview lookup did not deduplicate",
        ));
    }
    let expected_next_sequence = preview_message.sequence.checked_add(1).ok_or_else(|| {
        WorkloadFailure::message(
            "read capability preview lookup",
            "message sequence overflow",
        )
    })?;
    let after_preview = fixture
        .service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: fixture.scope.clone(),
            thread_id: fixture.thread_id.clone(),
            actor_id: "db-write-workload".to_string(),
            source_binding_id: Some("db-write-message-lookup".to_string()),
            reply_target_binding_id: Some("db-write-message-lookup-reply".to_string()),
            external_event_id: Some("message-lookup-after-preview".to_string()),
            content: MessageContent::text("Message after capability preview"),
        })
        .await
        .map_err(|source| WorkloadFailure::source("append message after preview", source))?;
    if after_preview.sequence != expected_next_sequence {
        return Err(WorkloadFailure::message(
            "read capability preview lookup",
            format!(
                "capability preview lookup consumed a sequence: expected {}, got {}",
                expected_next_sequence, after_preview.sequence
            ),
        ));
    }

    let listed = fixture
        .service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: fixture.scope.clone(),
            limit: Some(10),
            cursor: None,
        })
        .await
        .map_err(|source| WorkloadFailure::source("read first-user lookup", source))?;
    let listed_thread = listed
        .threads
        .iter()
        .find(|thread| thread.thread_id == fixture.thread_id)
        .ok_or_else(|| {
            WorkloadFailure::message("read first-user lookup", "thread index row missing")
        })?;
    if listed_thread.title.as_deref() != Some("First durable user message") {
        return Err(WorkloadFailure::message(
            "read first-user lookup",
            format!("unexpected derived title {:?}", listed_thread.title),
        ));
    }

    let history = fixture
        .service
        .list_thread_history(ThreadHistoryRequest {
            scope: fixture.scope,
            thread_id: fixture.thread_id,
        })
        .await
        .map_err(|source| WorkloadFailure::source("read message lookup history", source))?;
    if !history
        .messages
        .iter()
        .any(|message| message.message_id == first_user.message_id)
    {
        return Err(WorkloadFailure::message(
            "read message lookup history",
            "first user message did not round trip",
        ));
    }
    Ok(())
}

fn provider_call_reference() -> Result<ProviderToolCallReferenceEnvelope, WorkloadFailure> {
    Ok(ProviderToolCallReferenceEnvelope {
        provider_id: "db-write-provider".to_string(),
        provider_model_id: "db-write-model".to_string(),
        provider_turn_id: "db-write-provider-turn".to_string(),
        provider_call_id: "db-write-provider-call".to_string(),
        provider_tool_name: ProviderToolName::new("builtin__result_read")
            .map_err(|source| WorkloadFailure::source("build provider tool name", source))?,
        capability_id: CapabilityId::new("builtin.result_read")
            .map_err(|source| WorkloadFailure::source("build capability id", source))?,
        arguments: serde_json::json!({"offset": 0}),
        response_reasoning: None,
        reasoning: None,
        signature: None,
    })
}

fn capability_preview(
    invocation_id: InvocationId,
) -> Result<CapabilityDisplayPreviewEnvelope, WorkloadFailure> {
    CapabilityDisplayPreviewEnvelope::new(CapabilityDisplayPreviewEnvelopeInput {
        invocation_id,
        capability_id: CapabilityId::new("demo.echo")
            .map_err(|source| WorkloadFailure::source("build preview capability id", source))?,
        status: CapabilityDisplayPreviewStatus::Completed,
        title: "DB-write preview".to_string(),
        subtitle: None,
        input_summary: Some("{\"message\":\"hello\"}".to_string()),
        output_summary: Some("text output".to_string()),
        output_preview: Some("hello".to_string()),
        output_kind: Some("text".to_string()),
        output_bytes: Some(5),
        result_ref: Some("result:db-write-message-lookup".to_string()),
        truncated: false,
        updated_at: Utc::now(),
        activity_order: None,
    })
    .map_err(|source| WorkloadFailure::message("build capability preview", source))
}
