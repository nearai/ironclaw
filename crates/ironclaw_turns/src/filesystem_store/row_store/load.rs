//! Durable rehydration, row-collection reads, legacy-blob migration, and the
//! durable-read query paths for [`FilesystemTurnStateRowStore`]. Moved verbatim
//! from the module root during the #6263 decomposition; behavior is unchanged.

use std::sync::Arc;

use futures_util::stream::{self, StreamExt, TryStreamExt};
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FileType, FilesystemError, Page, RootFilesystem, SeqNo,
};
use ironclaw_host_api::{ResourceScope, ScopedPath, UserId};
use serde::de::DeserializeOwned;

use crate::filesystem_store::{io as legacy_blob_io, projection};
use crate::{
    EventCursor, GetLoopCheckpointRequest, GetRunStateRequest, LoopCheckpointRecord, TurnError,
    TurnEventPage, TurnLifecycleEvent, TurnPersistenceSnapshot, TurnRecord, TurnRunRecord,
    TurnRunState, TurnScope, events::project_turn_events,
};

use super::{
    FilesystemTurnStateRowStore, RowCollection,
    delta::{
        RowSnapshotState, RowStoreMeta, active_lock_record_key, event_record_key, keyed_records,
        row_store_hot_cache_snapshot, snapshot_delta,
    },
    events_index,
    io::{
        EventsIndexMarker, delta_log_path, deserialize_materialized_row, deserialize_row,
        events_index_marker_path, fs_error, meta_path, row_dir, row_path,
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
        let retention_floor = self.read_meta().await?.event_retention_floor;

        // Preferred path: an indexed `And(Eq{scope_key}, Range{cursor})` scan
        // that reads only this scope's event bodies after `after`, instead of
        // listing the whole events collection and reading every cross-thread
        // row after the cursor. `project_turn_events` still owns every
        // scope/owner/retention/rebase/pagination semantic, so feeding it the
        // scope-pruned (superset-safe) set yields output identical to the scan.
        if let Some(mut events) = self.read_scoped_events_via_query(scope, after).await? {
            events.sort_by_key(|event| event.cursor);
            return Ok(project_turn_events(
                &events,
                scope,
                owner_user_id,
                after,
                limit,
                retention_floor,
            ));
        }

        // Fallback for a mount without `query`/`ensure_index` (byte-only
        // backend): the legacy directory scan, unchanged.
        self.read_turn_events_via_scan(scope, owner_user_id, after, limit, retention_floor)
            .await
    }

    /// Legacy directory-scan read path, retained as a fallback for mounts that
    /// do not serve `query`/`ensure_index`.
    async fn read_turn_events_via_scan(
        &self,
        scope: &TurnScope,
        owner_user_id: Option<&UserId>,
        after: Option<EventCursor>,
        limit: usize,
        retention_floor: EventCursor,
    ) -> Result<TurnEventPage, TurnError> {
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

    /// Read this scope's events with `cursor > after` via the indexed query.
    /// Returns `Ok(None)` when the mount does not support `query`/`ensure_index`
    /// so the caller can fall back to the directory scan.
    async fn read_scoped_events_via_query(
        &self,
        scope: &TurnScope,
        after: Option<EventCursor>,
    ) -> Result<Option<Vec<TurnLifecycleEvent>>, TurnError> {
        if !self.ensure_events_index_ready().await? {
            return Ok(None);
        }
        let dir = row_dir(RowCollection::Events.as_str())?;
        let filter = events_index::events_query_filter(scope, after)?;
        let mut collected: Vec<TurnLifecycleEvent> = Vec::new();
        let mut offset = 0u64;
        loop {
            let page = Page::new(offset, Page::MAX_LIMIT);
            let entries = match self
                .filesystem
                .query(&ResourceScope::system(), &dir, &filter, page)
                .await
            {
                Ok(entries) => entries,
                Err(FilesystemError::Unsupported { .. }) => return Ok(None),
                Err(error) => return Err(fs_error(error)),
            };
            let fetched = entries.len();
            for versioned in entries {
                if let Some(event) = deserialize_materialized_row::<TurnLifecycleEvent>(
                    &versioned.entry.body,
                    RowCollection::Events.as_str(),
                )? {
                    collected.push(event);
                }
            }
            if fetched < Page::MAX_LIMIT as usize {
                break;
            }
            offset = offset.saturating_add(fetched as u64);
            // Total durable events are bounded by `max_events` (the engine
            // prunes beyond it and advances the retention floor), so a single
            // scope can never exceed it. This is a defensive stop that should
            // not fire; log rather than truncate silently if it ever does.
            if collected.len() >= self.limits.max_events {
                tracing::debug!(
                    scope_events = collected.len(),
                    max_events = self.limits.max_events,
                    "turn-state durable events query reached the max_events safety cap; stopping pagination",
                );
                break;
            }
        }
        Ok(Some(collected))
    }

    /// Declare the event-row indexes and run the one-time pre-projection
    /// backfill, exactly once per process. Caches `true` when the query path is
    /// usable and `false` when the mount cannot serve `query`/`ensure_index`.
    async fn ensure_events_index_ready(&self) -> Result<bool, TurnError> {
        let ready = self
            .events_index_ready
            .get_or_try_init(|| async {
                let dir = row_dir(RowCollection::Events.as_str())?;
                for spec in events_index::event_index_specs()? {
                    match self
                        .filesystem
                        .ensure_index(&ResourceScope::system(), &dir, &spec)
                        .await
                    {
                        Ok(()) => {}
                        Err(FilesystemError::Unsupported { .. }) => return Ok(false),
                        Err(error) => return Err(fs_error(error)),
                    }
                }
                self.backfill_event_indexes_if_needed(&dir).await?;
                Ok::<bool, TurnError>(true)
            })
            .await?;
        Ok(*ready)
    }

    /// Re-project `Entry::indexed` onto every event row written before the
    /// indexed-projection change so the query path finds historical events.
    /// Guarded by a durable marker (skipped on a fresh store or after a prior
    /// completed backfill) and idempotent (rows already projected are skipped).
    async fn backfill_event_indexes_if_needed(&self, dir: &ScopedPath) -> Result<(), TurnError> {
        if self.read_events_index_marker().await?.backfilled {
            return Ok(());
        }
        let entries = match self
            .filesystem
            .list_dir(&ResourceScope::system(), dir)
            .await
        {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => Vec::new(),
            Err(error) => return Err(fs_error(error)),
        };
        let scope_key = events_index::scope_index_key()?;
        let paths = entries
            .into_iter()
            .filter(|entry| entry.file_type == FileType::File)
            .filter_map(|entry| entry.name.strip_suffix(".json").map(ToString::to_string))
            .map(|key| row_path(RowCollection::Events.as_str(), &key))
            .collect::<Result<Vec<_>, _>>()?;
        // Re-project at the same bounded fan-out the row-collection read path
        // uses (`ROW_COLLECTION_READ_CONCURRENCY`): each row is an independent
        // CAS put, so ordering is irrelevant, and the one-time migration no
        // longer stalls serially over a large (tombstone-inclusive) events
        // collection where a per-row `get` latency would otherwise accumulate.
        let scope_key = &scope_key;
        let reprojected: usize = stream::iter(paths)
            .map(|path| async move {
                let Some(versioned) = self
                    .filesystem
                    .get(&ResourceScope::system(), &path)
                    .await
                    .map_err(fs_error)?
                else {
                    return Ok::<usize, TurnError>(0);
                };
                if versioned.entry.indexed.contains_key(scope_key) {
                    return Ok(0); // already projected (written after the upgrade)
                }
                let Some(event) = deserialize_materialized_row::<TurnLifecycleEvent>(
                    &versioned.entry.body,
                    RowCollection::Events.as_str(),
                )?
                else {
                    return Ok(0); // tombstone — no projection needed
                };
                let mut new_entry = Entry::bytes(versioned.entry.body.clone())
                    .with_content_type(ContentType::json());
                new_entry.indexed = events_index::event_indexed_projection(&event)?;
                match self
                    .filesystem
                    .put(
                        &ResourceScope::system(),
                        &path,
                        new_entry,
                        CasExpectation::Version(versioned.version),
                    )
                    .await
                {
                    Ok(_) => Ok(1),
                    // A concurrent materialize rewrote the row (with its own
                    // projection) or deleted it; either way our backfill is moot.
                    Err(FilesystemError::VersionMismatch { .. })
                    | Err(FilesystemError::NotFound { .. }) => Ok(0),
                    Err(error) => Err(fs_error(error)),
                }
            })
            .buffer_unordered(ROW_COLLECTION_READ_CONCURRENCY)
            .try_fold(
                0usize,
                |acc, reprojected| async move { Ok(acc + reprojected) },
            )
            .await?;
        if reprojected > 0 {
            tracing::debug!(
                reprojected,
                "backfilled turn-state event-row index projections"
            );
        }
        self.write_events_index_marker().await
    }

    async fn read_events_index_marker(&self) -> Result<EventsIndexMarker, TurnError> {
        match self
            .filesystem
            .get(&ResourceScope::system(), &events_index_marker_path()?)
            .await
        {
            Ok(Some(versioned)) => {
                deserialize_row(&versioned.entry.body, "turn-state events-index marker")
            }
            Ok(None) | Err(FilesystemError::NotFound { .. }) => Ok(EventsIndexMarker::default()),
            Err(error) => Err(fs_error(error)),
        }
    }

    async fn write_events_index_marker(&self) -> Result<(), TurnError> {
        let marker = EventsIndexMarker { backfilled: true };
        let body = serde_json::to_vec(&marker).map_err(|error| TurnError::Unavailable {
            reason: format!("turn-state events-index marker serialization failed: {error}"),
        })?;
        let entry = Entry::bytes(body).with_content_type(ContentType::json());
        // Idempotent single-value write (always `{backfilled:true}`), so a blind
        // last-writer-wins overwrite is correct and keeps it off any CAS loop.
        self.filesystem
            .put(
                &ResourceScope::system(),
                &events_index_marker_path()?,
                entry,
                CasExpectation::Any,
            )
            .await
            .map_err(fs_error)?;
        Ok(())
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
