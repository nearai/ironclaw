use std::collections::BTreeMap;
use std::env;
use std::future::Future;
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
    RebornHostBindings, RebornCompositionProfile, RebornProductionRuntimePolicy, RebornRuntime,
    RebornRuntimeIdentity, RebornRuntimeInput, build_libsql_production_host_runtime_services,
    build_postgres_production_host_runtime_services, build_reborn_runtime,
    hosted_single_tenant_runtime_policy, local_runtime_build_input,
};
use ironclaw_reborn_event_store::RebornEventStoreConfig;
use ironclaw_resources::{
    FilesystemResourceGovernor, ResourceAccount, ResourceGovernor, ResourceLimits,
};
use ironclaw_run_state::{ApprovalRequestStore, ApprovalRequestStorePort, ApprovalStatus};
use ironclaw_secrets::{SecretMaterial, SecretStore, SecretStorePort, SecretsCrypto};
use ironclaw_triggers::{
    LibSqlTriggerRepository, PostgresTriggerRepository, TriggerId, TriggerRecord,
    TriggerRepository, TriggerSchedule, TriggerSourceKind, TriggerState,
};
use ironclaw_turns::{
    AcceptedMessageRef, AllowAllTurnAdmissionPolicy, BlockedReason, CancelRunRequest,
    CheckpointSchemaId, TurnStateRowStore, GateRef, GetLoopCheckpointRequest,
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
use ironclaw_webui::{
    WebuiAuthentication, WebuiAuthenticator, WebuiServeConfig, webui_v2_app,
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
    let mode = match profile.as_str() {
        "holdout" => "holdout",
        "acceptance" => "acceptance",
        _ => "dev",
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

    let path_depths_shared: Arc<[usize]> = Arc::from(path_depths.clone());
    let payload_bytes_shared: Arc<[usize]> = Arc::from(payload_bytes.clone());

    let mut results = Vec::new();
    if backends.contains(&BackendName::Libsql) {
        let libsql_backend = open_backend(BackendName::Libsql, None).await?;
        let libsql_run_id = uuid::Uuid::new_v4().simple().to_string();
        for &workload in &workloads {
            for &concurrency in &concurrency {
                let run_id = format!("{libsql_run_id}-{}-c{concurrency}", workload.name);
                let row = run_workload(
                    WorkloadExecution {
                        backend_context: libsql_backend.clone(),
                        backend: BackendName::Libsql,
                        postgres_pool_size: None,
                        run_id: run_id.into(),
                        workload,
                        path_depths: Arc::clone(&path_depths_shared),
                        payload_bytes: Arc::clone(&payload_bytes_shared),
                    },
                    concurrency,
                    warmup,
                    samples,
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
                    let run_id = format!("{postgres_run_id}-{}-c{concurrency}", workload.name);
                    let row = run_workload(
                        WorkloadExecution {
                            backend_context: postgres_backend.clone(),
                            backend: BackendName::Postgres,
                            postgres_pool_size: Some(postgres_pool_size),
                            run_id: run_id.into(),
                            workload,
                            path_depths: Arc::clone(&path_depths_shared),
                            payload_bytes: Arc::clone(&payload_bytes_shared),
                        },
                        concurrency,
                        warmup,
                        samples,
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
    let gate_failed = scored_gate_failed(&report.mode, &report.results, &report.comparisons);
    println!("{}", serde_json::to_string_pretty(&report)?);
    if gate_failed {
        return Err(std::io::Error::other(
            "latency gate failed: scored rows had errors or required comparisons missed thresholds",
        )
        .into());
    }
    Ok(())
}

#[derive(Clone)]
struct BackendContext {
    fs: Arc<dyn RootFilesystem>,
    turn_state: Arc<dyn TurnLifecycleStore>,
    trigger_repository: Arc<dyn TriggerRepository>,
    approval_requests: Arc<dyn ApprovalRequestStorePort>,
    secret_store: Arc<dyn SecretStorePort>,
    resource_governor: Arc<dyn ResourceGovernor>,
    webui_session: Arc<OnceCell<WebuiRuntimeContext>>,
    webui_postgres_pool: Option<deadpool_postgres::Pool>,
    _tempdir: Option<Arc<tempfile::TempDir>>,
}

#[derive(Clone)]
struct WorkloadExecution {
    backend_context: BackendContext,
    backend: BackendName,
    postgres_pool_size: Option<usize>,
    run_id: Arc<str>,
    workload: Workload,
    path_depths: Arc<[usize]>,
    payload_bytes: Arc<[usize]>,
}

trait TurnLifecycleStore: TurnStateStore + TurnRunTransitionPort + LoopCheckpointStore {}

impl<T> TurnLifecycleStore for T where
    T: TurnStateStore + TurnRunTransitionPort + LoopCheckpointStore + Send + Sync
{
}

struct WebuiRuntimeContext {
    router: axum::Router,
    _runtime: RebornRuntime,
    _tempdir: tempfile::TempDir,
}

async fn open_backend(
    backend: BackendName,
    postgres_pool_size: Option<usize>,
) -> Result<BackendContext, Box<dyn std::error::Error>> {
    match backend {
        BackendName::Libsql => {
            let dir = tempfile::tempdir()?;
            let db_path = dir.path().join("latency-libsql.db");
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
                _tempdir: Some(Arc::new(dir)),
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
                _tempdir: None,
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
    // Both backends run the one production turn-state store; `backend` only
    // varies the durable filesystem mounted underneath `scoped` (above).
    Ok(Arc::new(TurnStateRowStore::new(scoped)))
}

struct ControlPlaneStores {
    approval_requests: Arc<dyn ApprovalRequestStorePort>,
    secret_store: Arc<dyn SecretStorePort>,
    resource_governor: Arc<dyn ResourceGovernor>,
}

fn control_plane_stores<F>(fs: Arc<F>) -> ControlPlaneStores
where
    F: RootFilesystem + 'static,
{
    let scoped = scoped_control_plane_fs(fs);
    let approval_requests = Arc::new(ApprovalRequestStore::new(Arc::clone(&scoped)));
    let secret_store = Arc::new(SecretStore::new(
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
    execution: WorkloadExecution,
    concurrency: usize,
    warmup: usize,
    samples: usize,
) -> Result<ResultRow, Box<dyn std::error::Error>> {
    for i in 0..warmup {
        setup_workload(execution.clone(), i)
            .await
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        let _ = run_one(execution.clone(), i).await;
    }

    let sem = Arc::new(Semaphore::new(concurrency.max(1)));
    let started = Instant::now();
    let mut tasks = Vec::with_capacity(samples);
    for i in 0..samples {
        let permit = Arc::clone(&sem).acquire_owned().await?;
        let execution = execution.clone();
        tasks.push(tokio::spawn(async move {
            let _permit = permit;
            let sample = i + warmup;
            setup_workload(execution.clone(), sample).await?;
            run_one(execution, sample).await
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
        backend: execution.backend,
        postgres_pool_size: execution.postgres_pool_size,
        workload: execution.workload.name,
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
    execution: WorkloadExecution,
    sample: usize,
) -> Result<Sample, Box<dyn std::error::Error + Send + Sync>> {
    let depth = execution.path_depths[sample % execution.path_depths.len()].max(1);
    let payload_len = execution.payload_bytes[sample % execution.payload_bytes.len()].max(1);
    let prefix = workload_prefix(
        execution.backend,
        &execution.run_id,
        execution.workload.name,
        depth,
    )?;
    let backend_context = execution.backend_context;
    let started = Instant::now();
    let state = match execution.workload.kind {
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
                execution.backend,
                execution.postgres_pool_size,
                &execution.run_id,
                sample,
            )
            .await?
        }
        WorkloadKind::ControlPlaneSnapshot => {
            control_plane_snapshot(
                backend_context.approval_requests,
                backend_context.secret_store,
                backend_context.resource_governor,
                execution.backend,
                execution.postgres_pool_size,
                &execution.run_id,
                sample,
            )
            .await?
        }
        WorkloadKind::TurnLifecycle => {
            turn_lifecycle(
                backend_context.turn_state,
                execution.backend,
                execution.postgres_pool_size,
                &execution.run_id,
                sample,
                payload_len,
            )
            .await?
        }
        WorkloadKind::WebuiSession => {
            webui_session(
                backend_context,
                execution.backend,
                execution.postgres_pool_size,
                sample,
            )
            .await?
        }
        WorkloadKind::HostedSubstrateBuild => {
            hosted_substrate_build(execution.backend, sample, execution.postgres_pool_size).await?
        }
    };
    Ok(Sample {
        elapsed: started.elapsed(),
        state,
    })
}

async fn setup_workload(
    execution: WorkloadExecution,
    sample: usize,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if matches!(
        execution.workload.kind,
        WorkloadKind::TriggerSeedList
            | WorkloadKind::TurnLifecycle
            | WorkloadKind::HostedSubstrateBuild
    ) {
        return Ok(());
    }

    if matches!(execution.workload.kind, WorkloadKind::WebuiSession) {
        ensure_webui_runtime_context(
            &execution.backend_context,
            execution.backend,
            execution.postgres_pool_size,
        )
        .await
        .map_err(|error| std::io::Error::other(error.to_string()))?;
        return Ok(());
    }

    if matches!(execution.workload.kind, WorkloadKind::ControlPlaneSnapshot) {
        setup_control_plane_indexes(execution.backend_context.fs).await?;
        return Ok(());
    }

    let depth = execution.path_depths[sample % execution.path_depths.len()].max(1);
    let prefix = workload_prefix(
        execution.backend,
        &execution.run_id,
        execution.workload.name,
        depth,
    )?;
    let fs = execution.backend_context.fs;
    setup_create_dir_all(Arc::clone(&fs), &prefix).await?;
    match execution.workload.kind {
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
            seed_query_exact_records(fs, &prefix, sample, &execution.payload_bytes).await?;
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
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    retry_setup_operation(|| {
        let fs = Arc::clone(&fs);
        let prefix = prefix.clone();
        async move { fs.create_dir_all(&prefix).await }
    })
    .await
}

async fn setup_control_plane_indexes(
    fs: Arc<dyn RootFilesystem>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let scoped = scoped_control_plane_fs(fs);
    let scope = ResourceScope::system();
    let secrets_root = ScopedPath::new("/secrets")?;
    let spec = IndexSpec::new(
        IndexName::new("secrets_by_tenant")?,
        vec![IndexKey::new("tenant_id")?],
        IndexKind::Exact,
    );
    retry_setup_operation(|| {
        let scoped = Arc::clone(&scoped);
        let scope = scope.clone();
        let secrets_root = secrets_root.clone();
        let spec = spec.clone();
        async move { scoped.ensure_index(&scope, &secrets_root, &spec).await }
    })
    .await
}

async fn setup_ensure_index(
    fs: Arc<dyn RootFilesystem>,
    prefix: &VirtualPath,
    spec: IndexSpec,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    retry_setup_operation(|| {
        let fs = Arc::clone(&fs);
        let prefix = prefix.clone();
        let spec = spec.clone();
        async move { fs.ensure_index(&prefix, &spec).await }
    })
    .await
}

async fn retry_setup_operation<F, Fut>(
    mut operation: F,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<(), ironclaw_filesystem::FilesystemError>>,
{
    for attempt in 0..5 {
        match operation().await {
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
        let latency_thresholds_ok = pg.p50_ms <= (libsql.p50_ms * 1.10).max(libsql.p50_ms + 3.0)
            && pg.p95_ms <= (libsql.p95_ms * 1.15).max(libsql.p95_ms + 8.0)
            && pg.p99_ms <= (libsql.p99_ms * 1.25).max(libsql.p99_ms + 15.0);
        let dev_pass = pg.errors == 0
            && libsql.errors == 0
            && state_hash_ok
            && latency_thresholds_ok
            && throughput_ratio >= 0.90
            && errors_ok;
        let latency_hard_fail = !latency_thresholds_ok && (p95_ratio > 1.5 || p99_ratio > 2.0);
        let hard_fail =
            libsql.errors > 0 || pg.errors > 0 || latency_hard_fail || !errors_ok || !state_hash_ok;
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

fn scored_gate_failed(mode: &str, results: &[ResultRow], comparisons: &[ComparisonRow]) -> bool {
    let scored_rows_failed = results.iter().any(|row| row.errors > 0);
    let hard_comparison_failed = comparisons.iter().any(|row| row.hard_fail);
    let threshold_comparison_failed =
        matches!(mode, "holdout" | "acceptance") && comparisons.iter().any(|row| !row.dev_pass);
    scored_rows_failed || hard_comparison_failed || threshold_comparison_failed
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
        .filter(|workload| requested.contains(&workload.name))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn result_row(backend: BackendName, workload: &'static str, errors: usize) -> ResultRow {
        ResultRow {
            backend,
            postgres_pool_size: None,
            workload,
            concurrency: 1,
            samples: 1,
            errors,
            first_error: None,
            throughput_ops_sec: 1.0,
            p50_ms: 1.0,
            p95_ms: 1.0,
            p99_ms: 1.0,
            state_hash: "state".to_string(),
        }
    }

    fn comparison_row(hard_fail: bool) -> ComparisonRow {
        ComparisonRow {
            workload: "put_get",
            concurrency: 1,
            postgres_pool_size: 1,
            postgres_p50_ratio: 1.0,
            postgres_p95_ratio: 1.0,
            postgres_p99_ratio: 1.0,
            postgres_throughput_ratio: 1.0,
            errors_ok: true,
            state_hash_ok: true,
            dev_pass: !hard_fail,
            hard_fail,
        }
    }

    #[test]
    fn scored_gate_fails_when_any_result_row_has_errors() {
        let results = vec![result_row(BackendName::Libsql, "put_get", 1)];

        assert!(scored_gate_failed("dev", &results, &[]));
    }

    #[test]
    fn scored_gate_fails_when_required_comparison_hard_fails() {
        let comparisons = vec![comparison_row(true)];

        assert!(scored_gate_failed("dev", &[], &comparisons));
    }

    #[test]
    fn scored_gate_fails_holdout_when_required_comparison_misses_threshold() {
        let mut comparison = comparison_row(false);
        comparison.dev_pass = false;

        assert!(scored_gate_failed("holdout", &[], &[comparison]));
    }

    #[test]
    fn scored_gate_fails_acceptance_when_required_comparison_misses_threshold() {
        let mut comparison = comparison_row(false);
        comparison.dev_pass = false;

        assert!(scored_gate_failed("acceptance", &[], &[comparison]));
    }

    #[test]
    fn scored_gate_allows_dev_threshold_miss_without_hard_failure() {
        let mut comparison = comparison_row(false);
        comparison.dev_pass = false;

        assert!(!scored_gate_failed("dev", &[], &[comparison]));
    }

    #[test]
    fn compare_allows_high_ratio_when_additive_latency_target_passes() {
        let mut libsql = result_row(BackendName::Libsql, "query_exact", 0);
        libsql.p50_ms = 0.2;
        libsql.p95_ms = 0.3;
        libsql.p99_ms = 0.4;
        libsql.throughput_ops_sec = 100.0;

        let mut postgres = result_row(BackendName::Postgres, "query_exact", 0);
        postgres.postgres_pool_size = Some(1);
        postgres.p50_ms = 0.6;
        postgres.p95_ms = 0.9;
        postgres.p99_ms = 1.2;
        postgres.throughput_ops_sec = 100.0;

        let comparisons = compare(&[libsql, postgres]);

        assert_eq!(comparisons.len(), 1);
        assert!(comparisons[0].dev_pass);
        assert!(!comparisons[0].hard_fail);
        assert!(comparisons[0].postgres_p95_ratio > 1.5);
    }

    #[test]
    fn scored_gate_passes_clean_rows_and_comparisons() {
        let results = vec![result_row(BackendName::Postgres, "put_get", 0)];
        let comparisons = vec![comparison_row(false)];

        assert!(!scored_gate_failed("holdout", &results, &comparisons));
    }
}
