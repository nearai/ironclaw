//! One-time rebuild of the message, summary, and exact-lookup projections.
//!
//! Split out of `thread_index` (which owns the thread-listing projection):
//! this is a separate transactional state machine with its own retry outcome,
//! backoff loop, page transaction, and completion marker, and a reader of the
//! listing projection does not need to understand any of it.

use ironclaw_filesystem::{
    CasExpectation, Entry, FileType, Filter, Page, RecordKind, RootFilesystem,
};
use ironclaw_host_api::{ids::ThreadId, path::ScopedPath};

use crate::{
    FilesystemSessionThreadService, SessionThreadError, SummaryArtifact, ThreadMessageRecord,
    ThreadScope,
};

use super::{
    IndexDeclarationPolicy, deserialize, invalid_path, is_not_found, messages_root,
    scope_axes_string, scoped_path, summaries_root,
};

/// Bounded retries for a transcript-migration page that lost a CAS or
/// writer-contention race against live turn writes.
const TRANSCRIPT_PAGE_CONFLICT_RETRIES: u32 = 5;

/// Outcome of one transactional transcript-migration page.
enum TranscriptPageOutcome {
    Committed,
    /// The page lost a concurrency race and should be re-read and retried;
    /// carries the underlying error so retry exhaustion can fail loud with
    /// the real cause.
    Conflict(ironclaw_filesystem::FilesystemError),
}

/// Whether a transcript-migration transaction error is a concurrency race
/// (safe to retry with fresh reads) rather than a real backend failure.
fn transcript_migration_conflict(error: &ironclaw_filesystem::FilesystemError) -> bool {
    matches!(
        error,
        ironclaw_filesystem::FilesystemError::VersionMismatch { .. }
            | ironclaw_filesystem::FilesystemError::BackendBusy { .. }
    )
}

impl<F> FilesystemSessionThreadService<F>
where
    F: RootFilesystem + 'static,
{
    /// Idempotent rebuild for message, summary, and exact-lookup projections.
    pub async fn migrate_transcript_indexes_for_scope(
        &self,
        scope: &ThreadScope,
    ) -> Result<usize, SessionThreadError> {
        let root = scoped_path(&format!("{}/threads", scope_axes_string(scope)))?;
        let entries = match self
            .filesystem
            .list_dir(&scope.to_resource_scope(), &root)
            .await
        {
            Ok(entries) => entries,
            Err(error) if is_not_found(&error) => Vec::new(),
            Err(error) => return Err(error.into()),
        };
        let thread_ids = entries
            .into_iter()
            .filter(|entry| entry.file_type == FileType::Directory)
            .map(|entry| ThreadId::new(entry.name).map_err(invalid_path))
            .collect::<Result<Vec<_>, _>>()?;
        let mut migrated = 0usize;
        self.declare_root_indexes(scope, IndexDeclarationPolicy::Required)
            .await?;
        for thread_id in thread_ids {
            for (prefix, messages) in [
                (messages_root(scope, &thread_id)?, true),
                (summaries_root(scope, &thread_id)?, false),
            ] {
                let mut offset = 0u64;
                let mut conflict_attempts = 0u32;
                loop {
                    let rows = self
                        .filesystem
                        .query(
                            &scope.to_resource_scope(),
                            &prefix,
                            &Filter::All,
                            Page::new(offset, Page::MAX_LIMIT),
                        )
                        .await?;
                    if rows.is_empty() {
                        break;
                    }
                    let received = rows.len();
                    match self.migrate_transcript_page(scope, messages, rows).await? {
                        TranscriptPageOutcome::Committed => {
                            conflict_attempts = 0;
                            migrated = migrated.saturating_add(received);
                            if received < Page::MAX_LIMIT as usize {
                                break;
                            }
                            offset = offset.saturating_add(received as u64);
                        }
                        TranscriptPageOutcome::Conflict(error) => {
                            // A live writer (turn finalization, preview append)
                            // landed between this page's in-transaction reads
                            // and its commit. Re-reading the page picks up the
                            // committed versions, so the retry converges; the
                            // bound keeps a pathological writer from pinning
                            // the migration forever.
                            conflict_attempts += 1;
                            if conflict_attempts > TRANSCRIPT_PAGE_CONFLICT_RETRIES {
                                return Err(error.into());
                            }
                            tokio::time::sleep(std::time::Duration::from_millis(
                                10u64.saturating_mul(conflict_attempts.into()),
                            ))
                            .await;
                        }
                    }
                }
            }
        }
        Ok(migrated)
    }

    /// One transactional page of [`Self::migrate_transcript_indexes_for_scope`].
    ///
    /// Returns `Conflict` instead of an error when the transaction lost a CAS
    /// or writer-contention race, so the caller can re-read and retry the page
    /// bounded — an escaped `VersionMismatch` here surfaced to WebUI timeline
    /// reads as a retryable 503 whenever the scope's first transcript read
    /// overlapped a running turn.
    async fn migrate_transcript_page(
        &self,
        scope: &ThreadScope,
        messages: bool,
        rows: Vec<ironclaw_filesystem::VersionedEntry>,
    ) -> Result<TranscriptPageOutcome, SessionThreadError> {
        // Writer admission is exactly where the observed QA failure happens:
        // acquiring the sole libSQL writer can time out as `BackendBusy`, and
        // propagating it here would escape as the retryable timeline 503 this
        // retry loop exists to absorb.
        let mut txn = match self
            .filesystem
            .begin(
                &scope.to_resource_scope(),
                &scoped_path(crate::filesystem_service::THREADS_PREFIX)?,
            )
            .await
        {
            Ok(txn) => txn,
            Err(error) if transcript_migration_conflict(&error) => {
                return Ok(TranscriptPageOutcome::Conflict(error));
            }
            Err(error) => return Err(error.into()),
        };
        for listed_row in rows {
            // The listing runs before BEGIN IMMEDIATE owns the
            // writer lock. Re-read inside the transaction so a
            // current-code message update in that window cannot
            // make this one-time migration fail with a stale CAS.
            let row = match txn.get(&listed_row.path).await {
                Ok(Some(row)) => row,
                Ok(None) => continue,
                Err(error) if transcript_migration_conflict(&error) => {
                    txn.rollback().await;
                    return Ok(TranscriptPageOutcome::Conflict(error));
                }
                Err(error) => return Err(error.into()),
            };
            let expected_kind = if messages {
                crate::filesystem_service::THREAD_MESSAGE_KIND
            } else {
                crate::filesystem_service::THREAD_SUMMARY_KIND
            };
            if row.entry.kind.as_ref().map(RecordKind::as_str) != Some(expected_kind) {
                continue;
            }
            let entry = if messages {
                let record = deserialize::<ThreadMessageRecord>(&row.entry.body)?;
                // Refresh the projection, keep the stored body. Rebuilding the
                // entry from `record` would round-trip the row through the
                // current struct, so any field a newer binary wrote and this
                // one does not know is dropped permanently -- a one-way rewrite
                // of durable transcript data. It also rewrites every body in
                // the scope, which is the write amplification this change is
                // trying to remove. The rebuild is only needed for `indexed`.
                let mut entry = row.entry.clone();
                entry.indexed = Self::message_entry(&record)?.indexed;
                entry
            } else {
                let record = deserialize::<SummaryArtifact>(&row.entry.body)?;
                let mut entry = row.entry.clone();
                entry.indexed = Self::summary_entry(&record)?.indexed;
                entry
            };
            match txn
                .put(&row.path, entry, CasExpectation::Version(row.version))
                .await
            {
                Ok(_) => {}
                Err(error) if transcript_migration_conflict(&error) => {
                    txn.rollback().await;
                    return Ok(TranscriptPageOutcome::Conflict(error));
                }
                Err(error) => return Err(error.into()),
            }
        }
        match txn.commit().await {
            Ok(()) => Ok(TranscriptPageOutcome::Committed),
            Err(error) if transcript_migration_conflict(&error) => {
                Ok(TranscriptPageOutcome::Conflict(error))
            }
            Err(error) => Err(error.into()),
        }
    }

    pub(super) async fn ensure_transcript_indexes_migrated(
        &self,
        scope: &ThreadScope,
    ) -> Result<(), SessionThreadError> {
        let marker = transcript_index_migration_marker_path(scope)?;
        if self
            .filesystem
            .get(&scope.to_resource_scope(), &marker)
            .await?
            .is_some()
        {
            return Ok(());
        }
        self.migrate_transcript_indexes_for_scope(scope).await?;
        self.filesystem
            .put(
                &scope.to_resource_scope(),
                &marker,
                Entry::bytes(b"transcript-index-v2".to_vec()),
                CasExpectation::Any,
            )
            .await?;
        if self
            .filesystem
            .get(&scope.to_resource_scope(), &marker)
            .await?
            .is_none()
        {
            return Err(SessionThreadError::Backend(
                "transcript index migration marker was not durable after write".to_string(),
            ));
        }
        Ok(())
    }
}

fn transcript_index_migration_marker_path(
    scope: &ThreadScope,
) -> Result<ScopedPath, SessionThreadError> {
    scoped_path(&format!(
        "{}/index-migrations/transcript-index-v2.complete",
        scope_axes_string(scope)
    ))
}
