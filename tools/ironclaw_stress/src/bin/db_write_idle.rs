use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use async_trait::async_trait;
use chrono::Utc;
use clap::{Parser, ValueEnum};
use ironclaw_filesystem::{RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    ids::{AgentId, InvocationId, ProcessId, ProjectId, TenantId, ThreadId, UserId},
    mount::{MountGrant, MountPermissions, MountView},
    path::{MountAlias, VirtualPath},
    resource::ResourceScope,
};
use ironclaw_processes::{
    ClaimProcessesRequest, ClaimedProcess, FailProcessRequest, GetProcessSnapshotRequest,
    JournalProcessExecutor, JournaledProcessSnapshot, ProcessExecutorFailure, ProcessJournalCommit,
    ProcessJournalCommitObserver, ProcessJournalCursor, ProcessJournalKind,
    ProcessJournalObserverRegistry, ProcessJournalSource, ProcessJournalStore,
    ProcessJournalStoreError, ProcessKind, ProcessLeaseRequest, ProcessLifecycleStatus,
    ProcessStateTransitionRequest, ProcessSubmissionPort, ProcessSupervisor,
    ProcessSupervisorConfig, ProcessSupervisorHandle, ProcessTransitionPort,
    RecoverExpiredProcessLeasesRequest, RecoverExpiredProcessLeasesResponse, SubmitProcessRequest,
    SuspendProcessRequest,
};
use ironclaw_stress::db_probe::{
    self, DbProbeConfig, DbProbeError, DbProbeSummary, DbWriteMeasurement, StatsScope,
    summarize_measurement,
};
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;
use tokio::sync::watch;

const WORKLOAD_NAME: &str = "long-lived-idle-process";
const OBSERVER_ID: &str = "ironclaw-stress-db-write-idle";
const POSTGRES_POOL_STATS_FLUSH_DELAY: Duration = Duration::from_millis(1_100);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
enum Backend {
    Libsql,
    Postgres,
}

impl Backend {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Libsql => "libsql",
            Self::Postgres => "postgres",
        }
    }
}

#[derive(Debug, Parser)]
#[command(about = "Measure durable writes from one long-lived idle process")]
struct Args {
    #[arg(long, value_enum, default_value_t = Backend::Libsql)]
    backend: Backend,

    #[arg(long)]
    libsql_path: Option<PathBuf>,

    /// Defaults to IRONCLAW_FILESYSTEM_POSTGRES_URL, then DATABASE_URL.
    #[arg(long)]
    postgres_url: Option<String>,

    #[arg(long, default_value_t = 4)]
    postgres_pool_size: usize,

    /// Time the claimed process must remain Running before counter capture.
    #[arg(long, default_value_t = 300)]
    idle_seconds: u64,

    #[arg(long, default_value_t = 5_000)]
    poll_interval_ms: u64,

    #[arg(long, default_value_t = 10_000)]
    recovery_interval_ms: u64,

    #[arg(long, default_value_t = 30_000)]
    heartbeat_interval_ms: u64,

    #[arg(long, default_value_t = 90_000)]
    lease_duration_ms: u64,

    #[arg(long, default_value_t = 30_000)]
    terminal_timeout_ms: u64,

    /// Reset current-database counters. Use only with an isolated database.
    #[arg(long)]
    db_write_reset_stats: bool,
}

#[derive(Debug, Clone, Copy)]
struct LifecycleConfig {
    idle: Duration,
    poll_interval: Duration,
    recovery_interval: Duration,
    heartbeat_interval: Duration,
    lease_duration: Duration,
    terminal_timeout: Duration,
}

impl LifecycleConfig {
    fn from_args(args: &Args) -> Result<Self, IdleError> {
        let config = Self {
            idle: Duration::from_secs(args.idle_seconds),
            poll_interval: Duration::from_millis(args.poll_interval_ms),
            recovery_interval: Duration::from_millis(args.recovery_interval_ms),
            heartbeat_interval: Duration::from_millis(args.heartbeat_interval_ms),
            lease_duration: Duration::from_millis(args.lease_duration_ms),
            terminal_timeout: Duration::from_millis(args.terminal_timeout_ms),
        };
        config.validate()?;
        Ok(config)
    }

    fn validate(self) -> Result<(), IdleError> {
        for (name, value) in [
            ("idle period", self.idle),
            ("poll interval", self.poll_interval),
            ("recovery interval", self.recovery_interval),
            ("heartbeat interval", self.heartbeat_interval),
            ("lease duration", self.lease_duration),
            ("terminal timeout", self.terminal_timeout),
        ] {
            if value.is_zero() {
                return Err(IdleError::Workload(format!("{name} must be nonzero")));
            }
        }
        if self.heartbeat_interval >= self.lease_duration {
            return Err(IdleError::Workload(
                "heartbeat interval must be shorter than the process lease".to_string(),
            ));
        }
        if self.idle < self.heartbeat_interval
            || self.idle < self.poll_interval
            || self.idle < self.recovery_interval
        {
            return Err(IdleError::Workload(
                "idle period must cover at least one heartbeat, claim poll, and recovery sweep"
                    .to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
enum IdleError {
    #[error(transparent)]
    Probe(#[from] DbProbeError),
    #[error("{0}")]
    Workload(String),
    #[error("{primary}; process cleanup also failed: {cleanup}")]
    ProcessCleanup {
        primary: Box<IdleError>,
        cleanup: Box<IdleError>,
    },
    #[error("{primary}; database instrumentation cleanup also failed: {cleanup}")]
    ProbeCleanup {
        primary: Box<IdleError>,
        cleanup: DbProbeError,
    },
}

#[derive(Debug, Serialize)]
struct IdleScenarioReport {
    workload: &'static str,
    backend: &'static str,
    lifecycle: LifecycleReport,
    write_families: IdleWriteFamilies,
    measurement: DbProbeSummary,
}

#[derive(Debug, Serialize)]
struct LifecycleReport {
    configured_idle_ms: u128,
    poll_interval_ms: u128,
    recovery_interval_ms: u128,
    heartbeat_interval_ms: u128,
    lease_duration_ms: u128,
    running_observed_after_ms: u128,
    capture_finished_after_ms: u128,
    terminal_observed_after_ms: u128,
    live_before_capture: bool,
    live_after_capture: bool,
    terminal_after_capture: bool,
    terminal_status: ProcessLifecycleStatus,
}

#[derive(Debug, Clone, Default, Serialize)]
struct IdleWriteFamilies {
    heartbeat_scheduler_calls: u64,
    /// Observer-visible heartbeat journal commits. Heartbeats update only the
    /// durable process row, so this remains zero after journal churn removal.
    heartbeat_writes: u64,
    claim_poll_calls: u64,
    claim_writes: u64,
    recovery_sweep_calls: u64,
    recovery_writes: u64,
    event_writes: Option<i128>,
    observer_checkpoint_writes: u64,
    process_store_writes: Option<i128>,
    measured_table_writes: BTreeMap<String, i128>,
}

#[derive(Default)]
struct SchedulerCounts {
    heartbeat_calls: AtomicU64,
    claim_calls: AtomicU64,
    recovery_calls: AtomicU64,
}

struct CountingRuntime<F>
where
    F: RootFilesystem,
{
    store: Arc<ProcessJournalStore<F>>,
    counts: Arc<SchedulerCounts>,
}

impl<F> CountingRuntime<F>
where
    F: RootFilesystem,
{
    fn new(store: Arc<ProcessJournalStore<F>>, counts: Arc<SchedulerCounts>) -> Self {
        Self { store, counts }
    }
}

#[async_trait]
impl<F> ProcessTransitionPort for CountingRuntime<F>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    type Error = ProcessJournalStoreError;

    async fn claim_next_processes(
        &self,
        request: ClaimProcessesRequest,
    ) -> Result<Vec<ClaimedProcess>, Self::Error> {
        self.counts.claim_calls.fetch_add(1, Ordering::SeqCst);
        self.store.claim_next_processes(request).await
    }

    async fn heartbeat_process(
        &self,
        request: ProcessLeaseRequest,
    ) -> Result<ProcessJournalCursor, Self::Error> {
        self.counts.heartbeat_calls.fetch_add(1, Ordering::SeqCst);
        self.store.heartbeat_process(request).await
    }

    async fn recover_expired_process_leases(
        &self,
        request: RecoverExpiredProcessLeasesRequest,
    ) -> Result<RecoverExpiredProcessLeasesResponse, Self::Error> {
        self.counts.recovery_calls.fetch_add(1, Ordering::SeqCst);
        self.store.recover_expired_process_leases(request).await
    }

    async fn suspend_process(
        &self,
        request: SuspendProcessRequest,
    ) -> Result<JournaledProcessSnapshot, Self::Error> {
        self.store.suspend_process(request).await
    }

    async fn complete_process(
        &self,
        request: ProcessStateTransitionRequest,
    ) -> Result<JournaledProcessSnapshot, Self::Error> {
        self.store.complete_process(request).await
    }

    async fn cancel_process(
        &self,
        request: ProcessStateTransitionRequest,
    ) -> Result<JournaledProcessSnapshot, Self::Error> {
        self.store.cancel_process(request).await
    }

    async fn fail_process(
        &self,
        request: FailProcessRequest,
    ) -> Result<JournaledProcessSnapshot, Self::Error> {
        self.store.fail_process(request).await
    }

    async fn relinquish_process(
        &self,
        request: ProcessLeaseRequest,
    ) -> Result<JournaledProcessSnapshot, Self::Error> {
        self.store.relinquish_process(request).await
    }
}

#[derive(Default)]
struct ObserverCounts {
    commits: AtomicU64,
    batches: AtomicU64,
    claims: AtomicU64,
    heartbeats: AtomicU64,
    recoveries: AtomicU64,
}

impl ObserverCounts {
    fn record(&self, kind: ProcessJournalKind) {
        self.commits.fetch_add(1, Ordering::SeqCst);
        match kind {
            ProcessJournalKind::Claimed => {
                self.claims.fetch_add(1, Ordering::SeqCst);
            }
            ProcessJournalKind::Heartbeat => {
                self.heartbeats.fetch_add(1, Ordering::SeqCst);
            }
            ProcessJournalKind::RecoveryRequired => {
                self.recoveries.fetch_add(1, Ordering::SeqCst);
            }
            _ => {}
        }
    }
}

struct CountingObserver {
    counts: Arc<ObserverCounts>,
}

#[async_trait]
impl ProcessJournalCommitObserver for CountingObserver {
    fn process_observer_id(&self) -> &'static str {
        OBSERVER_ID
    }

    async fn observe_process_commit(&self, commit: ProcessJournalCommit) -> Result<(), String> {
        self.counts.record(commit.kind);
        Ok(())
    }

    async fn observe_process_commits(
        &self,
        commits: Vec<ProcessJournalCommit>,
    ) -> Result<(), String> {
        self.counts.batches.fetch_add(1, Ordering::SeqCst);
        for commit in commits {
            self.counts.record(commit.kind);
        }
        Ok(())
    }
}

struct HeldCompletingExecutor {
    runtime: Arc<dyn ProcessTransitionPort<Error = ProcessJournalStoreError>>,
    release: watch::Receiver<bool>,
}

#[async_trait]
impl JournalProcessExecutor for HeldCompletingExecutor {
    async fn execute_claimed_process(
        &self,
        claimed: ClaimedProcess,
    ) -> Result<(), ProcessExecutorFailure> {
        let mut release = self.release.clone();
        while !*release.borrow() {
            release
                .changed()
                .await
                .map_err(|_| ProcessExecutorFailure::new("idle_release_closed"))?;
        }
        self.runtime
            .complete_process(ProcessStateTransitionRequest {
                lease: ProcessLeaseRequest {
                    process_id: claimed.state.process_id,
                    worker_id: claimed.worker_id,
                    lease_token: claimed.lease_token,
                },
                metadata: None,
            })
            .await
            .map_err(|_| ProcessExecutorFailure::new("idle_completion_failed"))?;
        Ok(())
    }
}

struct ActiveScenario<F>
where
    F: RootFilesystem,
{
    store: Arc<ProcessJournalStore<F>>,
    scope: ResourceScope,
    process_id: ProcessId,
    release: watch::Sender<bool>,
    supervisor: Option<ProcessSupervisorHandle>,
    scheduler_counts: Arc<SchedulerCounts>,
    observer_counts: Arc<ObserverCounts>,
    started: Instant,
}

struct CaptureOutcome {
    before: db_probe::DbProbeSnapshot,
    after: db_probe::DbProbeSnapshot,
    running_observed_after: Duration,
    capture_finished_after: Duration,
    write_families: IdleWriteFamilies,
    live_before_capture: ProcessLifecycleStatus,
    live_after_capture: ProcessLifecycleStatus,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let generated_path = if args.backend == Backend::Libsql && args.libsql_path.is_none() {
        Some(default_libsql_path())
    } else {
        None
    };
    let result = run(args, generated_path.clone()).await;
    if let Some(path) = generated_path {
        cleanup_libsql_files(&path).await;
    }
    match result {
        Ok(report) => match serde_json::to_string_pretty(&report) {
            Ok(json) => println!("{json}"),
            Err(error) => {
                eprintln!("serialize idle-process report: {error}");
                std::process::exit(1);
            }
        },
        Err(error) => {
            eprintln!("idle-process measurement failed: {error}");
            std::process::exit(1);
        }
    }
}

async fn run(args: Args, generated_path: Option<PathBuf>) -> Result<IdleScenarioReport, IdleError> {
    let lifecycle = LifecycleConfig::from_args(&args)?;
    match args.backend {
        Backend::Libsql => {
            let path = args
                .libsql_path
                .or(generated_path)
                .ok_or_else(|| IdleError::Workload("libSQL path was not selected".to_string()))?;
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await.map_err(|error| {
                    IdleError::Workload(format!("create libSQL directory: {error}"))
                })?;
            }
            let db = Arc::new(
                libsql::Builder::new_local(&path)
                    .build()
                    .await
                    .map_err(|error| {
                        IdleError::Workload(format!("open libSQL database: {error}"))
                    })?,
            );
            let root = Arc::new(ironclaw_filesystem::LibSqlRootFilesystem::new(db).map_err(
                |error| IdleError::Workload(format!("create libSQL filesystem: {error}")),
            )?);
            root.run_migrations().await.map_err(|error| {
                IdleError::Workload(format!("migrate libSQL filesystem: {error}"))
            })?;
            run_with_root(
                root,
                Backend::Libsql,
                DbProbeConfig::libsql(path, args.db_write_reset_stats),
                lifecycle,
            )
            .await
        }
        Backend::Postgres => {
            let url = resolve_postgres_url(&args)?;
            let config = url.parse::<tokio_postgres::Config>().map_err(|error| {
                IdleError::Workload(format!("parse Postgres connection settings: {error}"))
            })?;
            let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
            let pool = deadpool_postgres::Pool::builder(manager)
                .max_size(args.postgres_pool_size)
                .build()
                .map_err(|error| IdleError::Workload(format!("create Postgres pool: {error}")))?;
            let root = Arc::new(ironclaw_filesystem::PostgresRootFilesystem::new(pool));
            root.run_migrations().await.map_err(|error| {
                IdleError::Workload(format!("migrate Postgres filesystem: {error}"))
            })?;
            run_with_root(
                root,
                Backend::Postgres,
                DbProbeConfig::postgres(url, args.db_write_reset_stats),
                lifecycle,
            )
            .await
        }
    }
}

async fn run_with_root<F>(
    root: Arc<F>,
    backend: Backend,
    probe: DbProbeConfig,
    lifecycle: LifecycleConfig,
) -> Result<IdleScenarioReport, IdleError>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    let scope = idle_scope()?;
    let view = idle_mount_view(&scope)?;
    let filesystem = Arc::new(ScopedFilesystem::with_fixed_view(root, view));
    let store = Arc::new(
        ProcessJournalStore::new(filesystem).with_lease_duration(lifecycle.lease_duration),
    );
    let observer_counts = Arc::new(ObserverCounts::default());
    store
        .subscribe_process_observer(Arc::new(CountingObserver {
            counts: Arc::clone(&observer_counts),
        }))
        .map_err(|error| IdleError::Workload(format!("register process observer: {error}")))?;

    let before = db_probe::begin(&probe).await?;
    let scheduler_counts = Arc::new(SchedulerCounts::default());
    let runtime = Arc::new(CountingRuntime::new(
        Arc::clone(&store),
        Arc::clone(&scheduler_counts),
    ));
    let runtime_port: Arc<dyn ProcessTransitionPort<Error = ProcessJournalStoreError>> = runtime;
    let (release, release_rx) = watch::channel(false);
    let executor = Arc::new(HeldCompletingExecutor {
        runtime: Arc::clone(&runtime_port),
        release: release_rx,
    });
    let supervisor = ProcessSupervisor::new(
        Arc::clone(&runtime_port),
        executor,
        ProcessKind::CapabilityInvocation,
        ProcessSupervisorConfig::default()
            .with_max_concurrent_processes(1)
            .with_poll_interval(lifecycle.poll_interval)
            .with_lease_recovery_interval(lifecycle.recovery_interval)
            .with_heartbeat_interval(lifecycle.heartbeat_interval),
    )
    .start();
    let process_id = ProcessId::new();
    let mut active = ActiveScenario {
        store,
        scope,
        process_id,
        release,
        supervisor: Some(supervisor),
        scheduler_counts,
        observer_counts,
        started: Instant::now(),
    };

    let capture_result = capture_running_process(&mut active, before, &probe, lifecycle).await;
    let process_cleanup = finish_process(&mut active, lifecycle.terminal_timeout).await;
    let probe_cleanup = db_probe::finish(&probe).await;

    let capture = match combine_results(capture_result, process_cleanup, probe_cleanup) {
        Ok(capture) => capture,
        Err(error) => return Err(error),
    };
    let terminal = active
        .store
        .get_process_snapshot(GetProcessSnapshotRequest {
            scope: active.scope.clone(),
            process_id: active.process_id,
        })
        .await
        .map_err(|error| IdleError::Workload(format!("load terminal process: {error}")))?;
    let terminal_observed_after = active.started.elapsed();
    if terminal.status != ProcessLifecycleStatus::Completed {
        return Err(IdleError::Workload(format!(
            "idle process ended with unexpected status {:?}",
            terminal.status
        )));
    }
    let CaptureOutcome {
        before,
        after,
        running_observed_after,
        capture_finished_after,
        live_before_capture,
        live_after_capture,
        mut write_families,
    } = capture;

    let measurement = summarize_measurement(
        before,
        after,
        None,
        DbWriteMeasurement {
            workload: WORKLOAD_NAME.to_string(),
            tool_calls_per_turn: 0,
            idle_observation_seconds: lifecycle.idle.as_secs(),
            reset_stats: probe.reset_stats(),
            stats_scope: if probe.reset_stats() {
                StatsScope::ExplicitResetCurrentDatabase
            } else {
                StatsScope::SnapshotDeltaCurrentDatabase
            },
        },
    );
    add_database_write_families(&measurement, backend, &mut write_families);
    if write_families.process_store_writes.unwrap_or_default() <= 0 {
        return Err(IdleError::Workload(
            "idle process measurement recorded no process-store writes".to_string(),
        ));
    }

    Ok(IdleScenarioReport {
        workload: WORKLOAD_NAME,
        backend: backend.as_str(),
        lifecycle: LifecycleReport {
            configured_idle_ms: lifecycle.idle.as_millis(),
            poll_interval_ms: lifecycle.poll_interval.as_millis(),
            recovery_interval_ms: lifecycle.recovery_interval.as_millis(),
            heartbeat_interval_ms: lifecycle.heartbeat_interval.as_millis(),
            lease_duration_ms: lifecycle.lease_duration.as_millis(),
            running_observed_after_ms: running_observed_after.as_millis(),
            capture_finished_after_ms: capture_finished_after.as_millis(),
            terminal_observed_after_ms: terminal_observed_after.as_millis(),
            live_before_capture: live_before_capture == ProcessLifecycleStatus::Running,
            live_after_capture: live_after_capture == ProcessLifecycleStatus::Running,
            terminal_after_capture: terminal_observed_after > capture_finished_after,
            terminal_status: terminal.status,
        },
        write_families,
        measurement,
    })
}

async fn capture_running_process<F>(
    active: &mut ActiveScenario<F>,
    before: db_probe::DbProbeSnapshot,
    probe: &DbProbeConfig,
    lifecycle: LifecycleConfig,
) -> Result<CaptureOutcome, IdleError>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    active
        .store
        .submit_process(SubmitProcessRequest {
            process_id: active.process_id,
            process_kind: ProcessKind::CapabilityInvocation,
            scope: active.scope.clone(),
            exclusive_within_scope: false,
            operation_id: None,
            owner_user_id: Some(active.scope.user_id.clone()),
            concurrency_class: None,
            parent_process_id: None,
            root_process_id: None,
            spawn_tree_descendant_cap: None,
            dependency: None,
            checkpoint_ref: None,
            input: None,
            created_at: Utc::now(),
            metadata: Value::Null,
        })
        .await
        .map_err(|error| IdleError::Workload(format!("submit idle process: {error}")))?;
    active
        .supervisor
        .as_ref()
        .ok_or_else(|| IdleError::Workload("process supervisor is not running".to_string()))?
        .wake_notifier()
        .notify_scope(active.scope.clone())
        .map_err(|error| IdleError::Workload(format!("wake process supervisor: {error}")))?;

    wait_for_status(
        &active.store,
        &active.scope,
        active.process_id,
        ProcessLifecycleStatus::Running,
        lifecycle.terminal_timeout,
    )
    .await?;
    let running_observed_after = active.started.elapsed();
    tokio::time::sleep(lifecycle.idle).await;
    wait_for_scheduler_activity(active, lifecycle).await?;
    if matches!(probe.target(), db_probe::DbProbeTarget::Postgres { .. }) {
        tokio::time::sleep(POSTGRES_POOL_STATS_FLUSH_DELAY).await;
        let _ = require_running(active, "after PostgreSQL stats flush delay").await?;
    }
    let live_before_capture = require_running(active, "before counter capture").await?;
    let after = db_probe::capture_settled(probe).await?;
    let live_after_capture = require_running(active, "after counter capture").await?;
    let capture_finished_after = active.started.elapsed();

    let write_families = IdleWriteFamilies {
        heartbeat_scheduler_calls: active
            .scheduler_counts
            .heartbeat_calls
            .load(Ordering::SeqCst),
        heartbeat_writes: active.observer_counts.heartbeats.load(Ordering::SeqCst),
        claim_poll_calls: active.scheduler_counts.claim_calls.load(Ordering::SeqCst),
        claim_writes: active.observer_counts.claims.load(Ordering::SeqCst),
        recovery_sweep_calls: active
            .scheduler_counts
            .recovery_calls
            .load(Ordering::SeqCst),
        recovery_writes: active.observer_counts.recoveries.load(Ordering::SeqCst),
        observer_checkpoint_writes: active.observer_counts.batches.load(Ordering::SeqCst),
        ..IdleWriteFamilies::default()
    };
    if write_families.heartbeat_scheduler_calls == 0
        || write_families.claim_writes == 0
        || write_families.claim_poll_calls == 0
        || write_families.recovery_sweep_calls == 0
    {
        return Err(IdleError::Workload(
            "idle window did not exercise heartbeat, claim polling, and recovery scheduling"
                .to_string(),
        ));
    }

    Ok(CaptureOutcome {
        before,
        after,
        running_observed_after,
        capture_finished_after,
        live_before_capture,
        live_after_capture,
        write_families,
    })
}

async fn wait_for_scheduler_activity<F>(
    active: &ActiveScenario<F>,
    lifecycle: LifecycleConfig,
) -> Result<(), IdleError>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    tokio::time::timeout(lifecycle.terminal_timeout, async {
        loop {
            if active.scheduler_counts.claim_calls.load(Ordering::SeqCst) >= 1
                && active
                    .scheduler_counts
                    .recovery_calls
                    .load(Ordering::SeqCst)
                    >= 1
                && active.scheduler_counts.heartbeat_calls.load(Ordering::SeqCst) >= 1
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .map_err(|_| {
        IdleError::Workload(format!(
            "timed out waiting for idle scheduler activity: claim_calls={}, recovery_calls={}, heartbeat_calls={}",
            active.scheduler_counts.claim_calls.load(Ordering::SeqCst),
            active.scheduler_counts.recovery_calls.load(Ordering::SeqCst),
            active.scheduler_counts.heartbeat_calls.load(Ordering::SeqCst),
        ))
    })
}

async fn require_running<F>(
    active: &ActiveScenario<F>,
    phase: &'static str,
) -> Result<ProcessLifecycleStatus, IdleError>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    let snapshot = active
        .store
        .get_process_snapshot(GetProcessSnapshotRequest {
            scope: active.scope.clone(),
            process_id: active.process_id,
        })
        .await
        .map_err(|error| IdleError::Workload(format!("load idle process {phase}: {error}")))?;
    if snapshot.status != ProcessLifecycleStatus::Running {
        return Err(IdleError::Workload(format!(
            "idle process was {:?} {phase}, expected Running; failure={:?}",
            snapshot.status, snapshot.failure
        )));
    }
    Ok(snapshot.status)
}

async fn finish_process<F>(
    active: &mut ActiveScenario<F>,
    timeout: Duration,
) -> Result<(), IdleError>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    let release_result = active
        .release
        .send(true)
        .map_err(|_| IdleError::Workload("release idle process executor".to_string()));
    let terminal_result = if release_result.is_ok() {
        wait_for_terminal(&active.store, &active.scope, active.process_id, timeout)
            .await
            .map(|_| ())
    } else {
        release_result
    };
    if let Some(supervisor) = active.supervisor.take() {
        supervisor.shutdown().await;
    }
    terminal_result
}

fn combine_results<T>(
    primary: Result<T, IdleError>,
    process_cleanup: Result<(), IdleError>,
    probe_cleanup: Result<(), DbProbeError>,
) -> Result<T, IdleError> {
    let with_process = match (primary, process_cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(primary), Ok(())) => Err(primary),
        (Ok(_), Err(cleanup)) => Err(cleanup),
        (Err(primary), Err(cleanup)) => Err(IdleError::ProcessCleanup {
            primary: Box::new(primary),
            cleanup: Box::new(cleanup),
        }),
    };
    match (with_process, probe_cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(primary), Ok(())) => Err(primary),
        (Ok(_), Err(cleanup)) => Err(IdleError::Probe(cleanup)),
        (Err(primary), Err(cleanup)) => Err(IdleError::ProbeCleanup {
            primary: Box::new(primary),
            cleanup,
        }),
    }
}

async fn wait_for_status<F>(
    store: &ProcessJournalStore<F>,
    scope: &ResourceScope,
    process_id: ProcessId,
    expected: ProcessLifecycleStatus,
    timeout: Duration,
) -> Result<JournaledProcessSnapshot, IdleError>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    tokio::time::timeout(timeout, async {
        loop {
            let snapshot = store
                .get_process_snapshot(GetProcessSnapshotRequest {
                    scope: scope.clone(),
                    process_id,
                })
                .await
                .map_err(|error| IdleError::Workload(format!("load process status: {error}")))?;
            if snapshot.status == expected {
                return Ok(snapshot);
            }
            if snapshot.status.is_terminal() {
                return Err(IdleError::Workload(format!(
                    "process became {:?} while waiting for {:?}",
                    snapshot.status, expected
                )));
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .map_err(|_| IdleError::Workload(format!("timed out waiting for {expected:?}")))?
}

async fn wait_for_terminal<F>(
    store: &ProcessJournalStore<F>,
    scope: &ResourceScope,
    process_id: ProcessId,
    timeout: Duration,
) -> Result<JournaledProcessSnapshot, IdleError>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    tokio::time::timeout(timeout, async {
        loop {
            let snapshot = store
                .get_process_snapshot(GetProcessSnapshotRequest {
                    scope: scope.clone(),
                    process_id,
                })
                .await
                .map_err(|error| IdleError::Workload(format!("load terminal status: {error}")))?;
            if snapshot.status.is_terminal() {
                return Ok(snapshot);
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .map_err(|_| IdleError::Workload("timed out completing idle process".to_string()))?
}

fn add_database_write_families(
    measurement: &DbProbeSummary,
    backend: Backend,
    families: &mut IdleWriteFamilies,
) {
    let tables = match backend {
        Backend::Libsql => measurement
            .delta
            .libsql_table_writes
            .iter()
            .map(|row| (row.table.clone(), row.inserts + row.updates + row.deletes))
            .collect::<BTreeMap<_, _>>(),
        Backend::Postgres => {
            db_probe::postgres_relation_write_totals(&measurement.delta.postgres_table_writes)
        }
    };
    families.event_writes = tables.get("root_filesystem_events").copied();
    families.process_store_writes = tables.get("root_filesystem_entries").copied();
    families.measured_table_writes = tables;
}

fn idle_scope() -> Result<ResourceScope, IdleError> {
    Ok(ResourceScope {
        tenant_id: TenantId::new("stress-idle-tenant")
            .map_err(|error| IdleError::Workload(format!("create idle tenant id: {error}")))?,
        user_id: UserId::new("stress-idle-user")
            .map_err(|error| IdleError::Workload(format!("create idle user id: {error}")))?,
        agent_id: Some(
            AgentId::new("stress-idle-agent")
                .map_err(|error| IdleError::Workload(format!("create idle agent id: {error}")))?,
        ),
        project_id: Some(
            ProjectId::new("stress-idle-project")
                .map_err(|error| IdleError::Workload(format!("create idle project id: {error}")))?,
        ),
        mission_id: None,
        thread_id: Some(
            ThreadId::new("stress-idle-thread")
                .map_err(|error| IdleError::Workload(format!("create idle thread id: {error}")))?,
        ),
        invocation_id: InvocationId::new(),
    })
}

fn idle_mount_view(scope: &ResourceScope) -> Result<MountView, IdleError> {
    let path = format!(
        "/engine/ironclaw-stress/idle/{}/{}/{}",
        scope.tenant_id.as_str(),
        scope.user_id.as_str(),
        scope.invocation_id
    );
    MountView::new(vec![MountGrant::new(
        MountAlias::new("/processes")
            .map_err(|error| IdleError::Workload(format!("create process mount alias: {error}")))?,
        VirtualPath::new(path)
            .map_err(|error| IdleError::Workload(format!("create process mount path: {error}")))?,
        MountPermissions::read_write_list_delete(),
    )])
    .map_err(|error| IdleError::Workload(format!("create process mount view: {error}")))
}

fn resolve_postgres_url(args: &Args) -> Result<String, IdleError> {
    args.postgres_url
        .clone()
        .or_else(|| std::env::var("IRONCLAW_FILESYSTEM_POSTGRES_URL").ok())
        .or_else(|| std::env::var("DATABASE_URL").ok())
        .ok_or_else(|| {
            IdleError::Workload(
                "Postgres requires --postgres-url, IRONCLAW_FILESYSTEM_POSTGRES_URL, or DATABASE_URL"
                    .to_string(),
            )
        })
}

fn default_libsql_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "ironclaw-stress-idle-{}.db",
        uuid::Uuid::new_v4().simple()
    ))
}

async fn cleanup_libsql_files(path: &Path) {
    for candidate in [
        path.to_path_buf(),
        PathBuf::from(format!("{}-wal", path.display())),
        PathBuf::from(format!("{}-shm", path.display())),
    ] {
        if let Err(error) = tokio::fs::remove_file(&candidate).await
            && error.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!("clean generated libSQL database file: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn short_lifecycle() -> LifecycleConfig {
        LifecycleConfig {
            idle: Duration::from_millis(150),
            poll_interval: Duration::from_millis(20),
            recovery_interval: Duration::from_millis(50),
            heartbeat_interval: Duration::from_millis(100),
            lease_duration: Duration::from_secs(5),
            terminal_timeout: Duration::from_secs(10),
        }
    }
    fn valid_args() -> Args {
        Args {
            backend: Backend::Libsql,
            libsql_path: None,
            postgres_url: None,
            postgres_pool_size: 1,
            idle_seconds: 1,
            poll_interval_ms: 20,
            recovery_interval_ms: 50,
            heartbeat_interval_ms: 100,
            lease_duration_ms: 5_000,
            terminal_timeout_ms: 10_000,
            db_write_reset_stats: false,
        }
    }

    #[test]
    fn lifecycle_config_maps_cli_arguments() {
        let config = LifecycleConfig::from_args(&valid_args()).expect("valid lifecycle");

        assert_eq!(config.idle, Duration::from_secs(1));
        assert_eq!(config.poll_interval, Duration::from_millis(20));
        assert_eq!(config.recovery_interval, Duration::from_millis(50));
        assert_eq!(config.heartbeat_interval, Duration::from_millis(100));
        assert_eq!(config.lease_duration, Duration::from_millis(5_000));
        assert_eq!(config.terminal_timeout, Duration::from_millis(10_000));
    }

    #[test]
    fn lifecycle_config_rejects_each_zero_duration() {
        let setters: [fn(&mut Args); 6] = [
            |args| args.idle_seconds = 0,
            |args| args.poll_interval_ms = 0,
            |args| args.recovery_interval_ms = 0,
            |args| args.heartbeat_interval_ms = 0,
            |args| args.lease_duration_ms = 0,
            |args| args.terminal_timeout_ms = 0,
        ];

        for set_zero in setters {
            let mut args = valid_args();
            set_zero(&mut args);
            let error =
                LifecycleConfig::from_args(&args).expect_err("each zero duration must be rejected");
            assert!(error.to_string().contains("must be nonzero"));
        }
    }

    #[test]
    fn lifecycle_config_rejects_heartbeat_at_or_above_lease() {
        for heartbeat_interval_ms in [5_000, 5_001] {
            let mut args = valid_args();
            args.heartbeat_interval_ms = heartbeat_interval_ms;
            let error = LifecycleConfig::from_args(&args)
                .expect_err("heartbeat at or above lease must be rejected");
            assert!(
                error
                    .to_string()
                    .contains("heartbeat interval must be shorter")
            );
        }
    }

    #[test]
    fn lifecycle_config_rejects_idle_shorter_than_each_interval() {
        let setters: [fn(&mut Args); 3] = [
            |args| args.poll_interval_ms = 1_001,
            |args| args.recovery_interval_ms = 1_001,
            |args| args.heartbeat_interval_ms = 1_001,
        ];

        for exceed_idle in setters {
            let mut args = valid_args();
            exceed_idle(&mut args);
            let error = LifecycleConfig::from_args(&args)
                .expect_err("idle shorter than a scheduler interval must be rejected");
            assert!(error.to_string().contains("idle period must cover"));
        }
    }

    #[tokio::test]
    async fn short_libsql_idle_process_stays_live_and_writes_process_rows() {
        let path = default_libsql_path();
        let args = Args {
            backend: Backend::Libsql,
            libsql_path: Some(path.clone()),
            postgres_url: None,
            postgres_pool_size: 1,
            idle_seconds: 1,
            poll_interval_ms: 5,
            recovery_interval_ms: 7,
            heartbeat_interval_ms: 10,
            lease_duration_ms: 100,
            terminal_timeout_ms: 5_000,
            db_write_reset_stats: true,
        };
        let db = Arc::new(libsql::Builder::new_local(&path).build().await.unwrap());
        let root = Arc::new(ironclaw_filesystem::LibSqlRootFilesystem::new(db).unwrap());
        root.run_migrations().await.unwrap();
        let report = run_with_root(
            root,
            Backend::Libsql,
            DbProbeConfig::libsql(&path, args.db_write_reset_stats),
            short_lifecycle(),
        )
        .await
        .unwrap();

        assert!(report.lifecycle.live_before_capture);
        assert!(report.lifecycle.live_after_capture);
        assert!(report.lifecycle.terminal_after_capture);
        assert_eq!(
            report.lifecycle.terminal_status,
            ProcessLifecycleStatus::Completed
        );
        assert!(report.write_families.heartbeat_scheduler_calls > 0);
        assert_eq!(report.write_families.heartbeat_writes, 0);
        assert!(report.write_families.claim_poll_calls >= 1);
        assert!(report.write_families.recovery_sweep_calls > 0);
        assert_eq!(report.write_families.recovery_writes, 0);
        assert!(report.write_families.observer_checkpoint_writes > 0);
        assert_eq!(report.write_families.event_writes, Some(0));
        assert!(
            report
                .write_families
                .process_store_writes
                .unwrap_or_default()
                > 0
        );
        cleanup_libsql_files(&path).await;
    }

    #[tokio::test]
    #[ignore = "requires an isolated Postgres database"]
    async fn short_postgres_idle_process_stays_live_and_writes_process_rows() {
        let url = std::env::var("IRONCLAW_FILESYSTEM_POSTGRES_URL")
            .expect("IRONCLAW_FILESYSTEM_POSTGRES_URL");
        let config = url.parse::<tokio_postgres::Config>().unwrap();
        let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
        let pool = deadpool_postgres::Pool::builder(manager)
            .max_size(1)
            .build()
            .unwrap();
        let root = Arc::new(ironclaw_filesystem::PostgresRootFilesystem::new(pool));
        root.run_migrations().await.unwrap();
        let report = run_with_root(
            root,
            Backend::Postgres,
            DbProbeConfig::postgres(url, true),
            short_lifecycle(),
        )
        .await
        .unwrap();

        assert!(report.lifecycle.live_before_capture);
        assert!(report.lifecycle.live_after_capture);
        assert!(report.lifecycle.terminal_after_capture);
        assert!(report.write_families.heartbeat_scheduler_calls > 0);
        assert_eq!(report.write_families.heartbeat_writes, 0);
        assert_eq!(report.write_families.recovery_writes, 0);
        assert!(report.write_families.claim_poll_calls >= 1);
        assert!(report.write_families.recovery_sweep_calls > 0);
        assert!(report.write_families.observer_checkpoint_writes > 0);
        assert_eq!(report.write_families.event_writes, Some(0));
        assert!(
            report
                .write_families
                .process_store_writes
                .unwrap_or_default()
                > 0
        );
    }
}
