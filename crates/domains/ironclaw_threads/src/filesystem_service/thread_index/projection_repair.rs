//! One-time recovery for thread-index rows whose listing projection is absent.
//!
//! Repair is scheduled after a scope is first listed, but it never runs on the
//! listing request itself. A small admission limit prevents one process from
//! accumulating unbounded repair tasks. Completion is durable and written only
//! after every row succeeds; failures remain retryable on a later listing.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use ironclaw_filesystem::{
    CasApply, CasExpectation, Entry, FileType, Page, RootFilesystem, ScopedFilesystem, cas_update,
};
use ironclaw_host_api::{ids::ThreadId, path::ScopedPath};

use crate::{FilesystemSessionThreadService, SessionThreadError, ThreadScope};

use super::{
    THREAD_INDEX_SCOPE_CACHE_MAX_ENTRIES, THREAD_INDEX_SUFFIX, ThreadIndexRecord,
    evict_entry_over_limit, no_op_thread_index_record, thread_activity_index_spec,
    thread_index_cache_key, thread_index_record_path, thread_index_root,
};
use crate::filesystem_service::{
    deserialize, invalid_path, is_not_found, map_cas_error, scoped_path,
};

const MAX_CONCURRENT_PROJECTION_REPAIRS: usize = 2;
const MAX_PENDING_PROJECTION_REPAIRS: usize = 128;
const MAX_PROJECTION_REPAIR_FAILURES_PER_SCOPE: u32 = 3;

pub(crate) struct ThreadIndexProjectionRepairState {
    active_scopes: Mutex<HashSet<String>>,
    completed_scopes: Mutex<HashSet<String>>,
    failed_scope_attempts: Mutex<HashMap<String, u32>>,
    permits: Arc<tokio::sync::Semaphore>,
}

impl Default for ThreadIndexProjectionRepairState {
    fn default() -> Self {
        Self {
            active_scopes: Mutex::new(HashSet::new()),
            completed_scopes: Mutex::new(HashSet::new()),
            failed_scope_attempts: Mutex::new(HashMap::new()),
            permits: Arc::new(tokio::sync::Semaphore::new(
                MAX_CONCURRENT_PROJECTION_REPAIRS,
            )),
        }
    }
}

impl<F> FilesystemSessionThreadService<F>
where
    F: RootFilesystem + 'static,
{
    /// Schedule a retryable repair without adding latency to the listing that
    /// discovers the scope. The pending set is bounded; if it is full, a later
    /// listing retries admission after earlier repairs have drained.
    pub(super) fn schedule_thread_index_projection_repair(&self, scope: &ThreadScope) {
        let scope_key = thread_index_cache_key(scope);
        if self
            .thread_index_projection_repair_state
            .completed_scopes
            .lock()
            .is_ok_and(|completed| completed.contains(&scope_key))
        {
            return;
        }
        if self
            .thread_index_projection_repair_state
            .failed_scope_attempts
            .lock()
            .is_ok_and(|failed| {
                failed.get(&scope_key).copied().unwrap_or_default()
                    >= MAX_PROJECTION_REPAIR_FAILURES_PER_SCOPE
            })
        {
            return;
        }
        {
            let Ok(mut active) = self
                .thread_index_projection_repair_state
                .active_scopes
                .lock()
            else {
                return;
            };
            if active.contains(&scope_key) || active.len() >= MAX_PENDING_PROJECTION_REPAIRS {
                return;
            }
            active.insert(scope_key.clone());
        }

        let filesystem = Arc::clone(&self.filesystem);
        let state = Arc::clone(&self.thread_index_projection_repair_state);
        let scope = scope.clone();
        tokio::spawn(async move {
            let Ok(permit) = Arc::clone(&state.permits).acquire_owned().await else {
                if let Ok(mut active) = state.active_scopes.lock() {
                    active.remove(&scope_key);
                }
                return;
            };
            let result = repair_thread_index_projection(Arc::clone(&filesystem), &scope).await;
            let failed_attempts = if result.is_ok() {
                if let Ok(mut failed) = state.failed_scope_attempts.lock() {
                    failed.remove(&scope_key);
                }
                if let Ok(mut completed) = state.completed_scopes.lock() {
                    completed.insert(scope_key.clone());
                    evict_entry_over_limit(
                        &mut completed,
                        THREAD_INDEX_SCOPE_CACHE_MAX_ENTRIES,
                        &scope_key,
                    );
                }
                0
            } else if let Ok(mut failed) = state.failed_scope_attempts.lock() {
                if failed.len() >= THREAD_INDEX_SCOPE_CACHE_MAX_ENTRIES
                    && !failed.contains_key(&scope_key)
                    && let Some(victim) = failed.keys().next().cloned()
                {
                    failed.remove(&victim);
                }
                let attempts = failed.entry(scope_key.clone()).or_default();
                *attempts = attempts.saturating_add(1);
                *attempts
            } else {
                0
            };
            if let Err(error) = &result {
                if failed_attempts >= MAX_PROJECTION_REPAIR_FAILURES_PER_SCOPE {
                    tracing::warn!(
                        error = %error,
                        attempts = failed_attempts,
                        "thread-index projection repair exhausted its per-process retry budget"
                    );
                } else {
                    tracing::debug!(
                        error = %error,
                        attempts = failed_attempts,
                        "background thread-index projection repair failed; a later listing will retry"
                    );
                }
            }
            if let Ok(mut active) = state.active_scopes.lock() {
                active.remove(&scope_key);
            }
            drop(permit);
        });
    }
}

async fn repair_thread_index_projection<F>(
    filesystem: Arc<ScopedFilesystem<F>>,
    scope: &ThreadScope,
) -> Result<(), SessionThreadError>
where
    F: RootFilesystem + 'static,
{
    let marker = thread_index_projection_repair_marker_path(scope)?;
    if filesystem
        .get(&scope.to_resource_scope(), &marker)
        .await?
        .is_some()
    {
        return Ok(());
    }

    let root = thread_index_root(scope)?;
    let page_limit = Page::MAX_LIMIT as usize;
    let mut after = None;
    loop {
        let page = match filesystem
            .list_dir_page(
                &scope.to_resource_scope(),
                &root,
                after.as_deref(),
                page_limit,
            )
            .await
        {
            Ok(entries) => entries,
            Err(error) if is_not_found(&error) => Vec::new(),
            Err(error) => return Err(error.into()),
        };
        if page.is_empty() {
            break;
        }
        let page_len = page.len();
        for entry in &page {
            if entry.file_type != FileType::File {
                continue;
            }
            let Some(raw_id) = entry.name.strip_suffix(THREAD_INDEX_SUFFIX) else {
                continue;
            };
            let thread_id = ThreadId::new(raw_id.to_string()).map_err(invalid_path)?;
            restore_thread_index_projection(filesystem.as_ref(), scope, &thread_id).await?;
        }
        after = page.last().map(|entry| entry.name.clone());
        tokio::task::yield_now().await;
        if page_len < page_limit {
            break;
        }
    }

    filesystem
        .put(
            &scope.to_resource_scope(),
            &marker,
            Entry::bytes(b"thread-index-projection-v2".to_vec()),
            CasExpectation::Any,
        )
        .await?;
    if filesystem
        .get(&scope.to_resource_scope(), &marker)
        .await?
        .is_none()
    {
        return Err(SessionThreadError::Backend(
            "thread-index projection repair marker was not durable after write".to_string(),
        ));
    }
    Ok(())
}

async fn restore_thread_index_projection<F>(
    filesystem: &ScopedFilesystem<F>,
    scope: &ThreadScope,
    thread_id: &ThreadId,
) -> Result<(), SessionThreadError>
where
    F: RootFilesystem + 'static,
{
    let path = thread_index_record_path(scope, thread_id)?;
    let Some(versioned) = filesystem.get(&scope.to_resource_scope(), &path).await? else {
        return Ok(());
    };
    let record = deserialize::<ThreadIndexRecord>(&versioned.entry.body)?;
    if record.record.scope != *scope || record.record.thread_id != *thread_id {
        return Ok(());
    }
    let rebuilt = FilesystemSessionThreadService::<F>::thread_index_entry(&record)?;
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
    cas_update(
        filesystem,
        &resource_scope,
        &path,
        |bytes: &[u8]| deserialize::<ThreadIndexRecord>(bytes),
        |record: &ThreadIndexRecord| {
            FilesystemSessionThreadService::<F>::thread_index_entry(record)
        },
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
                if record.record.scope != scope || record.record.thread_id != thread_id {
                    return Ok(CasApply::no_op(record, false));
                }
                Ok(CasApply::force_write(record, true))
            }
        },
    )
    .await
    .map_err(map_cas_error)?;
    Ok(())
}

fn thread_index_projection_repair_marker_path(
    scope: &ThreadScope,
) -> Result<ScopedPath, SessionThreadError> {
    scoped_path(&format!(
        "{}/index-migrations/thread-index-projection-v2.complete",
        super::scope_axes_string(scope)
    ))
}
