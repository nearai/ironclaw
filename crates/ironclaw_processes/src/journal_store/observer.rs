use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use ironclaw_filesystem::{CasExpectation, Entry, RootFilesystem};
use ironclaw_host_api::{path::ScopedPath, resource::ResourceScope};
use tokio::sync::Mutex;

use super::{JOURNAL_READ_BATCH, ProcessJournalStore, ProcessJournalStoreError};
use crate::{
    ProcessJournalCommit, ProcessJournalCommitObserver, ProcessJournalCursor,
    ProcessJournalObserverRegistry,
};

#[derive(Clone)]
pub(super) struct RegisteredProcessObserver {
    pub(super) id: String,
    pub(super) observer: Arc<dyn ProcessJournalCommitObserver>,
    pub(super) delivery: Arc<Mutex<()>>,
    pub(super) replay_running: Arc<AtomicBool>,
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

    pub(super) async fn replay_durable_observer_once(
        &self,
        observer: &RegisteredProcessObserver,
        target: Option<ProcessJournalCursor>,
    ) -> Result<(), String> {
        let _delivery = observer.delivery.lock().await;
        let cursor_path =
            process_observer_cursor_path(&observer.id).map_err(|error| error.to_string())?;
        let mut after = self
            .filesystem
            .get(&ResourceScope::system(), &cursor_path)
            .await
            .map_err(|error| error.to_string())?
            .map(|versioned| {
                serde_json::from_slice::<u64>(&versioned.entry.body)
                    .map(ProcessJournalCursor)
                    .map_err(|error| error.to_string())
            })
            .transpose()?;
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
            for entry in &page.entries {
                if let Some(state) = entry.committed_state.as_deref() {
                    // Callbacks may re-enter the journal store; the marker lets
                    // those nested commits skip the delivery wait that this
                    // very call is holding (see `flusher`).
                    observer_delivery_scope(observer.observer.observe_process_commit(
                        ProcessJournalCommit {
                            state: state.clone(),
                            kind: entry.kind,
                            sanitized_reason: entry.sanitized_reason.clone(),
                        },
                    ))
                    .await?;
                }
                delivered = Some(entry.cursor);
                if target.is_some_and(|target| entry.cursor.0 >= target.0) {
                    reached_target = true;
                    break;
                }
            }
            // One cursor write per delivered page rather than per entry. A
            // crash between pages redelivers the page; observers already
            // tolerate redelivery because every retry path replays.
            if let Some(cursor) = delivered {
                let cursor_body =
                    serde_json::to_vec(&cursor.0).map_err(|error| error.to_string())?;
                self.filesystem
                    .put(
                        &ResourceScope::system(),
                        &cursor_path,
                        Entry::bytes(cursor_body),
                        CasExpectation::Any,
                    )
                    .await
                    .map_err(|error| error.to_string())?;
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
            loop {
                match store.replay_durable_observer_once(&observer, None).await {
                    Ok(()) => break,
                    Err(error) => {
                        tracing::warn!(
                            observer_id = %observer.id,
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
            delivery: Arc::new(Mutex::new(())),
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
                tracing::warn!(
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
