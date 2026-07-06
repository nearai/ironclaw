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
    FilesystemResourceGovernor, ResourceAccount, ResourceGovernor, ResourceLimits,
};
use ironclaw_run_state::{ApprovalRequestStore, ApprovalStatus, FilesystemApprovalRequestStore};
use ironclaw_secrets::{FilesystemSecretStore, SecretMaterial, SecretStore, SecretsCrypto};
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

mod runtime_workloads;
mod workloads;

use runtime_workloads::*;
use workloads::*;

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
            let trigger_repository = PostgresTriggerRepository::new(pool.clone());
            trigger_repository.run_migrations().await?;
            let control_plane = control_plane_stores(Arc::clone(&fs));
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
        BackendName::Libsql => FilesystemTurnStateStoreKind::row(scoped),
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
    let resource_governor = Arc::new(FilesystemResourceGovernor::new(scoped));
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
