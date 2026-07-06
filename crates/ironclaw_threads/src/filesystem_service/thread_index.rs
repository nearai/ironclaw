use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use chrono::Utc;
use futures::StreamExt;
use ironclaw_filesystem::{
    CasApply, ContentType, Entry, FileType, Filter, Page, RecordKind, RecordVersion,
    RootFilesystem, cas_update,
};
use ironclaw_host_api::{ScopedPath, ThreadId};
use serde::{Deserialize, Serialize};

use crate::{FilesystemSessionThreadService, SessionThreadError, SessionThreadRecord, ThreadScope};

use super::{
    LIST_THREADS_RECORD_READ_CONCURRENCY, StoredThreadRecord, deserialize, invalid_path,
    is_not_found, map_cas_error, scope_axes_string, scoped_path, serialize_pretty, thread_root,
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

    fn mark_thread_index_scope_complete(&self, scope: &ThreadScope) {
        if let Ok(mut complete) = self.complete_thread_index_scopes.lock() {
            let key = thread_index_cache_key(scope);
            complete.insert(key.clone());
            evict_hash_set_entry_over_limit(&mut complete, THREAD_INDEX_CACHE_MAX_SCOPES, &key);
        }
    }

    fn is_thread_index_scope_complete(&self, scope: &ThreadScope) -> bool {
        self.complete_thread_index_scopes
            .lock()
            .map(|complete| complete.contains(&thread_index_cache_key(scope)))
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
        self.invalidate_thread_index_cache(&merged.record.scope);
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
        if same_source_generation && existing.flags.title_present {
            source.record.title = existing.record.title;
            source.flags.title_present = true;
        }
        if same_source_generation && existing.flags.metadata_present {
            source.record.metadata_json = existing.record.metadata_json;
            source.flags.metadata_present = true;
        }
        if same_source_generation && existing.flags.goal_present {
            source.record.goal = existing.record.goal;
            source.flags.goal_present = true;
        }
        Ok(source)
    }

    pub(super) async fn touch_thread_index_updated_at(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
    ) -> Result<(), SessionThreadError> {
        let now = Utc::now();
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
                            stored.record.updated_at = Some(now);
                            Self::thread_index_record(&stored)
                        }
                    };
                    index.record.updated_at = Some(now);
                    Ok(CasApply::new(index, true))
                }
            },
        )
        .await
        .map_err(map_cas_error)?;
        if row_known {
            self.mark_thread_index_known(scope, thread_id);
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
            stored.record.updated_at = index.record.updated_at;
            if stored.record.title.is_none() {
                stored.record.title = index.record.title;
            }
        }
        Ok(stored.record)
    }

    pub(super) async fn cached_thread_index_for_scope(
        &self,
        scope: &ThreadScope,
    ) -> Result<Arc<Vec<ThreadIndexRecord>>, SessionThreadError> {
        self.load_thread_index_for_scope(scope).await.map(Arc::new)
    }

    async fn load_thread_index_for_scope(
        &self,
        scope: &ThreadScope,
    ) -> Result<Vec<ThreadIndexRecord>, SessionThreadError> {
        let mut entries = self.query_thread_index_records(scope).await?;
        if !self.is_thread_index_scope_complete(scope) {
            let bootstrap = self.bootstrap_thread_index_for_scope(scope).await?;
            let mut by_thread_id = entries
                .into_iter()
                .map(|record| (record.record.thread_id.clone(), record))
                .collect::<HashMap<_, _>>();
            for record in bootstrap.records {
                by_thread_id.insert(record.record.thread_id.clone(), record);
            }
            entries = by_thread_id.into_values().collect();
            if bootstrap.complete {
                self.mark_thread_index_scope_complete(scope);
            }
        }
        entries.sort_by(|a, b| compare_thread_activity(&a.record, &b.record));
        Ok(entries)
    }

    async fn query_thread_index_records(
        &self,
        scope: &ThreadScope,
    ) -> Result<Vec<ThreadIndexRecord>, SessionThreadError> {
        let root = thread_index_root(scope)?;
        let mut records = Vec::new();
        let mut stale_records = Vec::new();
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
                    if self
                        .thread_index_source_exists(&record.record.scope, &record.record.thread_id)
                        .await?
                    {
                        self.mark_thread_index_known(
                            &record.record.scope,
                            &record.record.thread_id,
                        );
                        records.push(record);
                    } else {
                        stale_records.push((record.record.scope, record.record.thread_id));
                    }
                }
            }
        }
        for (scope, thread_id) in stale_records {
            self.delete_thread_index_record(&scope, &thread_id).await?;
        }
        Ok(records)
    }

    async fn thread_index_source_exists(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
    ) -> Result<bool, SessionThreadError> {
        let root = thread_root(scope, thread_id)?;
        match self
            .filesystem
            .stat(&scope.to_resource_scope(), &root)
            .await
        {
            Ok(_) => Ok(true),
            Err(error) if is_not_found(&error) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    async fn bootstrap_thread_index_for_scope(
        &self,
        scope: &ThreadScope,
    ) -> Result<ThreadIndexBootstrap, SessionThreadError> {
        let root = scoped_path(&format!("{}/threads", scope_axes_string(scope)))?;
        let entries = match self
            .filesystem
            .list_dir(&scope.to_resource_scope(), &root)
            .await
        {
            Ok(entries) => entries,
            Err(error) if is_not_found(&error) => {
                return Ok(ThreadIndexBootstrap {
                    records: Vec::new(),
                    complete: true,
                });
            }
            Err(error) => return Err(error.into()),
        };
        let thread_ids: Vec<ThreadId> = entries
            .into_iter()
            .filter(|entry| entry.file_type == FileType::Directory)
            .map(|entry| ThreadId::new(entry.name).map_err(invalid_path))
            .collect::<Result<_, _>>()?;
        let reads: Vec<(ThreadId, ThreadRecordReadResult)> = futures::stream::iter(thread_ids)
            .map(|tid| async move {
                let result = self.read_thread_versioned(scope, &tid).await;
                (tid, result)
            })
            .buffer_unordered(LIST_THREADS_RECORD_READ_CONCURRENCY)
            .collect()
            .await;

        let mut records = Vec::with_capacity(reads.len());
        let mut complete = true;
        for (thread_id, result) in reads {
            match result {
                Ok(Some((stored, _))) if stored.record.scope == *scope => {
                    let index = Self::thread_index_record(&stored);
                    let index = self.merge_thread_index_record_from_source(index).await?;
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

#[derive(Default)]
struct ThreadIndexBootstrap {
    records: Vec<ThreadIndexRecord>,
    complete: bool,
}

type ThreadRecordReadResult =
    Result<Option<(StoredThreadRecord, RecordVersion)>, SessionThreadError>;

fn thread_index_root(scope: &ThreadScope) -> Result<ScopedPath, SessionThreadError> {
    scoped_path(&format!("{}/thread_index", scope_axes_string(scope)))
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
        SessionThreadService, ThreadScope,
    };

    use super::super::thread_record_path;

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
            .touch_thread_index_updated_at(&request_scope, &thread_id)
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
