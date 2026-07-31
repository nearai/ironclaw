//! Group-commit funnel for the process journal write path.
//!
//! Every lifecycle command used to own a backend transaction and reserve its
//! journal cursors one at a time from the single global sequence row. On
//! PostgreSQL that row's lock is held until commit, so unrelated commands
//! system-wide serialized behind one full transaction round trip each.
//!
//! A single writer task now drains the command queue and commits many commands
//! in one transaction: global metadata and the shared idempotency-order row are
//! read once per batch, and the whole cursor range is reserved with one call.
//! This is a single-writer funnel, not a lock around I/O — no process-local
//! mutex is held across a backend call (see `.claude/rules/database.md`).
//!
//! Observer delivery runs on its own task rather than inline in the flusher.
//! Observer callbacks legitimately re-enter the store (the sub-agent await-edge
//! resolver settles dependencies and resumes the parent process from inside
//! `observe_process_commit`), so a flusher that blocked on delivery could never
//! commit the command delivery is waiting on.

use std::mem;

use ironclaw_filesystem::{FilesystemError, FilesystemOperation, RootFilesystem};
use ironclaw_host_api::resource::ResourceScope;
use tokio::sync::{mpsc, oneshot};

use super::{
    MAX_TRANSACTION_RETRIES, ProcessJournalStore, ProcessJournalStoreError, StoredCommandOutcome,
    StoredProcessCommand, process_journal_sequence_path, processes_prefix, rows,
};
use crate::ProcessJournalCursor;

/// Upper bound on how many commands share one transaction. The flusher never
/// waits to fill a batch: it commits whatever is already queued, so a single
/// caller pays no added latency and batches only form under real contention.
const MAX_BATCH_COMMANDS: usize = 64;

type CommandResult = Result<StoredCommandOutcome, ProcessJournalStoreError>;
type CommandResponder = oneshot::Sender<CommandResult>;

pub(super) struct QueuedCommand {
    pub(super) command: StoredProcessCommand,
    pub(super) responder: CommandResponder,
    /// `false` for commands issued from inside an observer callback, which must
    /// not wait on the delivery their own caller is blocking.
    pub(super) awaits_observer_delivery: bool,
}

pub(super) struct FlusherHandle {
    commands: mpsc::UnboundedSender<QueuedCommand>,
}

/// The funnel is gone because every store handle that owned it was dropped.
pub(super) struct FunnelClosed;

impl FlusherHandle {
    pub(super) fn submit(&self, queued: QueuedCommand) -> Result<(), FunnelClosed> {
        match self.commands.send(queued) {
            Ok(()) => Ok(()),
            // The channel closes only when the flusher task is gone. The
            // command was never queued and its responder is dropped with it.
            Err(_never_queued) => Err(FunnelClosed),
        }
    }
}

/// The error every caller of a batch that could not reach a durable commit
/// observes, matching the per-command retry-exhaustion error.
pub(super) fn backend_busy<F>(store: &ProcessJournalStore<F>) -> ProcessJournalStoreError
where
    F: RootFilesystem,
{
    let resolved = processes_prefix().and_then(|prefix| {
        store
            .filesystem
            .resolve(&ResourceScope::system(), &prefix)
            .map_err(ProcessJournalStoreError::from)
    });
    match resolved {
        Ok(path) => ProcessJournalStoreError::Filesystem(FilesystemError::BackendBusy {
            path,
            operation: FilesystemOperation::BeginTxn,
        }),
        Err(error) => error,
    }
}

/// Start the funnel. `store` must be a detached handle so the tasks below do
/// not keep the command channel alive after every real store handle is dropped.
pub(super) fn spawn<F>(store: ProcessJournalStore<F>) -> FlusherHandle
where
    F: RootFilesystem + Send + Sync + 'static,
{
    let (commands, command_receiver) = mpsc::unbounded_channel();
    let (deliveries, delivery_receiver) = mpsc::unbounded_channel();
    tokio::spawn(run_delivery(store.clone(), delivery_receiver));
    tokio::spawn(run_flusher(store, command_receiver, deliveries));
    FlusherHandle { commands }
}

struct DeliveryBatch {
    target: ProcessJournalCursor,
    waiters: Vec<(CommandResponder, CommandResult)>,
}

impl DeliveryBatch {
    fn release(self) {
        for (responder, result) in self.waiters {
            let _ = responder.send(result);
        }
    }
}

async fn run_flusher<F>(
    store: ProcessJournalStore<F>,
    mut receiver: mpsc::UnboundedReceiver<QueuedCommand>,
    deliveries: mpsc::UnboundedSender<DeliveryBatch>,
) where
    F: RootFilesystem + Send + Sync + 'static,
{
    let mut carried: Option<QueuedCommand> = None;
    loop {
        let first = match carried.take() {
            Some(queued) => queued,
            None => match receiver.recv().await {
                Some(queued) => queued,
                None => break,
            },
        };
        let mut batch = Vec::with_capacity(1);
        let solo = first.command.requires_solo_transaction();
        batch.push(first);
        if !solo {
            while batch.len() < MAX_BATCH_COMMANDS {
                let Ok(queued) = receiver.try_recv() else {
                    break;
                };
                if queued.command.requires_solo_transaction() {
                    carried = Some(queued);
                    break;
                }
                batch.push(queued);
            }
        }
        flush_batch(&store, batch, &deliveries).await;
    }
}

async fn run_delivery<F>(
    store: ProcessJournalStore<F>,
    mut receiver: mpsc::UnboundedReceiver<DeliveryBatch>,
) where
    F: RootFilesystem + Send + Sync + 'static,
{
    while let Some(first) = receiver.recv().await {
        let mut target = first.target;
        let mut waiters = first.waiters;
        while let Ok(next) = receiver.try_recv() {
            if next.target.0 > target.0 {
                target = next.target;
            }
            waiters.extend(next.waiters);
        }
        for observer in store.registered_observers() {
            if let Err(error) = store
                .replay_durable_observer_once(&observer, Some(target))
                .await
            {
                tracing::warn!(
                    observer_id = %observer.id,
                    cursor = target.0,
                    %error,
                    "process journal observer delivery failed after durable commit"
                );
                // Commit durability is the caller's contract; delivery retries
                // in the background exactly as it did per commit.
                store.spawn_observer_replay(observer);
            }
        }
        DeliveryBatch { target, waiters }.release();
    }
}

struct PendingCommand {
    command: StoredProcessCommand,
    references: rows::LoadReferences,
    responder: Option<CommandResponder>,
    awaits_observer_delivery: bool,
    outcome: Option<StoredCommandOutcome>,
}

impl PendingCommand {
    fn is_live(&self) -> bool {
        self.responder.is_some()
    }

    fn respond(&mut self, result: CommandResult) {
        if let Some(responder) = self.responder.take() {
            let _ = responder.send(result);
        }
    }
}

async fn flush_batch<F>(
    store: &ProcessJournalStore<F>,
    batch: Vec<QueuedCommand>,
    deliveries: &mpsc::UnboundedSender<DeliveryBatch>,
) where
    F: RootFilesystem + Send + Sync + 'static,
{
    let mut pending = Vec::with_capacity(batch.len());
    for queued in batch {
        match queued.command.load_references() {
            Ok(references) => pending.push(PendingCommand {
                command: queued.command,
                references,
                responder: Some(queued.responder),
                awaits_observer_delivery: queued.awaits_observer_delivery,
                outcome: None,
            }),
            Err(error) => {
                let _ = queued.responder.send(Err(error));
            }
        }
    }
    let Err(error) = commit_pending(store, &mut pending, deliveries).await else {
        return;
    };
    let mut live = pending
        .into_iter()
        .filter(PendingCommand::is_live)
        .collect::<Vec<_>>();
    if live.len() <= 1 {
        respond_first_live(&mut live, error);
        return;
    }
    // The transaction failed for the batch as a whole. Retry each command in
    // its own transaction so every caller observes the error its own command
    // produced instead of a neighbour's.
    tracing::warn!(
        %error,
        commands = live.len(),
        "process journal group commit failed; retrying commands individually"
    );
    for command in live {
        let mut single = vec![command];
        if let Err(error) = commit_pending(store, &mut single, deliveries).await {
            respond_first_live(&mut single, error);
        }
    }
}

fn respond_first_live(pending: &mut [PendingCommand], error: ProcessJournalStoreError) {
    if let Some(command) = pending.iter_mut().find(|command| command.is_live()) {
        command.respond(Err(error));
    }
}

/// Commit every live command in `pending` in one transaction.
///
/// Per-command validation failures are answered here and are final, matching
/// the previous one-transaction-per-command semantics. `Err` means the
/// transaction itself failed non-retryably and the batch is still unanswered.
async fn commit_pending<F>(
    store: &ProcessJournalStore<F>,
    pending: &mut [PendingCommand],
    deliveries: &mpsc::UnboundedSender<DeliveryBatch>,
) -> Result<(), ProcessJournalStoreError>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    let prefix = processes_prefix()?;
    let sequence_path = store
        .filesystem
        .resolve(&ResourceScope::system(), &process_journal_sequence_path()?)?;
    'transaction: for attempt in 0..MAX_TRANSACTION_RETRIES {
        if !pending.iter().any(PendingCommand::is_live) {
            return Ok(());
        }
        let mut references = rows::LoadReferences::default();
        for command in pending.iter().filter(|command| command.is_live()) {
            references.merge_from(&command.references);
        }
        let mut loaded = match rows::load(store.filesystem.as_ref(), &references).await {
            Ok(loaded) => loaded,
            Err(ProcessJournalStoreError::Filesystem(error))
                if rows::retryable_transaction_error(&error) =>
            {
                rows::retry_transaction(attempt).await;
                continue;
            }
            Err(error) => return Err(error),
        };
        let mut state = mem::take(&mut loaded.state);

        // Determine the exact number of journal entries before touching the
        // global cursor allocator. Polling an empty queue used to reserve
        // `max_processes` cursors despite producing no entries, so idle
        // supervisors continuously contended with real submissions.
        let mut preview = state.clone();
        let mut reservations = 0_usize;
        for command in pending.iter_mut().filter(|command| command.is_live()) {
            let mut candidate = preview.clone();
            match candidate.apply_command(command.command.clone()) {
                Ok(_) => {
                    let generated = candidate.journal.len();
                    candidate.journal.clear();
                    reservations = reservations
                        .saturating_add(command.command.cursor_reservation_count(generated));
                    preview = candidate;
                }
                Err(error) => command.respond(Err(error)),
            }
        }
        drop(preview);
        if !pending.iter().any(PendingCommand::is_live) {
            return Ok(());
        }

        let mut txn = match store
            .filesystem
            .begin(&ResourceScope::system(), &prefix)
            .await
        {
            Ok(txn) => txn,
            Err(error) if rows::retryable_transaction_error(&error) => {
                rows::retry_transaction(attempt).await;
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        // Reserving the cursor range must stay inside the transaction: the
        // sequence row's lock is what orders commits, so an observer can never
        // advance past a lower cursor reserved by a writer that has not
        // committed yet.
        if reservations > 0 {
            let count = reservations as u64;
            let last = match txn.reserve_sequence_range(&sequence_path, count).await {
                Ok(last) => last,
                Err(error) if rows::retryable_transaction_error(&error) => {
                    txn.rollback().await;
                    rows::retry_transaction(attempt).await;
                    continue 'transaction;
                }
                Err(error) => {
                    txn.rollback().await;
                    return Err(error.into());
                }
            };
            state.next_cursor = last.get().saturating_sub(count).saturating_add(1);
        }

        let mut entries = Vec::new();
        for command in pending.iter_mut().filter(|command| command.is_live()) {
            // Apply against a clone so one command failing validation drops
            // only its own mutations and leaves the rest of the batch intact.
            // Its share of the reserved cursor range is then simply unused;
            // journal readers page by cursor order and tolerate gaps.
            let mut candidate = state.clone();
            match candidate.apply_command(command.command.clone()) {
                Ok(outcome) => {
                    entries.append(&mut candidate.journal);
                    state = candidate;
                    command.outcome = Some(outcome);
                }
                Err(error) => command.respond(Err(error)),
            }
        }
        let committed_cursor = entries.last().map(|entry| entry.cursor);
        let result = async {
            rows::persist(store.filesystem.as_ref(), txn.as_mut(), &loaded, &state).await?;
            rows::persist_journal(store.filesystem.as_ref(), txn.as_mut(), entries.as_slice())
                .await?;
            Ok::<(), ProcessJournalStoreError>(())
        }
        .await;
        match result {
            Ok(()) => match txn.commit().await {
                Ok(()) => {
                    release_committed_batch(store, pending, committed_cursor, deliveries);
                    return Ok(());
                }
                Err(error) if rows::retryable_transaction_error(&error) => {
                    discard_outcomes(pending);
                    rows::retry_transaction(attempt).await;
                }
                Err(error) => return Err(error.into()),
            },
            Err(ProcessJournalStoreError::Filesystem(error))
                if rows::retryable_transaction_error(&error) =>
            {
                txn.rollback().await;
                discard_outcomes(pending);
                rows::retry_transaction(attempt).await;
            }
            Err(error) => {
                txn.rollback().await;
                return Err(error);
            }
        }
    }
    for command in pending.iter_mut() {
        let error = backend_busy(store);
        command.respond(Err(error));
    }
    Ok(())
}

fn discard_outcomes(pending: &mut [PendingCommand]) {
    for command in pending.iter_mut() {
        command.outcome = None;
    }
}

/// Answer a committed batch, holding back callers that require read-your-writes
/// until this batch has been delivered to every registered observer.
fn release_committed_batch<F>(
    store: &ProcessJournalStore<F>,
    pending: &mut [PendingCommand],
    committed_cursor: Option<ProcessJournalCursor>,
    deliveries: &mpsc::UnboundedSender<DeliveryBatch>,
) where
    F: RootFilesystem + Send + Sync + 'static,
{
    let deliver = committed_cursor.is_some() && !store.registered_observers().is_empty();
    let mut waiters = Vec::new();
    for command in pending.iter_mut() {
        let Some(outcome) = command.outcome.take() else {
            continue;
        };
        let Some(responder) = command.responder.take() else {
            continue;
        };
        if deliver && command.awaits_observer_delivery {
            waiters.push((responder, Ok(outcome)));
        } else {
            let _ = responder.send(Ok(outcome));
        }
    }
    let Some(target) = committed_cursor.filter(|_| deliver) else {
        return;
    };
    let batch = DeliveryBatch { target, waiters };
    if let Err(undeliverable) = deliveries.send(batch) {
        tracing::warn!(
            cursor = target.0,
            "process journal delivery task is gone; committed batch will not be delivered"
        );
        undeliverable.0.release();
    }
}
