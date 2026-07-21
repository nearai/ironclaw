//! Durable rehydration, row-collection reads, legacy-blob migration, and the
//! durable-read query paths for [`FilesystemTurnStateRowStore`]. Moved verbatim
//! from the module root during the #6263 decomposition; behavior is unchanged.

use std::sync::Arc;

use futures_util::stream::{self, StreamExt, TryStreamExt};
use ironclaw_filesystem::{FileType, FilesystemError, RootFilesystem, SeqNo};
use ironclaw_host_api::{ResourceScope, UserId};
use serde::de::DeserializeOwned;

use crate::filesystem_store::{io as legacy_blob_io, projection};
use crate::{
    EventCursor, GetLoopCheckpointRequest, GetRunStateRequest, LoopCheckpointRecord, TurnError,
    TurnEventPage, TurnPersistenceSnapshot, TurnRecord, TurnRunRecord, TurnRunState, TurnScope,
    events::project_turn_events,
};

use super::{
    FilesystemTurnStateRowStore, RowCollection,
    delta::{
        RowSnapshotState, RowStoreMeta, active_lock_record_key, event_record_key, keyed_records,
        row_store_hot_cache_snapshot, snapshot_delta,
    },
    io::{
        delta_log_path, deserialize_materialized_row, deserialize_row, fs_error, meta_path,
        row_dir, row_path,
    },
    journal::materialize_delta_log,
};

const ROW_COLLECTION_READ_CONCURRENCY: usize = 32;

impl<F> FilesystemTurnStateRowStore<F>
where
    F: RootFilesystem,
{
    pub(super) async fn load_snapshot_from_rows(&self) -> Result<RowSnapshotState, TurnError> {
        materialize_delta_log(self.filesystem.as_ref(), &self.materialize_gate, None).await?;
        let snapshot = self.read_materialized_row_snapshot().await?;
        let snapshot = self.remove_orphan_active_locks(snapshot).await?;
        let snapshot = self.migrate_legacy_blob_if_needed(snapshot).await?;
        let snapshot = row_store_hot_cache_snapshot(snapshot, self.limits);
        let store = self.build_in_memory_store(snapshot)?;
        let snapshot = store.persistence_snapshot();
        let journal_seq = self.read_meta().await?.journal_seq;
        RowSnapshotState::new(snapshot, Arc::new(store), journal_seq)
    }

    pub(super) async fn read_materialized_row_snapshot(
        &self,
    ) -> Result<TurnPersistenceSnapshot, TurnError> {
        let meta = self.read_meta().await?;
        let turns = self.read_row_collection(RowCollection::Turns).await?;
        let runs = self.read_row_collection(RowCollection::Runs).await?;
        let active_locks = self.read_row_collection(RowCollection::ActiveLocks).await?;
        let checkpoints = self.read_row_collection(RowCollection::Checkpoints).await?;
        let loop_checkpoints = self
            .read_row_collection(RowCollection::LoopCheckpoints)
            .await?;
        let idempotency_records = self.read_row_collection(RowCollection::Idempotency).await?;
        let events = self.read_row_collection(RowCollection::Events).await?;
        let admission_reservations = self
            .read_row_collection(RowCollection::AdmissionReservations)
            .await?;
        let spawn_tree_reservations = self
            .read_row_collection(RowCollection::SpawnTreeReservations)
            .await?;

        Ok(TurnPersistenceSnapshot {
            turns,
            runs,
            active_locks,
            checkpoints,
            loop_checkpoints,
            idempotency_records,
            events,
            event_retention_floor: meta.event_retention_floor,
            admission_reservations,
            spawn_tree_reservations,
        })
    }

    async fn remove_orphan_active_locks(
        &self,
        mut snapshot: TurnPersistenceSnapshot,
    ) -> Result<TurnPersistenceSnapshot, TurnError> {
        let mut retained = Vec::with_capacity(snapshot.active_locks.len());
        for lock in snapshot.active_locks {
            if snapshot.runs.iter().any(|run| run.run_id == lock.run_id) {
                retained.push(lock);
                continue;
            }
            let key = active_lock_record_key(&lock)?;
            let path = row_path(RowCollection::ActiveLocks.as_str(), &key)?;
            match self
                .filesystem
                .delete(&ResourceScope::system(), &path)
                .await
            {
                Ok(()) => {
                    tracing::debug!(
                        active_lock_key = %key,
                        run_id = %lock.run_id,
                        "removed orphan turn-state active-lock row without a durable run row",
                    );
                }
                Err(FilesystemError::NotFound { .. }) => {
                    tracing::debug!(
                        active_lock_key = %key,
                        run_id = %lock.run_id,
                        "orphan turn-state active-lock row disappeared during cleanup",
                    );
                }
                Err(error) => {
                    tracing::debug!(
                        active_lock_key = %key,
                        run_id = %lock.run_id,
                        %error,
                        "failed to remove orphan turn-state active-lock row; continuing with filtered snapshot",
                    );
                }
            }
        }
        snapshot.active_locks = retained;
        Ok(snapshot)
    }

    async fn migrate_legacy_blob_if_needed(
        &self,
        materialized: TurnPersistenceSnapshot,
    ) -> Result<TurnPersistenceSnapshot, TurnError> {
        if materialized != TurnPersistenceSnapshot::default() {
            return Ok(materialized);
        }
        if self.read_meta().await?.journal_seq > SeqNo::ZERO {
            return Ok(materialized);
        }

        let _migration_guard = self.legacy_migration_gate.lock().await;
        materialize_delta_log(self.filesystem.as_ref(), &self.materialize_gate, None).await?;
        let current = self.read_materialized_row_snapshot().await?;
        if current != TurnPersistenceSnapshot::default() {
            return Ok(current);
        }
        if self.read_meta().await?.journal_seq > SeqNo::ZERO {
            return Ok(current);
        }

        let Some(legacy) = self.read_legacy_blob_snapshot().await? else {
            return Ok(current);
        };
        if legacy == TurnPersistenceSnapshot::default() {
            return Ok(current);
        }

        let delta = snapshot_delta(&TurnPersistenceSnapshot::default(), &legacy)?;
        if delta.is_empty() {
            return Ok(current);
        }

        tracing::debug!(
            turns = legacy.turns.len(),
            runs = legacy.runs.len(),
            events = legacy.events.len(),
            active_locks = legacy.active_locks.len(),
            checkpoints = legacy.checkpoints.len(),
            loop_checkpoints = legacy.loop_checkpoints.len(),
            idempotency_records = legacy.idempotency_records.len(),
            "migrating legacy turn-state blob into row store"
        );
        let ack = self.enqueue_delta(delta)?;
        self.await_delta_ack(ack).await?;
        materialize_delta_log(self.filesystem.as_ref(), &self.materialize_gate, None).await?;
        let migrated = self.read_materialized_row_snapshot().await?;
        tracing::debug!(
            turns = migrated.turns.len(),
            runs = migrated.runs.len(),
            events = migrated.events.len(),
            active_locks = migrated.active_locks.len(),
            checkpoints = migrated.checkpoints.len(),
            loop_checkpoints = migrated.loop_checkpoints.len(),
            idempotency_records = migrated.idempotency_records.len(),
            "legacy turn-state blob migration completed"
        );
        Ok(migrated)
    }

    async fn read_legacy_blob_snapshot(
        &self,
    ) -> Result<Option<TurnPersistenceSnapshot>, TurnError> {
        let path = legacy_blob_io::snapshot_path()?;
        match self.filesystem.get(&ResourceScope::system(), &path).await {
            Ok(Some(versioned)) => {
                legacy_blob_io::deserialize_snapshot(&versioned.entry.body).map(Some)
            }
            Ok(None) => Ok(None),
            Err(FilesystemError::NotFound { .. }) => Ok(None),
            Err(error) => Err(fs_error(error)),
        }
    }

    async fn read_meta(&self) -> Result<RowStoreMeta, TurnError> {
        let path = meta_path()?;
        match self.filesystem.get(&ResourceScope::system(), &path).await {
            Ok(Some(versioned)) => deserialize_row(&versioned.entry.body, "turn-state row meta"),
            Ok(None) => Ok(RowStoreMeta::default()),
            Err(error) => Err(fs_error(error)),
        }
    }

    pub(super) async fn delta_log_head_seq(&self) -> Result<SeqNo, TurnError> {
        let path = delta_log_path()?;
        let head = self
            .filesystem
            .head_seq(&ResourceScope::system(), &path, SeqNo::ZERO)
            .await
            .map_err(fs_error)?;
        Ok(head.unwrap_or(SeqNo::ZERO))
    }

    async fn read_row_collection<T>(&self, collection: RowCollection) -> Result<Vec<T>, TurnError>
    where
        T: DeserializeOwned,
    {
        self.read_row_collection_filtered(collection, |_| true)
            .await
    }

    async fn read_row_collection_filtered<T, P>(
        &self,
        collection: RowCollection,
        include_key: P,
    ) -> Result<Vec<T>, TurnError>
    where
        T: DeserializeOwned,
        P: Fn(&str) -> bool,
    {
        let dir = row_dir(collection.as_str())?;
        let entries = match self
            .filesystem
            .list_dir(&ResourceScope::system(), &dir)
            .await
        {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => Vec::new(),
            Err(error) => return Err(fs_error(error)),
        };
        let paths = entries
            .into_iter()
            .filter(|entry| entry.file_type == FileType::File)
            .filter_map(|entry| entry.name.strip_suffix(".json").map(ToString::to_string))
            .filter(|key| include_key(key))
            .map(|key| row_path(collection.as_str(), &key))
            .collect::<Result<Vec<_>, _>>()?;
        let records = stream::iter(paths)
            .map(|path| async move {
                match self.filesystem.get(&ResourceScope::system(), &path).await {
                    Ok(Some(versioned)) => {
                        deserialize_materialized_row(&versioned.entry.body, collection.as_str())
                    }
                    Ok(None) | Err(FilesystemError::NotFound { .. }) => Ok(None),
                    Err(error) => Err(fs_error(error)),
                }
            })
            .buffer_unordered(ROW_COLLECTION_READ_CONCURRENCY)
            .try_collect::<Vec<_>>()
            .await?;
        Ok(records.into_iter().flatten().collect())
    }

    async fn read_row_by_key<T>(
        &self,
        collection: RowCollection,
        key: &str,
    ) -> Result<Option<T>, TurnError>
    where
        T: DeserializeOwned,
    {
        let path = row_path(collection.as_str(), key)?;
        match self.filesystem.get(&ResourceScope::system(), &path).await {
            Ok(Some(versioned)) => {
                deserialize_materialized_row(&versioned.entry.body, collection.as_str())
            }
            Ok(None) | Err(FilesystemError::NotFound { .. }) => Ok(None),
            Err(error) => Err(fs_error(error)),
        }
    }

    pub(super) async fn read_run_state_from_durable_rows(
        &self,
        request: &GetRunStateRequest,
    ) -> Result<Option<TurnRunState>, TurnError> {
        self.flush_pending_write_behind_for_read().await?;
        materialize_delta_log(self.filesystem.as_ref(), &self.materialize_gate, None).await?;
        self.ensure_legacy_blob_migrated_for_direct_row_read()
            .await?;
        let run = self
            .read_row_by_key::<TurnRunRecord>(RowCollection::Runs, &request.run_id.to_string())
            .await?;

        let Some(run) = run.filter(|record| record.scope == request.scope) else {
            return Ok(None);
        };
        let turn_key = run.turn_id.to_string();
        let turn = self
            .read_row_by_key::<TurnRecord>(RowCollection::Turns, &turn_key)
            .await?
            .ok_or_else(|| TurnError::Unavailable {
                reason: "turn run references missing durable turn row".to_string(),
            })?;
        let run = self.runner_lease_store().overlay_run_record(run).await?;
        Ok(Some(projection::run_state_from_record(run, turn.actor)))
    }

    /// Read run state from the process-local hot snapshot (the write-behind
    /// authority). Used by cancellation reads and, under healthy write-behind,
    /// by [`get_run_state`](crate::TurnStateStore::get_run_state) to honor
    /// read-your-writes for a not-yet-flushed non-critical mutation.
    pub(crate) async fn read_run_state_from_hot_cache(
        &self,
        request: &GetRunStateRequest,
    ) -> Result<Option<TurnRunState>, TurnError> {
        let (run, turn) = {
            let mut guard = self.snapshot_state.lock().await;
            self.drop_cache_if_degraded(&mut guard);
            if guard.is_none() {
                *guard = Some(self.load_snapshot_from_rows().await?);
            }
            let Some(state) = guard.as_ref() else {
                return Ok(None);
            };
            let Some(run) = state.run_record(&request.scope, request.run_id) else {
                return Ok(None);
            };
            let turn = state.turn_record_for_run(&request.scope, &run)?;
            (run, turn)
        };
        let run = self.runner_lease_store().overlay_run_record(run).await?;
        Ok(Some(projection::run_state_from_record(run, turn.actor)))
    }

    pub(super) async fn read_turn_events_from_durable_rows(
        &self,
        scope: &TurnScope,
        owner_user_id: Option<&UserId>,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<TurnEventPage, TurnError> {
        self.flush_pending_write_behind_for_read().await?;
        materialize_delta_log(self.filesystem.as_ref(), &self.materialize_gate, None).await?;
        self.ensure_legacy_blob_migrated_for_direct_row_read()
            .await?;
        let after_key = after.map(|cursor| format!("{:020}", cursor.0));
        let events = keyed_records(
            &self
                .read_row_collection_filtered(RowCollection::Events, |key| {
                    after_key
                        .as_ref()
                        .is_none_or(|after_key| key > after_key.as_str())
                })
                .await?,
            &event_record_key,
        )?;
        let retention_floor = self.read_meta().await?.event_retention_floor;
        let mut events = events.into_values().collect::<Vec<_>>();
        events.sort_by_key(|event| event.cursor);
        Ok(project_turn_events(
            &events,
            scope,
            owner_user_id,
            after,
            limit,
            retention_floor,
        ))
    }

    pub(super) async fn read_loop_checkpoint_from_durable_rows(
        &self,
        request: &GetLoopCheckpointRequest,
    ) -> Result<Option<LoopCheckpointRecord>, TurnError> {
        self.flush_pending_write_behind_for_read().await?;
        materialize_delta_log(self.filesystem.as_ref(), &self.materialize_gate, None).await?;
        self.ensure_legacy_blob_migrated_for_direct_row_read()
            .await?;
        let key = request.checkpoint_id.as_uuid().to_string();
        let checkpoint = self
            .read_row_by_key::<LoopCheckpointRecord>(RowCollection::LoopCheckpoints, &key)
            .await?;
        Ok(checkpoint.filter(|record| {
            record.scope == request.scope
                && record.turn_id == request.turn_id
                && record.run_id == request.run_id
                && record.checkpoint_id == request.checkpoint_id
        }))
    }

    async fn ensure_legacy_blob_migrated_for_direct_row_read(&self) -> Result<(), TurnError> {
        if self.read_meta().await?.journal_seq > SeqNo::ZERO {
            return Ok(());
        }
        if self.read_legacy_blob_snapshot().await?.is_none() {
            return Ok(());
        }
        let mut guard = self.snapshot_state.lock().await;
        if guard.is_none() {
            *guard = Some(self.load_snapshot_from_rows().await?);
        }
        Ok(())
    }
}
