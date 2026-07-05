use std::collections::BTreeMap;
use std::env;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::{Body, to_bytes};
use axum::http::{HeaderValue, Method, Request, StatusCode, header};
use chrono::{DateTime, TimeZone, Utc};
use ironclaw_filesystem::{
    CasExpectation, Entry, Filter, IndexKey, IndexKind, IndexName, IndexSpec, IndexValue,
    LibSqlRootFilesystem, Page, PostgresRootFilesystem, RootFilesystem, ScopedFilesystem, SeqNo,
};
use ironclaw_host_api::{
    Action, AgentId, ApprovalRequest, ApprovalRequestId, AuditMode, CorrelationId, DeploymentMode,
    FilesystemBackendKind, MountAlias, MountGrant, MountPermissions, MountView, NetworkMode,
    Principal, ProcessBackendKind, ProjectId, ResourceEstimate, ResourceScope, ResourceUsage,
    RuntimeProfile, ScopedPath, SecretHandle, SecretMode, TenantId, ThreadId, UserId, VirtualPath,
    runtime_policy::{ApprovalPolicy, EffectiveRuntimePolicy},
};
use ironclaw_host_runtime::{
    CapabilitySurfaceVersion, CommandExecutionOutput, CommandExecutionRequest,
    ProductionWiringConfig, RuntimeProcessError, SandboxCommandTransport,
};
use ironclaw_reborn_composition::{
    LibSqlProductionSubstrateConfig, PollSettings, PostgresProductionSubstrateConfig,
    RebornBuildInput, RebornCompositionProfile, RebornProductionRuntimePolicy, RebornRuntime,
    RebornRuntimeIdentity, RebornRuntimeInput, WebuiAuthentication, WebuiAuthenticator,
    WebuiServeConfig, build_libsql_production_host_runtime_services,
    build_postgres_production_host_runtime_services, build_reborn_runtime, build_webui_services,
    hosted_single_tenant_runtime_policy, local_runtime_build_input, webui_v2_app,
};
use ironclaw_reborn_event_store::RebornEventStoreConfig;
use ironclaw_resources::{
    FilesystemResourceGovernorStore, PersistentResourceGovernor, PostgresResourceGovernor,
    ResourceAccount, ResourceGovernor, ResourceLimits,
};
use ironclaw_run_state::{ApprovalRequestStore, ApprovalStatus, FilesystemApprovalRequestStore};
use ironclaw_secrets::{
    FilesystemSecretStore, PostgresSecretStore, SecretMaterial, SecretStore, SecretsCrypto,
};
use ironclaw_triggers::{
    LibSqlTriggerRepository, PostgresTriggerRepository, TriggerId, TriggerRecord,
    TriggerRepository, TriggerSchedule, TriggerSourceKind, TriggerState,
};
use ironclaw_turns::{
    AcceptedMessageRef, AllowAllTurnAdmissionPolicy, BlockedReason, CancelRunRequest,
    CheckpointSchemaId, FilesystemTurnStateStoreKind, GateRef, GetLoopCheckpointRequest,
    GetRunStateRequest, IdempotencyKey, InMemoryRunProfileResolver, LoopCheckpointKind,
    LoopCheckpointStore, PutLoopCheckpointRequest, ReplyTargetBindingRef, ResumeTurnPrecondition,
    ResumeTurnRequest, RunProfileRequest, RunProfileVersion, SanitizedCancelReason,
    SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnCheckpointId, TurnId,
    TurnLeaseToken, TurnRunId, TurnRunWake, TurnRunWakeNotifier, TurnRunWakeNotifyError,
    TurnRunnerId, TurnScope, TurnStateStore, TurnStatus,
    run_profile::LoopCheckpointStateRef,
    runner::{
        BlockRunRequest, CancelRunCompletionRequest, ClaimRunRequest, CompleteRunRequest,
        TurnRunTransitionPort,
    },
};
use secrecy::ExposeSecret;
use serde::Serialize;
use tokio::sync::{OnceCell, Semaphore};
use tower::ServiceExt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
enum BackendName {
    Libsql,
    Postgres,
}

impl BackendName {
    fn as_str(self) -> &'static str {
        match self {
            Self::Libsql => "libsql",
            Self::Postgres => "postgres",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Workload {
    name: &'static str,
    kind: WorkloadKind,
}

#[derive(Debug, Clone, Copy)]
enum WorkloadKind {
    PutGet,
    QueryExact,
    AppendTail,
    ReserveSequence,
    TriggerSeedList,
    ControlPlaneSnapshot,
    TurnLifecycle,
    WebuiSession,
    HostedSubstrateBuild,
}

#[derive(Debug, Serialize)]
struct RunReport {
    profile: String,
    mode: String,
    warmup: usize,
    samples: usize,
    concurrency: Vec<usize>,
    backends: Vec<BackendName>,
    postgres_pool_sizes: Vec<usize>,
    path_depths: Vec<usize>,
    payload_bytes: Vec<usize>,
    acceptance_ready: bool,
    notes: Vec<&'static str>,
    results: Vec<ResultRow>,
    comparisons: Vec<ComparisonRow>,
}

#[derive(Debug, Serialize)]
struct ResultRow {
    backend: BackendName,
    postgres_pool_size: Option<usize>,
    workload: &'static str,
    concurrency: usize,
    samples: usize,
    errors: usize,
    first_error: Option<String>,
    throughput_ops_sec: f64,
    p50_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
    state_hash: String,
}

#[derive(Debug, Serialize)]
struct ComparisonRow {
    workload: &'static str,
    concurrency: usize,
    postgres_pool_size: usize,
    postgres_p50_ratio: f64,
    postgres_p95_ratio: f64,
    postgres_p99_ratio: f64,
    postgres_throughput_ratio: f64,
    errors_ok: bool,
    state_hash_ok: bool,
    dev_pass: bool,
    hard_fail: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let warmup = env_usize_allow_zero("LATENCY_WARMUP", 30);
    let samples = env_usize("LATENCY_SAMPLES", 300);
    let concurrency = env_list_usize("LATENCY_CONCURRENCY", &[1, 4, 16]);
    let backends = env_backend_list(
        "LATENCY_BACKENDS",
        &[BackendName::Libsql, BackendName::Postgres],
    );
    let postgres_pool_sizes = env_list_usize("LATENCY_POSTGRES_POOL_SIZES", &[1, 2]);
    let path_depths = env_list_usize("LATENCY_PATH_DEPTHS", &[2]);
    let payload_bytes = env_list_usize("LATENCY_PAYLOAD_BYTES", &[512]);
    let profile = env::var("LATENCY_PROFILE").unwrap_or_else(|_| "full-dev".to_string());
    let mode = if profile == "holdout" {
        "holdout"
    } else {
        "dev"
    }
    .to_string();

    let workloads = filter_workloads(vec![
        Workload {
            name: "put_get",
            kind: WorkloadKind::PutGet,
        },
        Workload {
            name: "query_exact",
            kind: WorkloadKind::QueryExact,
        },
        Workload {
            name: "append_tail",
            kind: WorkloadKind::AppendTail,
        },
        Workload {
            name: "reserve_sequence",
            kind: WorkloadKind::ReserveSequence,
        },
        Workload {
            name: "trigger_seed_list",
            kind: WorkloadKind::TriggerSeedList,
        },
        Workload {
            name: "control_plane_snapshot",
            kind: WorkloadKind::ControlPlaneSnapshot,
        },
        Workload {
            name: "turn_lifecycle",
            kind: WorkloadKind::TurnLifecycle,
        },
        Workload {
            name: "webui_session",
            kind: WorkloadKind::WebuiSession,
        },
        Workload {
            name: "hosted_substrate_build",
            kind: WorkloadKind::HostedSubstrateBuild,
        },
    ]);

    let mut results = Vec::new();
    if backends.contains(&BackendName::Libsql) {
        let libsql_backend = open_backend(BackendName::Libsql, None).await?;
        let libsql_run_id = uuid::Uuid::new_v4().simple().to_string();
        for &workload in &workloads {
            for &concurrency in &concurrency {
                let row = run_workload(
                    libsql_backend.clone(),
                    BackendName::Libsql,
                    None,
                    &libsql_run_id,
                    workload,
                    concurrency,
                    warmup,
                    samples,
                    &path_depths,
                    &payload_bytes,
                )
                .await?;
                results.push(row);
            }
        }
    }

    if backends.contains(&BackendName::Postgres) {
        for &postgres_pool_size in &postgres_pool_sizes {
            let postgres_backend =
                open_backend(BackendName::Postgres, Some(postgres_pool_size)).await?;
            let postgres_run_id = uuid::Uuid::new_v4().simple().to_string();
            for &workload in &workloads {
                for &concurrency in &concurrency {
                    let row = run_workload(
                        postgres_backend.clone(),
                        BackendName::Postgres,
                        Some(postgres_pool_size),
                        &postgres_run_id,
                        workload,
                        concurrency,
                        warmup,
                        samples,
                        &path_depths,
                        &payload_bytes,
                    )
                    .await?;
                    results.push(row);
                }
            }
        }
    }

    let comparisons = compare(&results);
    let report = RunReport {
        profile,
        mode,
        warmup,
        samples,
        concurrency,
        backends,
        postgres_pool_sizes,
        path_depths,
        payload_bytes,
        acceptance_ready: false,
        notes: vec![
            "dev scorer: storage hot paths, filesystem turn lifecycle, WebUI session, plus production-shaped hosted substrate build/readiness",
            "full acceptance still requires launch-ref libSQL baseline and request-level trigger/approval/resource workloads",
        ],
        results,
        comparisons,
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

#[derive(Clone)]
struct BackendContext {
    fs: Arc<dyn RootFilesystem>,
    turn_state: Arc<dyn TurnLifecycleStore>,
    trigger_repository: Arc<dyn TriggerRepository>,
    approval_requests: Arc<dyn ApprovalRequestStore>,
    secret_store: Arc<dyn SecretStore>,
    resource_governor: Arc<dyn ResourceGovernor>,
    webui_session: Arc<OnceCell<WebuiRuntimeContext>>,
    webui_postgres_pool: Option<deadpool_postgres::Pool>,
}

trait TurnLifecycleStore: TurnStateStore + TurnRunTransitionPort + LoopCheckpointStore {}

impl<T> TurnLifecycleStore for T where
    T: TurnStateStore + TurnRunTransitionPort + LoopCheckpointStore + Send + Sync
{
}

struct WebuiRuntimeContext {
    router: axum::Router,
    _runtime: RebornRuntime,
}

async fn open_backend(
    backend: BackendName,
    postgres_pool_size: Option<usize>,
) -> Result<BackendContext, Box<dyn std::error::Error>> {
    match backend {
        BackendName::Libsql => {
            let dir = tempfile::tempdir()?;
            let db_path = dir.keep().join("latency-libsql.db");
            let db = Arc::new(libsql::Builder::new_local(db_path).build().await?);
            let fs = Arc::new(LibSqlRootFilesystem::new(Arc::clone(&db)));
            fs.run_migrations().await?;
            let trigger_repository = LibSqlTriggerRepository::new(db);
            trigger_repository.run_migrations().await?;
            let turn_state = filesystem_turn_state_store(Arc::clone(&fs), backend, None)?;
            let control_plane = control_plane_stores(Arc::clone(&fs));
            Ok(BackendContext {
                fs,
                turn_state,
                trigger_repository: Arc::new(trigger_repository),
                approval_requests: control_plane.approval_requests,
                secret_store: control_plane.secret_store,
                resource_governor: control_plane.resource_governor,
                webui_session: Arc::new(OnceCell::new()),
                webui_postgres_pool: None,
            })
        }
        BackendName::Postgres => {
            let url = env::var("IRONCLAW_REBORN_POSTGRES_URL").unwrap_or_else(|_| {
                "postgres://postgres:postgres@localhost:5432/ironclaw_latency".to_string()
            });
            let config = url.parse::<tokio_postgres::Config>()?;
            let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
            let pool = deadpool_postgres::Pool::builder(manager)
                .max_size(
                    postgres_pool_size
                        .unwrap_or_else(|| env_usize("IRONCLAW_REBORN_POSTGRES_POOL_MAX_SIZE", 2)),
                )
                .build()?;
            let fs = Arc::new(PostgresRootFilesystem::new(pool.clone()));
            fs.run_migrations().await?;
            let secret_store = PostgresSecretStore::new(pool.clone(), latency_secrets_crypto());
            secret_store.run_migrations().await?;
            let resource_governor = PostgresResourceGovernor::new(pool.clone());
            resource_governor.run_migrations()?;
            let trigger_repository = PostgresTriggerRepository::new(pool.clone());
            trigger_repository.run_migrations().await?;
            let mut control_plane = control_plane_stores(Arc::clone(&fs));
            control_plane.secret_store = Arc::new(secret_store);
            control_plane.resource_governor = Arc::new(resource_governor);
            let turn_state =
                filesystem_turn_state_store(Arc::clone(&fs), backend, postgres_pool_size)?;
            Ok(BackendContext {
                fs,
                turn_state,
                trigger_repository: Arc::new(trigger_repository),
                approval_requests: control_plane.approval_requests,
                secret_store: control_plane.secret_store,
                resource_governor: control_plane.resource_governor,
                webui_session: Arc::new(OnceCell::new()),
                webui_postgres_pool: Some(pool),
            })
        }
    }
}

fn filesystem_turn_state_store<F>(
    fs: Arc<F>,
    backend: BackendName,
    postgres_pool_size: Option<usize>,
) -> Result<Arc<dyn TurnLifecycleStore>, Box<dyn std::error::Error>>
where
    F: RootFilesystem + 'static,
{
    let pool_label = postgres_pool_size
        .map(|pool_size| format!("pool-{pool_size}"))
        .unwrap_or_else(|| "baseline".to_string());
    let run_label = uuid::Uuid::new_v4().simple().to_string();
    let turns_root = VirtualPath::new(format!(
        "/tenants/latency-turns-{}-{pool_label}-{run_label}/users/latency-user/turns",
        backend.as_str()
    ))?;
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/turns")?,
        turns_root,
        MountPermissions::read_write_list_delete(),
    )])?;
    let scoped = Arc::new(ScopedFilesystem::with_fixed_view(fs, mounts));
    let store = match backend {
        BackendName::Libsql => FilesystemTurnStateStoreKind::blob(scoped),
        BackendName::Postgres => FilesystemTurnStateStoreKind::row(scoped),
    };
    Ok(Arc::new(store))
}

struct ControlPlaneStores {
    approval_requests: Arc<dyn ApprovalRequestStore>,
    secret_store: Arc<dyn SecretStore>,
    resource_governor: Arc<dyn ResourceGovernor>,
}

fn control_plane_stores<F>(fs: Arc<F>) -> ControlPlaneStores
where
    F: RootFilesystem + 'static,
{
    let scoped = scoped_control_plane_fs(fs);
    let approval_requests = Arc::new(FilesystemApprovalRequestStore::new(Arc::clone(&scoped)));
    let secret_store = Arc::new(FilesystemSecretStore::new(
        Arc::clone(&scoped),
        latency_secrets_crypto(),
    ));
    let resource_store = FilesystemResourceGovernorStore::new(scoped);
    let resource_governor = Arc::new(PersistentResourceGovernor::new(resource_store));
    ControlPlaneStores {
        approval_requests,
        secret_store,
        resource_governor,
    }
}

fn scoped_control_plane_fs<F>(fs: Arc<F>) -> Arc<ScopedFilesystem<F>>
where
    F: RootFilesystem + ?Sized,
{
    let mounts = MountView::new(vec![
        MountGrant::new(
            MountAlias::new("/approvals").expect("valid mount alias"),
            VirtualPath::new("/engine/tenants/latency/users/control/approvals")
                .expect("valid mount target"),
            MountPermissions::read_write_list_delete(),
        ),
        MountGrant::new(
            MountAlias::new("/resources").expect("valid mount alias"),
            VirtualPath::new("/engine/tenants/latency/users/control/resources")
                .expect("valid mount target"),
            MountPermissions::read_write_list_delete(),
        ),
        MountGrant::new(
            MountAlias::new("/secrets").expect("valid mount alias"),
            VirtualPath::new("/engine/tenants/latency/users/control/secrets")
                .expect("valid mount target"),
            MountPermissions::read_write_list_delete(),
        ),
    ])
    .expect("valid control-plane mount view");
    Arc::new(ScopedFilesystem::with_fixed_view(fs, mounts))
}

fn latency_secrets_crypto() -> Arc<SecretsCrypto> {
    Arc::new(
        SecretsCrypto::new(latency_secret_master_key())
            .expect("latency secret master key must be valid"),
    )
}

async fn run_workload(
    backend_context: BackendContext,
    backend: BackendName,
    postgres_pool_size: Option<usize>,
    run_id: &str,
    workload: Workload,
    concurrency: usize,
    warmup: usize,
    samples: usize,
    path_depths: &[usize],
    payload_bytes: &[usize],
) -> Result<ResultRow, Box<dyn std::error::Error>> {
    let workload_run_id = format!("{run_id}-{}-c{concurrency}", workload.name);
    for i in 0..warmup {
        setup_workload(
            backend_context.clone(),
            backend,
            postgres_pool_size,
            &workload_run_id,
            workload,
            i,
            path_depths,
            payload_bytes,
        )
        .await?;
        let _ = run_one(
            backend_context.clone(),
            backend,
            postgres_pool_size,
            &workload_run_id,
            workload,
            i,
            path_depths,
            payload_bytes,
        )
        .await;
    }

    let sem = Arc::new(Semaphore::new(concurrency.max(1)));
    let started = Instant::now();
    let mut tasks = Vec::with_capacity(samples);
    for i in 0..samples {
        setup_workload(
            backend_context.clone(),
            backend,
            postgres_pool_size,
            &workload_run_id,
            workload,
            i + warmup,
            path_depths,
            payload_bytes,
        )
        .await?;
        let permit = Arc::clone(&sem).acquire_owned().await?;
        let backend_context = backend_context.clone();
        let run_id = workload_run_id.clone();
        let path_depths = path_depths.to_vec();
        let payload_bytes = payload_bytes.to_vec();
        tasks.push(tokio::spawn(async move {
            let _permit = permit;
            run_one(
                backend_context,
                backend,
                postgres_pool_size,
                &run_id,
                workload,
                i + warmup,
                &path_depths,
                &payload_bytes,
            )
            .await
        }));
    }

    let mut latencies = Vec::with_capacity(samples);
    let mut errors = 0usize;
    let mut first_error = None;
    let mut state = 0u64;
    for task in tasks {
        match task.await {
            Ok(Ok(sample)) => {
                latencies.push(sample.elapsed);
                state = state.wrapping_add(sample.state);
            }
            Ok(Err(error)) => {
                errors += 1;
                if first_error.is_none() {
                    first_error = Some(describe_error_chain(error.as_ref()));
                }
            }
            Err(error) => {
                errors += 1;
                if first_error.is_none() {
                    first_error = Some(error.to_string());
                }
            }
        }
    }
    let total = started.elapsed();
    latencies.sort_unstable();

    Ok(ResultRow {
        backend,
        postgres_pool_size,
        workload: workload.name,
        concurrency,
        samples,
        errors,
        first_error,
        throughput_ops_sec: samples as f64 / total.as_secs_f64().max(0.001),
        p50_ms: percentile_ms(&latencies, 50.0),
        p95_ms: percentile_ms(&latencies, 95.0),
        p99_ms: percentile_ms(&latencies, 99.0),
        state_hash: format!("{state:016x}"),
    })
}

struct Sample {
    elapsed: Duration,
    state: u64,
}

async fn run_one(
    backend_context: BackendContext,
    backend: BackendName,
    postgres_pool_size: Option<usize>,
    run_id: &str,
    workload: Workload,
    sample: usize,
    path_depths: &[usize],
    payload_bytes: &[usize],
) -> Result<Sample, Box<dyn std::error::Error + Send + Sync>> {
    let depth = path_depths[sample % path_depths.len()].max(1);
    let payload_len = payload_bytes[sample % payload_bytes.len()].max(1);
    let prefix = workload_prefix(backend, run_id, workload.name, depth)?;
    let started = Instant::now();
    let state = match workload.kind {
        WorkloadKind::PutGet => put_get(backend_context.fs, &prefix, sample, payload_len).await?,
        WorkloadKind::QueryExact => {
            query_exact(backend_context.fs, &prefix, sample, payload_len).await?
        }
        WorkloadKind::AppendTail => {
            append_tail(backend_context.fs, &prefix, sample, payload_len).await?
        }
        WorkloadKind::ReserveSequence => {
            reserve_sequence(backend_context.fs, &prefix, sample).await?
        }
        WorkloadKind::TriggerSeedList => {
            trigger_seed_list(
                backend_context.trigger_repository,
                backend,
                postgres_pool_size,
                run_id,
                sample,
            )
            .await?
        }
        WorkloadKind::ControlPlaneSnapshot => {
            control_plane_snapshot(
                backend_context.approval_requests,
                backend_context.secret_store,
                backend_context.resource_governor,
                backend,
                postgres_pool_size,
                run_id,
                sample,
            )
            .await?
        }
        WorkloadKind::TurnLifecycle => {
            turn_lifecycle(
                backend_context.turn_state,
                backend,
                postgres_pool_size,
                run_id,
                sample,
                payload_len,
            )
            .await?
        }
        WorkloadKind::WebuiSession => {
            webui_session(backend_context, backend, postgres_pool_size, sample).await?
        }
        WorkloadKind::HostedSubstrateBuild => {
            hosted_substrate_build(backend, sample, postgres_pool_size).await?
        }
    };
    Ok(Sample {
        elapsed: started.elapsed(),
        state,
    })
}

async fn setup_workload(
    backend_context: BackendContext,
    backend: BackendName,
    postgres_pool_size: Option<usize>,
    run_id: &str,
    workload: Workload,
    sample: usize,
    path_depths: &[usize],
    payload_bytes: &[usize],
) -> Result<(), Box<dyn std::error::Error>> {
    if matches!(
        workload.kind,
        WorkloadKind::TriggerSeedList
            | WorkloadKind::TurnLifecycle
            | WorkloadKind::HostedSubstrateBuild
    ) {
        return Ok(());
    }

    if matches!(workload.kind, WorkloadKind::WebuiSession) {
        ensure_webui_runtime_context(&backend_context, backend, postgres_pool_size)
            .await
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        return Ok(());
    }

    if matches!(workload.kind, WorkloadKind::ControlPlaneSnapshot) {
        setup_control_plane_indexes(backend_context.fs).await?;
        return Ok(());
    }

    let depth = path_depths[sample % path_depths.len()].max(1);
    let prefix = workload_prefix(backend, run_id, workload.name, depth)?;
    let fs = backend_context.fs;
    setup_create_dir_all(Arc::clone(&fs), &prefix).await?;
    match workload.kind {
        WorkloadKind::PutGet => {
            let parent = child(&prefix, "entry")?;
            setup_create_dir_all(fs, &parent).await?;
        }
        WorkloadKind::QueryExact => {
            setup_ensure_index(
                Arc::clone(&fs),
                &prefix,
                IndexSpec::new(
                    IndexName::new("bucket_exact")?,
                    vec![IndexKey::new("bucket")?],
                    IndexKind::Exact,
                ),
            )
            .await?;
            seed_query_exact_records(fs, &prefix, sample, payload_bytes).await?;
        }
        WorkloadKind::AppendTail => {
            let parent = child(&prefix, "events")?;
            setup_create_dir_all(fs, &parent).await?;
        }
        WorkloadKind::ReserveSequence => {
            let parent = child(&prefix, "sequence")?;
            setup_create_dir_all(fs, &parent).await?;
        }
        WorkloadKind::TriggerSeedList
        | WorkloadKind::ControlPlaneSnapshot
        | WorkloadKind::TurnLifecycle
        | WorkloadKind::WebuiSession
        | WorkloadKind::HostedSubstrateBuild => {}
    }
    Ok(())
}

async fn setup_create_dir_all(
    fs: Arc<dyn RootFilesystem>,
    prefix: &VirtualPath,
) -> Result<(), Box<dyn std::error::Error>> {
    for attempt in 0..5 {
        match fs.create_dir_all(prefix).await {
            Ok(()) => return Ok(()),
            Err(error) if is_retryable_setup_error(&error) && attempt < 4 => {
                tokio::time::sleep(Duration::from_millis(10 * (attempt + 1))).await;
            }
            Err(error) => return Err(Box::new(error)),
        }
    }
    unreachable!("bounded setup retry loop always returns")
}

async fn setup_control_plane_indexes(
    fs: Arc<dyn RootFilesystem>,
) -> Result<(), Box<dyn std::error::Error>> {
    let scoped = scoped_control_plane_fs(fs);
    let scope = ResourceScope::system();
    let secrets_root = ScopedPath::new("/secrets")?;
    let spec = IndexSpec::new(
        IndexName::new("secrets_by_tenant")?,
        vec![IndexKey::new("tenant_id")?],
        IndexKind::Exact,
    );
    for attempt in 0..5 {
        match scoped.ensure_index(&scope, &secrets_root, &spec).await {
            Ok(()) => return Ok(()),
            Err(error) if is_retryable_setup_error(&error) && attempt < 4 => {
                tokio::time::sleep(Duration::from_millis(10 * (attempt + 1))).await;
            }
            Err(error) => return Err(Box::new(error)),
        }
    }
    unreachable!("bounded setup retry loop always returns")
}

async fn setup_ensure_index(
    fs: Arc<dyn RootFilesystem>,
    prefix: &VirtualPath,
    spec: IndexSpec,
) -> Result<(), Box<dyn std::error::Error>> {
    for attempt in 0..5 {
        match fs.ensure_index(prefix, &spec).await {
            Ok(()) => return Ok(()),
            Err(error) if is_retryable_setup_error(&error) && attempt < 4 => {
                tokio::time::sleep(Duration::from_millis(10 * (attempt + 1))).await;
            }
            Err(error) => return Err(Box::new(error)),
        }
    }
    unreachable!("bounded setup retry loop always returns")
}

fn is_retryable_setup_error(error: &ironclaw_filesystem::FilesystemError) -> bool {
    let message = error.to_string();
    message.contains("database is locked")
        || message.contains("database table is locked")
        || message.contains("bad parameter or other API misuse")
}

async fn put_get(
    fs: Arc<dyn RootFilesystem>,
    prefix: &VirtualPath,
    sample: usize,
    payload_len: usize,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    let path = child(prefix, "entry")?;
    let path = child(&path, &format!("sample-{sample}"))?;
    let payload = payload(sample, payload_len);
    let version = fs
        .put(&path, Entry::bytes(payload.clone()), CasExpectation::Any)
        .await?;
    let read = fs.get(&path).await?.ok_or("missing put_get readback")?;
    Ok(version.get() ^ read.version.get() ^ read.entry.body.len() as u64)
}

async fn query_exact(
    fs: Arc<dyn RootFilesystem>,
    prefix: &VirtualPath,
    sample: usize,
    _payload_len: usize,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    let key = IndexKey::new("bucket")?;
    let bucket = format!("b{}", sample % 8);
    let rows = fs
        .query(
            prefix,
            &Filter::Eq {
                key,
                value: IndexValue::Text(bucket),
            },
            Page::first(16),
        )
        .await?;
    Ok(rows.len() as u64)
}

async fn seed_query_exact_records(
    fs: Arc<dyn RootFilesystem>,
    prefix: &VirtualPath,
    sample: usize,
    payload_bytes: &[usize],
) -> Result<(), Box<dyn std::error::Error>> {
    let key = IndexKey::new("bucket")?;
    let kind = ironclaw_filesystem::RecordKind::new("latency_record")?;
    let bucket = format!("b{}", sample % 8);
    let payload_len = payload_bytes[sample % payload_bytes.len()].max(1);
    for i in 0..8 {
        let path = child(prefix, &format!("sample-{sample}/record-{i}"))?;
        let entry = Entry::record(
            kind.clone(),
            &serde_json::json!({"sample": sample, "row": i, "backend": "storage"}),
        )?
        .with_indexed(
            key.clone(),
            IndexValue::Text(if i == 0 {
                bucket.clone()
            } else {
                format!("other-{i}")
            }),
        )
        .with_indexed(IndexKey::new("size")?, IndexValue::I64(payload_len as i64));
        fs.put(&path, entry, CasExpectation::Any).await?;
    }
    Ok(())
}

async fn append_tail(
    fs: Arc<dyn RootFilesystem>,
    prefix: &VirtualPath,
    sample: usize,
    payload_len: usize,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    let path = child(prefix, "events")?;
    let path = child(&path, &format!("sample-{sample}"))?;
    let payloads = (0..8)
        .map(|i| payload(sample + i, payload_len))
        .collect::<Vec<_>>();
    let seqs = fs.append_batch(&path, payloads).await?;
    let events = fs.tail_bounded(&path, SeqNo::ZERO, 16).await?;
    let payload_bytes = events
        .iter()
        .map(|event| event.payload.len() as u64)
        .sum::<u64>();
    Ok((seqs.len() as u64) ^ (events.len() as u64) ^ payload_bytes)
}

async fn reserve_sequence(
    fs: Arc<dyn RootFilesystem>,
    prefix: &VirtualPath,
    sample: usize,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    let path = child(prefix, "sequence")?;
    let path = child(&path, &format!("sample-{sample}"))?;
    let first = fs.reserve_sequence(&path).await?;
    let second = fs.reserve_sequence(&path).await?;
    Ok(first.get() ^ second.get())
}

async fn trigger_seed_list(
    repository: Arc<dyn TriggerRepository>,
    backend: BackendName,
    postgres_pool_size: Option<usize>,
    run_id: &str,
    sample: usize,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    let pool_label = postgres_pool_size
        .map(|pool_size| format!("pool-{pool_size}"))
        .unwrap_or_else(|| "baseline".to_string());
    let scope = format!("{}-{pool_label}-{run_id}-{sample}", backend.as_str());
    let tenant_id = TenantId::new(format!("latency-trigger-tenant-{scope}"))?;
    let creator_user_id = UserId::new(format!("latency-trigger-user-{scope}"))?;
    let agent_id = AgentId::new(format!("latency-trigger-agent-{scope}"))?;
    let project_id = ProjectId::new(format!("latency-trigger-project-{scope}"))?;
    let record = trigger_record(
        sample,
        tenant_id.clone(),
        creator_user_id.clone(),
        agent_id.clone(),
        project_id.clone(),
    )?;
    repository.upsert_trigger(record).await?;
    let tenant_rows = repository.list_triggers(tenant_id.clone()).await?;
    let scoped_rows = repository
        .list_scoped_triggers(
            tenant_id,
            creator_user_id,
            Some(agent_id),
            Some(project_id),
            16,
            &[],
        )
        .await?;
    Ok((tenant_rows.len() as u64) ^ ((scoped_rows.len() as u64) << 8))
}

fn trigger_record(
    sample: usize,
    tenant_id: TenantId,
    creator_user_id: UserId,
    agent_id: AgentId,
    project_id: ProjectId,
) -> Result<TriggerRecord, Box<dyn std::error::Error + Send + Sync>> {
    let created_at = timestamp(1_704_067_000 + sample as i64)?;
    let next_run_at = timestamp(1_704_070_600 + sample as i64)?;
    Ok(TriggerRecord {
        trigger_id: TriggerId::new(),
        tenant_id,
        creator_user_id,
        agent_id: Some(agent_id),
        project_id: Some(project_id),
        name: format!("latency trigger {sample}"),
        source: TriggerSourceKind::Schedule,
        schedule: TriggerSchedule::cron("0 8 * * *")?,
        prompt: "run the deterministic latency fixture".to_string(),
        state: TriggerState::Scheduled,
        next_run_at,
        last_run_at: None,
        last_fired_slot: None,
        last_status: None,
        active_fire_slot: None,
        active_run_ref: None,
        created_at,
    })
}

fn timestamp(seconds: i64) -> Result<DateTime<Utc>, Box<dyn std::error::Error + Send + Sync>> {
    DateTime::from_timestamp(seconds, 0).ok_or_else(|| "invalid trigger timestamp".into())
}

async fn control_plane_snapshot(
    approval_requests: Arc<dyn ApprovalRequestStore>,
    secret_store: Arc<dyn SecretStore>,
    resource_governor: Arc<dyn ResourceGovernor>,
    backend: BackendName,
    postgres_pool_size: Option<usize>,
    run_id: &str,
    sample: usize,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    let scope = control_plane_scope(backend, postgres_pool_size, run_id, sample)?;

    let request_id = ApprovalRequestId::new();
    let approval = ApprovalRequest {
        id: request_id,
        correlation_id: CorrelationId::new(),
        requested_by: Principal::User(scope.user_id.clone()),
        action: Box::new(Action::ReserveResources {
            estimate: resource_estimate(sample),
        }),
        invocation_fingerprint: None,
        reason: format!("latency control-plane sample {sample}"),
        reusable_scope: None,
    };
    let pending = approval_requests
        .save_pending(scope.clone(), approval)
        .await?;
    let approved = approval_requests.approve(&scope, request_id).await?;
    let approval_rows = approval_requests.records_for_scope(&scope).await?;

    let handle = SecretHandle::new(format!("latency_secret_{sample}"))?;
    secret_store
        .put(
            scope.clone(),
            handle.clone(),
            SecretMaterial::from(format!("secret-material-{sample}-{run_id}")),
            None,
        )
        .await?;
    let metadata = secret_store
        .metadata(&scope, &handle)
        .await?
        .ok_or("missing secret metadata")?;
    let metadata_rows = secret_store.metadata_for_scope(&scope).await?;
    let lease = secret_store.lease_once(&scope, &handle).await?;
    let material = secret_store.consume(&scope, lease.id).await?;

    let account = ResourceAccount::project(
        scope.tenant_id.clone(),
        scope.user_id.clone(),
        scope
            .project_id
            .clone()
            .ok_or("control-plane scope missing project id")?,
    );
    let (account_snapshot, receipt_has_actual) =
        resource_governor_round_trip(resource_governor, account, scope.clone(), sample).await?;
    let account_snapshot = account_snapshot.ok_or("missing resource account snapshot")?;

    let approval_state = match (pending.status, approved.status) {
        (ApprovalStatus::Pending, ApprovalStatus::Approved) => 0x11,
        _ => 0xff,
    };
    Ok(approval_state
        ^ ((approval_rows.len() as u64) << 8)
        ^ ((metadata_rows.len() as u64) << 16)
        ^ ((metadata.handle.as_str().len() as u64) << 24)
        ^ ((material.expose_secret().len() as u64) << 32)
        ^ ((receipt_has_actual as u64) << 40)
        ^ ((account_snapshot.ledger.spent.output_bytes as u64) << 48))
}

async fn resource_governor_round_trip(
    resource_governor: Arc<dyn ResourceGovernor>,
    account: ResourceAccount,
    scope: ResourceScope,
    sample: usize,
) -> Result<
    (Option<ironclaw_resources::AccountSnapshot>, bool),
    Box<dyn std::error::Error + Send + Sync>,
> {
    tokio::task::spawn_blocking(move || {
        resource_governor.set_limit(account.clone(), resource_limits())?;
        let reservation = resource_governor.reserve(scope, resource_estimate(sample))?;
        let receipt = resource_governor.reconcile(reservation.id, resource_usage(sample))?;
        let account_snapshot = resource_governor.account_snapshot(&account)?;
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>((
            account_snapshot,
            receipt.actual.is_some(),
        ))
    })
    .await?
}

fn control_plane_scope(
    backend: BackendName,
    postgres_pool_size: Option<usize>,
    run_id: &str,
    sample: usize,
) -> Result<ResourceScope, Box<dyn std::error::Error + Send + Sync>> {
    let pool_label = postgres_pool_size
        .map(|pool_size| format!("pool-{pool_size}"))
        .unwrap_or_else(|| "baseline".to_string());
    let scope = format!("{}-{pool_label}-{run_id}-{sample}", backend.as_str());
    Ok(ResourceScope {
        tenant_id: TenantId::new(format!("latency-control-tenant-{scope}"))?,
        user_id: UserId::new(format!("latency-control-user-{scope}"))?,
        agent_id: Some(AgentId::new(format!("latency-control-agent-{scope}"))?),
        project_id: Some(ProjectId::new(format!("latency-control-project-{scope}"))?),
        mission_id: None,
        thread_id: None,
        invocation_id: ironclaw_host_api::InvocationId::new(),
    })
}

fn resource_estimate(sample: usize) -> ResourceEstimate {
    ResourceEstimate {
        input_tokens: Some(64 + sample as u64 % 16),
        output_tokens: Some(32 + sample as u64 % 8),
        wall_clock_ms: Some(250),
        output_bytes: Some(512),
        concurrency_slots: Some(1),
        ..Default::default()
    }
}

fn resource_usage(sample: usize) -> ResourceUsage {
    ResourceUsage {
        input_tokens: 64 + sample as u64 % 16,
        output_tokens: 32 + sample as u64 % 8,
        wall_clock_ms: 125,
        output_bytes: 256,
        network_egress_bytes: 0,
        process_count: 0,
        ..Default::default()
    }
}

fn resource_limits() -> ResourceLimits {
    ResourceLimits {
        max_input_tokens: Some(1_000_000),
        max_output_tokens: Some(1_000_000),
        max_wall_clock_ms: Some(1_000_000),
        max_output_bytes: Some(1_000_000),
        max_concurrency_slots: Some(10_000),
        ..Default::default()
    }
}

async fn turn_lifecycle(
    store: Arc<dyn TurnLifecycleStore>,
    backend: BackendName,
    postgres_pool_size: Option<usize>,
    run_id: &str,
    sample: usize,
    payload_len: usize,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    let key = turn_lifecycle_key(backend, postgres_pool_size, run_id, sample);
    let actor = turn_lifecycle_actor(sample)?;
    let resolver = InMemoryRunProfileResolver::default();

    let complete_scope = turn_lifecycle_scope(&key, sample, "complete")?;
    let complete_submit = store
        .submit_turn(
            turn_lifecycle_submit_request(
                complete_scope.clone(),
                actor.clone(),
                &key,
                "complete",
                payload_len,
            )?,
            &AllowAllTurnAdmissionPolicy,
            &resolver,
        )
        .await?;
    let (complete_turn_id, complete_run_id, complete_submit_status) =
        accepted_run(&complete_submit);
    let (runner_id, lease_token, claimed_state) = claim_expected_run(
        Arc::clone(&store),
        Some(complete_scope.clone()),
        complete_run_id,
        "complete first claim",
    )
    .await?;

    let gate_ref = GateRef::new(format!("gate:latency-{key}-approval"))?;
    let complete_checkpoint_id = TurnCheckpointId::new();
    let complete_checkpoint_ref = LoopCheckpointStateRef::new(format!("checkpoint:latency-{key}"))?;
    let blocked = store
        .block_run(BlockRunRequest {
            run_id: complete_run_id,
            runner_id,
            lease_token,
            checkpoint_id: complete_checkpoint_id,
            state_ref: complete_checkpoint_ref.clone(),
            reason: BlockedReason::Approval {
                gate_ref: gate_ref.clone(),
            },
        })
        .await?;
    ensure_status(blocked.status, TurnStatus::BlockedApproval, "block_run")?;
    let checkpoint_code = record_turn_lifecycle_checkpoints(
        Arc::clone(&store),
        &complete_scope,
        complete_turn_id,
        complete_run_id,
        complete_checkpoint_ref,
        &key,
        payload_len,
    )
    .await?;

    let resumed = store
        .resume_turn(ResumeTurnRequest {
            scope: complete_scope.clone(),
            actor: actor.clone(),
            run_id: complete_run_id,
            gate_resolution_ref: gate_ref,
            source_binding_ref: SourceBindingRef::new(format!("source-{key}-resume"))?,
            reply_target_binding_ref: ReplyTargetBindingRef::new(format!("reply-{key}-resume"))?,
            idempotency_key: IdempotencyKey::new(format!("idem-{key}-resume"))?,
            precondition: ResumeTurnPrecondition::BlockedApprovalGate,
            resume_disposition: None,
        })
        .await?;
    ensure_status(resumed.status, TurnStatus::Queued, "resume_turn")?;

    let (runner_id, lease_token, reclaimed_state) = claim_expected_run(
        Arc::clone(&store),
        Some(complete_scope.clone()),
        complete_run_id,
        "complete reclaim",
    )
    .await?;
    let completed = store
        .complete_run(CompleteRunRequest {
            run_id: complete_run_id,
            runner_id,
            lease_token,
        })
        .await?;
    ensure_status(completed.status, TurnStatus::Completed, "complete_run")?;
    let completed_readback = store
        .get_run_state(GetRunStateRequest {
            scope: complete_scope,
            run_id: complete_run_id,
        })
        .await?;
    ensure_status(
        completed_readback.status,
        TurnStatus::Completed,
        "complete readback",
    )?;

    let cancel_scope = turn_lifecycle_scope(&key, sample, "cancel")?;
    let cancel_submit = store
        .submit_turn(
            turn_lifecycle_submit_request(
                cancel_scope.clone(),
                actor.clone(),
                &key,
                "cancel",
                payload_len,
            )?,
            &AllowAllTurnAdmissionPolicy,
            &resolver,
        )
        .await?;
    let (_cancel_turn_id, cancel_run_id, cancel_submit_status) = accepted_run(&cancel_submit);
    let (cancel_runner_id, cancel_lease_token, cancel_claimed_state) = claim_expected_run(
        Arc::clone(&store),
        Some(cancel_scope.clone()),
        cancel_run_id,
        "cancel claim",
    )
    .await?;
    let cancel_requested = store
        .request_cancel(CancelRunRequest {
            scope: cancel_scope.clone(),
            actor,
            run_id: cancel_run_id,
            reason: SanitizedCancelReason::UserRequested,
            idempotency_key: IdempotencyKey::new(format!("idem-{key}-cancel"))?,
        })
        .await?;
    ensure_status(
        cancel_requested.status,
        TurnStatus::CancelRequested,
        "request_cancel",
    )?;
    let cancelled = store
        .cancel_run(CancelRunCompletionRequest {
            run_id: cancel_run_id,
            runner_id: cancel_runner_id,
            lease_token: cancel_lease_token,
        })
        .await?;
    ensure_status(cancelled.status, TurnStatus::Cancelled, "cancel_run")?;
    let cancelled_readback = store
        .get_run_state(GetRunStateRequest {
            scope: cancel_scope,
            run_id: cancel_run_id,
        })
        .await?;
    ensure_status(
        cancelled_readback.status,
        TurnStatus::Cancelled,
        "cancel readback",
    )?;

    Ok(status_code(complete_submit_status)
        ^ (status_code(claimed_state.status) << 4)
        ^ (option_code(claimed_state.checkpoint_id.is_some()) << 8)
        ^ (status_code(blocked.status) << 12)
        ^ (status_code(resumed.status) << 16)
        ^ (status_code(reclaimed_state.status) << 20)
        ^ (option_code(reclaimed_state.checkpoint_id.is_some()) << 24)
        ^ (status_code(completed.status) << 28)
        ^ (status_code(completed_readback.status) << 32)
        ^ (status_code(cancel_submit_status) << 36)
        ^ (status_code(cancel_claimed_state.status) << 40)
        ^ (option_code(cancel_claimed_state.checkpoint_id.is_some()) << 44)
        ^ (status_code(cancel_requested.status) << 48)
        ^ (status_code(cancelled.status) << 52)
        ^ (status_code(cancelled_readback.status) << 56)
        ^ checkpoint_code)
}

async fn record_turn_lifecycle_checkpoints(
    store: Arc<dyn TurnLifecycleStore>,
    scope: &TurnScope,
    turn_id: TurnId,
    run_id: TurnRunId,
    first_state_ref: LoopCheckpointStateRef,
    key: &str,
    payload_len: usize,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    let checkpoint_count = turn_lifecycle_checkpoint_count(payload_len);
    let schema_id = CheckpointSchemaId::new("latency_turn_state")?;
    let schema_version = RunProfileVersion::new(1);
    let mut checksum = checkpoint_count as u64;

    for index in 0..checkpoint_count {
        let state_ref = if index == 0 {
            first_state_ref.clone()
        } else {
            LoopCheckpointStateRef::new(format!("checkpoint:latency-{key}:{index}"))?
        };
        let record = store
            .put_loop_checkpoint(PutLoopCheckpointRequest {
                scope: scope.clone(),
                turn_id,
                run_id,
                state_ref,
                schema_id: schema_id.clone(),
                schema_version,
                kind: LoopCheckpointKind::BeforeBlock,
                gate_ref: None,
            })
            .await?;
        let readback = store
            .get_loop_checkpoint(GetLoopCheckpointRequest {
                scope: scope.clone(),
                turn_id,
                run_id,
                checkpoint_id: record.checkpoint_id,
            })
            .await?
            .ok_or_else(|| format!("checkpoint {index} missing after put"))?;
        if readback != record {
            return Err(format!("checkpoint {index} readback did not match put").into());
        }
        checksum ^= ((index as u64 + 1) << (index % 16)) ^ schema_version.as_u64();
    }

    Ok(checksum)
}

fn turn_lifecycle_checkpoint_count(payload_len: usize) -> usize {
    (payload_len / 256).clamp(1, 16)
}

async fn claim_expected_run(
    store: Arc<dyn TurnLifecycleStore>,
    scope_filter: Option<TurnScope>,
    expected_run_id: TurnRunId,
    operation: &'static str,
) -> Result<
    (TurnRunnerId, TurnLeaseToken, ironclaw_turns::TurnRunState),
    Box<dyn std::error::Error + Send + Sync>,
> {
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    let claimed = store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter,
        })
        .await?
        .ok_or_else(|| format!("{operation} did not claim a run"))?;
    if claimed.state.run_id != expected_run_id {
        return Err(format!(
            "{operation} claimed {}, expected {expected_run_id}",
            claimed.state.run_id
        )
        .into());
    }
    ensure_status(claimed.state.status, TurnStatus::Running, operation)?;
    Ok((runner_id, lease_token, claimed.state))
}

fn turn_lifecycle_key(
    backend: BackendName,
    postgres_pool_size: Option<usize>,
    run_id: &str,
    sample: usize,
) -> String {
    let pool_label = postgres_pool_size
        .map(|pool_size| format!("p{pool_size}"))
        .unwrap_or_else(|| "base".to_string());
    format!("{}-{pool_label}-{run_id}-{sample}", backend.as_str())
}

fn turn_lifecycle_scope(
    key: &str,
    sample: usize,
    lane: &str,
) -> Result<TurnScope, Box<dyn std::error::Error + Send + Sync>> {
    let owner = turn_lifecycle_user(sample)?;
    Ok(TurnScope::new_with_owner(
        TenantId::new(format!("latency-turn-tenant-{lane}"))?,
        Some(AgentId::new(format!("latency-turn-agent-{lane}"))?),
        Some(ProjectId::new(format!("latency-turn-project-{lane}"))?),
        ThreadId::new(format!("latency-turn-{lane}-{key}"))?,
        Some(owner),
    ))
}

fn turn_lifecycle_actor(
    sample: usize,
) -> Result<TurnActor, Box<dyn std::error::Error + Send + Sync>> {
    Ok(TurnActor::new(turn_lifecycle_user(sample)?))
}

fn turn_lifecycle_user(sample: usize) -> Result<UserId, Box<dyn std::error::Error + Send + Sync>> {
    Ok(UserId::new(format!("latency-turn-user-{}", sample % 8))?)
}

fn turn_lifecycle_submit_request(
    scope: TurnScope,
    actor: TurnActor,
    key: &str,
    lane: &str,
    payload_len: usize,
) -> Result<SubmitTurnRequest, Box<dyn std::error::Error + Send + Sync>> {
    let pad_len = payload_len.min(96);
    let pad = "x".repeat(pad_len);
    Ok(SubmitTurnRequest {
        scope,
        actor,
        accepted_message_ref: AcceptedMessageRef::new(format!("message-{lane}-{key}-{pad}"))?,
        source_binding_ref: SourceBindingRef::new(format!("source-{lane}-{key}"))?,
        reply_target_binding_ref: ReplyTargetBindingRef::new(format!("reply-{lane}-{key}"))?,
        requested_run_profile: Some(RunProfileRequest::new("default")?),
        idempotency_key: IdempotencyKey::new(format!("idem-{lane}-{key}"))?,
        received_at: Utc.with_ymd_and_hms(2026, 7, 5, 0, 0, 0).unwrap(),
        requested_run_id: None,
        parent_run_id: None,
        subagent_depth: 0,
        spawn_tree_root_run_id: None,
        product_context: None,
    })
}

fn accepted_run(response: &SubmitTurnResponse) -> (TurnId, TurnRunId, TurnStatus) {
    let SubmitTurnResponse::Accepted {
        turn_id,
        run_id,
        status,
        ..
    } = response;
    (*turn_id, *run_id, *status)
}

fn ensure_status(
    actual: TurnStatus,
    expected: TurnStatus,
    operation: &'static str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if actual == expected {
        return Ok(());
    }
    Err(format!("{operation} returned {actual:?}, expected {expected:?}").into())
}

fn status_code(status: TurnStatus) -> u64 {
    match status {
        TurnStatus::Queued => 1,
        TurnStatus::Running => 2,
        TurnStatus::BlockedApproval => 3,
        TurnStatus::BlockedAuth => 4,
        TurnStatus::BlockedResource => 5,
        TurnStatus::BlockedDependentRun => 6,
        TurnStatus::BlockedExternalTool => 7,
        TurnStatus::CancelRequested => 8,
        TurnStatus::Cancelled => 9,
        TurnStatus::Completed => 10,
        TurnStatus::Failed => 11,
        TurnStatus::RecoveryRequired => 12,
    }
}

fn option_code(present: bool) -> u64 {
    if present { 1 } else { 0 }
}

const WEBUI_SESSION_TOKEN: &str = "latency-webui-token";
const WEBUI_SESSION_TENANT: &str = "latency-webui-tenant";
const WEBUI_SESSION_RUNTIME_USER: &str = "latency-webui-user-0";
const WEBUI_SESSION_USER_PREFIX: &str = "latency-webui-user-";
const WEBUI_SESSION_AGENT: &str = "latency-webui-agent";
const WEBUI_SESSION_USER_BUCKETS: usize = 64;

struct LatencyWebuiAuthenticator;

#[async_trait::async_trait]
impl WebuiAuthenticator for LatencyWebuiAuthenticator {
    async fn authenticate(&self, token: &str) -> Option<WebuiAuthentication> {
        let user = token.strip_prefix(WEBUI_SESSION_TOKEN)?;
        let user = user.strip_prefix('-')?;
        let user_id = UserId::new(format!("{WEBUI_SESSION_USER_PREFIX}{user}")).ok()?;
        Some(WebuiAuthentication::user(user_id))
    }
}

async fn webui_session(
    backend_context: BackendContext,
    backend: BackendName,
    postgres_pool_size: Option<usize>,
    sample: usize,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    let webui = ensure_webui_runtime_context(&backend_context, backend, postgres_pool_size).await?;
    let request = Request::builder()
        .method(Method::GET)
        .uri("/api/webchat/v2/session")
        .header(
            header::AUTHORIZATION,
            format!(
                "Bearer {WEBUI_SESSION_TOKEN}-{}",
                sample % WEBUI_SESSION_USER_BUCKETS
            ),
        )
        .body(Body::empty())?;
    let response = webui
        .router
        .clone()
        .oneshot(request)
        .await
        .map_err(|error| format!("webui session request failed: {error}"))?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 256 * 1024).await?;
    if status != StatusCode::OK {
        return Err(format!(
            "webui session returned {status}: {}",
            String::from_utf8_lossy(&bytes)
        )
        .into());
    }
    let response: serde_json::Value = serde_json::from_slice(&bytes)?;
    ensure_json_field(&response, "tenant_id", WEBUI_SESSION_TENANT)?;
    ensure_json_field(
        &response,
        "user_id",
        &format!(
            "{WEBUI_SESSION_USER_PREFIX}{}",
            sample % WEBUI_SESSION_USER_BUCKETS
        ),
    )?;
    let mut state = stable_hash_bytes(status.as_u16() as u64, &bytes);
    state = state.wrapping_add(option_code(
        response
            .get("features")
            .and_then(|features| features.get("global_auto_approve"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
    ));
    Ok(state)
}

async fn ensure_webui_runtime_context<'a>(
    backend_context: &'a BackendContext,
    backend: BackendName,
    postgres_pool_size: Option<usize>,
) -> Result<&'a WebuiRuntimeContext, Box<dyn std::error::Error + Send + Sync>> {
    let postgres_pool = backend_context.webui_postgres_pool.clone();
    backend_context
        .webui_session
        .get_or_try_init(|| async move {
            build_webui_runtime_context(backend, postgres_pool_size, postgres_pool).await
        })
        .await
}

async fn build_webui_runtime_context(
    backend: BackendName,
    postgres_pool_size: Option<usize>,
    postgres_pool: Option<deadpool_postgres::Pool>,
) -> Result<WebuiRuntimeContext, Box<dyn std::error::Error + Send + Sync>> {
    let root = tempfile::tempdir()?.keep();
    let storage_root = root.join(format!(
        "webui-{}-{}",
        backend.as_str(),
        uuid::Uuid::new_v4().simple()
    ));
    let workspace_root = root.join("workspace");
    let mut build_input = match backend {
        BackendName::Libsql => local_runtime_build_input(
            RebornCompositionProfile::HostedSingleTenantVolume,
            WEBUI_SESSION_RUNTIME_USER,
            storage_root,
        )?,
        BackendName::Postgres => {
            let pool = postgres_pool.ok_or_else(|| {
                format!(
                    "webui session postgres backend missing pool for size {:?}",
                    postgres_pool_size
                )
            })?;
            RebornBuildInput::hosted_single_tenant_postgres(
                RebornCompositionProfile::HostedSingleTenant,
                WEBUI_SESSION_RUNTIME_USER,
                storage_root,
                pool,
                latency_secret_master_key(),
            )?
            .with_runtime_policy(hosted_single_tenant_runtime_policy()?)
        }
    }
    .with_local_runtime_workspace_root(workspace_root);
    let tenant_id = TenantId::new(WEBUI_SESSION_TENANT)?;
    let agent_id = AgentId::new(WEBUI_SESSION_AGENT)?;
    build_input = build_input.with_local_runtime_identity(tenant_id.clone(), agent_id.clone());
    let runtime_input = RebornRuntimeInput::from_services(build_input)
        .with_identity(RebornRuntimeIdentity {
            tenant_id: WEBUI_SESSION_TENANT.to_string(),
            agent_id: WEBUI_SESSION_AGENT.to_string(),
            source_binding_id: "latency-webui-source".to_string(),
            reply_target_binding_id: "latency-webui-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(10),
        });
    let runtime = build_reborn_runtime(runtime_input).await?;
    let bundle = build_webui_services(&runtime, None)?;
    let config = WebuiServeConfig::new(
        tenant_id,
        Arc::new(LatencyWebuiAuthenticator),
        vec![HeaderValue::from_static("http://localhost:0")],
    )
    .with_default_agent_id(agent_id);
    let router = webui_v2_app(bundle, config)?;
    Ok(WebuiRuntimeContext {
        router,
        _runtime: runtime,
    })
}

fn ensure_json_field(
    value: &serde_json::Value,
    field: &'static str,
    expected: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let actual = value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("webui session response missing `{field}`"))?;
    if actual == expected {
        return Ok(());
    }
    Err(format!("webui session `{field}` was `{actual}`, expected `{expected}`").into())
}

fn stable_hash_bytes(seed: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(seed ^ 0xcbf29ce484222325, |state, byte| {
        state.wrapping_mul(0x100000001b3) ^ u64::from(*byte)
    })
}

async fn hosted_substrate_build(
    backend: BackendName,
    sample: usize,
    postgres_pool_size: Option<usize>,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    match backend {
        BackendName::Libsql => hosted_libsql_substrate_build(sample).await,
        BackendName::Postgres => {
            hosted_postgres_substrate_build(sample, postgres_pool_size.unwrap_or(2)).await
        }
    }
}

async fn hosted_libsql_substrate_build(
    sample: usize,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    let dir = tempfile::tempdir()?;
    let state_db_path = dir.path().join(format!("state-{sample}.db"));
    let events_db_path = dir.path().join(format!("events-{sample}.db"));
    let database = Arc::new(
        libsql::Builder::new_local(state_db_path.display().to_string())
            .build()
            .await?,
    );
    let services = build_libsql_production_host_runtime_services(LibSqlProductionSubstrateConfig {
        database,
        event_store: RebornEventStoreConfig::Libsql {
            path_or_url: events_db_path.display().to_string(),
            auth_token: None,
        },
        secret_master_key: Some(latency_secret_master_key()),
        trust_policy: Arc::new(ironclaw_trust::HostTrustPolicy::fail_closed()),
        runtime_policy: production_runtime_policy()?,
        turn_run_wake_notifier: Arc::new(RecordingSchedulerWakeNotifier),
        surface_version: latency_surface_version(sample)?,
    })
    .await?;
    services
        .validate_production_wiring(&hosted_substrate_wiring_config())
        .map_err(|report| format!("hosted libSQL substrate wiring failed: {report:?}"))?;
    Ok(0x71_00_u64 ^ sample as u64)
}

async fn hosted_postgres_substrate_build(
    sample: usize,
    postgres_pool_size: usize,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    let url = env::var("IRONCLAW_REBORN_POSTGRES_URL").unwrap_or_else(|_| {
        "postgres://postgres:postgres@localhost:5432/ironclaw_latency".to_string()
    });
    let config = url.parse::<tokio_postgres::Config>()?;
    let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    let pool = deadpool_postgres::Pool::builder(manager)
        .max_size(postgres_pool_size)
        .build()?;
    let services =
        build_postgres_production_host_runtime_services(PostgresProductionSubstrateConfig {
            pool,
            event_store: RebornEventStoreConfig::Postgres {
                url: ironclaw_secrets::SecretMaterial::from(url),
                tls_options: Default::default(),
            },
            secret_master_key: Some(latency_secret_master_key()),
            trust_policy: Arc::new(ironclaw_trust::HostTrustPolicy::fail_closed()),
            runtime_policy: production_runtime_policy()?,
            turn_run_wake_notifier: Arc::new(RecordingSchedulerWakeNotifier),
            surface_version: latency_surface_version(sample)?,
        })
        .await?;
    services
        .validate_production_wiring(&hosted_substrate_wiring_config())
        .map_err(|report| format!("hosted Postgres substrate wiring failed: {report:?}"))?;
    Ok(0x71_00_u64 ^ sample as u64)
}

fn hosted_substrate_wiring_config() -> ProductionWiringConfig {
    ProductionWiringConfig::new([])
        .require_runtime_http_egress()
        .require_credential_broker()
}

fn production_runtime_policy()
-> Result<RebornProductionRuntimePolicy, Box<dyn std::error::Error + Send + Sync>> {
    let policy = EffectiveRuntimePolicy {
        deployment: DeploymentMode::HostedMultiTenant,
        requested_profile: RuntimeProfile::HostedSafe,
        resolved_profile: RuntimeProfile::HostedSafe,
        filesystem_backend: FilesystemBackendKind::TenantWorkspace,
        process_backend: ProcessBackendKind::TenantSandbox,
        network_mode: NetworkMode::Brokered,
        secret_mode: SecretMode::TenantBroker,
        approval_policy: ApprovalPolicy::AskDestructive,
        audit_mode: AuditMode::Standard,
    };
    Ok(
        RebornProductionRuntimePolicy::with_tenant_sandbox_process_port(
            policy,
            Arc::new(ironclaw_host_runtime::TenantSandboxProcessPort::new(
                Arc::new(RecordingSandboxTransport),
            )),
        )?,
    )
}

fn latency_secret_master_key() -> ironclaw_secrets::SecretMaterial {
    ironclaw_secrets::SecretMaterial::from("01234567890123456789012345678901")
}

fn latency_surface_version(
    sample: usize,
) -> Result<CapabilitySurfaceVersion, Box<dyn std::error::Error + Send + Sync>> {
    Ok(CapabilitySurfaceVersion::new(format!("latency-{sample}"))?)
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
            saved_output: None,
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

fn compare(results: &[ResultRow]) -> Vec<ComparisonRow> {
    let mut libsql_by_key: BTreeMap<(&'static str, usize), &ResultRow> = BTreeMap::new();
    for row in results
        .iter()
        .filter(|row| row.backend == BackendName::Libsql)
    {
        libsql_by_key.insert((row.workload, row.concurrency), row);
    }
    let mut comparisons = Vec::new();
    for pg in results
        .iter()
        .filter(|row| row.backend == BackendName::Postgres)
    {
        let Some(libsql) = libsql_by_key.get(&(pg.workload, pg.concurrency)) else {
            continue;
        };
        let p50_ratio = ratio(pg.p50_ms, libsql.p50_ms);
        let p95_ratio = ratio(pg.p95_ms, libsql.p95_ms);
        let p99_ratio = ratio(pg.p99_ms, libsql.p99_ms);
        let throughput_ratio = ratio(pg.throughput_ops_sec, libsql.throughput_ops_sec);
        let errors_ok = pg.errors <= libsql.errors;
        let state_hash_ok = pg.state_hash == libsql.state_hash;
        let dev_pass = pg.errors == 0
            && libsql.errors == 0
            && state_hash_ok
            && pg.p50_ms <= (libsql.p50_ms * 1.10).max(libsql.p50_ms + 3.0)
            && pg.p95_ms <= (libsql.p95_ms * 1.15).max(libsql.p95_ms + 8.0)
            && pg.p99_ms <= (libsql.p99_ms * 1.25).max(libsql.p99_ms + 15.0)
            && throughput_ratio >= 0.90
            && errors_ok;
        let hard_fail = libsql.errors > 0
            || pg.errors > 0
            || p95_ratio > 1.5
            || p99_ratio > 2.0
            || !errors_ok
            || !state_hash_ok;
        comparisons.push(ComparisonRow {
            workload: pg.workload,
            concurrency: pg.concurrency,
            postgres_pool_size: pg.postgres_pool_size.unwrap_or_default(),
            postgres_p50_ratio: p50_ratio,
            postgres_p95_ratio: p95_ratio,
            postgres_p99_ratio: p99_ratio,
            postgres_throughput_ratio: throughput_ratio,
            errors_ok,
            state_hash_ok,
            dev_pass,
            hard_fail,
        });
    }
    comparisons
}

fn percentile_ms(latencies: &[Duration], percentile: f64) -> f64 {
    if latencies.is_empty() {
        return 0.0;
    }
    let rank = ((percentile / 100.0) * (latencies.len().saturating_sub(1) as f64)).ceil() as usize;
    latencies[rank.min(latencies.len() - 1)].as_secs_f64() * 1000.0
}

fn ratio(numerator: f64, denominator: f64) -> f64 {
    if denominator <= f64::EPSILON {
        return 0.0;
    }
    numerator / denominator
}

fn describe_error_chain(error: &(dyn std::error::Error + 'static)) -> String {
    let mut reason = error.to_string();
    let mut source = error.source();
    while let Some(error) = source {
        reason.push_str(": ");
        reason.push_str(&error.to_string());
        source = error.source();
    }
    reason
}

fn env_usize(name: &str, default: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn env_usize_allow_zero(name: &str, default: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(default)
}

fn env_list_usize(name: &str, default: &[usize]) -> Vec<usize> {
    env::var(name)
        .ok()
        .map(|value| {
            value
                .split(',')
                .filter_map(|part| part.trim().parse::<usize>().ok())
                .filter(|value| *value > 0)
                .collect::<Vec<_>>()
        })
        .filter(|values| !values.is_empty())
        .unwrap_or_else(|| default.to_vec())
}

fn env_backend_list(name: &str, default: &[BackendName]) -> Vec<BackendName> {
    env::var(name)
        .ok()
        .map(|value| {
            value
                .split(',')
                .filter_map(|part| match part.trim() {
                    "libsql" => Some(BackendName::Libsql),
                    "postgres" => Some(BackendName::Postgres),
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
        .filter(|values| !values.is_empty())
        .unwrap_or_else(|| default.to_vec())
}

fn filter_workloads(workloads: Vec<Workload>) -> Vec<Workload> {
    let Ok(raw) = env::var("LATENCY_WORKLOADS") else {
        return workloads;
    };
    let requested = raw
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();
    let filtered = workloads
        .iter()
        .copied()
        .filter(|workload| requested.iter().any(|name| *name == workload.name))
        .collect::<Vec<_>>();
    if filtered.is_empty() {
        workloads
    } else {
        filtered
    }
}

fn workload_prefix(
    backend: BackendName,
    run_id: &str,
    workload: &str,
    depth: usize,
) -> Result<VirtualPath, ironclaw_host_api::HostApiError> {
    let mut path = format!(
        "/engine/tenants/latency/users/{}/runs/{run_id}/{workload}",
        backend.as_str()
    );
    for i in 0..depth {
        path.push_str(&format!("/d{i}"));
    }
    VirtualPath::new(path)
}

fn child(prefix: &VirtualPath, name: &str) -> Result<VirtualPath, ironclaw_host_api::HostApiError> {
    VirtualPath::new(format!("{}/{name}", prefix.as_str().trim_end_matches('/')))
}

fn payload(seed: usize, len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| ((seed.wrapping_mul(31).wrapping_add(i)) % 251) as u8)
        .collect()
}
