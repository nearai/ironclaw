use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    sync::Arc,
    time::SystemTime,
};

use tokio::sync::{Mutex, OwnedMutexGuard, RwLock};

use super::{CodingCapabilityError, CodingCapabilityRequest};

use super::guest_error;

pub(crate) type SharedCodingReadState = Arc<RwLock<CodingReadState>>;
pub(crate) type SharedCodingEditLocks = Arc<CodingEditLocks>;

/// Striped per-path async locks that serialize the read+write critical
/// section of `write_file` / `apply_patch` against concurrent edits to the
/// same scope+virtual path. A fixed stripe count keeps memory bounded even
/// when callers churn through unique paths.
const EDIT_LOCK_STRIPES: usize = 64;

#[derive(Debug)]
pub(crate) struct CodingEditLocks {
    stripes: Vec<Arc<Mutex<()>>>,
}

impl Default for CodingEditLocks {
    fn default() -> Self {
        let stripes = (0..EDIT_LOCK_STRIPES)
            .map(|_| Arc::new(Mutex::new(())))
            .collect();
        Self { stripes }
    }
}

impl CodingEditLocks {
    pub(super) async fn lock_edit(
        &self,
        scope: &CodingReadScopeKey,
        path: &str,
    ) -> OwnedMutexGuard<()> {
        let mut hasher = DefaultHasher::new();
        scope.hash(&mut hasher);
        path.hash(&mut hasher);
        let idx = (hasher.finish() as usize) % self.stripes.len();
        self.stripes[idx].clone().lock_owned().await
    }
}

#[derive(Debug, Default)]
pub(crate) struct CodingReadState {
    entries: HashMap<(CodingReadScopeKey, String), CodingReadEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct CodingReadScopeKey {
    tenant_id: String,
    user_id: String,
    agent_id: Option<String>,
    project_id: Option<String>,
    mission_id: Option<String>,
    thread_id: Option<String>,
}

#[derive(Debug, Clone)]
struct CodingReadEntry {
    modified: Option<SystemTime>,
    content_hash: String,
    partial: bool,
}

impl CodingReadState {
    pub(super) fn record_read(
        &mut self,
        scope: CodingReadScopeKey,
        path: String,
        modified: Option<SystemTime>,
        content_hash: String,
        partial: bool,
    ) {
        self.entries.insert(
            (scope, path),
            CodingReadEntry {
                modified,
                content_hash,
                partial,
            },
        );
    }

    pub(super) fn check_before_edit(
        &self,
        scope: &CodingReadScopeKey,
        path: &str,
        current_content_hash: &str,
    ) -> Result<(), CodingCapabilityError> {
        let key = (scope.clone(), path.to_string());
        let Some(entry) = self.entries.get(&key) else {
            return Err(guest_error());
        };
        if entry.partial || entry.content_hash != current_content_hash {
            return Err(guest_error());
        }
        Ok(())
    }

    pub(super) fn update_after_write(
        &mut self,
        scope: &CodingReadScopeKey,
        path: &str,
        modified: Option<SystemTime>,
        content_hash: String,
    ) {
        let key = (scope.clone(), path.to_string());
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.modified = modified;
            entry.content_hash = content_hash;
            entry.partial = false;
        }
    }
}

pub(super) fn content_hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

pub(super) fn read_scope_key(request: &CodingCapabilityRequest) -> CodingReadScopeKey {
    CodingReadScopeKey {
        tenant_id: request.scope.tenant_id.as_str().to_string(),
        user_id: request.scope.user_id.as_str().to_string(),
        agent_id: request
            .scope
            .agent_id
            .as_ref()
            .map(|value| value.as_str().to_string()),
        project_id: request
            .scope
            .project_id
            .as_ref()
            .map(|value| value.as_str().to_string()),
        mission_id: request
            .scope
            .mission_id
            .as_ref()
            .map(|value| value.as_str().to_string()),
        thread_id: request
            .scope
            .thread_id
            .as_ref()
            .map(|value| value.as_str().to_string()),
    }
}
