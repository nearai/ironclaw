use std::{
    any::Any,
    collections::BTreeMap,
    env::{self, VarError},
    io::ErrorKind,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, mpsc},
    thread,
    thread::JoinHandle,
    time::{Duration, Instant},
};

use clap::{Parser, ValueEnum};
use ironclaw_filesystem::{RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    InvocationId, MountAlias, MountGrant, MountPermissions, MountView, ResourceEstimate,
    ResourceScope, ResourceUsage, TenantId, UserId, VirtualPath,
};
use ironclaw_resources::{
    FilesystemResourceGovernorStore, PersistentResourceGovernor, ResourceAccount, ResourceError,
    ResourceGovernor,
};
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Parser)]
#[command(
    name = "ironclaw_storage_stress",
    about = "Stress current filesystem-backed resource governor storage"
)]
struct Args {
    #[arg(long, value_enum)]
    backend: Backend,

    /// OS processes to run against the same snapshot path. Use >1 to exercise
    /// cross-process CAS contention that the in-process lock cannot serialize.
    #[arg(long, default_value_t = 1)]
    processes: usize,

    /// Threads per process.
    #[arg(long, default_value_t = 8)]
    concurrency: usize,

    /// Operations per thread.
    #[arg(long, default_value_t = 200)]
    operations: usize,

    /// Synthetic users distributed across operations.
    #[arg(long, default_value_t = 50)]
    users: usize,

    /// Synthetic tenants distributed across users.
    #[arg(long, default_value_t = 1)]
    tenants: usize,

    #[arg(long, value_enum, default_value_t = Scenario::ReserveRelease)]
    scenario: Scenario,

    /// Shared run id. Defaults to a fresh UUID.
    #[arg(long)]
    run_id: Option<String>,

    /// libSQL database path. Defaults to a temp-file path printed in output.
    #[arg(long)]
    libsql_path: Option<PathBuf>,

    /// Postgres URL. Defaults to IRONCLAW_FILESYSTEM_POSTGRES_URL, then DATABASE_URL.
    #[arg(long)]
    postgres_url: Option<String>,

    /// Postgres pool size per process.
    #[arg(long, default_value_t = 4)]
    postgres_pool_size: usize,

    #[arg(long, hide = true)]
    child_index: Option<usize>,
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Backend {
    Libsql,
    Postgres,
}

impl Backend {
    fn as_str(self) -> &'static str {
        match self {
            Self::Libsql => "libsql",
            Self::Postgres => "postgres",
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Scenario {
    ReserveRelease,
    ReserveReconcile,
}

impl Scenario {
    fn as_str(self) -> &'static str {
        match self {
            Self::ReserveRelease => "reserve-release",
            Self::ReserveReconcile => "reserve-reconcile",
        }
    }
}

struct BackendHandle {
    governor: Arc<dyn ResourceGovernor>,
    target: String,
}

#[derive(Debug, Clone)]
struct Sample {
    latency: Duration,
    error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct LatencySummary {
    min_us: u128,
    p50_us: u128,
    p95_us: u128,
    p99_us: u128,
    max_us: u128,
}

#[derive(Debug, Serialize, Deserialize)]
struct RunSummary {
    backend: Backend,
    scenario: Scenario,
    run_id: String,
    target: String,
    child_index: Option<usize>,
    processes: usize,
    concurrency: usize,
    operations_per_thread: usize,
    users: usize,
    tenants: usize,
    attempted: u64,
    succeeded: u64,
    failed: u64,
    duration_ms: u128,
    throughput_ops_sec: f64,
    latency: LatencySummary,
    errors: BTreeMap<String, u64>,
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let mut args = Args::parse();
    validate_args(&args)?;

    let run_id = args
        .run_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
    args.run_id = Some(run_id.clone());
    let generated_libsql_path = if args.child_index.is_none()
        && matches!(args.backend, Backend::Libsql)
        && args.libsql_path.is_none()
    {
        let path = default_libsql_path();
        args.libsql_path = Some(path.clone());
        Some(path)
    } else {
        None
    };

    let result = if args.child_index.is_none() && args.processes > 1 {
        match prewarm(&args, &run_id).await {
            Ok(()) => run_child_processes(&args, &run_id)
                .and_then(|summaries| print_parent_summary(&args, &run_id, &summaries)),
            Err(error) => Err(error),
        }
    } else {
        match run_in_process(&args, &run_id).await {
            Ok(summary) => serde_json::to_string_pretty(&summary)
                .map_err(|error| error.to_string())
                .map(|encoded| println!("{encoded}")),
            Err(error) => Err(error),
        }
    };

    if let Some(path) = generated_libsql_path {
        cleanup_generated_libsql_path(&path).await;
    }

    result
}

fn validate_args(args: &Args) -> Result<(), String> {
    if args.processes == 0 {
        return Err("--processes must be greater than 0".to_string());
    }
    if args.concurrency == 0 {
        return Err("--concurrency must be greater than 0".to_string());
    }
    if args.operations == 0 {
        return Err("--operations must be greater than 0".to_string());
    }
    if args.users == 0 {
        return Err("--users must be greater than 0".to_string());
    }
    if args.tenants == 0 {
        return Err("--tenants must be greater than 0".to_string());
    }
    if args.postgres_pool_size == 0 {
        return Err("--postgres-pool-size must be greater than 0".to_string());
    }
    Ok(())
}

async fn prewarm(args: &Args, run_id: &str) -> Result<(), String> {
    let backend = build_backend(args, run_id).await?;
    let account = ResourceAccount::tenant(TenantId::new("stress-prewarm").map_err(display_err)?);
    backend
        .governor
        .account_snapshot(&account)
        .map_err(|error| format!("prewarm failed: {error:?}"))?;
    Ok(())
}

fn run_child_processes(args: &Args, run_id: &str) -> Result<Vec<RunSummary>, String> {
    let current_exe =
        std::env::current_exe().map_err(|error| format!("resolve current executable: {error}"))?;
    let libsql_path = args.libsql_path.clone().unwrap_or_else(default_libsql_path);

    let mut children = Vec::with_capacity(args.processes);
    for child_index in 0..args.processes {
        let mut command = Command::new(&current_exe);
        command
            .arg("--backend")
            .arg(args.backend.as_str())
            .arg("--processes")
            .arg("1")
            .arg("--concurrency")
            .arg(args.concurrency.to_string())
            .arg("--operations")
            .arg(args.operations.to_string())
            .arg("--users")
            .arg(args.users.to_string())
            .arg("--tenants")
            .arg(args.tenants.to_string())
            .arg("--scenario")
            .arg(args.scenario.as_str())
            .arg("--postgres-pool-size")
            .arg(args.postgres_pool_size.to_string())
            .arg("--run-id")
            .arg(run_id)
            .arg("--child-index")
            .arg(child_index.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if matches!(args.backend, Backend::Libsql) {
            command.arg("--libsql-path").arg(&libsql_path);
        }
        if let Some(url) = &args.postgres_url {
            command.env("IRONCLAW_FILESYSTEM_POSTGRES_URL", url);
        }
        match command.spawn() {
            Ok(child) => children.push((child_index, child)),
            Err(error) => {
                terminate_children(&mut children);
                return Err(format!("spawn child {child_index}: {error}"));
            }
        }
    }

    let mut summaries = Vec::with_capacity(children.len());
    while !children.is_empty() {
        let (child_index, child) = children.remove(0);
        let output = match child.wait_with_output() {
            Ok(output) => output,
            Err(error) => {
                terminate_children(&mut children);
                return Err(format!("wait for child {child_index}: {error}"));
            }
        };
        if !output.status.success() {
            terminate_children(&mut children);
            return Err(format!(
                "child {child_index} failed with status {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        let stdout = match String::from_utf8(output.stdout) {
            Ok(stdout) => stdout,
            Err(error) => {
                terminate_children(&mut children);
                return Err(format!(
                    "child {child_index} emitted non-utf8 stdout: {error}"
                ));
            }
        };
        let summary: RunSummary = match serde_json::from_str(stdout.trim()) {
            Ok(summary) => summary,
            Err(error) => {
                terminate_children(&mut children);
                return Err(format!(
                    "parse child {child_index} summary: {error}: {stdout}"
                ));
            }
        };
        summaries.push(summary);
    }
    summaries.sort_by_key(|summary| summary.child_index.unwrap_or(usize::MAX));
    Ok(summaries)
}

async fn run_in_process(args: &Args, run_id: &str) -> Result<RunSummary, String> {
    let backend = build_backend(args, run_id).await?;
    let identities = Arc::new(SyntheticIds::new(args)?);
    let started = Instant::now();
    let governor = Arc::clone(&backend.governor);
    let args_clone = args.clone();
    let samples =
        tokio::task::spawn_blocking(move || run_threads(&governor, &args_clone, &identities))
            .await
            .map_err(|error| {
                if error.is_panic() {
                    eprintln!("run_threads task panicked: {error:?}");
                    "run_threads task panicked".to_string()
                } else {
                    eprintln!("run_threads task cancelled: {error:?}");
                    "run_threads task cancelled".to_string()
                }
            })??;
    let elapsed = started.elapsed();
    Ok(summarize(args, run_id, backend.target, elapsed, samples))
}

fn run_threads(
    governor: &Arc<dyn ResourceGovernor>,
    args: &Args,
    identities: &Arc<SyntheticIds>,
) -> Result<Vec<Sample>, String> {
    let (sender, receiver) = mpsc::channel();
    let mut handles = Vec::with_capacity(args.concurrency);

    for worker_index in 0..args.concurrency {
        let governor = Arc::clone(governor);
        let identities = Arc::clone(identities);
        let sender = sender.clone();
        let args = args.clone();
        let handle = match thread::Builder::new()
            .name(format!("storage-stress-{worker_index}"))
            .spawn(move || -> Result<(), String> {
                let mut samples = Vec::with_capacity(args.operations);
                for operation_index in 0..args.operations {
                    samples.push(run_one_operation(
                        &governor,
                        &args,
                        &identities,
                        worker_index,
                        operation_index,
                    ));
                }
                sender
                    .send(samples)
                    .map_err(|_| "sample receiver dropped".to_string())
            }) {
            Ok(handle) => handle,
            Err(error) => {
                join_workers(handles)?;
                return Err(format!("spawn worker {worker_index}: {error}"));
            }
        };
        handles.push((worker_index, handle));
    }
    drop(sender);

    let mut samples = Vec::with_capacity(args.concurrency * args.operations);
    for worker_samples in receiver {
        samples.extend(worker_samples);
    }
    join_workers(handles)?;
    let expected = args.concurrency * args.operations;
    if samples.len() != expected {
        return Err(format!(
            "collected {} samples but expected {expected}",
            samples.len()
        ));
    }
    Ok(samples)
}

fn run_one_operation(
    governor: &Arc<dyn ResourceGovernor>,
    args: &Args,
    identities: &SyntheticIds,
    worker_index: usize,
    operation_index: usize,
) -> Sample {
    let scope = identities.scope(args, worker_index, operation_index);
    let estimate = ResourceEstimate {
        usd: Some(dec!(0.000001)),
        input_tokens: Some(8),
        output_tokens: Some(4),
        wall_clock_ms: Some(1),
        output_bytes: Some(16),
        network_egress_bytes: Some(0),
        process_count: Some(0),
        concurrency_slots: Some(1),
    };
    let usage = ResourceUsage {
        usd: dec!(0.000001),
        input_tokens: 8,
        output_tokens: 4,
        wall_clock_ms: 1,
        output_bytes: 16,
        network_egress_bytes: 0,
        process_count: 0,
    };

    let started = Instant::now();
    let outcome = match args.scenario {
        Scenario::ReserveRelease => governor
            .reserve(scope, estimate)
            .and_then(|reservation| governor.release(reservation.id).map(|_| ())),
        Scenario::ReserveReconcile => governor
            .reserve(scope, estimate)
            .and_then(|reservation| governor.reconcile(reservation.id, usage).map(|_| ())),
    };
    let latency = started.elapsed();
    Sample {
        latency,
        error: outcome.err().map(|error| classify_error(&error)),
    }
}

fn summarize(
    args: &Args,
    run_id: &str,
    target: String,
    elapsed: Duration,
    samples: Vec<Sample>,
) -> RunSummary {
    let mut errors = BTreeMap::new();
    let mut latencies: Vec<u128> = samples
        .iter()
        .map(|sample| sample.latency.as_micros())
        .collect();
    latencies.sort_unstable();
    let failed = samples
        .iter()
        .filter_map(|sample| sample.error.as_ref())
        .map(|error| {
            *errors.entry(error.clone()).or_insert(0) += 1;
        })
        .count() as u64;
    let attempted = samples.len() as u64;
    let succeeded = attempted.saturating_sub(failed);
    let elapsed_secs = elapsed.as_secs_f64().max(f64::MIN_POSITIVE);

    RunSummary {
        backend: args.backend,
        scenario: args.scenario,
        run_id: run_id.to_string(),
        target,
        child_index: args.child_index,
        processes: args.processes,
        concurrency: args.concurrency,
        operations_per_thread: args.operations,
        users: args.users,
        tenants: args.tenants,
        attempted,
        succeeded,
        failed,
        duration_ms: elapsed.as_millis(),
        throughput_ops_sec: attempted as f64 / elapsed_secs,
        latency: latency_summary(&latencies),
        errors,
    }
}

fn latency_summary(latencies: &[u128]) -> LatencySummary {
    if latencies.is_empty() {
        return LatencySummary {
            min_us: 0,
            p50_us: 0,
            p95_us: 0,
            p99_us: 0,
            max_us: 0,
        };
    }
    LatencySummary {
        min_us: latencies[0],
        p50_us: percentile(latencies, 50),
        p95_us: percentile(latencies, 95),
        p99_us: percentile(latencies, 99),
        max_us: latencies[latencies.len() - 1],
    }
}

fn percentile(sorted: &[u128], percentile: usize) -> u128 {
    let last = sorted.len().saturating_sub(1);
    let index = (last * percentile).div_ceil(100);
    sorted[index.min(last)]
}

fn print_parent_summary(args: &Args, run_id: &str, summaries: &[RunSummary]) -> Result<(), String> {
    let attempted: u64 = summaries.iter().map(|summary| summary.attempted).sum();
    let succeeded: u64 = summaries.iter().map(|summary| summary.succeeded).sum();
    let failed: u64 = summaries.iter().map(|summary| summary.failed).sum();
    let mut errors = BTreeMap::new();
    for summary in summaries {
        for (error, count) in &summary.errors {
            *errors.entry(error.clone()).or_insert(0) += count;
        }
    }
    let max_duration_ms = summaries
        .iter()
        .map(|summary| summary.duration_ms)
        .max()
        .unwrap_or(0);
    let throughput_ops_sec = if max_duration_ms == 0 {
        0.0
    } else {
        attempted as f64 / (max_duration_ms as f64 / 1000.0)
    };
    let p99_us = summaries
        .iter()
        .map(|summary| summary.latency.p99_us)
        .max()
        .unwrap_or(0);
    let max_us = summaries
        .iter()
        .map(|summary| summary.latency.max_us)
        .max()
        .unwrap_or(0);
    let target = summaries
        .first()
        .map(|summary| summary.target.as_str())
        .unwrap_or("unknown");

    let aggregate = serde_json::json!({
        "backend": args.backend,
        "scenario": args.scenario,
        "run_id": run_id,
        "target": target,
        "processes": args.processes,
        "concurrency_per_process": args.concurrency,
        "attempted": attempted,
        "succeeded": succeeded,
        "failed": failed,
        "max_duration_ms": max_duration_ms,
        "throughput_ops_sec": throughput_ops_sec,
        "worst_child_p99_us": p99_us,
        "worst_child_max_us": max_us,
        "errors": errors,
        "children": summaries,
    });
    let encoded = serde_json::to_string_pretty(&aggregate).map_err(display_err)?;
    println!("{encoded}");
    Ok(())
}

fn classify_error(error: &ResourceError) -> String {
    match error {
        ResourceError::Storage { reason } if reason.contains("cross-process CAS contention") => {
            "storage_cross_process_cas_contention".to_string()
        }
        ResourceError::Storage { .. } => "storage".to_string(),
        ResourceError::LimitExceeded { .. } => "limit_exceeded".to_string(),
        ResourceError::RequiresApproval { .. } => "requires_approval".to_string(),
        ResourceError::ReservationAlreadyExists { .. } => "reservation_already_exists".to_string(),
        ResourceError::InvalidEstimate { .. } => "invalid_estimate".to_string(),
        ResourceError::ReservationMismatch { .. } => "reservation_mismatch".to_string(),
        ResourceError::UnknownReservation { .. } => "unknown_reservation".to_string(),
        ResourceError::ReservationClosed { .. } => "reservation_closed".to_string(),
    }
}

async fn build_backend(args: &Args, run_id: &str) -> Result<BackendHandle, String> {
    match args.backend {
        Backend::Libsql => build_libsql_backend(args, run_id).await,
        Backend::Postgres => build_postgres_backend(args, run_id).await,
    }
}

#[cfg(feature = "libsql")]
async fn build_libsql_backend(args: &Args, run_id: &str) -> Result<BackendHandle, String> {
    use ironclaw_filesystem::LibSqlRootFilesystem;

    let path = args.libsql_path.clone().unwrap_or_else(default_libsql_path);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(display_err)?;
    }
    let db = Arc::new(
        libsql::Builder::new_local(&path)
            .build()
            .await
            .map_err(display_err)?,
    );
    let filesystem = Arc::new(LibSqlRootFilesystem::new(db));
    filesystem.run_migrations().await.map_err(display_err)?;
    Ok(BackendHandle {
        governor: governor_from_root(filesystem, run_id)?,
        target: redact_libsql_path(&path),
    })
}

#[cfg(not(feature = "libsql"))]
async fn build_libsql_backend(_args: &Args, _run_id: &str) -> Result<BackendHandle, String> {
    Err("binary was built without the libsql feature".to_string())
}

#[cfg(feature = "postgres")]
async fn build_postgres_backend(args: &Args, run_id: &str) -> Result<BackendHandle, String> {
    use ironclaw_filesystem::PostgresRootFilesystem;

    let url = resolve_postgres_url(args)?;
    let config = url
        .parse::<tokio_postgres::Config>()
        .map_err(|error| format!("parse Postgres URL: {error}"))?;
    let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    let pool = deadpool_postgres::Pool::builder(manager)
        .max_size(args.postgres_pool_size)
        .build()
        .map_err(display_err)?;
    let filesystem = Arc::new(PostgresRootFilesystem::new(pool));
    filesystem.run_migrations().await.map_err(display_err)?;
    Ok(BackendHandle {
        governor: governor_from_root(filesystem, run_id)?,
        target: redact_postgres_url(&url),
    })
}

#[cfg(not(feature = "postgres"))]
async fn build_postgres_backend(_args: &Args, _run_id: &str) -> Result<BackendHandle, String> {
    Err("binary was built without the postgres feature".to_string())
}

fn governor_from_root<F>(root: Arc<F>, run_id: &str) -> Result<Arc<dyn ResourceGovernor>, String>
where
    F: RootFilesystem + 'static,
{
    let view = resource_mount_view(run_id)?;
    let scoped = Arc::new(ScopedFilesystem::with_fixed_view(root, view));
    let store = FilesystemResourceGovernorStore::new(scoped);
    Ok(Arc::new(PersistentResourceGovernor::new(store)))
}

fn resource_mount_view(run_id: &str) -> Result<MountView, String> {
    MountView::new(vec![MountGrant::new(
        MountAlias::new("/resources").map_err(display_err)?,
        VirtualPath::new(format!("/resources/stress/{run_id}")).map_err(display_err)?,
        MountPermissions::read_write_list_delete(),
    )])
    .map_err(display_err)
}

fn default_libsql_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "ironclaw-storage-stress-{}.db",
        uuid::Uuid::new_v4().simple()
    ))
}

async fn cleanup_generated_libsql_path(path: &Path) {
    for candidate in [
        path.to_path_buf(),
        path.with_extension("db-wal"),
        path.with_extension("db-shm"),
    ] {
        match tokio::fs::remove_file(&candidate).await {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => eprintln!(
                "failed to remove generated libSQL file {}: {error}",
                candidate.display()
            ),
        }
    }
}

fn terminate_children(children: &mut Vec<(usize, Child)>) {
    for (_, child) in children.iter_mut() {
        let _ = child.kill();
    }
    for (_, mut child) in children.drain(..) {
        let _ = child.wait();
    }
}

fn join_workers(handles: Vec<(usize, JoinHandle<Result<(), String>>)>) -> Result<(), String> {
    for (worker_index, handle) in handles {
        match handle.join() {
            Ok(result) => result.map_err(|error| format!("worker {worker_index}: {error}"))?,
            Err(payload) => {
                return Err(format!(
                    "worker {worker_index} panicked: {}",
                    panic_payload_to_string(&payload)
                ));
            }
        }
    }
    Ok(())
}

fn panic_payload_to_string(payload: &Box<dyn Any + Send + 'static>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        return (*message).to_string();
    }
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    "non-string panic payload".to_string()
}

struct SyntheticIds {
    tenants: Vec<TenantId>,
    users: Vec<UserId>,
}

impl SyntheticIds {
    fn new(args: &Args) -> Result<Self, String> {
        let tenants = (0..args.tenants)
            .map(|tenant_index| {
                TenantId::new(format!("tenant-{tenant_index:04}"))
                    .map_err(|error| format!("build synthetic tenant id: {error}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let users = (0..args.users)
            .map(|user_index| {
                UserId::new(format!("user-{user_index:06}"))
                    .map_err(|error| format!("build synthetic user id: {error}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { tenants, users })
    }

    fn scope(&self, args: &Args, worker_index: usize, operation_index: usize) -> ResourceScope {
        let global_index = worker_index
            .saturating_mul(args.operations)
            .saturating_add(operation_index);
        let user_index = global_index % self.users.len();
        let tenant_index = user_index % self.tenants.len();
        ResourceScope {
            tenant_id: self.tenants[tenant_index].clone(),
            user_id: self.users[user_index].clone(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }
}

fn resolve_postgres_url(args: &Args) -> Result<String, String> {
    if let Some(url) = args.postgres_url.clone() {
        return Ok(url);
    }
    if let Some(url) = optional_env_var("IRONCLAW_FILESYSTEM_POSTGRES_URL")? {
        return Ok(url);
    }
    // silent-ok: IRONCLAW_FILESYSTEM_POSTGRES_URL is optional; DATABASE_URL is the documented fallback.
    if let Some(url) = optional_env_var("DATABASE_URL")? {
        return Ok(url);
    }
    Err(
        "Postgres requires --postgres-url, IRONCLAW_FILESYSTEM_POSTGRES_URL, or DATABASE_URL"
            .to_string(),
    )
}

fn optional_env_var(name: &str) -> Result<Option<String>, String> {
    match env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(VarError::NotPresent) => Ok(None),
        Err(VarError::NotUnicode(_)) => Err(format!("{name} is not valid Unicode")),
    }
}

fn redact_libsql_path(_path: &Path) -> String {
    "libsql://<redacted-local-path>".to_string()
}

fn redact_postgres_url(url: &str) -> String {
    if let Some(redacted) = redact_postgres_uri(url, "postgres://") {
        return redacted;
    }
    if let Some(redacted) = redact_postgres_uri(url, "postgresql://") {
        return redacted;
    }
    if let Some(redacted) = redact_postgres_key_value_config(url) {
        return redacted;
    }
    "postgres://<redacted>".to_string()
}

fn redact_postgres_uri(url: &str, scheme: &str) -> Option<String> {
    let rest = url.strip_prefix(scheme)?;
    let redacted_rest = match rest.find('@') {
        Some(at) => format!("<redacted>@{}", redact_uri_password_query(&rest[at + 1..])),
        None => redact_uri_password_query(rest),
    };
    Some(format!("{scheme}{redacted_rest}"))
}

fn redact_uri_password_query(rest: &str) -> String {
    let Some((prefix, query)) = rest.split_once('?') else {
        return rest.to_string();
    };
    let redacted_query = query
        .split('&')
        .map(|pair| {
            let key = pair.split_once('=').map(|(key, _)| key).unwrap_or(pair);
            if key.eq_ignore_ascii_case("password") {
                format!("{key}=<redacted>")
            } else {
                pair.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("&");
    format!("{prefix}?{redacted_query}")
}

fn redact_postgres_key_value_config(config: &str) -> Option<String> {
    let mut saw_assignment = false;
    let parts = config
        .split_whitespace()
        .map(|part| {
            let Some((key, _)) = part.split_once('=') else {
                return part.to_string();
            };
            saw_assignment = true;
            if key.eq_ignore_ascii_case("password") {
                format!("{key}=<redacted>")
            } else {
                part.to_string()
            }
        })
        .collect::<Vec<_>>();
    if saw_assignment {
        Some(parts.join(" "))
    } else {
        None
    }
}

fn display_err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_postgres_uri_credentials_but_keeps_host() {
        let redacted = redact_postgres_url("postgres://user:secret@localhost:5432/app");

        assert_eq!(redacted, "postgres://<redacted>@localhost:5432/app");
        assert!(!redacted.contains("secret"));
    }

    #[test]
    fn redacts_postgresql_uri_password_query_parameter() {
        let redacted =
            redact_postgres_url("postgresql://localhost/app?sslmode=require&password=secret");

        assert_eq!(
            redacted,
            "postgresql://localhost/app?sslmode=require&password=<redacted>"
        );
        assert!(!redacted.contains("secret"));
    }

    #[test]
    fn redacts_key_value_postgres_password() {
        let redacted =
            redact_postgres_url("host=localhost user=postgres password=secret dbname=app");

        assert_eq!(
            redacted,
            "host=localhost user=postgres password=<redacted> dbname=app"
        );
        assert!(!redacted.contains("secret"));
    }

    #[test]
    fn redacts_libsql_absolute_path() {
        let redacted = redact_libsql_path(Path::new("/tmp/ironclaw-storage-stress-secret.db"));

        assert_eq!(redacted, "libsql://<redacted-local-path>");
        assert!(!redacted.contains("/tmp"));
    }

    #[test]
    fn synthetic_ids_are_generated_once_for_requested_cardinality() {
        let args = test_args();
        let ids = SyntheticIds::new(&args).expect("synthetic ids build");

        assert_eq!(ids.tenants.len(), args.tenants);
        assert_eq!(ids.users.len(), args.users);
    }

    fn test_args() -> Args {
        Args {
            backend: Backend::Libsql,
            processes: 1,
            concurrency: 2,
            operations: 3,
            users: 4,
            tenants: 2,
            scenario: Scenario::ReserveRelease,
            run_id: None,
            libsql_path: None,
            postgres_url: None,
            postgres_pool_size: 4,
            child_index: None,
        }
    }
}
