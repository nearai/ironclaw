use std::collections::HashSet;

use chrono::{DateTime, Utc};
use ironclaw_filesystem::{
    CasApply, CasExpectation, ContentType, Entry, FileType, Filter, IndexKey, IndexKind, IndexName,
    IndexSpec, IndexValue, OrderedPage, OrderedQueryCursor, Page, RecordKind, RootFilesystem,
    SortDirection, cas_update,
};
use ironclaw_host_api::{ScopedPath, ThreadId};
use serde::{Deserialize, Serialize};

use crate::{
    FilesystemSessionThreadService, SessionThreadError, SessionThreadRecord, SummaryArtifact,
    ThreadMessageRecord, ThreadScope,
};

use super::{
    StoredThreadRecord, deserialize, invalid_path, is_not_found, map_cas_error, messages_root,
    scope_axes_string, scoped_path, serialize_pretty, summaries_root,
};

const THREAD_INDEX_KIND: &str = "thread_index";
const THREAD_SCOPE_INDEX_KEY: &str = "scope_key";
const THREAD_ACTIVITY_SORT_KEY: &str = "activity_sort";
const THREAD_ID_INDEX_KEY: &str = "thread_id";
const THREAD_INDEX_KNOWN_ROW_MAX: usize = 100_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ThreadIndexRecord {
    #[serde(flatten)]
    pub(super) record: SessionThreadRecord,
    pub(super) next_sequence: u64,
    flags: ThreadIndexFlags,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct ThreadIndexFlags {
    title_present: bool,
    metadata_present: bool,
    goal_present: bool,
}

impl<F> FilesystemSessionThreadService<F>
where
    F: RootFilesystem,
{
    fn thread_index_entry(record: &ThreadIndexRecord) -> Result<Entry, SessionThreadError> {
        let body = serialize_pretty(record)?;
        let kind = RecordKind::new(THREAD_INDEX_KIND).map_err(|error| {
            SessionThreadError::Backend(format!("invalid thread_index record kind: {error}"))
        })?;
        let mut entry = Entry::bytes(body).with_content_type(ContentType::json());
        entry.kind = Some(kind);
        Ok(entry
            .with_indexed(
                thread_index_key(THREAD_SCOPE_INDEX_KEY)?,
                IndexValue::Text(thread_index_cache_key(&record.record.scope)),
            )
            .with_indexed(
                thread_index_key(THREAD_ACTIVITY_SORT_KEY)?,
                IndexValue::Text(thread_activity_sort_key(&record.record)),
            )
            .with_indexed(
                thread_index_key(THREAD_ID_INDEX_KEY)?,
                IndexValue::Text(record.record.thread_id.as_str().to_string()),
            ))
    }

    async fn ensure_thread_index_query(
        &self,
        scope: &ThreadScope,
        required: bool,
    ) -> Result<(), SessionThreadError> {
        let scope_key = thread_index_cache_key(scope);
        if self
            .ready_thread_index_scopes
            .lock()
            .map(|ready| ready.contains(&scope_key))
            .unwrap_or(false)
        {
            return Ok(());
        }
        let _declaration_guard = self.thread_index_declaration_lock.lock().await;
        if self
            .ready_thread_index_scopes
            .lock()
            .map(|ready| ready.contains(&scope_key))
            .unwrap_or(false)
        {
            return Ok(());
        }
        let root = thread_index_root(scope)?;
        let spec = IndexSpec::new(
            thread_index_name()?,
            vec![
                thread_index_key(THREAD_SCOPE_INDEX_KEY)?,
                thread_index_key(THREAD_ACTIVITY_SORT_KEY)?,
                thread_index_key(THREAD_ID_INDEX_KEY)?,
            ],
            IndexKind::Exact,
        );
        let result = self
            .filesystem
            .ensure_index(&scope.to_resource_scope(), &root, &spec)
            .await;
        if let Err(ironclaw_filesystem::FilesystemError::Unsupported { .. }) = result
            && !required
        {
            return Ok(());
        }
        result.map_err(SessionThreadError::from)?;
        if let Ok(mut ready) = self.ready_thread_index_scopes.lock() {
            ready.insert(scope_key.clone());
            evict_hash_set_entry_over_limit(&mut ready, 128, &scope_key);
        }
        if required {
            let marker = thread_index_migration_marker_path(scope)?;
            if self
                .filesystem
                .get(&scope.to_resource_scope(), &marker)
                .await?
                .is_none()
            {
                if let Err(error) = self.migrate_thread_index_for_scope(scope).await {
                    if let Ok(mut ready) = self.ready_thread_index_scopes.lock() {
                        ready.remove(&scope_key);
                    }
                    return Err(error);
                }
                self.filesystem
                    .put(
                        &scope.to_resource_scope(),
                        &marker,
                        Entry::bytes(b"thread-index-v1".to_vec()),
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
                        "thread index migration marker was not durable after write".to_string(),
                    ));
                }
            }
        }
        Ok(())
    }

    fn thread_index_record(stored: &StoredThreadRecord) -> ThreadIndexRecord {
        ThreadIndexRecord {
            record: stored.record.clone(),
            next_sequence: stored.next_sequence,
            flags: ThreadIndexFlags {
                title_present: stored.record.title.is_some(),
                metadata_present: stored.record.metadata_json.is_some(),
                goal_present: stored.record.goal.is_some(),
            },
        }
    }

    pub(super) fn forget_thread_index_row(&self, scope: &ThreadScope, thread_id: &ThreadId) {
        if let Ok(mut known) = self.known_thread_index_rows.lock() {
            known.remove(&thread_index_record_cache_key(scope, thread_id));
        }
    }

    pub(super) async fn delete_thread_index_record(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
    ) -> Result<(), SessionThreadError> {
        let index_path = thread_index_record_path(scope, thread_id)?;
        match self
            .filesystem
            .delete(&scope.to_resource_scope(), &index_path)
            .await
        {
            Ok(()) => {}
            Err(error) if is_not_found(&error) => {}
            Err(error) => {
                self.forget_thread_index_row(scope, thread_id);
                return Err(error.into());
            }
        }
        self.forget_thread_index_row(scope, thread_id);
        Ok(())
    }

    fn mark_thread_index_known(&self, scope: &ThreadScope, thread_id: &ThreadId) {
        if let Ok(mut known) = self.known_thread_index_rows.lock() {
            let key = thread_index_record_cache_key(scope, thread_id);
            known.insert(key.clone());
            evict_hash_set_entry_over_limit(&mut known, THREAD_INDEX_KNOWN_ROW_MAX, &key);
        }
    }

    pub(super) fn is_thread_index_known(&self, scope: &ThreadScope, thread_id: &ThreadId) -> bool {
        self.known_thread_index_rows
            .lock()
            .map(|known| known.contains(&thread_index_record_cache_key(scope, thread_id)))
            .unwrap_or(false)
    }

    pub(super) async fn refresh_thread_index_from_source(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
    ) -> Result<(), SessionThreadError> {
        let Some((stored, _)) = self.read_thread_versioned(scope, thread_id).await? else {
            return Ok(());
        };
        let index = Self::thread_index_record(&stored);
        self.merge_thread_index_record_from_source(index).await?;
        Ok(())
    }

    async fn merge_thread_index_record_from_source(
        &self,
        source: ThreadIndexRecord,
    ) -> Result<ThreadIndexRecord, SessionThreadError> {
        self.ensure_thread_index_query(&source.record.scope, false)
            .await?;
        self.merge_thread_index_record_declared(source).await
    }

    async fn merge_thread_index_record_declared(
        &self,
        source: ThreadIndexRecord,
    ) -> Result<ThreadIndexRecord, SessionThreadError> {
        let path = thread_index_record_path(&source.record.scope, &source.record.thread_id)?;
        let resource_scope = source.record.scope.to_resource_scope();
        let source_for_retry = source.clone();
        let merged = cas_update(
            self.filesystem.as_ref(),
            &resource_scope,
            &path,
            |bytes: &[u8]| deserialize::<ThreadIndexRecord>(bytes),
            |record: &ThreadIndexRecord| Self::thread_index_entry(record),
            |current: Option<ThreadIndexRecord>| {
                let source = source_for_retry.clone();
                async move {
                    let merged = match current {
                        Some(existing) => Self::merge_thread_index_records(source, existing)?,
                        None => source,
                    };
                    Ok(CasApply::new(merged.clone(), merged))
                }
            },
        )
        .await
        .map_err(map_cas_error)?;
        self.mark_thread_index_known(&merged.record.scope, &merged.record.thread_id);
        Ok(merged)
    }

    fn merge_thread_index_records(
        mut source: ThreadIndexRecord,
        existing: ThreadIndexRecord,
    ) -> Result<ThreadIndexRecord, SessionThreadError> {
        if existing.record.scope != source.record.scope
            || existing.record.thread_id != source.record.thread_id
        {
            return Err(SessionThreadError::ThreadScopeMismatch {
                thread_id: source.record.thread_id,
            });
        }
        let same_source_generation = existing.record.created_at.is_some()
            && existing.record.created_at == source.record.created_at;
        if same_source_generation && existing.record.updated_at > source.record.updated_at {
            source.record.updated_at = existing.record.updated_at;
        }
        if same_source_generation {
            source.next_sequence = source.next_sequence.max(existing.next_sequence);
        }
        if same_source_generation && !source.flags.title_present && existing.flags.title_present {
            source.record.title = existing.record.title;
            source.flags.title_present = true;
        }
        if same_source_generation
            && !source.flags.metadata_present
            && existing.flags.metadata_present
        {
            source.record.metadata_json = existing.record.metadata_json;
            source.flags.metadata_present = true;
        }
        if same_source_generation && !source.flags.goal_present && existing.flags.goal_present {
            source.record.goal = existing.record.goal;
            source.flags.goal_present = true;
        }
        Ok(source)
    }

    pub(super) async fn touch_thread_index_updated_at(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        updated_at: DateTime<Utc>,
    ) -> Result<(), SessionThreadError> {
        self.ensure_thread_index_query(scope, false).await?;
        let path = thread_index_record_path(scope, thread_id)?;
        let resource_scope = scope.to_resource_scope();
        let scope_for_retry = scope.clone();
        let thread_id_for_retry = thread_id.clone();
        let row_known = cas_update(
            self.filesystem.as_ref(),
            &resource_scope,
            &path,
            |bytes: &[u8]| deserialize::<ThreadIndexRecord>(bytes),
            |record: &ThreadIndexRecord| Self::thread_index_entry(record),
            |current: Option<ThreadIndexRecord>| {
                let scope = scope_for_retry.clone();
                let thread_id = thread_id_for_retry.clone();
                async move {
                    let mut index = match current {
                        Some(index) => {
                            if index.record.scope != scope || index.record.thread_id != thread_id {
                                return Err(SessionThreadError::ThreadScopeMismatch { thread_id });
                            }
                            index
                        }
                        None => {
                            let Some((mut stored, _)) =
                                self.read_thread_versioned(&scope, &thread_id).await?
                            else {
                                return Ok(CasApply::no_op(
                                    no_op_thread_index_record(scope, thread_id),
                                    false,
                                ));
                            };
                            stored.record.updated_at = Some(updated_at);
                            Self::thread_index_record(&stored)
                        }
                    };
                    index.record.updated_at = Some(updated_at);
                    Ok(CasApply::new(index, true))
                }
            },
        )
        .await
        .map_err(map_cas_error)?;
        if row_known {
            self.mark_thread_index_known(scope, thread_id);
        }
        Ok(())
    }

    async fn read_thread_index_record(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
    ) -> Result<Option<ThreadIndexRecord>, SessionThreadError> {
        let path = thread_index_record_path(scope, thread_id)?;
        let Some(versioned) = self
            .filesystem
            .get(&scope.to_resource_scope(), &path)
            .await?
        else {
            return Ok(None);
        };
        let record = deserialize::<ThreadIndexRecord>(&versioned.entry.body)?;
        if record.record.scope != *scope || record.record.thread_id != *thread_id {
            return Ok(None);
        }
        Ok(Some(record))
    }

    pub(super) async fn list_thread_index_page(
        &self,
        scope: &ThreadScope,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<ThreadIndexRecord>, bool), SessionThreadError> {
        self.ensure_thread_index_query(scope, true).await?;
        let root = thread_index_root(scope)?;
        let mut page = OrderedPage::new(
            thread_index_name()?,
            thread_index_key(THREAD_ACTIVITY_SORT_KEY)?,
            thread_index_key(THREAD_ID_INDEX_KEY)?,
            SortDirection::Ascending,
            u32::try_from(limit.saturating_add(1))
                .unwrap_or(Page::MAX_LIMIT)
                .min(Page::MAX_LIMIT),
        );
        if let Some(cursor) = cursor {
            let cursor = self.decode_thread_index_cursor(scope, cursor).await?;
            page = page.after(OrderedQueryCursor {
                value: IndexValue::Text(cursor.activity_sort),
                tie_breaker: IndexValue::Text(cursor.thread_id),
            });
        }
        let rows = self
            .filesystem
            .query_ordered(
                &scope.to_resource_scope(),
                &root,
                &Filter::Eq {
                    key: thread_index_key(THREAD_SCOPE_INDEX_KEY)?,
                    value: IndexValue::Text(thread_index_cache_key(scope)),
                },
                &page,
            )
            .await?;
        let has_more = rows.len() > limit;
        let records = rows
            .into_iter()
            .take(limit)
            .map(|row| deserialize::<ThreadIndexRecord>(&row.entry.body))
            .collect::<Result<Vec<_>, _>>()?;
        // Index rows are the list projection. Source validation belongs to the
        // explicit migration/repair path; rereading every source row here
        // turns each user-facing page into an N+1 storage operation.
        Ok((records, has_more))
    }

    async fn decode_thread_index_cursor(
        &self,
        scope: &ThreadScope,
        cursor: &str,
    ) -> Result<ThreadIndexCursor, SessionThreadError> {
        if let Ok(cursor) = serde_json::from_str::<ThreadIndexCursor>(cursor) {
            return Ok(cursor);
        }
        let thread_id = ThreadId::new(cursor.to_string()).map_err(invalid_path)?;
        let record = self
            .read_thread_index_record(scope, &thread_id)
            .await?
            .ok_or_else(|| SessionThreadError::UnknownThread {
                thread_id: thread_id.clone(),
            })?;
        Ok(ThreadIndexCursor::from_record(&record.record))
    }

    pub(super) fn encode_thread_index_cursor(
        record: &SessionThreadRecord,
    ) -> Result<String, SessionThreadError> {
        serde_json::to_string(&ThreadIndexCursor::from_record(record))
            .map_err(|error| SessionThreadError::Serialization(error.to_string()))
    }

    /// Idempotent legacy repair used before indexed listing is exposed.
    pub async fn migrate_thread_index_for_scope(
        &self,
        scope: &ThreadScope,
    ) -> Result<usize, SessionThreadError> {
        let root = scoped_path(&format!("{}/threads", scope_axes_string(scope)))?;
        let entries = self
            .filesystem
            .list_dir(&scope.to_resource_scope(), &root)
            .await?;
        let thread_ids = entries
            .into_iter()
            .filter(|entry| entry.file_type == FileType::Directory)
            .map(|entry| ThreadId::new(entry.name).map_err(invalid_path))
            .collect::<Result<Vec<_>, _>>()?;
        for thread_id in &thread_ids {
            if let Some((stored, _)) = self.read_thread_versioned(scope, thread_id).await? {
                self.merge_thread_index_record_declared(Self::thread_index_record(&stored))
                    .await?;
            }
        }
        let source_ids = thread_ids
            .iter()
            .map(ThreadId::as_str)
            .collect::<HashSet<_>>();
        let index_root = thread_index_root(scope)?;
        for entry in self
            .filesystem
            .list_dir(&scope.to_resource_scope(), &index_root)
            .await?
        {
            let Some(raw_id) = entry.name.strip_suffix(".json") else {
                continue;
            };
            if !source_ids.contains(raw_id) {
                let stale_id = ThreadId::new(raw_id.to_string()).map_err(invalid_path)?;
                self.delete_thread_index_record(scope, &stale_id).await?;
            }
        }
        Ok(thread_ids.len())
    }

    /// Idempotent rebuild for message, summary, and exact-lookup projections.
    pub async fn migrate_transcript_indexes_for_scope(
        &self,
        scope: &ThreadScope,
    ) -> Result<usize, SessionThreadError> {
        let root = scoped_path(&format!("{}/threads", scope_axes_string(scope)))?;
        let entries = self
            .filesystem
            .list_dir(&scope.to_resource_scope(), &root)
            .await?;
        let thread_ids = entries
            .into_iter()
            .filter(|entry| entry.file_type == FileType::Directory)
            .map(|entry| ThreadId::new(entry.name).map_err(invalid_path))
            .collect::<Result<Vec<_>, _>>()?;
        let mut migrated = 0usize;
        for thread_id in thread_ids {
            self.ensure_thread_record_indexes(scope, &thread_id).await?;
            for (prefix, messages) in [
                (messages_root(scope, &thread_id)?, true),
                (summaries_root(scope, &thread_id)?, false),
            ] {
                let mut offset = 0u64;
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
                    let mut txn = self
                        .filesystem
                        .begin(
                            &scope.to_resource_scope(),
                            &scoped_path(crate::filesystem_service::THREADS_PREFIX)?,
                        )
                        .await?;
                    for row in rows {
                        let entry = if messages {
                            let record = deserialize::<ThreadMessageRecord>(&row.entry.body)?;
                            for (lookup_path, lookup_entry, expectation) in
                                crate::filesystem_service::message_lookup_index::MessageLookupIndexStore::<F>::entries_for_message(
                                    scope,
                                    &thread_id,
                                    &record,
                                )?
                            {
                                let virtual_path = self
                                    .filesystem
                                    .resolve(&scope.to_resource_scope(), &lookup_path)?;
                                if matches!(expectation, CasExpectation::Absent)
                                    && txn.get(&virtual_path).await?.is_some()
                                {
                                    continue;
                                }
                                txn.put(&virtual_path, lookup_entry, expectation).await?;
                            }
                            Self::message_entry(&record)?
                        } else {
                            let record = deserialize::<SummaryArtifact>(&row.entry.body)?;
                            Self::summary_entry(&record)?
                        };
                        txn.put(&row.path, entry, CasExpectation::Version(row.version))
                            .await?;
                    }
                    txn.commit().await?;
                    migrated = migrated.saturating_add(received);
                    if received < Page::MAX_LIMIT as usize {
                        break;
                    }
                    offset = offset.saturating_add(received as u64);
                }
            }
        }
        Ok(migrated)
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
                Entry::bytes(b"transcript-index-v1".to_vec()),
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

    pub(super) async fn thread_record_with_index_overlay(
        &self,
        mut stored: StoredThreadRecord,
    ) -> Result<SessionThreadRecord, SessionThreadError> {
        if let Some(index) = self
            .read_thread_index_record(&stored.record.scope, &stored.record.thread_id)
            .await?
        {
            let same_source_generation = index.record.created_at.is_some()
                && index.record.created_at == stored.record.created_at;
            if same_source_generation {
                stored.record.updated_at = index.record.updated_at;
                if stored.record.title.is_none() {
                    stored.record.title = index.record.title;
                }
            } else {
                self.refresh_thread_index_from_source(
                    &stored.record.scope,
                    &stored.record.thread_id,
                )
                .await?;
            }
        }
        Ok(stored.record)
    }
}

fn thread_index_name() -> Result<IndexName, SessionThreadError> {
    IndexName::new("thread_activity_v2")
        .map_err(|error| SessionThreadError::Backend(error.to_string()))
}

#[derive(Serialize, Deserialize)]
struct ThreadIndexCursor {
    activity_sort: String,
    thread_id: String,
}

impl ThreadIndexCursor {
    fn from_record(record: &SessionThreadRecord) -> Self {
        Self {
            activity_sort: thread_activity_sort_key(record),
            thread_id: record.thread_id.as_str().to_string(),
        }
    }
}

fn thread_index_root(scope: &ThreadScope) -> Result<ScopedPath, SessionThreadError> {
    scoped_path(&format!("{}/thread_index", scope_axes_string(scope)))
}

fn thread_index_migration_marker_path(
    scope: &ThreadScope,
) -> Result<ScopedPath, SessionThreadError> {
    scoped_path(&format!(
        "{}/index-migrations/thread-index-v1.complete",
        scope_axes_string(scope)
    ))
}

fn transcript_index_migration_marker_path(
    scope: &ThreadScope,
) -> Result<ScopedPath, SessionThreadError> {
    scoped_path(&format!(
        "{}/index-migrations/transcript-index-v1.complete",
        scope_axes_string(scope)
    ))
}

pub(super) fn thread_index_record_path(
    scope: &ThreadScope,
    thread_id: &ThreadId,
) -> Result<ScopedPath, SessionThreadError> {
    scoped_path(&format!(
        "{}/thread_index/{}.json",
        scope_axes_string(scope),
        thread_id.as_str()
    ))
}
fn thread_index_cache_key(scope: &ThreadScope) -> String {
    format!("{}:{}", scope.tenant_id.as_str(), scope_axes_string(scope))
}

fn thread_index_record_cache_key(scope: &ThreadScope, thread_id: &ThreadId) -> String {
    format!("{}:{}", thread_index_cache_key(scope), thread_id.as_str())
}

fn thread_index_key(raw: &str) -> Result<IndexKey, SessionThreadError> {
    IndexKey::new(raw).map_err(|error| SessionThreadError::Backend(error.to_string()))
}

fn thread_activity_sort_key(record: &SessionThreadRecord) -> String {
    let timestamp = record
        .updated_at
        .or(record.created_at)
        .map(|value| value.timestamp_micros())
        .unwrap_or(i64::MIN);
    let descending_rank = i128::from(i64::MAX) - i128::from(timestamp);
    format!("{descending_rank:020}")
}

fn evict_hash_set_entry_over_limit(set: &mut HashSet<String>, max_entries: usize, keep: &str) {
    if set.len() <= max_entries {
        return;
    }
    let mut keys = set.iter();
    let victim = match keys.next() {
        Some(first) if first.as_str() == keep => keys.next().cloned(),
        Some(first) => Some(first.clone()),
        None => None,
    };
    if let Some(victim) = victim {
        set.remove(&victim);
    }
}

fn no_op_thread_index_record(scope: ThreadScope, thread_id: ThreadId) -> ThreadIndexRecord {
    ThreadIndexRecord {
        record: SessionThreadRecord {
            scope,
            thread_id,
            created_by_actor_id: String::new(),
            title: None,
            metadata_json: None,
            goal: None,
            created_at: None,
            updated_at: None,
        },
        next_sequence: 0,
        flags: ThreadIndexFlags::default(),
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{
        AgentId, MountAlias, MountGrant, MountPermissions, MountView, ProjectId, TenantId,
        ThreadId, UserId, VirtualPath,
    };

    use crate::{
        EnsureThreadRequest, FilesystemSessionThreadService, ListThreadsForScopeRequest,
        SessionThreadRecord, SessionThreadService, ThreadScope,
    };

    use super::super::thread_record_path;
    use super::{ThreadIndexFlags, ThreadIndexRecord};

    #[test]
    fn merge_thread_index_records_prefers_present_source_fields() {
        let request_scope = scope("merge-source-fields");
        let thread_id = ThreadId::new("thread-merge-source-fields").unwrap();
        let created_at = chrono::Utc::now();
        let source = ThreadIndexRecord {
            record: SessionThreadRecord {
                scope: request_scope.clone(),
                thread_id: thread_id.clone(),
                created_by_actor_id: "actor-a".into(),
                title: Some("source title".into()),
                metadata_json: Some("{\"source\":true}".into()),
                goal: None,
                created_at: Some(created_at),
                updated_at: Some(created_at),
            },
            next_sequence: 3,
            flags: ThreadIndexFlags {
                title_present: true,
                metadata_present: true,
                goal_present: false,
            },
        };
        let existing = ThreadIndexRecord {
            record: SessionThreadRecord {
                scope: request_scope,
                thread_id,
                created_by_actor_id: "actor-a".into(),
                title: Some("stale title".into()),
                metadata_json: Some("{\"stale\":true}".into()),
                goal: None,
                created_at: Some(created_at),
                updated_at: Some(created_at),
            },
            next_sequence: 7,
            flags: ThreadIndexFlags {
                title_present: true,
                metadata_present: true,
                goal_present: false,
            },
        };

        let merged = FilesystemSessionThreadService::<InMemoryBackend>::merge_thread_index_records(
            source, existing,
        )
        .unwrap();

        assert_eq!(merged.record.title.as_deref(), Some("source title"));
        assert_eq!(
            merged.record.metadata_json.as_deref(),
            Some("{\"source\":true}")
        );
        assert_eq!(merged.next_sequence, 7);
    }

    fn scope(label: &str) -> ThreadScope {
        ThreadScope {
            tenant_id: TenantId::new(format!("tenant-{label}")).unwrap(),
            agent_id: AgentId::new(format!("agent-{label}")).unwrap(),
            project_id: Some(ProjectId::new(format!("project-{label}")).unwrap()),
            owner_user_id: Some(UserId::new(format!("user-{label}")).unwrap()),
            mission_id: None,
        }
    }

    fn scoped_threads_fs_at(
        backend: Arc<InMemoryBackend>,
        tenant: &str,
        user: &str,
    ) -> Arc<ScopedFilesystem<InMemoryBackend>> {
        let target = format!("/tenants/{tenant}/users/{user}/threads");
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/threads").expect("alias"),
            VirtualPath::new(target).expect("target"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("mount view");
        Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts))
    }

    #[tokio::test]
    async fn filesystem_thread_index_missing_touch_does_not_hide_recreated_thread() {
        let backend = Arc::new(InMemoryBackend::new());
        let scoped = scoped_threads_fs_at(backend, "tenant-missing-touch", "alice");
        let service = FilesystemSessionThreadService::new(Arc::clone(&scoped));
        let request_scope = scope("missing-touch");
        let thread_id = ThreadId::new("thread-missing-touch").unwrap();

        service
            .touch_thread_index_updated_at(&request_scope, &thread_id, chrono::Utc::now())
            .await
            .expect("missing touch is a no-op");

        service
            .ensure_thread(EnsureThreadRequest {
                scope: request_scope.clone(),
                thread_id: Some(thread_id.clone()),
                created_by_actor_id: "actor-a".into(),
                title: Some("recreated".into()),
                metadata_json: None,
            })
            .await
            .unwrap();

        let listed = service
            .list_threads_for_scope(ListThreadsForScopeRequest {
                scope: request_scope,
                limit: None,
                cursor: None,
            })
            .await
            .unwrap();
        assert!(
            listed
                .threads
                .iter()
                .any(|record| record.thread_id == thread_id),
            "a no-op touch for a missing row must not suppress index creation after recreate"
        );
    }

    #[tokio::test]
    async fn filesystem_thread_index_recreate_does_not_reuse_stale_metadata() {
        let backend = Arc::new(InMemoryBackend::new());
        let scoped = scoped_threads_fs_at(backend, "tenant-stale-index", "alice");
        let service = FilesystemSessionThreadService::new(Arc::clone(&scoped));
        let request_scope = scope("stale-index");
        let thread_id = ThreadId::new("thread-stale-index").unwrap();

        service
            .ensure_thread(EnsureThreadRequest {
                scope: request_scope.clone(),
                thread_id: Some(thread_id.clone()),
                created_by_actor_id: "actor-a".into(),
                title: Some("deleted title".into()),
                metadata_json: Some("{\"deleted\":true}".into()),
            })
            .await
            .unwrap();
        scoped
            .delete(
                &request_scope.to_resource_scope(),
                &thread_record_path(&request_scope, &thread_id).unwrap(),
            )
            .await
            .expect("test setup deletes only source thread row");
        tokio::time::sleep(Duration::from_millis(2)).await;

        service
            .ensure_thread(EnsureThreadRequest {
                scope: request_scope.clone(),
                thread_id: Some(thread_id.clone()),
                created_by_actor_id: "actor-a".into(),
                title: Some("recreated title".into()),
                metadata_json: Some("{\"recreated\":true}".into()),
            })
            .await
            .unwrap();
        service.clear_thread_index_cache_for_scope(&request_scope);

        let listed = service
            .list_threads_for_scope(ListThreadsForScopeRequest {
                scope: request_scope,
                limit: None,
                cursor: None,
            })
            .await
            .unwrap();
        let recreated = listed
            .threads
            .iter()
            .find(|record| record.thread_id == thread_id)
            .expect("recreated thread is listed");

        assert_eq!(recreated.title.as_deref(), Some("recreated title"));
        assert_eq!(
            recreated.metadata_json.as_deref(),
            Some("{\"recreated\":true}")
        );
    }
}
