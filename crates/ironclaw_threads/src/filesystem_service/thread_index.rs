use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use chrono::{DateTime, Utc};
use futures::StreamExt;
use ironclaw_filesystem::{
    CasApply, ContentType, Entry, FileType, Filter, Page, RecordKind, RootFilesystem, cas_update,
};
use ironclaw_host_api::{ScopedPath, ThreadId};
use serde::{Deserialize, Serialize};

use crate::{FilesystemSessionThreadService, SessionThreadError, SessionThreadRecord, ThreadScope};

use super::{
    LIST_THREADS_MISSING_INDEX_READ_CONCURRENCY, StoredThreadRecord, deserialize, invalid_path,
    is_not_found, map_cas_error, scope_axes_string, scoped_path, serialize_pretty, weak_keyed_lock,
};

const THREAD_INDEX_KIND: &str = "thread_index";
const THREAD_INDEX_CACHE_MAX_SCOPES: usize = 128;
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
        Ok(entry)
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

    pub(super) fn invalidate_thread_index_cache(&self, scope: &ThreadScope) {
        let key = thread_index_cache_key(scope);
        if let Ok(mut cache) = self.thread_index_cache.lock() {
            cache.remove(&key);
        }
        if let Ok(mut positions) = self.thread_index_cursor_positions.lock() {
            positions.remove(&key);
        }
        if let Ok(mut epochs) = self.thread_index_cache_epochs.lock() {
            let epoch = epochs.entry(key).or_insert(0);
            *epoch = epoch.saturating_add(1);
        }
    }

    pub(super) fn clear_thread_index_cache_for_scope_once(&self, scope: &ThreadScope) {
        let key = thread_index_cache_key(scope);
        let current_epoch = self.thread_index_cache_epoch(&key);
        let should_clear = self
            .thread_index_manual_clear_epochs
            .lock()
            .map(|mut manual_clears| {
                if manual_clears.get(&key).copied() == Some(current_epoch) {
                    return false;
                }
                manual_clears.insert(key.clone(), current_epoch.saturating_add(1));
                true
            })
            .unwrap_or(true);
        if should_clear {
            let had_cache = self
                .thread_index_cache
                .lock()
                .map(|cache| cache.contains_key(&key))
                .unwrap_or(false);
            if had_cache {
                self.force_thread_source_validation(&key);
            }
            self.invalidate_thread_index_cache(scope);
        }
    }

    fn force_thread_source_validation(&self, key: &str) {
        if let Ok(mut scopes) = self.thread_index_force_validate_scopes.lock() {
            scopes.insert(key.to_string());
        }
    }

    pub(super) fn forget_thread_index_row(&self, scope: &ThreadScope, thread_id: &ThreadId) {
        if let Ok(mut known) = self.known_thread_index_rows.lock() {
            known.remove(&thread_index_record_cache_key(scope, thread_id));
        }
        self.forget_thread_source_row(scope, thread_id);
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
                self.invalidate_thread_index_cache(scope);
                return Err(error.into());
            }
        }
        self.forget_thread_index_row(scope, thread_id);
        self.invalidate_thread_index_cache(scope);
        Ok(())
    }

    fn mark_thread_index_known(&self, scope: &ThreadScope, thread_id: &ThreadId) {
        if let Ok(mut known) = self.known_thread_index_rows.lock() {
            let key = thread_index_record_cache_key(scope, thread_id);
            known.insert(key.clone());
            evict_hash_set_entry_over_limit(&mut known, THREAD_INDEX_KNOWN_ROW_MAX, &key);
        }
    }

    fn mark_thread_source_known(&self, scope: &ThreadScope, thread_id: &ThreadId) {
        if let Ok(mut known) = self.known_thread_source_rows.lock() {
            known
                .entry(thread_index_cache_key(scope))
                .or_default()
                .insert(thread_id.clone());
        }
    }

    fn forget_thread_source_row(&self, scope: &ThreadScope, thread_id: &ThreadId) {
        if let Ok(mut known) = self.known_thread_source_rows.lock() {
            let key = thread_index_cache_key(scope);
            if let Some(ids) = known.get_mut(&key) {
                ids.remove(thread_id);
                if ids.is_empty() {
                    known.remove(&key);
                }
            }
        }
    }

    pub(super) fn is_thread_index_known(&self, scope: &ThreadScope, thread_id: &ThreadId) -> bool {
        self.known_thread_index_rows
            .lock()
            .map(|known| known.contains(&thread_index_record_cache_key(scope, thread_id)))
            .unwrap_or(false)
    }

    fn mark_thread_index_scope_complete(&self, scope: &ThreadScope) {
        if let Ok(mut complete) = self.complete_thread_index_scopes.lock() {
            let key = thread_index_cache_key(scope);
            complete.insert(key.clone());
            evict_hash_set_entry_over_limit(&mut complete, THREAD_INDEX_CACHE_MAX_SCOPES, &key);
        }
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
        self.merge_thread_index_record_from_source(index, true)
            .await?;
        Ok(())
    }

    async fn merge_thread_index_record_from_source(
        &self,
        source: ThreadIndexRecord,
        invalidate_cache: bool,
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
        self.mark_thread_source_known(&merged.record.scope, &merged.record.thread_id);
        if invalidate_cache {
            self.invalidate_thread_index_cache(&merged.record.scope);
        }
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
            self.mark_thread_source_known(scope, thread_id);
            self.invalidate_thread_index_cache(scope);
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

    pub(super) async fn cached_thread_index_for_scope(
        &self,
        scope: &ThreadScope,
    ) -> Result<Arc<Vec<ThreadIndexRecord>>, SessionThreadError> {
        let key = thread_index_cache_key(scope);
        if let Some(cached) = self.cached_thread_index(&key) {
            return Ok(cached);
        }

        let load_lock = self.thread_index_load_lock(&key);
        let _guard = load_lock.lock().await;
        if let Some(cached) = self.cached_thread_index(&key) {
            return Ok(cached);
        }

        let start_epoch = self.thread_index_cache_epoch(&key);
        let loaded = self.load_thread_index_for_scope(scope).await?;
        let cacheable = loaded.cacheable;
        let records = Arc::new(loaded.records);
        if cacheable && self.thread_index_cache_epoch(&key) == start_epoch {
            self.store_thread_index_cache(key, Arc::clone(&records));
        }
        Ok(records)
    }

    async fn load_thread_index_for_scope(
        &self,
        scope: &ThreadScope,
    ) -> Result<ThreadIndexLoad, SessionThreadError> {
        let entries = self.query_thread_index_records(scope).await?;
        let source = self.thread_source_listing_for_index_load(scope).await?;
        let mut by_thread_id = HashMap::new();
        let mut indexed_by_thread_id = entries
            .into_iter()
            .map(|record| (record.record.thread_id.clone(), record))
            .collect::<HashMap<_, _>>();

        let mut missing_thread_ids = Vec::new();
        for thread_id in &source.thread_ids {
            if let Some(existing) = indexed_by_thread_id.remove(thread_id) {
                by_thread_id.insert(thread_id.clone(), existing);
            } else {
                missing_thread_ids.push(thread_id.clone());
            }
        }

        let missing = self
            .bootstrap_missing_thread_index_records(scope, missing_thread_ids)
            .await?;
        for source_record in missing.records {
            let thread_id = source_record.record.thread_id.clone();
            by_thread_id.insert(thread_id, source_record);
        }

        if source.complete {
            for stale in indexed_by_thread_id.into_values() {
                self.delete_thread_index_record(&stale.record.scope, &stale.record.thread_id)
                    .await?;
            }
            if missing.complete {
                self.mark_thread_index_scope_complete(scope);
            }
        } else {
            by_thread_id.extend(indexed_by_thread_id);
        }
        let mut entries: Vec<_> = by_thread_id.into_values().collect();
        entries.sort_by(|a, b| compare_thread_activity(&a.record, &b.record));
        Ok(ThreadIndexLoad {
            records: entries,
            cacheable: source.complete && missing.complete,
        })
    }

    fn cached_thread_index(&self, key: &str) -> Option<Arc<Vec<ThreadIndexRecord>>> {
        self.thread_index_cache
            .lock()
            .ok()
            .and_then(|cache| cache.get(key).cloned())
    }

    fn thread_index_cache_epoch(&self, key: &str) -> u64 {
        self.thread_index_cache_epochs
            .lock()
            .ok()
            .and_then(|epochs| epochs.get(key).copied())
            .unwrap_or(0)
    }

    fn store_thread_index_cache(&self, key: String, value: Arc<Vec<ThreadIndexRecord>>) {
        let cursor_positions = Arc::new(
            value
                .iter()
                .enumerate()
                .map(|(index, record)| (record.record.thread_id.as_str().to_string(), index))
                .collect::<HashMap<_, _>>(),
        );
        if let Ok(mut cache) = self.thread_index_cache.lock() {
            cache.insert(key.clone(), value);
            evict_hash_map_entry_over_limit(&mut cache, THREAD_INDEX_CACHE_MAX_SCOPES, &key);
        }
        if let Ok(mut positions) = self.thread_index_cursor_positions.lock() {
            positions.insert(key.clone(), cursor_positions);
            evict_hash_map_entry_over_limit(&mut positions, THREAD_INDEX_CACHE_MAX_SCOPES, &key);
        }
    }

    pub(super) fn thread_index_start_after_cursor(
        &self,
        scope: &ThreadScope,
        cursor: &str,
        fallback: impl FnOnce() -> usize,
    ) -> usize {
        let key = thread_index_cache_key(scope);
        self.thread_index_cursor_positions
            .lock()
            .ok()
            .and_then(|positions| positions.get(&key).cloned())
            .and_then(|positions| positions.get(cursor).copied())
            .map(|index| index + 1)
            .unwrap_or_else(fallback)
    }

    fn thread_index_load_lock(&self, key: &str) -> Arc<tokio::sync::Mutex<()>> {
        weak_keyed_lock(&self.thread_index_load_locks, key.to_string())
    }

    async fn thread_source_listing_for_index_load(
        &self,
        scope: &ThreadScope,
    ) -> Result<ThreadSourceListing, SessionThreadError> {
        let key = thread_index_cache_key(scope);
        let force_validate = self
            .thread_index_force_validate_scopes
            .lock()
            .map(|scopes| scopes.contains(&key))
            .unwrap_or(true);
        if !force_validate && let Some(thread_ids) = self.cached_thread_source_ids(&key) {
            return Ok(ThreadSourceListing {
                thread_ids,
                complete: true,
            });
        }
        let listing = self.list_thread_source_ids(scope).await?;
        self.store_thread_source_ids(key, &listing.thread_ids);
        Ok(listing)
    }

    fn cached_thread_source_ids(&self, key: &str) -> Option<HashSet<ThreadId>> {
        let source_complete = self
            .complete_thread_source_scopes
            .lock()
            .ok()
            .map(|complete| complete.contains(key))
            .unwrap_or(false);
        if !source_complete {
            return None;
        }
        self.known_thread_source_rows
            .lock()
            .ok()
            .and_then(|known| known.get(key).cloned())
    }

    fn store_thread_source_ids(&self, key: String, ids: &HashSet<ThreadId>) {
        if let Ok(mut known) = self.known_thread_source_rows.lock() {
            known.insert(key.clone(), ids.clone());
            evict_hash_map_entry_over_limit(&mut known, THREAD_INDEX_CACHE_MAX_SCOPES, &key);
        }
        if let Ok(mut complete) = self.complete_thread_source_scopes.lock() {
            complete.insert(key.clone());
            evict_hash_set_entry_over_limit(&mut complete, THREAD_INDEX_CACHE_MAX_SCOPES, &key);
        }
        if let Ok(mut scopes) = self.thread_index_force_validate_scopes.lock() {
            scopes.remove(&key);
        }
    }

    async fn query_thread_index_records(
        &self,
        scope: &ThreadScope,
    ) -> Result<Vec<ThreadIndexRecord>, SessionThreadError> {
        let root = thread_index_root(scope)?;
        let mut records = Vec::new();
        let mut offset = 0_u64;
        loop {
            let page = self
                .filesystem
                .query(
                    &scope.to_resource_scope(),
                    &root,
                    &Filter::All,
                    Page::new(offset, Page::MAX_LIMIT),
                )
                .await?;
            if page.is_empty() {
                break;
            }
            offset += page.len() as u64;
            for versioned in page {
                let record = deserialize::<ThreadIndexRecord>(&versioned.entry.body)?;
                if record.record.scope == *scope {
                    self.mark_thread_index_known(&record.record.scope, &record.record.thread_id);
                    records.push(record);
                }
            }
        }
        Ok(records)
    }

    async fn list_thread_source_ids(
        &self,
        scope: &ThreadScope,
    ) -> Result<ThreadSourceListing, SessionThreadError> {
        let root = thread_source_root(scope)?;
        let entries = match self
            .filesystem
            .list_dir(&scope.to_resource_scope(), &root)
            .await
        {
            Ok(entries) => entries,
            Err(error) if is_not_found(&error) => {
                return Ok(ThreadSourceListing {
                    thread_ids: HashSet::new(),
                    complete: true,
                });
            }
            Err(error) => return Err(error.into()),
        };
        let thread_ids = entries
            .into_iter()
            .filter(|entry| entry.file_type == FileType::Directory)
            .map(|entry| ThreadId::new(entry.name).map_err(invalid_path))
            .collect::<Result<HashSet<_>, _>>()?;
        Ok(ThreadSourceListing {
            thread_ids,
            complete: true,
        })
    }

    async fn bootstrap_missing_thread_index_records(
        &self,
        scope: &ThreadScope,
        thread_ids: Vec<ThreadId>,
    ) -> Result<ThreadIndexBootstrap, SessionThreadError> {
        if thread_ids.is_empty() {
            return Ok(ThreadIndexBootstrap {
                records: Vec::new(),
                complete: true,
            });
        }
        let reads: Vec<(ThreadId, ThreadRecordReadResult)> = futures::stream::iter(thread_ids)
            .map(|tid| async move {
                let result = self.read_thread_versioned(scope, &tid).await;
                (tid, result)
            })
            .buffer_unordered(LIST_THREADS_MISSING_INDEX_READ_CONCURRENCY)
            .collect()
            .await;

        let mut records = Vec::with_capacity(reads.len());
        let mut complete = true;
        for (thread_id, result) in reads {
            match result {
                Ok(Some((stored, _))) if stored.record.scope == *scope => {
                    let index = Self::thread_index_record(&stored);
                    let index = self
                        .merge_thread_index_record_from_source(index, false)
                        .await?;
                    records.push(index);
                }
                Ok(_) => {}
                Err(error) => {
                    complete = false;
                    tracing::debug!(
                        thread_id = %thread_id.as_str(),
                        scope = ?scope,
                        ?error,
                        "skipping unreadable thread record during thread index bootstrap",
                    );
                }
            }
        }
        Ok(ThreadIndexBootstrap { records, complete })
    }
}

struct ThreadSourceListing {
    thread_ids: HashSet<ThreadId>,
    complete: bool,
}

struct ThreadIndexLoad {
    records: Vec<ThreadIndexRecord>,
    cacheable: bool,
}

#[derive(Default)]
struct ThreadIndexBootstrap {
    records: Vec<ThreadIndexRecord>,
    complete: bool,
}

type ThreadRecordReadResult =
    Result<Option<(StoredThreadRecord, ironclaw_filesystem::RecordVersion)>, SessionThreadError>;

fn thread_index_root(scope: &ThreadScope) -> Result<ScopedPath, SessionThreadError> {
    scoped_path(&format!("{}/thread_index", scope_axes_string(scope)))
}

fn thread_source_root(scope: &ThreadScope) -> Result<ScopedPath, SessionThreadError> {
    scoped_path(&format!("{}/threads", scope_axes_string(scope)))
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

fn evict_hash_map_entry_over_limit<T>(
    map: &mut HashMap<String, T>,
    max_entries: usize,
    keep: &str,
) {
    if map.len() <= max_entries {
        return;
    }
    let mut keys = map.keys();
    let victim = match keys.next() {
        Some(first) if first.as_str() == keep => keys.next().cloned(),
        Some(first) => Some(first.clone()),
        None => None,
    };
    if let Some(victim) = victim {
        map.remove(&victim);
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

fn compare_thread_activity(a: &SessionThreadRecord, b: &SessionThreadRecord) -> std::cmp::Ordering {
    let a_key = a.updated_at.or(a.created_at);
    let b_key = b.updated_at.or(b.created_at);
    std::cmp::Reverse(a_key)
        .cmp(&std::cmp::Reverse(b_key))
        .then_with(|| a.thread_id.as_str().cmp(b.thread_id.as_str()))
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
        service.mark_thread_index_scope_complete(&request_scope);

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
