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
    StoredProcessCommand, observer::CommittedBatch, process_journal_sequence_path,
    processes_prefix, rows,
};
use crate::{ProcessJournalCommit, ProcessJournalEntry};

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

/// Queue depth for lifecycle commands awaiting a batch.
///
/// The funnel is the one writer, so an unbounded queue would let a stalled
/// backend accumulate one boxed request per in-flight caller with no ceiling.
/// A bounded queue turns that into backpressure at `execute`. It also bounds
/// the delivery queue transitively: every delivery batch comes from commands
/// that passed through here.
const COMMAND_QUEUE_CAPACITY: usize = 1_024;

pub(super) struct FlusherHandle {
    commands: mpsc::Sender<QueuedCommand>,
}

/// The funnel is gone because every store handle that owned it was dropped.
pub(super) struct FunnelClosed;

impl FlusherHandle {
    /// Enqueue a command, waiting for room when the funnel is saturated.
    ///
    /// Waiting here is the backpressure: the flusher drains continuously and
    /// never blocks on a caller, so a queued caller cannot stall the drain
    /// (including the re-entrant commands observer callbacks issue).
    pub(super) async fn submit(&self, queued: QueuedCommand) -> Result<(), FunnelClosed> {
        match self.commands.send(queued).await {
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
    let (commands, command_receiver) = mpsc::channel(COMMAND_QUEUE_CAPACITY);
    // Deliveries stay unbounded deliberately. Observer callbacks re-enter the
    // store (the await-edge resolver settles dependencies from inside one), so
    // a full delivery queue would block the flusher while the delivery task
    // waits on a command the flusher can no longer accept. Its depth is
    // already bounded by the command queue that feeds it.
    let (deliveries, delivery_receiver) = mpsc::unbounded_channel();
    tokio::spawn(run_delivery(store.clone(), delivery_receiver));
    tokio::spawn(run_flusher(store, command_receiver, deliveries));
    FlusherHandle { commands }
}

struct DeliveryBatch {
    committed: CommittedBatch,
    waiters: Vec<(CommandResponder, CommandResult)>,
}

impl DeliveryBatch {
    /// Whether `next` starts exactly where this batch ends.
    ///
    /// Only then may the two be delivered as one range: a gap could hold
    /// another writer's committed entries, and folding over it would advance
    /// the observer cursor past entries that were never delivered.
    fn precedes(&self, next: &DeliveryBatch) -> bool {
        self.committed.last.0.saturating_add(1) == next.committed.first.0
    }

    /// Fold a directly following batch into this one so a run of commits is
    /// delivered in a single pass, in cursor order.
    fn absorb(&mut self, next: DeliveryBatch) {
        self.committed.last = next.committed.last;
        self.committed.commits.extend(next.committed.commits);
        self.waiters.extend(next.waiters);
    }

    fn release(self) {
        for (responder, result) in self.waiters {
            let _ = responder.send(result);
        }
    }
}

async fn run_flusher<F>(
    store: ProcessJournalStore<F>,
    mut receiver: mpsc::Receiver<QueuedCommand>,
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
    let mut carried: Option<DeliveryBatch> = None;
    loop {
        let mut batch = match carried.take() {
            Some(batch) => batch,
            None => match receiver.recv().await {
                Some(batch) => batch,
                None => break,
            },
        };
        while let Ok(next) = receiver.try_recv() {
            if batch.precedes(&next) {
                batch.absorb(next);
            } else {
                carried = Some(next);
                break;
            }
        }
        deliver_batch(&store, &batch.committed).await;
        batch.release();
    }
}

/// Deliver one committed range to every observer.
///
/// Observers own independent cursors and stores, so they run concurrently:
/// delivery sits between a commit and the release of its foreground waiters,
/// and serializing independent observers there adds their latencies together.
async fn deliver_batch<F>(store: &ProcessJournalStore<F>, committed: &CommittedBatch)
where
    F: RootFilesystem + Send + Sync + 'static,
{
    let observers = store.registered_observers();
    futures::future::join_all(observers.into_iter().map(|observer| async move {
        if let Err(error) = store.deliver_committed_batch(&observer, committed).await {
            tracing::warn!(
                observer_id = %observer.id,
                cursor = committed.last.0,
                %error,
                "process journal observer delivery failed after durable commit"
            );
            // Commit durability is the caller's contract; delivery retries in
            // the background exactly as it did per commit.
            store.spawn_observer_replay(observer);
        }
    }))
    .await;
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
    // The transaction failed as a whole and nothing in it committed. Every
    // live command observes that failure.
    //
    // Re-executing them individually to attribute the error would be wrong:
    // the batch already consumed whatever externally observable effect the
    // backend applied before failing, so a command whose write was supposed
    // to fail can succeed on the replay and report a durable state it never
    // reached (a one-shot store fault reaching its retry, observed as
    // Completed/Failed where the contract requires Running). Reporting the
    // rollback to all of them is truthful — none of them committed — and a
    // caller that wants another attempt re-issues its own command.
    tracing::debug!(
        %error,
        commands = live.len(),
        "process journal group commit failed; failing the batch without replay"
    );
    for command in live.iter_mut() {
        command.respond(Err(clone_transaction_error(&error)));
    }
}

/// Reproduce a batch-wide transaction failure for each caller.
///
/// `ProcessJournalStoreError` is not `Clone` (it carries backend error types),
/// so the batch failure is re-expressed per command with its cause preserved
/// in the message.
fn clone_transaction_error(error: &ProcessJournalStoreError) -> ProcessJournalStoreError {
    ProcessJournalStoreError::GroupCommitFailed {
        reason: error.to_string(),
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
        // One pass over the fully merged batch, so a row is read once.
        references.normalize();
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
                    // The committed entries are the delivery payload: observers
                    // are fed from memory instead of paging the journal back
                    // out of the backend.
                    release_committed_batch(store, pending, committed_batch(entries), deliveries);
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

/// Turn the entries this transaction committed into the delivery payload.
///
/// Entries without committed state advance the observer cursor but carry
/// nothing to deliver, so they bound the range without joining `commits`.
fn committed_batch(entries: Vec<ProcessJournalEntry>) -> Option<CommittedBatch> {
    let first = entries.first()?.cursor;
    let last = entries.last()?.cursor;
    let commits = entries
        .into_iter()
        .filter_map(|entry| {
            entry.committed_state.map(|state| ProcessJournalCommit {
                state: *state,
                kind: entry.kind,
                sanitized_reason: entry.sanitized_reason,
            })
        })
        .collect();
    Some(CommittedBatch {
        first,
        last,
        commits,
    })
}

/// Answer a committed batch, holding back callers that require read-your-writes
/// until this batch has been delivered to every registered observer.
fn release_committed_batch<F>(
    store: &ProcessJournalStore<F>,
    pending: &mut [PendingCommand],
    committed: Option<CommittedBatch>,
    deliveries: &mpsc::UnboundedSender<DeliveryBatch>,
) where
    F: RootFilesystem + Send + Sync + 'static,
{
    let deliver = committed.is_some() && !store.registered_observers().is_empty();
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
    let Some(committed) = committed.filter(|_| deliver) else {
        return;
    };
    let cursor = committed.last.0;
    let batch = DeliveryBatch { committed, waiters };
    if let Err(undeliverable) = deliveries.send(batch) {
        tracing::warn!(
            cursor,
            "process journal delivery task is gone; committed batch will not be delivered"
        );
        undeliverable.0.release();
    }
}
