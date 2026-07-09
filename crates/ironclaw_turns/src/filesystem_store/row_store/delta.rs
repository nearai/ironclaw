use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use chrono::Utc;
use ironclaw_filesystem::SeqNo;
use serde::Serialize;

use crate::{
    EventCursor, IdempotencyKey, InMemoryTurnStateStore, InMemoryTurnStateStoreLimits,
    LoopCheckpointRecord, PutLoopCheckpointRequest, SpawnTreeReservation, SubmitTurnResponse,
    TurnActiveLockRecord, TurnAdmissionReservationRecord, TurnCheckpointId, TurnCheckpointRecord,
    TurnError, TurnIdempotencyOperationKind, TurnIdempotencyOutcomeKind, TurnIdempotencyRecord,
    TurnIdempotencyReplay, TurnLifecycleEvent, TurnPersistenceSnapshot, TurnRecord, TurnRunId,
    TurnRunRecord, TurnRunState, TurnScope, runner::ClaimedTurnRun,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub(super) struct RowStoreMeta {
    pub(super) event_retention_floor: EventCursor,
    #[serde(default = "zero_seq_no")]
    pub(super) journal_seq: SeqNo,
}

impl Default for RowStoreMeta {
    fn default() -> Self {
        Self {
            event_retention_floor: EventCursor::default(),
            journal_seq: SeqNo::ZERO,
        }
    }
}

fn zero_seq_no() -> SeqNo {
    SeqNo::ZERO
}

pub(super) struct RowSnapshotState {
    pub(super) snapshot: TurnPersistenceSnapshot,
    pub(super) store: Arc<InMemoryTurnStateStore>,
    pub(super) journal_seq: SeqNo,
    latest_event_cursor: EventCursor,
    indexes: RowSnapshotIndexes,
}

impl RowSnapshotState {
    pub(super) fn new(
        snapshot: TurnPersistenceSnapshot,
        store: Arc<InMemoryTurnStateStore>,
        journal_seq: SeqNo,
    ) -> Result<Self, TurnError> {
        let latest_event_cursor = latest_event_cursor(&snapshot);
        let indexes = RowSnapshotIndexes::from_snapshot(&snapshot)?;
        Ok(Self {
            snapshot,
            store,
            journal_seq,
            latest_event_cursor,
            indexes,
        })
    }

    pub(super) fn latest_event_cursor(&self) -> EventCursor {
        self.latest_event_cursor
    }

    pub(super) fn run_record(&self, scope: &TurnScope, run_id: TurnRunId) -> Option<TurnRunRecord> {
        let key = run_id.to_string();
        let index = self.indexes.runs.get(&key).copied()?;
        let record = self.snapshot.runs.get(index)?.clone();
        (record.scope == *scope).then_some(record)
    }

    pub(super) fn run_record_by_id(&self, run_id: TurnRunId) -> Option<TurnRunRecord> {
        let key = run_id.to_string();
        let index = self.indexes.runs.get(&key).copied()?;
        self.snapshot.runs.get(index).cloned()
    }

    pub(super) fn turn_record_for_run(
        &self,
        scope: &TurnScope,
        run: &TurnRunRecord,
    ) -> Result<TurnRecord, TurnError> {
        let key = run.turn_id.to_string();
        let Some(index) = self.indexes.turns.get(&key).copied() else {
            return Err(TurnError::Unavailable {
                reason: "turn run references missing cached turn row".to_string(),
            });
        };
        let Some(record) = self.snapshot.turns.get(index).cloned() else {
            return Err(TurnError::Unavailable {
                reason: "row-store snapshot turn index is out of bounds".to_string(),
            });
        };
        if record.scope != *scope {
            return Err(TurnError::Unavailable {
                reason: "turn run references turn row outside requested scope".to_string(),
            });
        }
        Ok(record)
    }

    pub(super) fn apply_delta(
        &mut self,
        delta: SnapshotDelta,
        journal_seq: SeqNo,
    ) -> Result<(), TurnError> {
        let latest_event_cursor = latest_event_cursor_after_delta(self.latest_event_cursor, &delta);
        apply_delta_indexed(&mut self.snapshot, &mut self.indexes, delta)?;
        self.journal_seq = self.journal_seq.max(journal_seq);
        self.latest_event_cursor = latest_event_cursor;
        Ok(())
    }
}

#[derive(Debug, Default)]
struct RowSnapshotIndexes {
    turns: HashMap<String, usize>,
    runs: HashMap<String, usize>,
    active_locks: HashMap<String, usize>,
    checkpoints: HashMap<String, usize>,
    loop_checkpoints: HashMap<String, usize>,
    idempotency_records: HashMap<String, usize>,
    events: HashMap<String, usize>,
    admission_reservations: HashMap<String, usize>,
    spawn_tree_reservations: HashMap<String, usize>,
}

impl RowSnapshotIndexes {
    fn from_snapshot(snapshot: &TurnPersistenceSnapshot) -> Result<Self, TurnError> {
        Ok(Self {
            turns: indexed_records(&snapshot.turns, &turn_record_key)?,
            runs: indexed_records(&snapshot.runs, &run_record_key)?,
            active_locks: indexed_records(&snapshot.active_locks, &active_lock_record_key)?,
            checkpoints: indexed_records(&snapshot.checkpoints, &checkpoint_record_key)?,
            loop_checkpoints: indexed_records(
                &snapshot.loop_checkpoints,
                &loop_checkpoint_record_key,
            )?,
            idempotency_records: indexed_records(
                &snapshot.idempotency_records,
                &idempotency_record_key,
            )?,
            events: indexed_records(&snapshot.events, &event_record_key)?,
            admission_reservations: indexed_records(
                &snapshot.admission_reservations,
                &admission_reservation_record_key,
            )?,
            spawn_tree_reservations: indexed_records(
                &snapshot.spawn_tree_reservations,
                &spawn_tree_reservation_record_key,
            )?,
        })
    }
}

#[derive(Debug, Clone, Default, Serialize, serde::Deserialize)]
pub(super) struct SnapshotDelta {
    pub(super) turns_upsert: Vec<TurnRecord>,
    pub(super) turns_delete: Vec<String>,
    pub(super) runs_upsert: Vec<TurnRunRecord>,
    pub(super) runs_delete: Vec<String>,
    pub(super) active_locks_upsert: Vec<TurnActiveLockRecord>,
    pub(super) active_locks_delete: Vec<String>,
    pub(super) checkpoints_upsert: Vec<TurnCheckpointRecord>,
    pub(super) checkpoints_delete: Vec<String>,
    pub(super) loop_checkpoints_upsert: Vec<LoopCheckpointRecord>,
    pub(super) loop_checkpoints_delete: Vec<String>,
    pub(super) idempotency_upsert: Vec<TurnIdempotencyRecord>,
    pub(super) idempotency_delete: Vec<String>,
    pub(super) events_upsert: Vec<TurnLifecycleEvent>,
    pub(super) events_delete: Vec<String>,
    pub(super) admission_reservations_upsert: Vec<TurnAdmissionReservationRecord>,
    pub(super) admission_reservations_delete: Vec<String>,
    pub(super) spawn_tree_reservations_upsert: Vec<SpawnTreeReservation>,
    pub(super) spawn_tree_reservations_delete: Vec<String>,
    pub(super) event_retention_floor: Option<EventCursor>,
}

impl SnapshotDelta {
    pub(super) fn set_turns_upsert(mut self, turns_upsert: Vec<TurnRecord>) -> Self {
        self.turns_upsert = turns_upsert;
        self
    }

    pub(super) fn set_runs_upsert(mut self, runs_upsert: Vec<TurnRunRecord>) -> Self {
        self.runs_upsert = runs_upsert;
        self
    }

    pub(super) fn set_loop_checkpoints_upsert(
        mut self,
        loop_checkpoints_upsert: Vec<LoopCheckpointRecord>,
    ) -> Self {
        self.loop_checkpoints_upsert = loop_checkpoints_upsert;
        self
    }

    pub(super) fn is_empty(&self) -> bool {
        self.turns_upsert.is_empty()
            && self.turns_delete.is_empty()
            && self.runs_upsert.is_empty()
            && self.runs_delete.is_empty()
            && self.active_locks_upsert.is_empty()
            && self.active_locks_delete.is_empty()
            && self.checkpoints_upsert.is_empty()
            && self.checkpoints_delete.is_empty()
            && self.loop_checkpoints_upsert.is_empty()
            && self.loop_checkpoints_delete.is_empty()
            && self.idempotency_upsert.is_empty()
            && self.idempotency_delete.is_empty()
            && self.events_upsert.is_empty()
            && self.events_delete.is_empty()
            && self.admission_reservations_upsert.is_empty()
            && self.admission_reservations_delete.is_empty()
            && self.spawn_tree_reservations_upsert.is_empty()
            && self.spawn_tree_reservations_delete.is_empty()
            && self.event_retention_floor.is_none()
    }
}

pub(super) enum RowPersistError {
    Turn(TurnError),
}

impl RowPersistError {
    pub(super) fn into_turn(self) -> TurnError {
        match self {
            Self::Turn(error) => error,
        }
    }
}

impl From<TurnError> for RowPersistError {
    fn from(error: TurnError) -> Self {
        Self::Turn(error)
    }
}

#[derive(Serialize)]
struct SpawnTreeReservationKeyForPath<'a> {
    scope: &'a TurnScope,
    root_run_id: TurnRunId,
}

pub(super) fn turn_record_key(record: &TurnRecord) -> Result<String, TurnError> {
    Ok(record.turn_id.to_string())
}

pub(super) fn run_record_key(record: &TurnRunRecord) -> Result<String, TurnError> {
    Ok(record.run_id.to_string())
}

pub(super) fn active_lock_record_key(record: &TurnActiveLockRecord) -> Result<String, TurnError> {
    hash_key(&record.key)
}

pub(super) fn checkpoint_record_key(record: &TurnCheckpointRecord) -> Result<String, TurnError> {
    Ok(record.checkpoint_id.as_uuid().to_string())
}

pub(super) fn loop_checkpoint_record_key(
    record: &LoopCheckpointRecord,
) -> Result<String, TurnError> {
    Ok(record.checkpoint_id.as_uuid().to_string())
}

pub(super) fn idempotency_record_key(record: &TurnIdempotencyRecord) -> Result<String, TurnError> {
    hash_key(record)
}

pub(super) fn event_record_key(record: &TurnLifecycleEvent) -> Result<String, TurnError> {
    Ok(format!("{:020}", record.cursor.0))
}

pub(super) fn admission_reservation_record_key(
    record: &TurnAdmissionReservationRecord,
) -> Result<String, TurnError> {
    Ok(record.run_id.to_string())
}

pub(super) fn spawn_tree_reservation_record_key(
    record: &SpawnTreeReservation,
) -> Result<String, TurnError> {
    hash_key(&SpawnTreeReservationKeyForPath {
        scope: &record.scope,
        root_run_id: record.root_run_id,
    })
}

pub(super) fn snapshot_delta(
    old: &TurnPersistenceSnapshot,
    new: &TurnPersistenceSnapshot,
) -> Result<SnapshotDelta, RowPersistError> {
    let (turns_upsert, turns_delete) = delta_collection(&old.turns, &new.turns, |record| {
        Ok(record.turn_id.to_string())
    })?;
    let (runs_upsert, runs_delete) =
        delta_collection(&old.runs, &new.runs, |record| Ok(record.run_id.to_string()))?;
    let (active_locks_upsert, active_locks_delete) =
        delta_collection(&old.active_locks, &new.active_locks, |record| {
            hash_key(&record.key)
        })?;
    let (checkpoints_upsert, checkpoints_delete) =
        delta_collection(&old.checkpoints, &new.checkpoints, |record| {
            Ok(record.checkpoint_id.as_uuid().to_string())
        })?;
    let (loop_checkpoints_upsert, loop_checkpoints_delete) =
        delta_collection(&old.loop_checkpoints, &new.loop_checkpoints, |record| {
            Ok(record.checkpoint_id.as_uuid().to_string())
        })?;
    let (idempotency_upsert, idempotency_delete) =
        delta_collection(&old.idempotency_records, &new.idempotency_records, hash_key)?;
    let (events_upsert, events_delete) = delta_collection(&old.events, &new.events, |record| {
        Ok(format!("{:020}", record.cursor.0))
    })?;
    let (admission_reservations_upsert, admission_reservations_delete) = delta_collection(
        &old.admission_reservations,
        &new.admission_reservations,
        |record| Ok(record.run_id.to_string()),
    )?;
    let (spawn_tree_reservations_upsert, spawn_tree_reservations_delete) = delta_collection(
        &old.spawn_tree_reservations,
        &new.spawn_tree_reservations,
        |record| {
            hash_key(&SpawnTreeReservationKeyForPath {
                scope: &record.scope,
                root_run_id: record.root_run_id,
            })
        },
    )?;

    Ok(SnapshotDelta {
        turns_upsert,
        turns_delete,
        runs_upsert,
        runs_delete,
        active_locks_upsert,
        active_locks_delete,
        checkpoints_upsert,
        checkpoints_delete,
        loop_checkpoints_upsert,
        loop_checkpoints_delete,
        idempotency_upsert,
        idempotency_delete,
        events_upsert,
        events_delete,
        admission_reservations_upsert,
        admission_reservations_delete,
        spawn_tree_reservations_upsert,
        spawn_tree_reservations_delete,
        event_retention_floor: (old.event_retention_floor != new.event_retention_floor)
            .then_some(new.event_retention_floor),
    })
}

fn apply_delta_indexed(
    snapshot: &mut TurnPersistenceSnapshot,
    indexes: &mut RowSnapshotIndexes,
    delta: SnapshotDelta,
) -> Result<(), TurnError> {
    if !delta.turns_upsert.is_empty() || !delta.turns_delete.is_empty() {
        apply_delta_collection_indexed(
            &mut snapshot.turns,
            &mut indexes.turns,
            delta.turns_upsert,
            delta.turns_delete,
            turn_record_key,
        )?;
    }
    if !delta.runs_upsert.is_empty() || !delta.runs_delete.is_empty() {
        apply_delta_collection_indexed(
            &mut snapshot.runs,
            &mut indexes.runs,
            delta.runs_upsert,
            delta.runs_delete,
            run_record_key,
        )?;
    }
    if !delta.active_locks_upsert.is_empty() || !delta.active_locks_delete.is_empty() {
        apply_delta_collection_indexed(
            &mut snapshot.active_locks,
            &mut indexes.active_locks,
            delta.active_locks_upsert,
            delta.active_locks_delete,
            active_lock_record_key,
        )?;
    }
    if !delta.checkpoints_upsert.is_empty() || !delta.checkpoints_delete.is_empty() {
        apply_delta_collection_indexed(
            &mut snapshot.checkpoints,
            &mut indexes.checkpoints,
            delta.checkpoints_upsert,
            delta.checkpoints_delete,
            checkpoint_record_key,
        )?;
    }
    if !delta.loop_checkpoints_upsert.is_empty() || !delta.loop_checkpoints_delete.is_empty() {
        apply_delta_collection_indexed(
            &mut snapshot.loop_checkpoints,
            &mut indexes.loop_checkpoints,
            delta.loop_checkpoints_upsert,
            delta.loop_checkpoints_delete,
            loop_checkpoint_record_key,
        )?;
    }
    if !delta.idempotency_upsert.is_empty() || !delta.idempotency_delete.is_empty() {
        apply_delta_collection_indexed(
            &mut snapshot.idempotency_records,
            &mut indexes.idempotency_records,
            delta.idempotency_upsert,
            delta.idempotency_delete,
            idempotency_record_key,
        )?;
    }
    if !delta.events_upsert.is_empty() || !delta.events_delete.is_empty() {
        apply_delta_collection_indexed(
            &mut snapshot.events,
            &mut indexes.events,
            delta.events_upsert,
            delta.events_delete,
            event_record_key,
        )?;
    }
    if !delta.admission_reservations_upsert.is_empty()
        || !delta.admission_reservations_delete.is_empty()
    {
        apply_delta_collection_indexed(
            &mut snapshot.admission_reservations,
            &mut indexes.admission_reservations,
            delta.admission_reservations_upsert,
            delta.admission_reservations_delete,
            admission_reservation_record_key,
        )?;
    }
    if !delta.spawn_tree_reservations_upsert.is_empty()
        || !delta.spawn_tree_reservations_delete.is_empty()
    {
        apply_delta_collection_indexed(
            &mut snapshot.spawn_tree_reservations,
            &mut indexes.spawn_tree_reservations,
            delta.spawn_tree_reservations_upsert,
            delta.spawn_tree_reservations_delete,
            spawn_tree_reservation_record_key,
        )?;
    }
    if let Some(event_retention_floor) = delta.event_retention_floor {
        snapshot.event_retention_floor = event_retention_floor;
    }
    Ok(())
}

fn delta_collection<T, K>(
    old: &[T],
    new: &[T],
    key_fn: K,
) -> Result<(Vec<T>, Vec<String>), RowPersistError>
where
    T: Clone + PartialEq,
    K: Fn(&T) -> Result<String, TurnError>,
{
    let old_map = keyed_records(old, &key_fn)?;
    let new_map = keyed_records(new, &key_fn)?;
    let upsert = new_map
        .iter()
        .filter(|(key, record)| old_map.get(*key) != Some(*record))
        .map(|(_key, record)| record.clone())
        .collect();
    let new_keys = new_map.keys().cloned().collect::<HashSet<_>>();
    let delete = old_map
        .keys()
        .filter(|key| !new_keys.contains(*key))
        .cloned()
        .collect();
    Ok((upsert, delete))
}

fn apply_delta_collection_indexed<T, K>(
    records: &mut Vec<T>,
    index: &mut HashMap<String, usize>,
    upsert: Vec<T>,
    delete: Vec<String>,
    key_fn: K,
) -> Result<(), TurnError>
where
    K: Fn(&T) -> Result<String, TurnError>,
{
    if !delete.is_empty() {
        let deleted = delete.into_iter().collect::<HashSet<_>>();
        let mut retained = Vec::with_capacity(records.len());
        for record in records.drain(..) {
            if !deleted.contains(&key_fn(&record)?) {
                retained.push(record);
            }
        }
        *records = retained;
        *index = indexed_records(records, &key_fn)?;
    }

    for record in upsert {
        let key = key_fn(&record)?;
        if let Some(record_index) = index.get(&key).copied() {
            if record_index >= records.len() {
                return Err(TurnError::Unavailable {
                    reason: "row-store snapshot index is out of bounds".to_string(),
                });
            }
            records[record_index] = record;
        } else {
            let record_index = records.len();
            records.push(record);
            index.insert(key, record_index);
        }
    }
    Ok(())
}

fn indexed_records<T, K>(records: &[T], key_fn: &K) -> Result<HashMap<String, usize>, TurnError>
where
    K: Fn(&T) -> Result<String, TurnError>,
{
    let mut index = HashMap::with_capacity(records.len());
    for (record_index, record) in records.iter().enumerate() {
        index.insert(key_fn(record)?, record_index);
    }
    Ok(index)
}

pub(super) fn keyed_records<T, K>(
    records: &[T],
    key_fn: &K,
) -> Result<HashMap<String, T>, RowPersistError>
where
    T: Clone,
    K: Fn(&T) -> Result<String, TurnError>,
{
    records
        .iter()
        .map(|record| Ok((key_fn(record)?, record.clone())))
        .collect()
}

pub(super) fn submit_turn_targeted_delta(
    snapshot: &TurnPersistenceSnapshot,
    latest_event_cursor: EventCursor,
    store: &InMemoryTurnStateStore,
    response: &SubmitTurnResponse,
    idempotency_key: &IdempotencyKey,
) -> Result<SnapshotDelta, TurnError> {
    let SubmitTurnResponse::Accepted {
        turn_id, run_id, ..
    } = response;
    let turn = store
        .turn_record(*turn_id)
        .ok_or_else(|| TurnError::Unavailable {
            reason: "accepted turn missing from row-store hot state".to_string(),
        })?;
    let run = store
        .run_record(*run_id)
        .ok_or_else(|| TurnError::Unavailable {
            reason: "accepted run missing from row-store hot state".to_string(),
        })?;
    let mut delta = SnapshotDelta::default()
        .set_turns_upsert(vec![turn.clone()])
        .set_runs_upsert(vec![run]);
    if let Some(lock) = store.active_lock_record(&turn.scope) {
        delta.active_locks_upsert.push(lock);
    }
    if let Some(reservation) = store.admission_reservation(*run_id) {
        delta.admission_reservations_upsert.push(reservation);
    }
    if !snapshot.idempotency_records.iter().any(|record| {
        record.operation == TurnIdempotencyOperationKind::Submit
            && record.scope == turn.scope
            && record.key == *idempotency_key
    }) {
        delta.idempotency_upsert.push(TurnIdempotencyRecord {
            scope: turn.scope.clone(),
            operation: TurnIdempotencyOperationKind::Submit,
            key: idempotency_key.clone(),
            turn_id: Some(*turn_id),
            run_id: Some(*run_id),
            outcome: TurnIdempotencyOutcomeKind::Accepted,
            replay: TurnIdempotencyReplay::SubmitAccepted(response.clone()),
            created_at: turn.created_at,
            expires_at: None,
        });
    }
    add_event_delta(snapshot, latest_event_cursor, store, &mut delta)?;
    Ok(delta)
}

pub(super) fn full_snapshot_delta(
    snapshot: &TurnPersistenceSnapshot,
    store: &InMemoryTurnStateStore,
) -> Result<SnapshotDelta, TurnError> {
    let mut new_snapshot = store.persistence_snapshot();
    preserve_loop_checkpoints(snapshot, &mut new_snapshot);
    snapshot_delta(snapshot, &new_snapshot).map_err(|error| match error {
        RowPersistError::Turn(error) => error,
    })
}

pub(super) fn row_store_durable_delta(mut delta: SnapshotDelta) -> SnapshotDelta {
    // Tier-2 rows are the durable run record. Row-store cache limits may evict
    // old terminal runs/events from memory, but persistence must not encode
    // those cache evictions as data deletion.
    delta.turns_delete.clear();
    delta.runs_delete.clear();
    delta.events_delete.clear();
    delta.event_retention_floor = None;
    // Admission reservations are coordination scaffolding. The in-memory store
    // rebuilds them from non-terminal run records during snapshot load, so
    // keeping them in every submit/complete journal entry only adds write
    // amplification without improving recovery.
    delta.admission_reservations_upsert.clear();
    delta.admission_reservations_delete.clear();
    delta
}

pub(super) fn row_store_hot_cache_snapshot(
    mut snapshot: TurnPersistenceSnapshot,
    limits: InMemoryTurnStateStoreLimits,
) -> TurnPersistenceSnapshot {
    let mut terminal_runs = snapshot
        .runs
        .iter()
        .filter(|record| record.status.is_terminal())
        .map(|record| (record.event_cursor, record.run_id))
        .collect::<Vec<_>>();
    terminal_runs.sort_by_key(|(cursor, _)| *cursor);
    let evicted_terminal_run_ids = terminal_runs
        .len()
        .saturating_sub(limits.max_terminal_records);
    let evicted_terminal_run_ids = terminal_runs
        .into_iter()
        .take(evicted_terminal_run_ids)
        .map(|(_, run_id)| run_id)
        .collect::<HashSet<_>>();

    if !evicted_terminal_run_ids.is_empty() {
        snapshot
            .runs
            .retain(|record| !evicted_terminal_run_ids.contains(&record.run_id));
        let retained_run_ids = snapshot
            .runs
            .iter()
            .map(|record| record.run_id)
            .collect::<HashSet<_>>();
        let retained_turn_ids = snapshot
            .runs
            .iter()
            .map(|record| record.turn_id)
            .collect::<HashSet<_>>();
        let active_spawn_roots = snapshot
            .runs
            .iter()
            .filter(|record| !record.status.is_terminal())
            .filter_map(|record| record.spawn_tree_root_run_id)
            .collect::<HashSet<_>>();

        snapshot
            .turns
            .retain(|record| retained_turn_ids.contains(&record.turn_id));
        snapshot
            .active_locks
            .retain(|record| retained_run_ids.contains(&record.run_id));
        snapshot
            .checkpoints
            .retain(|record| retained_run_ids.contains(&record.run_id));
        snapshot
            .loop_checkpoints
            .retain(|record| retained_run_ids.contains(&record.run_id));
        snapshot
            .admission_reservations
            .retain(|record| retained_run_ids.contains(&record.run_id));
        snapshot.spawn_tree_reservations.retain(|record| {
            retained_run_ids.contains(&record.root_run_id)
                || active_spawn_roots.contains(&record.root_run_id)
        });
    }

    snapshot.events.sort_by_key(|event| event.cursor);
    if snapshot.events.len() > limits.max_events {
        let excess = snapshot.events.len() - limits.max_events;
        if let Some(last_pruned) = snapshot.events.get(excess.saturating_sub(1)) {
            snapshot.event_retention_floor = snapshot.event_retention_floor.max(last_pruned.cursor);
        }
        snapshot.events.drain(0..excess);
    }

    let max_idempotency_records = limits.max_idempotency_records.saturating_mul(3);
    snapshot
        .idempotency_records
        .sort_by_key(|record| record.created_at);
    if snapshot.idempotency_records.len() > max_idempotency_records {
        let excess = snapshot.idempotency_records.len() - max_idempotency_records;
        snapshot.idempotency_records.drain(0..excess);
    }

    snapshot
}

pub(super) fn preserve_loop_checkpoints(
    baseline: &TurnPersistenceSnapshot,
    new_snapshot: &mut TurnPersistenceSnapshot,
) {
    new_snapshot.loop_checkpoints = baseline.loop_checkpoints.clone();
}

pub(super) fn loop_checkpoint_record_from_request(
    request: PutLoopCheckpointRequest,
) -> LoopCheckpointRecord {
    LoopCheckpointRecord {
        checkpoint_id: TurnCheckpointId::new(),
        scope: request.scope,
        turn_id: request.turn_id,
        run_id: request.run_id,
        state_ref: request.state_ref,
        schema_id: request.schema_id,
        schema_version: request.schema_version,
        kind: request.kind,
        gate_ref: request.gate_ref,
        created_at: Utc::now(),
    }
}

pub(super) fn claimed_run_targeted_delta(
    snapshot: &TurnPersistenceSnapshot,
    latest_event_cursor: EventCursor,
    store: &InMemoryTurnStateStore,
    claimed: &Option<ClaimedTurnRun>,
) -> Result<SnapshotDelta, TurnError> {
    let Some(claimed) = claimed else {
        return Ok(SnapshotDelta::default());
    };
    run_state_targeted_delta(
        snapshot,
        latest_event_cursor,
        store,
        claimed.state.run_id,
        &claimed.state.scope,
    )
}

pub(super) fn run_state_targeted_delta(
    snapshot: &TurnPersistenceSnapshot,
    latest_event_cursor: EventCursor,
    store: &InMemoryTurnStateStore,
    run_id: TurnRunId,
    scope: &TurnScope,
) -> Result<SnapshotDelta, TurnError> {
    let mut delta = SnapshotDelta::default();
    match store.run_record(run_id) {
        Some(run) => {
            delta.runs_upsert.push(run);
        }
        None => {
            delta.runs_delete.push(run_id.to_string());
            if let Some(old_run) = snapshot.runs.iter().find(|record| record.run_id == run_id)
                && store.turn_record(old_run.turn_id).is_none()
            {
                delta.turns_delete.push(old_run.turn_id.to_string());
            }
        }
    }

    if let Some(lock) = store.active_lock_record(scope) {
        delta.active_locks_upsert.push(lock);
    } else if let Some(old_lock) = snapshot
        .active_locks
        .iter()
        .find(|record| record.key.scope == *scope)
    {
        delta.active_locks_delete.push(hash_key(&old_lock.key)?);
    }

    if let Some(reservation) = store.admission_reservation(run_id) {
        delta.admission_reservations_upsert.push(reservation);
    } else if snapshot
        .admission_reservations
        .iter()
        .any(|record| record.run_id == run_id)
    {
        delta.admission_reservations_delete.push(run_id.to_string());
    }

    add_event_delta(snapshot, latest_event_cursor, store, &mut delta)?;
    Ok(delta)
}

pub(super) fn run_state_with_idempotency_targeted_delta(
    snapshot: &TurnPersistenceSnapshot,
    latest_event_cursor: EventCursor,
    store: &InMemoryTurnStateStore,
    run_id: TurnRunId,
    scope: &TurnScope,
    operation: crate::TurnIdempotencyOperationKind,
) -> Result<SnapshotDelta, TurnError> {
    let mut delta = run_state_targeted_delta(snapshot, latest_event_cursor, store, run_id, scope)?;
    add_run_idempotency_delta(snapshot, store, &mut delta, run_id, operation);
    Ok(delta)
}

pub(super) fn blocked_run_targeted_delta(
    snapshot: &TurnPersistenceSnapshot,
    latest_event_cursor: EventCursor,
    store: &InMemoryTurnStateStore,
    state: &TurnRunState,
) -> Result<SnapshotDelta, TurnError> {
    let mut delta = run_state_targeted_delta(
        snapshot,
        latest_event_cursor,
        store,
        state.run_id,
        &state.scope,
    )?;
    if let Some(checkpoint_id) = state.checkpoint_id {
        let checkpoint =
            store
                .checkpoint_record(checkpoint_id)
                .ok_or_else(|| TurnError::Unavailable {
                    reason: "blocked run checkpoint missing from row-store hot state".to_string(),
                })?;
        delta.checkpoints_upsert.push(checkpoint);
    }
    Ok(delta)
}

fn add_run_idempotency_delta(
    snapshot: &TurnPersistenceSnapshot,
    store: &InMemoryTurnStateStore,
    delta: &mut SnapshotDelta,
    run_id: TurnRunId,
    operation: crate::TurnIdempotencyOperationKind,
) {
    delta.idempotency_upsert.extend(
        store
            .idempotency_records_for_run_operation(run_id, operation)
            .into_iter()
            .filter(|record| !snapshot.idempotency_records.contains(record)),
    );
}

fn latest_event_cursor(snapshot: &TurnPersistenceSnapshot) -> EventCursor {
    snapshot
        .events
        .iter()
        .map(|event| event.cursor)
        .max()
        .unwrap_or(snapshot.event_retention_floor)
        .max(snapshot.event_retention_floor)
}

fn latest_event_cursor_after_delta(current: EventCursor, delta: &SnapshotDelta) -> EventCursor {
    let event_cursor = delta
        .events_upsert
        .iter()
        .map(|event| event.cursor)
        .max()
        .unwrap_or(current);
    let retention_floor = delta.event_retention_floor.unwrap_or(current);
    current.max(event_cursor).max(retention_floor)
}

fn add_event_delta(
    snapshot: &TurnPersistenceSnapshot,
    latest_event_cursor: EventCursor,
    store: &InMemoryTurnStateStore,
    delta: &mut SnapshotDelta,
) -> Result<(), TurnError> {
    delta
        .events_upsert
        .extend(store.events_after(latest_event_cursor));
    let event_retention_floor = store.event_retention_floor();
    if event_retention_floor != snapshot.event_retention_floor {
        delta.event_retention_floor = Some(event_retention_floor);
        for event in snapshot
            .events
            .iter()
            .filter(|event| event.cursor <= event_retention_floor)
        {
            delta.events_delete.push(format!("{:020}", event.cursor.0));
        }
    }
    Ok(())
}

fn hash_key<T>(record: &T) -> Result<String, TurnError>
where
    T: Serialize,
{
    let bytes = serde_jcs::to_vec(record).map_err(|error| TurnError::Unavailable {
        reason: format!("turn-state row key serialization failed: {error}"),
    })?;
    Ok(hex::encode(blake3::hash(&bytes).as_bytes()))
}
