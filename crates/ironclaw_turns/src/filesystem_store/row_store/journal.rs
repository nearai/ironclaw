use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};

use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, EventRecord, FilesystemError, RootFilesystem,
    ScopedFilesystem, SeqNo,
};
use ironclaw_host_api::ResourceScope;
use tokio::sync::{Mutex as AsyncMutex, mpsc, oneshot};

use crate::TurnError;

use super::{
    ACTIVE_LOCK_ROWS, ADMISSION_RESERVATION_ROWS, CHECKPOINT_ROWS, EVENT_ROWS, IDEMPOTENCY_ROWS,
    LOOP_CHECKPOINT_ROWS, RUN_ROWS, SPAWN_TREE_RESERVATION_ROWS, TURN_ROWS,
    delta::{
        RowStoreMeta, SnapshotDelta, active_lock_record_key, admission_reservation_record_key,
        checkpoint_record_key, event_record_key, idempotency_record_key,
        loop_checkpoint_record_key, run_record_key, spawn_tree_reservation_record_key,
        turn_record_key,
    },
    io::{delta_log_path, deserialize_row, fs_error, meta_path, row_path},
};

const DELTA_JOURNAL_MAX_BATCH: usize = 256;
const DELTA_JOURNAL_MATERIALIZE_IDLE_DELAY: Duration = Duration::from_millis(25);
static MATERIALIZE_GATE: OnceLock<Arc<AsyncMutex<()>>> = OnceLock::new();

pub(super) type DeltaAck = oneshot::Receiver<Result<(), TurnError>>;

pub(super) struct DeltaJournal {
    sender: mpsc::UnboundedSender<DeltaJournalRequest>,
}

struct DeltaJournalRequest {
    delta: SnapshotDelta,
    ack: oneshot::Sender<Result<(), TurnError>>,
}

impl DeltaJournal {
    pub(super) fn new<F>(filesystem: Arc<ScopedFilesystem<F>>) -> Self
    where
        F: RootFilesystem + 'static,
    {
        let (sender, receiver) = mpsc::unbounded_channel();
        let (materialize_sender, materialize_receiver) = mpsc::unbounded_channel();
        tokio::spawn(run_delta_journal_materializer(
            Arc::clone(&filesystem),
            materialize_receiver,
        ));
        tokio::spawn(run_delta_journal_flusher(
            filesystem,
            receiver,
            materialize_sender,
        ));
        Self { sender }
    }

    pub(super) fn enqueue(&self, delta: SnapshotDelta) -> Result<Option<DeltaAck>, TurnError> {
        if delta.is_empty() {
            return Ok(None);
        }
        let (ack, receiver) = oneshot::channel();
        self.sender
            .send(DeltaJournalRequest { delta, ack })
            .map_err(|_| delta_journal_stopped())?;
        Ok(Some(receiver))
    }

    pub(super) async fn await_ack(ack: Option<DeltaAck>) -> Result<(), TurnError> {
        let Some(ack) = ack else {
            return Ok(());
        };
        ack.await.map_err(|_| delta_journal_stopped())?
    }
}

async fn run_delta_journal_flusher<F>(
    filesystem: Arc<ScopedFilesystem<F>>,
    mut receiver: mpsc::UnboundedReceiver<DeltaJournalRequest>,
    materialize_sender: mpsc::UnboundedSender<SeqNo>,
) where
    F: RootFilesystem,
{
    while let Some(first) = receiver.recv().await {
        let mut requests = Vec::with_capacity(DELTA_JOURNAL_MAX_BATCH);
        requests.push(first);
        tokio::task::yield_now().await;
        while requests.len() < DELTA_JOURNAL_MAX_BATCH {
            match receiver.try_recv() {
                Ok(request) => requests.push(request),
                Err(mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected) => {
                    break;
                }
            }
        }
        let result = append_delta_journal_batch(filesystem.as_ref(), &requests).await;
        match result {
            Ok(seqs) => {
                let target_seq = seqs.iter().copied().max();
                for request in requests {
                    let _ = request.ack.send(Ok(()));
                }
                if let Some(target_seq) = target_seq {
                    let _ = materialize_sender.send(target_seq);
                }
            }
            Err(error) => {
                for request in requests {
                    let _ = request.ack.send(Err(error.clone()));
                }
            }
        }
    }
}

async fn run_delta_journal_materializer<F>(
    filesystem: Arc<ScopedFilesystem<F>>,
    mut receiver: mpsc::UnboundedReceiver<SeqNo>,
) where
    F: RootFilesystem,
{
    while let Some(mut target_seq) = receiver.recv().await {
        loop {
            tokio::time::sleep(DELTA_JOURNAL_MATERIALIZE_IDLE_DELAY).await;
            let mut saw_new_target = false;
            while let Ok(next_seq) = receiver.try_recv() {
                target_seq = target_seq.max(next_seq);
                saw_new_target = true;
            }
            if !saw_new_target {
                break;
            }
        }
        if let Err(error) = materialize_delta_batch(filesystem.as_ref(), &target_seq).await {
            tracing::warn!(
                error = %error,
                target_seq = target_seq.get(),
                "turn-state row materialization failed after durable delta append",
            );
        }
    }
}

async fn append_delta_journal_batch<F>(
    filesystem: &ScopedFilesystem<F>,
    requests: &[DeltaJournalRequest],
) -> Result<Vec<SeqNo>, TurnError>
where
    F: RootFilesystem,
{
    let path = delta_log_path()?;
    let mut payloads = Vec::with_capacity(requests.len());
    for request in requests {
        payloads.push(serde_json::to_vec(&request.delta).map_err(|error| {
            TurnError::Unavailable {
                reason: format!("turn-state delta serialization failed: {error}"),
            }
        })?);
    }
    let seqs = if let [payload] = payloads.as_slice() {
        vec![
            filesystem
                .append(&ResourceScope::system(), &path, payload.clone())
                .await
                .map_err(fs_error)?,
        ]
    } else {
        filesystem
            .append_batch(&ResourceScope::system(), &path, payloads)
            .await
            .map_err(fs_error)?
    };
    if seqs.len() != requests.len() {
        return Err(TurnError::Unavailable {
            reason: "turn-state delta batch append returned an unexpected ack count".to_string(),
        });
    }
    Ok(seqs)
}

async fn materialize_delta_batch<F>(
    filesystem: &ScopedFilesystem<F>,
    target_seq: &SeqNo,
) -> Result<(), TurnError>
where
    F: RootFilesystem,
{
    materialize_delta_log(filesystem, Some(*target_seq)).await
}

pub(super) async fn materialize_delta_log<F>(
    filesystem: &ScopedFilesystem<F>,
    target_seq: Option<SeqNo>,
) -> Result<(), TurnError>
where
    F: RootFilesystem,
{
    let gate = materialize_gate();
    let _guard = gate.lock().await;
    materialize_delta_log_unlocked(filesystem, target_seq).await
}

async fn materialize_delta_log_unlocked<F>(
    filesystem: &ScopedFilesystem<F>,
    target_seq: Option<SeqNo>,
) -> Result<(), TurnError>
where
    F: RootFilesystem,
{
    let mut meta = read_meta(filesystem).await?;
    if let Some(target_seq) = target_seq
        && target_seq <= meta.journal_seq
    {
        return Ok(());
    }
    let path = delta_log_path()?;
    let max_records = target_seq
        .map(|target_seq| target_seq.get().saturating_sub(meta.journal_seq.get()) as usize)
        .unwrap_or(usize::MAX);
    let records = match filesystem
        .tail_bounded(
            &ResourceScope::system(),
            &path,
            meta.journal_seq,
            max_records,
        )
        .await
    {
        Ok(records) => records,
        Err(FilesystemError::NotFound { .. }) | Err(FilesystemError::Unsupported { .. }) => {
            Vec::new()
        }
        Err(error) => return Err(fs_error(error)),
    };
    for record in records {
        if target_seq.is_some_and(|target_seq| record.seq > target_seq) {
            break;
        }
        materialize_delta_record(filesystem, &mut meta, record).await?;
    }
    Ok(())
}

fn materialize_gate() -> Arc<AsyncMutex<()>> {
    Arc::clone(MATERIALIZE_GATE.get_or_init(|| Arc::new(AsyncMutex::new(()))))
}

async fn materialize_delta_record<F>(
    filesystem: &ScopedFilesystem<F>,
    meta: &mut RowStoreMeta,
    record: EventRecord,
) -> Result<(), TurnError>
where
    F: RootFilesystem,
{
    let delta: SnapshotDelta =
        serde_json::from_slice(&record.payload).map_err(|error| TurnError::Unavailable {
            reason: format!("turn-state delta deserialization failed: {error}"),
        })?;
    materialize_delta(filesystem, &delta).await?;
    if let Some(floor) = delta.event_retention_floor {
        meta.event_retention_floor = meta.event_retention_floor.max(floor);
    }
    meta.journal_seq = record.seq;
    write_meta(filesystem, meta).await
}

async fn materialize_delta<F>(
    filesystem: &ScopedFilesystem<F>,
    delta: &SnapshotDelta,
) -> Result<(), TurnError>
where
    F: RootFilesystem,
{
    for record in &delta.turns_upsert {
        put_row(filesystem, TURN_ROWS, &turn_record_key(record)?, record).await?;
    }
    for key in &delta.turns_delete {
        delete_row(filesystem, TURN_ROWS, key).await?;
    }
    for record in &delta.runs_upsert {
        put_row(filesystem, RUN_ROWS, &run_record_key(record)?, record).await?;
    }
    for key in &delta.runs_delete {
        delete_row(filesystem, RUN_ROWS, key).await?;
    }
    for record in &delta.active_locks_upsert {
        put_row(
            filesystem,
            ACTIVE_LOCK_ROWS,
            &active_lock_record_key(record)?,
            record,
        )
        .await?;
    }
    for key in &delta.active_locks_delete {
        delete_row(filesystem, ACTIVE_LOCK_ROWS, key).await?;
    }
    for record in &delta.checkpoints_upsert {
        put_row(
            filesystem,
            CHECKPOINT_ROWS,
            &checkpoint_record_key(record)?,
            record,
        )
        .await?;
    }
    for key in &delta.checkpoints_delete {
        delete_row(filesystem, CHECKPOINT_ROWS, key).await?;
    }
    for record in &delta.loop_checkpoints_upsert {
        put_row(
            filesystem,
            LOOP_CHECKPOINT_ROWS,
            &loop_checkpoint_record_key(record)?,
            record,
        )
        .await?;
    }
    for key in &delta.loop_checkpoints_delete {
        delete_row(filesystem, LOOP_CHECKPOINT_ROWS, key).await?;
    }
    for record in &delta.idempotency_upsert {
        put_row(
            filesystem,
            IDEMPOTENCY_ROWS,
            &idempotency_record_key(record)?,
            record,
        )
        .await?;
    }
    for key in &delta.idempotency_delete {
        delete_row(filesystem, IDEMPOTENCY_ROWS, key).await?;
    }
    for record in &delta.events_upsert {
        put_row(filesystem, EVENT_ROWS, &event_record_key(record)?, record).await?;
    }
    for key in &delta.events_delete {
        delete_row(filesystem, EVENT_ROWS, key).await?;
    }
    for record in &delta.admission_reservations_upsert {
        put_row(
            filesystem,
            ADMISSION_RESERVATION_ROWS,
            &admission_reservation_record_key(record)?,
            record,
        )
        .await?;
    }
    for key in &delta.admission_reservations_delete {
        delete_row(filesystem, ADMISSION_RESERVATION_ROWS, key).await?;
    }
    for record in &delta.spawn_tree_reservations_upsert {
        put_row(
            filesystem,
            SPAWN_TREE_RESERVATION_ROWS,
            &spawn_tree_reservation_record_key(record)?,
            record,
        )
        .await?;
    }
    for key in &delta.spawn_tree_reservations_delete {
        delete_row(filesystem, SPAWN_TREE_RESERVATION_ROWS, key).await?;
    }
    Ok(())
}

async fn put_row<F, T>(
    filesystem: &ScopedFilesystem<F>,
    collection: &'static str,
    key: &str,
    record: &T,
) -> Result<(), TurnError>
where
    F: RootFilesystem,
    T: serde::Serialize,
{
    let body = serde_json::to_vec(record).map_err(|error| TurnError::Unavailable {
        reason: format!("turn-state {collection} row serialization failed: {error}"),
    })?;
    let entry = Entry::bytes(body).with_content_type(ContentType::json());
    filesystem
        .put(
            &ResourceScope::system(),
            &row_path(collection, key)?,
            entry,
            CasExpectation::Any,
        )
        .await
        .map_err(fs_error)?;
    Ok(())
}

async fn delete_row<F>(
    filesystem: &ScopedFilesystem<F>,
    collection: &'static str,
    key: &str,
) -> Result<(), TurnError>
where
    F: RootFilesystem,
{
    match filesystem
        .delete(&ResourceScope::system(), &row_path(collection, key)?)
        .await
    {
        Ok(()) | Err(FilesystemError::NotFound { .. }) => Ok(()),
        Err(error) => Err(fs_error(error)),
    }
}

async fn read_meta<F>(filesystem: &ScopedFilesystem<F>) -> Result<RowStoreMeta, TurnError>
where
    F: RootFilesystem,
{
    match filesystem
        .get(&ResourceScope::system(), &meta_path()?)
        .await
    {
        Ok(Some(versioned)) => deserialize_row(&versioned.entry.body, "turn-state row meta"),
        Ok(None) | Err(FilesystemError::NotFound { .. }) => Ok(RowStoreMeta::default()),
        Err(error) => Err(fs_error(error)),
    }
}

async fn write_meta<F>(
    filesystem: &ScopedFilesystem<F>,
    meta: &RowStoreMeta,
) -> Result<(), TurnError>
where
    F: RootFilesystem,
{
    let body = serde_json::to_vec(meta).map_err(|error| TurnError::Unavailable {
        reason: format!("turn-state row meta serialization failed: {error}"),
    })?;
    let entry = Entry::bytes(body).with_content_type(ContentType::json());
    filesystem
        .put(
            &ResourceScope::system(),
            &meta_path()?,
            entry,
            CasExpectation::Any,
        )
        .await
        .map_err(fs_error)?;
    Ok(())
}

fn delta_journal_stopped() -> TurnError {
    TurnError::Unavailable {
        reason: "turn-state delta journal stopped".to_string(),
    }
}
