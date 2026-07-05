use std::collections::BTreeMap;
use std::env;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ironclaw_filesystem::{
    CasExpectation, Entry, Filter, IndexKey, IndexKind, IndexName, IndexSpec, IndexValue,
    LibSqlRootFilesystem, Page, PostgresRootFilesystem, RootFilesystem, SeqNo,
};
use ironclaw_host_api::VirtualPath;
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
    LibSqlProductionSubstrateConfig, PostgresProductionSubstrateConfig,
    RebornProductionRuntimePolicy, build_libsql_production_host_runtime_services,
    build_postgres_production_host_runtime_services,
};
use ironclaw_reborn_event_store::RebornEventStoreConfig;
use ironclaw_turns::{TurnRunWake, TurnRunWakeNotifier, TurnRunWakeNotifyError};
use serde::Serialize;
use tokio::sync::Semaphore;

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
    HostedSubstrateBuild,
}

#[derive(Debug, Serialize)]
struct RunReport {
    profile: String,
    mode: String,
    warmup: usize,
    samples: usize,
    concurrency: Vec<usize>,
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
            name: "hosted_substrate_build",
            kind: WorkloadKind::HostedSubstrateBuild,
        },
    ]);

    let mut results = Vec::new();
    let libsql_fs = open_backend(BackendName::Libsql, None).await?;
    let libsql_run_id = uuid::Uuid::new_v4().simple().to_string();
    for &workload in &workloads {
        for &concurrency in &concurrency {
            let row = run_workload(
                Arc::clone(&libsql_fs),
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

    for &postgres_pool_size in &postgres_pool_sizes {
        let postgres_fs = open_backend(BackendName::Postgres, Some(postgres_pool_size)).await?;
        let postgres_run_id = uuid::Uuid::new_v4().simple().to_string();
        for &workload in &workloads {
            for &concurrency in &concurrency {
                let row = run_workload(
                    Arc::clone(&postgres_fs),
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

    let comparisons = compare(&results);
    let report = RunReport {
        profile,
        mode,
        warmup,
        samples,
        concurrency,
        postgres_pool_sizes,
        path_depths,
        payload_bytes,
        acceptance_ready: false,
        notes: vec![
            "dev scorer: storage hot paths plus production-shaped hosted substrate build/readiness",
            "full acceptance still requires launch-ref libSQL baseline and hosted profile/WebUI/turn/trigger/approval/resource request workloads",
        ],
        results,
        comparisons,
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

async fn open_backend(
    backend: BackendName,
    postgres_pool_size: Option<usize>,
) -> Result<Arc<dyn RootFilesystem>, Box<dyn std::error::Error>> {
    match backend {
        BackendName::Libsql => {
            let dir = tempfile::tempdir()?;
            let db_path = dir.keep().join("latency-libsql.db");
            let db = Arc::new(libsql::Builder::new_local(db_path).build().await?);
            let fs = LibSqlRootFilesystem::new(db);
            fs.run_migrations().await?;
            Ok(Arc::new(fs))
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
            let fs = PostgresRootFilesystem::new(pool);
            fs.run_migrations().await?;
            Ok(Arc::new(fs))
        }
    }
}

async fn run_workload(
    fs: Arc<dyn RootFilesystem>,
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
    for i in 0..warmup {
        setup_workload(Arc::clone(&fs), backend, run_id, workload, i, path_depths).await?;
        let _ = run_one(
            Arc::clone(&fs),
            backend,
            postgres_pool_size,
            run_id,
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
            Arc::clone(&fs),
            backend,
            run_id,
            workload,
            i + warmup,
            path_depths,
        )
        .await?;
        let permit = Arc::clone(&sem).acquire_owned().await?;
        let fs = Arc::clone(&fs);
        let run_id = run_id.to_string();
        let path_depths = path_depths.to_vec();
        let payload_bytes = payload_bytes.to_vec();
        tasks.push(tokio::spawn(async move {
            let _permit = permit;
            run_one(
                fs,
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
    fs: Arc<dyn RootFilesystem>,
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
        WorkloadKind::PutGet => put_get(fs, &prefix, sample, payload_len).await?,
        WorkloadKind::QueryExact => query_exact(fs, &prefix, sample, payload_len).await?,
        WorkloadKind::AppendTail => append_tail(fs, &prefix, sample, payload_len).await?,
        WorkloadKind::ReserveSequence => reserve_sequence(fs, &prefix, sample).await?,
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
    fs: Arc<dyn RootFilesystem>,
    backend: BackendName,
    run_id: &str,
    workload: Workload,
    sample: usize,
    path_depths: &[usize],
) -> Result<(), Box<dyn std::error::Error>> {
    let depth = path_depths[sample % path_depths.len()].max(1);
    let prefix = workload_prefix(backend, run_id, workload.name, depth)?;
    if matches!(workload.kind, WorkloadKind::QueryExact) {
        fs.ensure_index(
            &prefix,
            &IndexSpec::new(
                IndexName::new("bucket_exact")?,
                vec![IndexKey::new("bucket")?],
                IndexKind::Exact,
            ),
        )
        .await?;
    }
    Ok(())
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
    payload_len: usize,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    let key = IndexKey::new("bucket")?;
    let kind = ironclaw_filesystem::RecordKind::new("latency_record")?;
    let bucket = format!("b{}", sample % 8);
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
