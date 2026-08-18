use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use ironclaw_filesystem::{FilesystemError, RootFilesystem, ScopedFilesystem, SeqNo};
use ironclaw_host_api::{
    ids::ResourceReservationId,
    path::ScopedPath,
    resource::{ReservationStatus, ResourceScope},
};
use serde::{Deserialize, Serialize};
use tokio::runtime::{Handle, RuntimeFlavor};
use tracing::warn;

use crate::{
    ResourceAccount, ResourceError, ResourceEstimate, ResourceGovernorStore,
    ResourceGovernorStorePort, ResourceLimits, ResourceState, ResourceUsage,
    account_snapshot_in_state, reconcile_in_state, release_in_state, reserve_with_outcome_in_state,
    set_limit_in_state,
};

use super::{fs_error, storage_error};

const DELTA_LOG_PATH: &str = "/resources/deltas/log";
const DELTA_JOURNAL_MAX_BATCH: usize = 256;
// The window must cover at least one FULL backend append attempt, or a single
// transient `BackendBusy` exhausts it with zero retries. A local libSQL
// backend attempt can block for the writer-pool checkout timeout (10s) plus
// the connection's `busy_timeout` (5s) before returning `BackendBusy` — 15s
// worst case, already triple the legacy 5s window. 60s keeps the journal
// retrying through a multi-attempt burst (run-start write storms on slow
// runners have measured 20-30s+ of continuous writer contention in Reborn
// E2E); a persistent hold beyond the window still fails closed (no budget
// guarantee is weakened).
const DEFAULT_BUSY_RETRY_POLICY: BusyRetryPolicy = BusyRetryPolicy {
    max_retries: 3,
    max_elapsed: Duration::from_secs(60),
    backoff_base: Duration::from_millis(25),
    backoff_max: Duration::from_millis(250),
    jitter: true,
};

#[derive(Clone, Copy)]
struct BusyRetryPolicy {
    max_retries: usize,
    /// Bounds how long the journal will continue starting additional database
    /// attempts. An individual backend operation remains bounded by the
    /// backend's own worst case — for local libSQL that is the writer-pool
    /// checkout timeout (10s) plus the connection's `busy_timeout` (5s), so
    /// the window must exceed that single-attempt bound or the first
    /// `BackendBusy` consumes the whole window with no retry left.
    max_elapsed: Duration,
    backoff_base: Duration,
    backoff_max: Duration,
    jitter: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum ResourceGovernorDelta {
    SetLimit {
        account: ResourceAccount,
        limits: ResourceLimits,
        at: DateTime<Utc>,
    },
    Reserve {
        scope: ResourceScope,
        estimate: ResourceEstimate,
        reservation_id: ResourceReservationId,
        at: DateTime<Utc>,
    },
    Reconcile {
        reservation_id: ResourceReservationId,
        actual: ResourceUsage,
        at: DateTime<Utc>,
    },
    Release {
        reservation_id: ResourceReservationId,
        at: DateTime<Utc>,
    },
    AccountSnapshot {
        account: ResourceAccount,
        at: DateTime<Utc>,
    },
}

impl ResourceGovernorDelta {
    fn apply_to(self, state: &mut ResourceState) -> Result<(), ResourceError> {
        match self {
            Self::SetLimit {
                account,
                limits,
                at,
            } => {
                set_limit_in_state(state, account, limits, at);
                Ok(())
            }
            Self::Reserve {
                scope,
                estimate,
                reservation_id,
                at,
            } => reserve_with_outcome_in_state(state, scope, estimate, reservation_id, at)
                .map(|_| ()),
            Self::Reconcile {
                reservation_id,
                actual,
                at,
            } => reconcile_in_state(state, reservation_id, actual, at).map(|_| ()),
            Self::Release { reservation_id, at } => {
                release_in_state(state, reservation_id, at).map(|_| ())
            }
            Self::AccountSnapshot { account, at } => {
                let _ = account_snapshot_in_state(state, &account, at);
                Ok(())
            }
        }
    }
}

pub(super) struct ResourceDeltaJournal<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    sender: Mutex<mpsc::Sender<DeltaJournalRequest>>,
    retry_policy: BusyRetryPolicy,
    #[cfg(test)]
    restart_hook: Option<JournalRestartHook>,
}

#[cfg(test)]
pub(super) type JournalRestartHook = Arc<dyn Fn() + Send + Sync>;

pub(super) struct PendingResourceDelta {
    ack: mpsc::Receiver<Result<SeqNo, ResourceError>>,
}

struct DeltaJournalRequest {
    delta: ResourceGovernorDelta,
    ack: mpsc::Sender<Result<SeqNo, ResourceError>>,
}

impl<F> ResourceDeltaJournal<F>
where
    F: RootFilesystem + 'static,
{
    pub(super) fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self::with_retry_policy(filesystem, DEFAULT_BUSY_RETRY_POLICY)
    }

    fn with_retry_policy(
        filesystem: Arc<ScopedFilesystem<F>>,
        retry_policy: BusyRetryPolicy,
    ) -> Self {
        let sender = match spawn_delta_journal_flusher(Arc::clone(&filesystem), retry_policy) {
            Ok(sender) => sender,
            Err(_) => {
                warn!(
                    error_kind = "thread_spawn",
                    "resource governor delta journal thread failed to start"
                );
                let (sender, receiver) = mpsc::channel();
                drop(receiver);
                sender
            }
        };
        Self {
            filesystem,
            sender: Mutex::new(sender),
            retry_policy,
            #[cfg(test)]
            restart_hook: None,
        }
    }

    #[cfg(test)]
    fn new_with_retry_policy(
        filesystem: Arc<ScopedFilesystem<F>>,
        retry_policy: BusyRetryPolicy,
    ) -> Self {
        Self::with_retry_policy(filesystem, retry_policy)
    }

    pub(super) fn restart(&self) -> Result<(), ResourceError> {
        #[cfg(test)]
        if let Some(hook) = &self.restart_hook {
            hook();
        }
        let replacement =
            spawn_delta_journal_flusher(Arc::clone(&self.filesystem), self.retry_policy)?;
        *self
            .sender
            .lock()
            .map_err(|_| storage_error("resource governor delta journal sender lock poisoned"))? =
            replacement;
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn with_restart_hook(mut self, hook: JournalRestartHook) -> Self {
        self.restart_hook = Some(hook);
        self
    }

    pub(super) fn enqueue(
        &self,
        delta: ResourceGovernorDelta,
    ) -> Result<PendingResourceDelta, ResourceError> {
        let (ack, receiver) = mpsc::channel();
        self.sender
            .lock()
            .map_err(|_| storage_error("resource governor delta journal sender lock poisoned"))?
            .send(DeltaJournalRequest { delta, ack })
            .map_err(|_| storage_error("resource governor delta journal stopped"))?;
        Ok(PendingResourceDelta { ack: receiver })
    }

    #[cfg(test)]
    pub(super) fn poison_sender_lock(&self) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = self.sender.lock().expect("sender lock"); // safety: test-only helper deliberately poisons this lock under #[cfg(test)].
            panic!("poison journal sender lock for restart failure coverage");
        }));
    }
}

fn spawn_delta_journal_flusher<F>(
    filesystem: Arc<ScopedFilesystem<F>>,
    retry_policy: BusyRetryPolicy,
) -> Result<mpsc::Sender<DeltaJournalRequest>, ResourceError>
where
    F: RootFilesystem + 'static,
{
    let (sender, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name("resource-governor-delta-journal".to_string())
        .spawn(move || run_delta_journal_flusher(filesystem, receiver, retry_policy))
        .map_err(|error| {
            storage_error(format!("resource governor delta journal thread: {error}"))
        })?;
    Ok(sender)
}

impl PendingResourceDelta {
    pub(super) fn wait(self) -> Result<SeqNo, ResourceError> {
        if let Ok(handle) = Handle::try_current()
            && handle.runtime_flavor() == RuntimeFlavor::MultiThread
        {
            return tokio::task::block_in_place(|| self.recv_ack());
        }
        self.recv_ack()
    }

    fn recv_ack(self) -> Result<SeqNo, ResourceError> {
        self.ack
            .recv()
            .map_err(|_| storage_error("resource governor delta journal stopped"))?
    }
}

fn run_delta_journal_flusher<F>(
    filesystem: Arc<ScopedFilesystem<F>>,
    receiver: mpsc::Receiver<DeltaJournalRequest>,
    retry_policy: BusyRetryPolicy,
) where
    F: RootFilesystem + 'static,
{
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            while let Ok(request) = receiver.recv() {
                let _ = request.ack.send(Err(storage_error(format!(
                    "resource governor delta journal runtime failed: {error}"
                ))));
            }
            return;
        }
    };
    while let Ok(first) = receiver.recv() {
        let mut requests = Vec::with_capacity(DELTA_JOURNAL_MAX_BATCH);
        requests.push(first);
        std::thread::yield_now();
        while requests.len() < DELTA_JOURNAL_MAX_BATCH {
            match receiver.try_recv() {
                Ok(request) => requests.push(request),
                Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => break,
            }
        }
        let result = runtime.block_on(persist_delta_journal_batch(
            filesystem.as_ref(),
            &requests,
            retry_policy,
        ));
        match result {
            Ok(seqs) => {
                for (request, seq) in requests.into_iter().zip(seqs) {
                    let _ = request.ack.send(Ok(seq));
                }
            }
            Err(error) => {
                for request in requests {
                    let _ = request.ack.send(Err(error.clone()));
                }
                break;
            }
        }
    }
}

async fn persist_delta_journal_batch<F>(
    filesystem: &ScopedFilesystem<F>,
    requests: &[DeltaJournalRequest],
    retry_policy: BusyRetryPolicy,
) -> Result<Vec<SeqNo>, ResourceError>
where
    F: RootFilesystem,
{
    let path = delta_log_path()?;
    let payloads = requests
        .iter()
        .map(|request| serde_json::to_vec(&request.delta).map_err(storage_error))
        .collect::<Result<Vec<_>, _>>()?;
    let started = Instant::now();
    let mut retry = 0;
    let seqs = loop {
        let scope = ResourceScope::system();
        let append_result = match payloads.as_slice() {
            [payload] => filesystem
                .append(&scope, &path, payload.clone())
                .await
                .map(|seq| vec![seq]),
            _ => {
                filesystem
                    .append_batch(&scope, &path, payloads.clone())
                    .await
            }
        };
        match append_result {
            Ok(seqs) => break seqs,
            Err(error @ FilesystemError::BackendBusy { .. }) => {
                let elapsed = started.elapsed();
                if retry >= retry_policy.max_retries || elapsed >= retry_policy.max_elapsed {
                    warn!(
                        attempts = retry + 1,
                        max_retries = retry_policy.max_retries,
                        elapsed_ms = elapsed.as_millis(),
                        retry_window_ms = retry_policy.max_elapsed.as_millis(),
                        batch_size = requests.len(),
                        "resource governor delta journal filesystem contention exhausted"
                    );
                    return Err(fs_error(error));
                }
                retry += 1;
                let delay = busy_retry_delay(retry, retry_policy)
                    .min(retry_policy.max_elapsed.saturating_sub(elapsed));
                warn!(
                    retry,
                    max_retries = retry_policy.max_retries,
                    delay_ms = delay.as_millis(),
                    elapsed_ms = elapsed.as_millis(),
                    retry_window_ms = retry_policy.max_elapsed.as_millis(),
                    batch_size = requests.len(),
                    "resource governor delta journal waiting for filesystem writer"
                );
                tokio::time::sleep(delay).await;
            }
            Err(error) => return Err(fs_error(error)),
        }
    };
    if seqs.len() != requests.len() {
        return Err(storage_error(
            "resource governor delta batch append returned an unexpected ack count",
        ));
    }
    Ok(seqs)
}

fn busy_retry_delay(attempt: usize, policy: BusyRetryPolicy) -> Duration {
    let exponent = u32::try_from(attempt.saturating_sub(1)).unwrap_or(u32::MAX);
    let multiplier = 2u32.saturating_pow(exponent);
    let base = policy
        .backoff_base
        .saturating_mul(multiplier)
        .min(policy.backoff_max);
    if !policy.jitter {
        return base;
    }
    let jitter_ceiling_ms = base.as_millis().max(1) as u64;
    let jitter_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as u64
        % jitter_ceiling_ms;
    base.saturating_add(Duration::from_millis(jitter_ms))
        .min(policy.backoff_max)
}

pub(super) fn compact_resource_governor_snapshot<F>(
    snapshot_store: ResourceGovernorStore<F>,
    filesystem: Arc<ScopedFilesystem<F>>,
) -> Result<(), ResourceError>
where
    F: RootFilesystem + 'static,
{
    let snapshot = snapshot_store.inspect(|snapshot| Ok(snapshot.clone()))?;
    let from = SeqNo::from_backend(snapshot.journal_seq);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(storage_error)?;
    let (state, latest_seq) = runtime.block_on(replay_journal(filesystem, snapshot.state, from))?;
    snapshot_store.update(move |snapshot| {
        if snapshot.journal_seq >= latest_seq.get() {
            return Ok(());
        }
        snapshot.schema_version = crate::RESOURCE_GOVERNOR_SNAPSHOT_SCHEMA_VERSION;
        snapshot.state = state.clone();
        snapshot.journal_seq = latest_seq.get();
        Ok(())
    })
}

pub(super) async fn replay_journal<F>(
    filesystem: Arc<ScopedFilesystem<F>>,
    mut state: ResourceState,
    from: SeqNo,
) -> Result<(ResourceState, SeqNo), ResourceError>
where
    F: RootFilesystem,
{
    rebuild_active_holds_from_reservations(&mut state);
    let path = delta_log_path()?;
    let records = match filesystem.tail(&ResourceScope::system(), &path, from).await {
        Ok(records) => records,
        Err(FilesystemError::NotFound { .. }) | Err(FilesystemError::Unsupported { .. }) => {
            Vec::new()
        }
        Err(error) => return Err(fs_error(error)),
    };
    let mut latest = from;
    for record in records {
        latest = record.seq;
        let delta: ResourceGovernorDelta = serde_json::from_slice(&record.payload)
            .map_err(|error| storage_error(format!("decode resource governor delta: {error}")))?;
        delta.apply_to(&mut state)?;
    }
    Ok((state, latest))
}

fn rebuild_active_holds_from_reservations(state: &mut ResourceState) {
    state.reserved_by_account.clear();
    for record in state.reservations.values() {
        if record.status == ReservationStatus::Active {
            for account in &record.accounts {
                state
                    .reserved_by_account
                    .entry(account.clone())
                    .or_default()
                    .add_assign(&record.tally);
            }
        }
    }
}

fn delta_log_path() -> Result<ScopedPath, ResourceError> {
    ScopedPath::new(DELTA_LOG_PATH.to_string()).map_err(|error| {
        storage_error(format!("invalid resource governor delta log path: {error}"))
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use async_trait::async_trait;
    use ironclaw_filesystem::{
        BackendCapabilities, Capability, DirEntry, FileStat, FilesystemError, FilesystemOperation,
        RootFilesystem,
    };
    use ironclaw_host_api::{
        ids::TenantId,
        mount::{MountGrant, MountPermissions, MountView},
        path::{MountAlias, VirtualPath},
    };
    use tokio::sync::oneshot;

    use super::*;

    struct GatedAppendFilesystem {
        release: Mutex<Option<oneshot::Receiver<()>>>,
    }

    struct BusyOnceAppendFilesystem {
        append_calls: AtomicUsize,
        append_batch_calls: AtomicUsize,
    }

    struct BusyOnceAppendBatchFilesystem {
        append_calls: AtomicUsize,
        append_batch_calls: AtomicUsize,
    }

    struct SlowBusyAppendFilesystem {
        append_attempts: AtomicUsize,
        delay: Duration,
    }

    /// First append sleeps `delay` then returns `BackendBusy` (mimicking a
    /// libSQL writer checkout that blocks before failing); every later append
    /// succeeds. Pins the retry-window-vs-single-attempt contract.
    struct SlowBusyOnceAppendFilesystem {
        append_attempts: AtomicUsize,
        delay: Duration,
    }

    impl BusyOnceAppendFilesystem {
        fn new() -> Self {
            Self {
                append_calls: AtomicUsize::new(0),
                append_batch_calls: AtomicUsize::new(0),
            }
        }
    }

    impl BusyOnceAppendBatchFilesystem {
        fn new() -> Self {
            Self {
                append_calls: AtomicUsize::new(0),
                append_batch_calls: AtomicUsize::new(0),
            }
        }
    }

    impl SlowBusyAppendFilesystem {
        fn new(delay: Duration) -> Self {
            Self {
                append_attempts: AtomicUsize::new(0),
                delay,
            }
        }

        async fn wait_then_busy<T>(&self, path: &VirtualPath) -> Result<T, FilesystemError> {
            self.append_attempts.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(self.delay).await;
            Err(FilesystemError::BackendBusy {
                path: path.clone(),
                operation: FilesystemOperation::Append,
            })
        }
    }

    impl SlowBusyOnceAppendFilesystem {
        fn new(delay: Duration) -> Self {
            Self {
                append_attempts: AtomicUsize::new(0),
                delay,
            }
        }

        async fn wait_then_busy_then_succeed(
            &self,
            path: &VirtualPath,
        ) -> Result<SeqNo, FilesystemError> {
            let attempt = self.append_attempts.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(self.delay).await;
            if attempt == 0 {
                return Err(FilesystemError::BackendBusy {
                    path: path.clone(),
                    operation: FilesystemOperation::Append,
                });
            }
            Ok(SeqNo::from_backend(1))
        }

        async fn wait_then_busy_then_succeed_batch(
            &self,
            path: &VirtualPath,
            payloads: &[Vec<u8>],
        ) -> Result<Vec<SeqNo>, FilesystemError> {
            self.wait_then_busy_then_succeed(path).await?;
            Ok((1..=payloads.len() as u64)
                .map(SeqNo::from_backend)
                .collect())
        }
    }

    impl GatedAppendFilesystem {
        fn new(release: oneshot::Receiver<()>) -> Self {
            Self {
                release: Mutex::new(Some(release)),
            }
        }
    }

    #[async_trait]
    impl RootFilesystem for GatedAppendFilesystem {
        fn capabilities(&self) -> BackendCapabilities {
            BackendCapabilities::sql_typical().with(Capability::Events)
        }

        async fn append(
            &self,
            path: &VirtualPath,
            _payload: Vec<u8>,
        ) -> Result<SeqNo, FilesystemError> {
            self.wait_for_release(path).await?;
            Ok(SeqNo::from_backend(1))
        }

        async fn append_batch(
            &self,
            path: &VirtualPath,
            payloads: Vec<Vec<u8>>,
        ) -> Result<Vec<SeqNo>, FilesystemError> {
            self.wait_for_release(path).await?;
            Ok((1..=payloads.len() as u64)
                .map(SeqNo::from_backend)
                .collect())
        }

        async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            Err(unsupported(path, FilesystemOperation::ListDir))
        }

        async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
            Err(unsupported(path, FilesystemOperation::Stat))
        }
    }

    #[async_trait]
    impl RootFilesystem for BusyOnceAppendFilesystem {
        fn capabilities(&self) -> BackendCapabilities {
            BackendCapabilities::sql_typical().with(Capability::Events)
        }

        async fn append(
            &self,
            path: &VirtualPath,
            _payload: Vec<u8>,
        ) -> Result<SeqNo, FilesystemError> {
            let call = self.append_calls.fetch_add(1, Ordering::SeqCst);
            if call == 0 {
                return Err(FilesystemError::BackendBusy {
                    path: path.clone(),
                    operation: FilesystemOperation::Append,
                });
            }
            Ok(SeqNo::from_backend(1))
        }

        async fn append_batch(
            &self,
            path: &VirtualPath,
            _payloads: Vec<Vec<u8>>,
        ) -> Result<Vec<SeqNo>, FilesystemError> {
            self.append_batch_calls.fetch_add(1, Ordering::SeqCst);
            Err(backend_error(
                path,
                "singleton append unexpectedly used the batch API",
            ))
        }

        async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            Err(unsupported(path, FilesystemOperation::ListDir))
        }

        async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
            Err(unsupported(path, FilesystemOperation::Stat))
        }
    }

    #[async_trait]
    impl RootFilesystem for BusyOnceAppendBatchFilesystem {
        fn capabilities(&self) -> BackendCapabilities {
            BackendCapabilities::sql_typical().with(Capability::Events)
        }

        async fn append(
            &self,
            path: &VirtualPath,
            _payload: Vec<u8>,
        ) -> Result<SeqNo, FilesystemError> {
            self.append_calls.fetch_add(1, Ordering::SeqCst);
            Err(backend_error(
                path,
                "multi-delta flush unexpectedly used the singleton API",
            ))
        }

        async fn append_batch(
            &self,
            path: &VirtualPath,
            payloads: Vec<Vec<u8>>,
        ) -> Result<Vec<SeqNo>, FilesystemError> {
            let call = self.append_batch_calls.fetch_add(1, Ordering::SeqCst);
            if call == 0 {
                return Err(FilesystemError::BackendBusy {
                    path: path.clone(),
                    operation: FilesystemOperation::Append,
                });
            }
            Ok((1..=payloads.len() as u64)
                .map(SeqNo::from_backend)
                .collect())
        }

        async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            Err(unsupported(path, FilesystemOperation::ListDir))
        }

        async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
            Err(unsupported(path, FilesystemOperation::Stat))
        }
    }

    #[async_trait]
    impl RootFilesystem for SlowBusyAppendFilesystem {
        fn capabilities(&self) -> BackendCapabilities {
            BackendCapabilities::sql_typical().with(Capability::Events)
        }

        async fn append(
            &self,
            path: &VirtualPath,
            _payload: Vec<u8>,
        ) -> Result<SeqNo, FilesystemError> {
            self.wait_then_busy(path).await
        }

        async fn append_batch(
            &self,
            path: &VirtualPath,
            _payloads: Vec<Vec<u8>>,
        ) -> Result<Vec<SeqNo>, FilesystemError> {
            self.wait_then_busy(path).await
        }

        async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            Err(unsupported(path, FilesystemOperation::ListDir))
        }

        async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
            Err(unsupported(path, FilesystemOperation::Stat))
        }
    }

    #[async_trait]
    impl RootFilesystem for SlowBusyOnceAppendFilesystem {
        fn capabilities(&self) -> BackendCapabilities {
            BackendCapabilities::sql_typical().with(Capability::Events)
        }

        async fn append(
            &self,
            path: &VirtualPath,
            _payload: Vec<u8>,
        ) -> Result<SeqNo, FilesystemError> {
            self.wait_then_busy_then_succeed(path).await
        }

        async fn append_batch(
            &self,
            path: &VirtualPath,
            payloads: Vec<Vec<u8>>,
        ) -> Result<Vec<SeqNo>, FilesystemError> {
            self.wait_then_busy_then_succeed_batch(path, &payloads)
                .await
        }

        async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            Err(unsupported(path, FilesystemOperation::ListDir))
        }

        async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
            Err(unsupported(path, FilesystemOperation::Stat))
        }
    }

    impl GatedAppendFilesystem {
        async fn wait_for_release(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
            let release = self
                .release
                .lock()
                .map_err(|_| backend_error(path, "gated append lock poisoned"))?
                .take()
                .ok_or_else(|| backend_error(path, "gated append released more than once"))?;
            release
                .await
                .map_err(|_| backend_error(path, "gated append release sender dropped"))
        }
    }

    fn backend_error(path: &VirtualPath, reason: impl Into<String>) -> FilesystemError {
        FilesystemError::Backend {
            path: path.clone(),
            operation: FilesystemOperation::Append,
            reason: reason.into(),
        }
    }

    fn unsupported(path: &VirtualPath, operation: FilesystemOperation) -> FilesystemError {
        FilesystemError::Unsupported {
            path: path.clone(),
            operation,
        }
    }

    fn scoped_gated_filesystem(
        release: oneshot::Receiver<()>,
    ) -> Arc<ScopedFilesystem<GatedAppendFilesystem>> {
        let backend = Arc::new(GatedAppendFilesystem::new(release));
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/resources").expect("alias"),
            VirtualPath::new("/resources").expect("target"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("mount view");
        Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts))
    }

    fn scoped_busy_once_filesystem(
        backend: Arc<BusyOnceAppendFilesystem>,
    ) -> Arc<ScopedFilesystem<BusyOnceAppendFilesystem>> {
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/resources").expect("alias"),
            VirtualPath::new("/resources").expect("target"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("mount view");
        Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts))
    }

    fn scoped_slow_busy_filesystem(
        backend: Arc<SlowBusyAppendFilesystem>,
    ) -> Arc<ScopedFilesystem<SlowBusyAppendFilesystem>> {
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/resources").expect("alias"),
            VirtualPath::new("/resources").expect("target"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("mount view");
        Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts))
    }

    fn scoped_slow_busy_once_filesystem(
        backend: Arc<SlowBusyOnceAppendFilesystem>,
    ) -> Arc<ScopedFilesystem<SlowBusyOnceAppendFilesystem>> {
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/resources").expect("alias"),
            VirtualPath::new("/resources").expect("target"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("mount view");
        Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts))
    }

    fn scoped_busy_once_batch_filesystem(
        backend: Arc<BusyOnceAppendBatchFilesystem>,
    ) -> Arc<ScopedFilesystem<BusyOnceAppendBatchFilesystem>> {
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/resources").expect("alias"),
            VirtualPath::new("/resources").expect("target"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("mount view");
        Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts))
    }

    #[test]
    fn transient_busy_single_append_is_retried_without_using_batch() {
        let backend = Arc::new(BusyOnceAppendFilesystem::new());
        let journal = ResourceDeltaJournal::new(scoped_busy_once_filesystem(Arc::clone(&backend)));
        let pending = journal
            .enqueue(ResourceGovernorDelta::AccountSnapshot {
                account: ResourceAccount::tenant(TenantId::new("tenant1").expect("tenant id")),
                at: Utc::now(),
            })
            .expect("enqueue delta");

        let seq = pending.wait().expect("busy append should recover");

        assert_eq!(seq, SeqNo::from_backend(1));
        assert_eq!(backend.append_calls.load(Ordering::SeqCst), 2);
        assert_eq!(backend.append_batch_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn transient_busy_multi_delta_batch_is_retried_atomically() {
        let backend = Arc::new(BusyOnceAppendBatchFilesystem::new());
        let filesystem = scoped_busy_once_batch_filesystem(Arc::clone(&backend));
        let account = ResourceAccount::tenant(TenantId::new("tenant1").expect("tenant id"));
        let requests = [
            DeltaJournalRequest {
                delta: ResourceGovernorDelta::AccountSnapshot {
                    account: account.clone(),
                    at: Utc::now(),
                },
                ack: std::sync::mpsc::channel().0,
            },
            DeltaJournalRequest {
                delta: ResourceGovernorDelta::AccountSnapshot {
                    account,
                    at: Utc::now(),
                },
                ack: std::sync::mpsc::channel().0,
            },
        ];

        let seqs =
            persist_delta_journal_batch(filesystem.as_ref(), &requests, DEFAULT_BUSY_RETRY_POLICY)
                .await
                .expect("busy batch append should recover atomically");

        assert_eq!(seqs, vec![SeqNo::from_backend(1), SeqNo::from_backend(2)]);
        assert_eq!(backend.append_calls.load(Ordering::SeqCst), 0);
        assert_eq!(backend.append_batch_calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn busy_retry_window_stops_starting_new_database_attempts_after_deadline() {
        let backend = Arc::new(SlowBusyAppendFilesystem::new(Duration::from_millis(30)));
        let journal = ResourceDeltaJournal::new_with_retry_policy(
            scoped_slow_busy_filesystem(Arc::clone(&backend)),
            BusyRetryPolicy {
                max_retries: 10,
                max_elapsed: Duration::from_millis(50),
                backoff_base: Duration::from_millis(10),
                backoff_max: Duration::from_millis(10),
                jitter: false,
            },
        );
        let pending = journal
            .enqueue(ResourceGovernorDelta::AccountSnapshot {
                account: ResourceAccount::tenant(TenantId::new("tenant1").expect("tenant id")),
                at: Utc::now(),
            })
            .expect("enqueue delta");

        assert!(
            pending.wait().is_err(),
            "busy retry window must fail closed"
        );
        assert_eq!(
            backend.append_attempts.load(Ordering::SeqCst),
            2,
            "the journal must not start a third database attempt after the retry window is exhausted"
        );
    }

    #[test]
    fn default_busy_retry_window_covers_a_single_backend_attempt() {
        // The libSQL root filesystem serializes writers through a one-connection
        // pool: a checkout can block up to the 10s pool timeout and the
        // connection then waits up to its 5s `busy_timeout` before the append
        // returns `BackendBusy` — one attempt can take ~15s. The legacy 5s
        // window was exhausted by that single attempt (`attempts=1`,
        // `elapsed_ms=10003` in the provider-contracts E2E flake), so the
        // retry machinery never ran and a transient writer hold failed the
        // in-flight resource reservation. The window must exceed the backend's
        // worst-case single attempt or `max_retries` is meaningless.
        assert!(
            DEFAULT_BUSY_RETRY_POLICY.max_elapsed
                >= Duration::from_secs(10) + Duration::from_secs(5),
            "retry window must cover one full libSQL writer attempt"
        );
    }

    #[test]
    fn busy_retry_window_shorter_than_one_attempt_gets_no_retries() {
        // Bug shape from the E2E flake: when a single `BackendBusy` attempt
        // outlasts the retry window, the journal gives up after that one
        // attempt (`append_attempts == 1`) instead of retrying — so a
        // transient writer hold fails the flush.
        let backend = Arc::new(SlowBusyOnceAppendFilesystem::new(Duration::from_millis(
            200,
        )));
        let journal = ResourceDeltaJournal::new_with_retry_policy(
            scoped_slow_busy_once_filesystem(Arc::clone(&backend)),
            BusyRetryPolicy {
                max_retries: 10,
                max_elapsed: Duration::from_millis(150),
                backoff_base: Duration::from_millis(10),
                backoff_max: Duration::from_millis(10),
                jitter: false,
            },
        );
        let pending = journal
            .enqueue(ResourceGovernorDelta::AccountSnapshot {
                account: ResourceAccount::tenant(TenantId::new("tenant1").expect("tenant id")),
                at: Utc::now(),
            })
            .expect("enqueue delta");

        assert!(
            pending.wait().is_err(),
            "a window shorter than one backend attempt must fail closed"
        );
        assert_eq!(
            backend.append_attempts.load(Ordering::SeqCst),
            1,
            "the first attempt exhausted the window; no retry may start"
        );
    }

    #[test]
    fn busy_retry_window_longer_than_one_attempt_recovers() {
        // The fixed contract: with the window covering one full attempt, a
        // transient busy that outlasts the legacy window still retries and
        // the flush succeeds instead of invalidating the governor.
        let backend = Arc::new(SlowBusyOnceAppendFilesystem::new(Duration::from_millis(
            200,
        )));
        let journal = ResourceDeltaJournal::new_with_retry_policy(
            scoped_slow_busy_once_filesystem(Arc::clone(&backend)),
            BusyRetryPolicy {
                max_retries: 10,
                max_elapsed: Duration::from_millis(500),
                backoff_base: Duration::from_millis(10),
                backoff_max: Duration::from_millis(10),
                jitter: false,
            },
        );
        let pending = journal
            .enqueue(ResourceGovernorDelta::AccountSnapshot {
                account: ResourceAccount::tenant(TenantId::new("tenant1").expect("tenant id")),
                at: Utc::now(),
            })
            .expect("enqueue delta");

        assert_eq!(
            pending.wait().expect("busy append must recover"),
            SeqNo::from_backend(1)
        );
        assert_eq!(
            backend.append_attempts.load(Ordering::SeqCst),
            2,
            "first attempt timed out as busy; the second must succeed"
        );
    }

    #[test]
    fn pending_delta_wait_does_not_park_the_only_tokio_worker() {
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .expect("runtime");

            let result = runtime.block_on(async { pending_delta_wait_with_gated_append().await });
            let _ = done_tx.send(result);
        });

        let seq = done_rx
            .recv_timeout(std::time::Duration::from_secs(3))
            .expect("pending delta wait should not starve the only runtime worker")
            .expect("pending delta ack");
        assert_eq!(seq, SeqNo::from_backend(1));
    }

    async fn pending_delta_wait_with_gated_append() -> Result<SeqNo, ResourceError> {
        let (release_tx, release_rx) = oneshot::channel();
        let journal = ResourceDeltaJournal::new(scoped_gated_filesystem(release_rx));
        let pending = journal
            .enqueue(ResourceGovernorDelta::AccountSnapshot {
                account: ResourceAccount::tenant(TenantId::new("tenant1").expect("tenant id")),
                at: Utc::now(),
            })
            .expect("enqueue delta");

        tokio::spawn(async move {
            tokio::task::yield_now().await;
            let _ = release_tx.send(());
        });

        pending.wait()
    }
}
