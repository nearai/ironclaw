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
    pub(super) async fn notify_process_commit(
        &self,
        state: crate::JournaledProcessSnapshot,
        kind: crate::ProcessJournalKind,
        sanitized_reason: Option<String>,
    ) where
        F: Send + Sync + 'static,
    {
        let observers = match self.observers.lock() {
            Ok(observers) => observers.clone(),
            Err(poisoned) => {
                tracing::error!(
                    "process journal observer registry mutex was poisoned; recovering registry"
                );
                poisoned.into_inner().clone()
            }
        };
        let target_cursor = state.journal_cursor;
        for observer in observers {
            if let Err(error) = self
                .replay_durable_observer_once(&observer, Some(target_cursor))
                .await
            {
                tracing::warn!(
                    process_id = %state.process_id,
                    cursor = target_cursor.0,
                    observer_id = %observer.id,
                    kind = ?kind,
                    reason = ?sanitized_reason,
                    %error,
                    "process journal observer delivery failed after durable commit"
                );
                self.spawn_observer_replay(observer);
            }
        }
    }

    async fn replay_durable_observer_once(
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
            let page = self
                .read_journal_page(None, None, after, JOURNAL_READ_BATCH - 1)
                .await
                .map_err(|error| error.to_string())?;
            for entry in &page.entries {
                if let Some(state) = entry.committed_state.as_deref() {
                    observer
                        .observer
                        .observe_process_commit(ProcessJournalCommit {
                            state: state.clone(),
                            kind: entry.kind,
                            sanitized_reason: entry.sanitized_reason.clone(),
                        })
                        .await?;
                }
                let cursor_body =
                    serde_json::to_vec(&entry.cursor.0).map_err(|error| error.to_string())?;
                self.filesystem
                    .put(
                        &ResourceScope::system(),
                        &cursor_path,
                        Entry::bytes(cursor_body),
                        CasExpectation::Any,
                    )
                    .await
                    .map_err(|error| error.to_string())?;
                if target.is_some_and(|target| entry.cursor.0 >= target.0) {
                    return Ok(());
                }
            }
            if !page.truncated {
                return Ok(());
            }
            after = Some(page.next_cursor);
        }
    }

    fn spawn_observer_replay(&self, observer: RegisteredProcessObserver)
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
