use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    sync::Arc,
};

use tokio::sync::{Mutex, OwnedMutexGuard};

use super::CodingCapabilityRequest;

pub(crate) type SharedCodingEditLocks = Arc<CodingEditLocks>;
pub(crate) type SharedCodingReadStates = Arc<CodingReadStates>;

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
        key: &CodingEditLockKey,
        path: &str,
    ) -> OwnedMutexGuard<()> {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        path.hash(&mut hasher);
        let idx = (hasher.finish() as usize) % self.stripes.len();
        self.stripes[idx].clone().lock_owned().await
    }
}

/// Content fingerprints recorded by full `read_file` calls, keyed by
/// scope + resolved virtual path. `write_file` and `apply_patch` on an
/// existing file require a recorded entry (read-before-edit) whose
/// fingerprint still matches the file's current bytes (mid-air collision
/// detection); a successful edit refreshes the entry so chained edits keep
/// working. The registry is bounded; eviction and process restarts fail
/// safe — a missing entry only forces a fresh `read_file` before editing.
const MAX_READ_STATE_ENTRIES: usize = 8192;

type ReadStateKey = (CodingReadScopeKey, String);

#[derive(Debug, Default)]
pub(crate) struct CodingReadStates {
    entries: std::sync::Mutex<HashMap<ReadStateKey, u64>>,
}

impl CodingReadStates {
    pub(super) fn record(&self, scope: &CodingReadScopeKey, path: &str, fingerprint: u64) {
        let mut entries = self.lock_entries();
        let key = (scope.clone(), path.to_string());
        if entries.len() >= MAX_READ_STATE_ENTRIES && !entries.contains_key(&key) {
            // Evict an arbitrary entry to keep memory bounded; the evicted
            // path just requires a fresh read_file before its next edit.
            if let Some(evicted) = entries.keys().next().cloned() {
                entries.remove(&evicted);
            }
        }
        entries.insert(key, fingerprint);
    }

    pub(super) fn recorded(&self, scope: &CodingReadScopeKey, path: &str) -> Option<u64> {
        self.lock_entries()
            .get(&(scope.clone(), path.to_string()))
            .copied()
    }

    fn lock_entries(&self) -> std::sync::MutexGuard<'_, HashMap<ReadStateKey, u64>> {
        // A poisoned lock means another thread panicked mid-update; the map
        // itself stays coherent (single insert/remove ops), so keep serving.
        match self.entries.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

pub(super) fn content_fingerprint(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

/// Scope dimensions shared by the read-state and edit-lock keys, EXCLUDING the
/// run identity. Edit serialization must span runs: the striped edit lock only
/// serializes the read-verify-write critical section if two concurrent runs
/// editing the same scope+path contend on the same stripe, so this key
/// deliberately omits `run_id`. Folding `run_id` in here would send concurrent
/// runs to different stripes and let both pass their fingerprint check and
/// overwrite each other (lost update).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct CodingEditLockKey {
    tenant_id: String,
    user_id: String,
    agent_id: Option<String>,
    project_id: Option<String>,
    mission_id: Option<String>,
    thread_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct CodingReadScopeKey {
    edit_lock: CodingEditLockKey,
    /// Loop turn-run identity. Read-before-edit is a within-run policy: the
    /// model must have seen the file during the CURRENT run, so a read
    /// recorded in one run never authorizes edits in a later run even when
    /// the content fingerprint still matches. `None` (non-loop callers) is
    /// its own bucket, never a wildcard.
    run_id: Option<ironclaw_host_api::RunId>,
}

impl CodingReadScopeKey {
    /// The run-agnostic key that serializes edits to this scope+path across
    /// concurrent runs. See [`CodingEditLockKey`] for why `run_id` is excluded.
    pub(super) fn edit_lock_key(&self) -> &CodingEditLockKey {
        &self.edit_lock
    }
}

pub(super) fn read_scope_key(request: &CodingCapabilityRequest<'_>) -> CodingReadScopeKey {
    CodingReadScopeKey {
        edit_lock: CodingEditLockKey {
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
        },
        run_id: request.run_id,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ironclaw_host_api::RunId;

    use super::*;

    const RUN_A: &str = "11111111-1111-4111-8111-111111111111";
    const RUN_B: &str = "22222222-2222-4222-8222-222222222222";

    fn read_scope(run_id: Option<RunId>) -> CodingReadScopeKey {
        CodingReadScopeKey {
            edit_lock: CodingEditLockKey {
                tenant_id: "tenant".to_string(),
                user_id: "user".to_string(),
                agent_id: None,
                project_id: Some("project".to_string()),
                mission_id: None,
                thread_id: Some("thread".to_string()),
            },
            run_id,
        }
    }

    #[test]
    fn edit_lock_key_ignores_run_id_while_read_state_key_is_run_scoped() {
        let a = read_scope(Some(RunId::parse(RUN_A).unwrap()));
        let b = read_scope(Some(RunId::parse(RUN_B).unwrap()));
        let none = read_scope(None);

        // Two runs editing the same scope+path share one edit-lock key, so they
        // contend on the same stripe and the read-verify-write section stays
        // serialized across runs (no lost update).
        assert_eq!(a.edit_lock_key(), b.edit_lock_key());
        assert_eq!(a.edit_lock_key(), none.edit_lock_key());

        // But the read-state key stays run-scoped, so a read recorded in one run
        // never authorizes an edit in another run.
        assert_ne!(a, b);
        assert_ne!(a, none);
    }

    #[tokio::test]
    async fn lock_edit_serializes_the_same_path_across_concurrent_runs() {
        let locks = CodingEditLocks::default();
        let run_a = read_scope(Some(RunId::parse(RUN_A).unwrap()));
        let run_b = read_scope(Some(RunId::parse(RUN_B).unwrap()));
        let path = "/workspace/main.rs";

        let held = locks.lock_edit(run_a.edit_lock_key(), path).await;
        // A second run editing the same path must block on the stripe the first
        // run holds; before the key split it took a run_id-specific stripe and
        // sailed through concurrently, enabling a lost update.
        let contended = locks.lock_edit(run_b.edit_lock_key(), path);
        assert!(
            tokio::time::timeout(Duration::from_millis(50), contended)
                .await
                .is_err(),
            "a different run must not acquire the edit lock while another run holds it"
        );
        drop(held);
        // Once released, the other run acquires it — the stripe was shared, not
        // permanently blocked.
        locks.lock_edit(run_b.edit_lock_key(), path).await;
    }
}
