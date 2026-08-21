use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use ironclaw_filesystem::{CasExpectation, Entry, FilesystemError, RecordVersion, RootFilesystem};
use ironclaw_host_api::{path::ScopedPath, resource::ResourceScope};
use tokio::sync::Mutex;

use super::{JOURNAL_READ_BATCH, ProcessJournalStore, ProcessJournalStoreError};
use crate::{
    ProcessJournalCommit, ProcessJournalCommitObserver, ProcessJournalCursor,
    ProcessJournalObserverRegistry,
};

const MAX_OBSERVER_REPLAY_ATTEMPTS: u32 = 8;

#[derive(Clone)]
pub(super) struct RegisteredProcessObserver {
    pub(super) id: String,
    pub(super) observer: Arc<dyn ProcessJournalCommitObserver>,
    pub(super) delivery: Arc<Mutex<ObserverDeliveryState>>,
    pub(super) replay_running: Arc<AtomicBool>,
}

/// Delivery progress for one observer, guarded by its delivery lock.
#[derive(Default)]
pub(super) struct ObserverDeliveryState {
    /// Last cursor known to be durably acknowledged, when this process has
    /// already read or written the observer's cursor row.
    ///
    /// Caching it is what lets a committed batch be delivered straight from the
    /// entries the flusher already holds: without it every batch would re-read
    /// the cursor row and page the journal back out of the backend just to
    /// rediscover what it had in memory.
    acknowledged: Option<ProcessJournalCursor>,
    /// Version of the cursor row observed alongside `acknowledged`, or `None`
    /// when the row was absent at that observation.
    ///
    /// Every cursor write is conditional on it. Two store instances can share
    /// one observer id across a rolling restart; without the condition, an
    /// instance holding a stale cache would overwrite a newer durable cursor
    /// with a lower one and silently rewind acknowledged progress, redelivering
    /// the difference after the next restart.
    version: Option<RecordVersion>,
}

/// Why a cursor acknowledgement did not land.
enum AcknowledgeError {
    /// The durable row moved under us: another store instance advanced this
    /// observer. The cached view is dropped and the caller must fall back to
    /// the authoritative durable replay rather than retry the stale write.
    Conflict,
    Backend(String),
}

/// One or more committed transactions' deliverable entries, carried in memory
/// from the flusher to observer delivery so the common path never reads the
/// journal back out of the backend.
pub(super) struct CommittedBatch {
    /// Cursor of the first journal entry the batch committed.
    pub(super) first: ProcessJournalCursor,
    /// Cursor of the last journal entry the batch committed.
    pub(super) last: ProcessJournalCursor,
    /// Entries that carry committed state, in cursor order. Entries without
    /// committed state advance the cursor but are not delivered.
    pub(super) commits: Vec<ProcessJournalCommit>,
}

tokio::task_local! {
    /// Set while an observer callback runs on this task.
    ///
    /// Observer callbacks legitimately re-enter the journal store (the
    /// sub-agent await-edge resolver settles dependencies and resumes the
    /// parent process from inside `observe_process_commit`). Such a nested
    /// commit must not wait for observer delivery, because delivery is exactly
    /// what is blocked on the callback that issued it.
    static OBSERVER_DELIVERY: ();
}

/// Run an observer callback with the re-entrancy marker set.
pub(super) async fn observer_delivery_scope<T>(future: impl Future<Output = T>) -> T {
    OBSERVER_DELIVERY.scope((), future).await
}

/// Whether the current task is inside an observer callback.
pub(super) fn inside_observer_delivery() -> bool {
    OBSERVER_DELIVERY.try_with(|()| ()).is_ok()
}

struct ObserverReplayGuard(Arc<AtomicBool>);

impl Drop for ObserverReplayGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl<F> ProcessJournalStore<F>
where
    F: RootFilesystem,
{
    /// Snapshot of the currently registered observers.
    pub(super) fn registered_observers(&self) -> Vec<RegisteredProcessObserver> {
        match self.observers.lock() {
            Ok(observers) => observers.clone(),
            Err(poisoned) => {
                tracing::error!(
                    "process journal observer registry mutex was poisoned; recovering registry"
                );
                poisoned.into_inner().clone()
            }
        }
    }

    /// Deliver one committed batch straight from the entries the flusher holds.
    ///
    /// This is the hot path: it costs one cursor write per observer and reads
    /// nothing back out of the backend. It is only valid when the observer's
    /// acknowledged cursor sits immediately before the batch, which is the
    /// normal case because the flusher delivers batches in commit order. Any
    /// gap — a cold cache, another writer's entries, or a cursor range this
    /// process reserved but did not use — falls back to the paged replay, which
    /// is authoritative.
    pub(super) async fn deliver_committed_batch(
        &self,
        observer: &RegisteredProcessObserver,
        batch: &CommittedBatch,
    ) -> Result<(), String> {
        let mut delivery = observer.delivery.lock().await;
        let contiguous = delivery
            .acknowledged
            .is_some_and(|acknowledged| acknowledged.0.saturating_add(1) == batch.first.0);
        if !contiguous {
            drop(delivery);
            return self
                .replay_durable_observer_once(observer, Some(batch.last))
                .await;
        }
        if !batch.commits.is_empty() {
            // Callbacks may re-enter the journal store; the marker lets those
            // nested commits skip the delivery wait that this very call holds
            // (see `flusher`).
            observer_delivery_scope(
                observer
                    .observer
                    .observe_process_commits(batch.commits.clone()),
            )
            .await?;
        }
        match self
            .acknowledge_observer_cursor(observer, &mut delivery, batch.last)
            .await
        {
            Ok(()) => Ok(()),
            Err(AcknowledgeError::Backend(error)) => Err(error),
            Err(AcknowledgeError::Conflict) => {
                // Another instance owns newer progress. Re-derive from the
                // durable row instead of forcing our cursor onto it; the
                // entries above may be delivered twice, which every replay
                // path already permits.
                drop(delivery);
                self.replay_durable_observer_once(observer, Some(batch.last))
                    .await
            }
        }
    }

    /// Persist an observer's delivery cursor and refresh the cached value.
    async fn acknowledge_observer_cursor(
        &self,
        observer: &RegisteredProcessObserver,
        delivery: &mut ObserverDeliveryState,
        cursor: ProcessJournalCursor,
    ) -> Result<(), AcknowledgeError> {
        let cursor_path = process_observer_cursor_path(&observer.id)
            .map_err(|error| AcknowledgeError::Backend(error.to_string()))?;
        let cursor_body = serde_json::to_vec(&cursor.0)
            .map_err(|error| AcknowledgeError::Backend(error.to_string()))?;
        // Conditional on the version this process last observed, so the write
        // can only advance the row it actually read.
        let expectation = match delivery.version {
            Some(version) => CasExpectation::Version(version),
            None => CasExpectation::Absent,
        };
        match self
            .filesystem
            .put(
                &ResourceScope::system(),
                &cursor_path,
                Entry::bytes(cursor_body),
                expectation,
            )
            .await
        {
            Ok(version) => {
                delivery.acknowledged = Some(cursor);
                delivery.version = Some(version);
                Ok(())
            }
            Err(FilesystemError::VersionMismatch { .. }) => {
                delivery.acknowledged = None;
                delivery.version = None;
                Err(AcknowledgeError::Conflict)
            }
            Err(error) => Err(AcknowledgeError::Backend(error.to_string())),
        }
    }

    pub(super) async fn replay_durable_observer_once(
        &self,
        observer: &RegisteredProcessObserver,
        target: Option<ProcessJournalCursor>,
    ) -> Result<(), String> {
        let mut delivery = observer.delivery.lock().await;
        let cursor_path =
            process_observer_cursor_path(&observer.id).map_err(|error| error.to_string())?;
        let read_cursor = self
            .filesystem
            .get(&ResourceScope::system(), &cursor_path)
            .await
            .map_err(|error| error.to_string())?;
        let version = read_cursor.as_ref().map(|versioned| versioned.version);
        let mut after = read_cursor
            .map(|versioned| {
                serde_json::from_slice::<u64>(&versioned.entry.body)
                    .map(ProcessJournalCursor)
                    .map_err(|error| error.to_string())
            })
            .transpose()?;
        // Cursor 0 precedes every real entry, so an observer with no cursor row
        // has provably acknowledged nothing. Recording that (rather than
        // "unknown") lets a journal that starts empty take the in-memory
        // delivery path from its very first batch.
        delivery.acknowledged = Some(after.unwrap_or(ProcessJournalCursor(0)));
        delivery.version = version;
        // Bound on losing the cursor CAS, not on pages: a page that makes
        // progress resets it. Two instances sharing an observer id can each
        // hold a valid version and win in turn, and if the re-read cursor does
        // not advance past the page just delivered, `after` is unchanged and
        // the same page is re-read, re-delivered, and conflicts again. Every
        // iteration costs a journal read plus a full observer callback, so
        // unbounded retries live-lock while redelivering. Giving up returns an
        // error, which hands the retry to `spawn_observer_replay` and its
        // backoff instead of spinning here.
        const MAX_CURSOR_CONFLICT_RETRIES: u32 = 8;
        let mut cursor_conflicts = 0_u32;
        loop {
            // A concurrent replay may already have delivered past this target
            // while this call waited for the delivery lock.
            if let (Some(after), Some(target)) = (after, target)
                && after.0 >= target.0
            {
                return Ok(());
            }
            let page = self
                .read_journal_page(None, None, after, JOURNAL_READ_BATCH - 1)
                .await
                .map_err(|error| error.to_string())?;
            let mut delivered = None;
            let mut reached_target = false;
            let mut commits = Vec::new();
            for entry in &page.entries {
                if let Some(state) = entry.committed_state.as_deref() {
                    commits.push(ProcessJournalCommit {
                        state: state.clone(),
                        kind: entry.kind,
                        occurred_at: entry.occurred_at,
                        sanitized_reason: entry.sanitized_reason.clone(),
                    });
                }
                delivered = Some(entry.cursor);
                if target.is_some_and(|target| entry.cursor.0 >= target.0) {
                    reached_target = true;
                    break;
                }
            }
            if !commits.is_empty() {
                observer_delivery_scope(observer.observer.observe_process_commits(commits)).await?;
            }
            // One cursor write per delivered page rather than per entry. A
            // crash mid-page redelivers the page; observers already tolerate
            // redelivery because every retry path replays.
            if let Some(cursor) = delivered {
                match self
                    .acknowledge_observer_cursor(observer, &mut delivery, cursor)
                    .await
                {
                    // A page that lands is progress, so a long healthy replay is
                    // never killed by conflicts accumulated across pages.
                    Ok(()) => cursor_conflicts = 0,
                    Err(AcknowledgeError::Backend(error)) => return Err(error),
                    Err(AcknowledgeError::Conflict) => {
                        cursor_conflicts += 1;
                        if cursor_conflicts > MAX_CURSOR_CONFLICT_RETRIES {
                            return Err(format!(
                                "process journal observer {} lost the cursor CAS \
                                 {MAX_CURSOR_CONFLICT_RETRIES} times without \
                                 advancing; deferring to backoff",
                                observer.id
                            ));
                        }
                        // Another instance advanced this observer while the
                        // page was in flight. Re-read the durable row and
                        // resume from wherever it now stands; its cursor is
                        // authoritative and may already cover the target.
                        let read_cursor = self
                            .filesystem
                            .get(&ResourceScope::system(), &cursor_path)
                            .await
                            .map_err(|error| error.to_string())?;
                        delivery.version = read_cursor.as_ref().map(|versioned| versioned.version);
                        after = read_cursor
                            .map(|versioned| {
                                serde_json::from_slice::<u64>(&versioned.entry.body)
                                    .map(ProcessJournalCursor)
                                    .map_err(|error| error.to_string())
                            })
                            .transpose()?;
                        delivery.acknowledged = Some(after.unwrap_or(ProcessJournalCursor(0)));
                        continue;
                    }
                }
            }
            if reached_target || !page.truncated {
                return Ok(());
            }
            after = Some(page.next_cursor);
        }
    }

    pub(super) fn spawn_observer_replay(&self, observer: RegisteredProcessObserver)
    where
        F: Send + Sync + 'static,
    {
        if observer.replay_running.swap(true, Ordering::AcqRel) {
            return;
        }
        let store = self.clone();
        tokio::spawn(async move {
            let _replay_guard = ObserverReplayGuard(Arc::clone(&observer.replay_running));
            let mut delay = Duration::from_millis(250);
            let mut attempt = 0_u32;
            loop {
                attempt += 1;
                match store.replay_durable_observer_once(&observer, None).await {
                    Ok(()) => break,
                    Err(error) if attempt >= MAX_OBSERVER_REPLAY_ATTEMPTS => {
                        tracing::error!(
                            observer_id = %observer.id,
                            attempts = attempt,
                            %error,
                            "durable process observer replay exhausted its retry budget; \
                             cursor remains unacknowledged"
                        );
                        break;
                    }
                    Err(error) => {
                        tracing::debug!(
                            observer_id = %observer.id,
                            attempt,
                            %error,
                            "durable process observer delivery will retry"
                        );
                        tokio::time::sleep(delay).await;
                        delay = delay.saturating_mul(2).min(Duration::from_secs(30));
                    }
                }
            }
        });
    }
}

impl<F> ProcessJournalObserverRegistry for ProcessJournalStore<F>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    fn subscribe_process_observer(
        &self,
        observer: Arc<dyn ProcessJournalCommitObserver>,
    ) -> Result<(), String> {
        let mut observers = self
            .observers
            .lock()
            .map_err(|_| "process journal observer registry mutex poisoned".to_string())?;
        let registered = RegisteredProcessObserver {
            id: observer.process_observer_id().to_string(),
            observer,
            delivery: Arc::new(Mutex::new(ObserverDeliveryState::default())),
            replay_running: Arc::new(AtomicBool::new(false)),
        };
        if observers
            .iter()
            .any(|existing| existing.id == registered.id)
        {
            return Err(format!(
                "process journal observer {} is already registered",
                registered.id
            ));
        }
        observers.push(registered.clone());
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| "process journal observer registration requires a Tokio runtime")?;
        let store = self.clone();
        runtime.spawn(async move {
            if let Err(error) = store.replay_durable_observer_once(&registered, None).await {
                tracing::debug!(
                    observer_id = %registered.id,
                    %error,
                    "initial durable process observer replay failed"
                );
                store.spawn_observer_replay(registered);
            }
        });
        Ok(())
    }
}

fn process_observer_cursor_path(observer_id: &str) -> Result<ScopedPath, ProcessJournalStoreError> {
    let digest = blake3::hash(observer_id.as_bytes()).to_hex();
    ScopedPath::new(format!("/processes/materialized/observer-cursor/{digest}"))
        .map_err(|error| ProcessJournalStoreError::InvalidPath(error.to_string()))
}
