use std::{
    hash::{DefaultHasher, Hash, Hasher},
    sync::Arc,
};

use tokio::sync::{Mutex, OwnedMutexGuard};

use super::CodingCapabilityRequest;

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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct CodingReadScopeKey {
    tenant_id: String,
    user_id: String,
    agent_id: Option<String>,
    project_id: Option<String>,
    mission_id: Option<String>,
    thread_id: Option<String>,
}

pub(super) fn read_scope_key(request: &CodingCapabilityRequest<'_>) -> CodingReadScopeKey {
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
