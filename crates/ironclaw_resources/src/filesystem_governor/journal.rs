use std::sync::{Arc, mpsc};

use chrono::{DateTime, Utc};
use ironclaw_filesystem::{FilesystemError, RootFilesystem, ScopedFilesystem, SeqNo};
use ironclaw_host_api::{ReservationStatus, ResourceReservationId, ResourceScope, ScopedPath};
use serde::{Deserialize, Serialize};
use tokio::runtime::{Handle, RuntimeFlavor};
use tracing::warn;

use crate::{
    FilesystemResourceGovernorStore, ResourceAccount, ResourceError, ResourceEstimate,
    ResourceGovernorStore, ResourceLimits, ResourceState, ResourceUsage, account_snapshot_in_state,
    reconcile_in_state, release_in_state, reserve_with_outcome_in_state, set_limit_in_state,
};

use super::{fs_error, storage_error};

const DELTA_LOG_PATH: &str = "/resources/deltas/log";
const DELTA_JOURNAL_MAX_BATCH: usize = 256;

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
    sender: mpsc::Sender<DeltaJournalRequest>,
    _filesystem: std::marker::PhantomData<F>,
}

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
        let (sender, receiver) = mpsc::channel();
        if let Err(error) = std::thread::Builder::new()
            .name("resource-governor-delta-journal".to_string())
            .spawn(move || run_delta_journal_flusher(filesystem, receiver))
        {
            warn!(reason = %error, "resource governor delta journal thread failed to start");
        }
        Self {
            sender,
            _filesystem: std::marker::PhantomData,
        }
    }

    pub(super) fn enqueue(
        &self,
        delta: ResourceGovernorDelta,
    ) -> Result<PendingResourceDelta, ResourceError> {
        let (ack, receiver) = mpsc::channel();
        self.sender
            .send(DeltaJournalRequest { delta, ack })
            .map_err(|_| storage_error("resource governor delta journal stopped"))?;
        Ok(PendingResourceDelta { ack: receiver })
    }
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
        let result = runtime.block_on(persist_delta_journal_batch(filesystem.as_ref(), &requests));
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
) -> Result<Vec<SeqNo>, ResourceError>
where
    F: RootFilesystem,
{
    let path = delta_log_path()?;
    let payloads = requests
        .iter()
        .map(|request| serde_json::to_vec(&request.delta).map_err(storage_error))
        .collect::<Result<Vec<_>, _>>()?;
    if let [payload] = payloads.as_slice() {
        return filesystem
            .append(&ResourceScope::system(), &path, payload.clone())
            .await
            .map(|seq| vec![seq])
            .map_err(fs_error);
    }
    let seqs = filesystem
        .append_batch(&ResourceScope::system(), &path, payloads)
        .await
        .map_err(fs_error)?;
    if seqs.len() != requests.len() {
        return Err(storage_error(
            "resource governor delta batch append returned an unexpected ack count",
        ));
    }
    Ok(seqs)
}

pub(super) fn compact_resource_governor_snapshot<F>(
    snapshot_store: FilesystemResourceGovernorStore<F>,
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
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use ironclaw_filesystem::{
        BackendCapabilities, Capability, DirEntry, FileStat, FilesystemError, FilesystemOperation,
        RootFilesystem,
    };
    use ironclaw_host_api::{
        MountAlias, MountGrant, MountPermissions, MountView, TenantId, VirtualPath,
    };
    use tokio::sync::oneshot;

    use super::*;

    struct GatedAppendFilesystem {
        release: Mutex<Option<oneshot::Receiver<()>>>,
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
