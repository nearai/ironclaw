//! Recovery for thread-index rows that exist durably but project no listing row.
//!
//! The sidebar reads the ordered projection, never the index directory, and the
//! projection only carries a row when the stored entry holds the listing keys.
//! A row written without them is therefore invisible to `list_threads` even
//! though its thread record, index record and messages are all intact — and
//! because a scope's completion marker suppresses the migration that would
//! rewrite it, nothing repairs it for the life of the volume.
//!
//! Kept beside the index query rather than inside it: this is migration-shaped
//! repair work, the same concern `startup_migration` and `transcript_migration`
//! own for their projections.

use ironclaw_filesystem::{CasApply, FileType, Page, RootFilesystem, cas_update};
use ironclaw_host_api::ids::ThreadId;

use crate::{FilesystemSessionThreadService, SessionThreadError, ThreadScope};

use super::{
    THREAD_INDEX_SUFFIX, ThreadIndexRecord, no_op_thread_index_record, thread_activity_index_spec,
    thread_index_record_path, thread_index_root,
};
use crate::filesystem_service::{deserialize, invalid_path, is_not_found, map_cas_error};

impl<F> FilesystemSessionThreadService<F>
where
    F: RootFilesystem,
{
    /// Whether this process has already run required-path repair for
    /// `scope_key` at least once. Kept separate from the durable migration
    /// marker (see the field doc on `reconciled_thread_index_scopes`): the
    /// marker can already be complete on disk from a previous process, and
    /// gating on it directly would let the reconcile step be skipped forever
    /// once an optional call has declared the scope in this process.
    pub(super) fn thread_index_reconciled(&self, scope_key: &str) -> bool {
        self.reconciled_thread_index_scopes
            .lock()
            .map(|reconciled| reconciled.contains(scope_key))
            .unwrap_or(false)
    }

    /// Returns the short-lived lock for one scope's required repair path.
    /// Weak entries let the registry shed idle scopes without a separate cache
    /// eviction policy; active callers keep the `Arc` alive through their
    /// awaitable repair work.
    pub(super) fn thread_index_reconcile_lock(
        &self,
        scope_key: &str,
    ) -> std::sync::Arc<tokio::sync::Mutex<()>> {
        let mut locks = self
            .thread_index_reconcile_locks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(scope_key).and_then(std::sync::Weak::upgrade) {
            return lock;
        }
        let lock = std::sync::Arc::new(tokio::sync::Mutex::new(()));
        locks.insert(scope_key.to_string(), std::sync::Arc::downgrade(&lock));
        lock
    }

    /// Repair rows the listing projection cannot see, when there are any.
    ///
    /// Discovery cannot run through the projection, because a damaged row is
    /// exactly what the projection is missing. It compares the durable index
    /// directory against the projected rows instead and only pays for repair
    /// when they disagree.
    ///
    /// Bounded on purpose: the comparison costs one directory listing and one
    /// capped query. This runs inside a live listing request, so an unbounded
    /// scan would put a latency spike proportional to scope size on whichever
    /// request arrives first after a restart. The listing asks for one entry
    /// past [`Page::MAX_LIMIT`], which is enough to recognise an oversized
    /// scope without materializing all of it on backends that stop early.
    ///
    /// A scope holding more rows than that cap is skipped, and today nothing
    /// else repairs it: `migrate_thread_index_for_scope` runs only while the
    /// scope's completion marker is absent, and both the listing path and the
    /// deployment-wide startup migration reach this function once the marker
    /// exists. Such a scope therefore keeps its damaged rows until an explicit
    /// repair path is built for it — see the follow-up tracked from #7470.
    pub(super) async fn reconcile_thread_index_projection(
        &self,
        scope: &ThreadScope,
    ) -> Result<(), SessionThreadError> {
        let root = thread_index_root(scope)?;
        let durable = match self
            .filesystem
            .list_dir_bounded(
                &scope.to_resource_scope(),
                &root,
                Page::MAX_LIMIT as usize + 1,
            )
            .await
        {
            Ok(entries) => entries,
            // A scope that has never written an index row has nothing to
            // reconcile; the initial migration owns that case.
            Err(error) if is_not_found(&error) => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        let index_rows: Vec<&str> = durable
            .iter()
            .filter(|entry| entry.file_type == FileType::File)
            .filter_map(|entry| entry.name.strip_suffix(THREAD_INDEX_SUFFIX))
            .collect();
        if index_rows.is_empty() {
            return Ok(());
        }
        if index_rows.len() > Page::MAX_LIMIT as usize {
            // Surfaced rather than skipped silently: an oversized scope with
            // damaged rows has no repair path at all, so leaving no trace would
            // make it invisible in telemetry as well as in the sidebar.
            tracing::debug!(
                index_rows = index_rows.len(),
                "thread index scope exceeds the reconcile cap; projection repair skipped"
            );
            return Ok(());
        }
        if self.count_projected_thread_index_rows(scope).await? >= index_rows.len() {
            return Ok(());
        }
        for raw_id in index_rows {
            let thread_id = ThreadId::new(raw_id.to_string()).map_err(invalid_path)?;
            self.restore_thread_index_projection(scope, &thread_id)
                .await?;
        }
        Ok(())
    }

    /// Count the rows the listing projection can actually see for `scope`.
    ///
    /// Capped at [`Page::MAX_LIMIT`]; callers only reach this after confirming
    /// the scope holds no more index rows than that, so a single query answers
    /// the comparison exactly.
    async fn count_projected_thread_index_rows(
        &self,
        scope: &ThreadScope,
    ) -> Result<usize, SessionThreadError> {
        let page = Self::thread_index_ordered_page(Page::MAX_LIMIT)?;
        let rows = self.query_thread_index_rows(scope, &page).await?;
        Ok(rows.len())
    }

    /// Rewrite one index row so the ordered projection picks it up again.
    ///
    /// Uses the shared bounded CAS helper so a concurrent index-row writer is
    /// never clobbered. `CasApply::force_write` intentionally bypasses only
    /// the decoded-body equality fast-path: entry sidecars are not represented
    /// in `ThreadIndexRecord`, so a row whose body already matches but whose
    /// indexed projection keys are gone still needs a physical rewrite.
    async fn restore_thread_index_projection(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
    ) -> Result<(), SessionThreadError> {
        let path = thread_index_record_path(scope, thread_id)?;
        let Some(versioned) = self
            .filesystem
            .get(&scope.to_resource_scope(), &path)
            .await?
        else {
            return Ok(());
        };
        let record = deserialize::<ThreadIndexRecord>(&versioned.entry.body)?;
        // A row whose body disagrees with its own path is not ours to rewrite;
        // stale and cross-scope rows belong to the explicit migration.
        if record.record.scope != *scope || record.record.thread_id != *thread_id {
            return Ok(());
        }
        // Selective repair matters: one missing projection row should not
        // rewrite every otherwise-valid row in the bounded scope. The CAS loop
        // below takes a fresh read before writing, so this precheck only admits
        // rows whose sidecars are currently proven stale; it is not itself a
        // read-modify-write operation.
        let rebuilt = Self::thread_index_entry(&record)?;
        let projection_current = thread_activity_index_spec()?
            .keys
            .iter()
            .all(|key| versioned.entry.indexed.get(key) == rebuilt.indexed.get(key));
        if projection_current {
            return Ok(());
        }
        let resource_scope = scope.to_resource_scope();
        let scope_for_retry = scope.clone();
        let thread_id_for_retry = thread_id.clone();
        let repaired = cas_update(
            self.filesystem.as_ref(),
            &resource_scope,
            &path,
            |bytes: &[u8]| deserialize::<ThreadIndexRecord>(bytes),
            |record: &ThreadIndexRecord| Self::thread_index_entry(record),
            |current: Option<ThreadIndexRecord>| {
                let scope = scope_for_retry.clone();
                let thread_id = thread_id_for_retry.clone();
                async move {
                    let Some(record) = current else {
                        return Ok(CasApply::no_op(
                            no_op_thread_index_record(scope, thread_id),
                            false,
                        ));
                    };
                    // A row whose body disagrees with its own path is not ours
                    // to rewrite; stale and cross-scope rows belong to the
                    // explicit migration.
                    if record.record.scope != scope || record.record.thread_id != thread_id {
                        return Ok(CasApply::no_op(record, false));
                    }
                    // The CAS helper intentionally passes only the decoded
                    // record to `apply`; entry-sidecar metadata is rebuilt by
                    // `encode`. The precheck admitted only a row whose
                    // sidecar is stale, so force-write it without treating
                    // body equality as proof that its ordered keys are
                    // current.
                    Ok(CasApply::force_write(record, true))
                }
            },
        )
        .await
        .map_err(map_cas_error)?;
        if repaired {
            self.mark_thread_index_known(scope, thread_id);
        }
        Ok(())
    }
}
