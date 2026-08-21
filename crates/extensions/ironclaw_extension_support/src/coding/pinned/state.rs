//! Bounded, run-bound snapshot registry for the pinned hashline edit engine.
//!
//! Mirrors `coding/state.rs` semantics: read tags are keyed by scope
//! dimensions PLUS the run identity, so a read recorded in one run never
//! authorizes edits in a later run. The registry is bounded (evicts the
//! oldest entry); a missing entry fails safe with the "not from this
//! session" stale-anchor message.
//!
//! A successful edit refreshes the recorded snapshot, so chained edits on
//! the same file keep working without an intervening read.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, MutexGuard};

use ironclaw_host_api::ids::RunId;
use ironclaw_host_api::resource::ResourceScope;

use super::CodingEngineErrorKind;

/// Maximum retained (scope, path) snapshots. Eviction is FIFO; an evicted
/// path simply requires a fresh `read` before its next edit.
const MAX_SNAPSHOT_ENTRIES: usize = 8192;

/// Scope dimensions shared by the read-state key, INCLUDING the run
/// identity: read-before-edit is a within-run policy (mirrors
/// `coding::state::CodingReadScopeKey`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct CodingScopeKey {
    tenant_id: String,
    user_id: String,
    agent_id: Option<String>,
    project_id: Option<String>,
    mission_id: Option<String>,
    thread_id: Option<String>,
    run_id: Option<RunId>,
}

impl CodingScopeKey {
    pub(crate) fn from_scope(scope: &ResourceScope, run_id: Option<RunId>) -> Self {
        Self {
            tenant_id: scope.tenant_id.as_str().to_string(),
            user_id: scope.user_id.as_str().to_string(),
            agent_id: scope.agent_id.as_ref().map(|id| id.as_str().to_string()),
            project_id: scope.project_id.as_ref().map(|id| id.as_str().to_string()),
            mission_id: scope.mission_id.as_ref().map(|id| id.as_str().to_string()),
            thread_id: scope.thread_id.as_ref().map(|id| id.as_str().to_string()),
            run_id,
        }
    }
}

/// One recorded snapshot: the 4-hex uppercase content tag observed by a
/// read (or produced by a successful edit). The full text is not retained —
/// nothing reads it back; the tag alone authorizes chained edits.
#[derive(Debug, Clone)]
struct SnapshotEntry {
    tag: String,
    fingerprint: [u8; 32],
}

type SnapshotKey = (CodingScopeKey, String);

/// Bounded registry of hashline snapshot tags keyed by (scope, virtual path).
#[derive(Debug)]
pub struct CodingSnapshotRegistry {
    state: Mutex<SnapshotState>,
    max_entries: usize,
}

#[derive(Debug, Default)]
struct SnapshotState {
    entries: HashMap<SnapshotKey, SnapshotEntry>,
    order: VecDeque<SnapshotKey>,
}

impl Default for CodingSnapshotRegistry {
    fn default() -> Self {
        Self {
            state: Mutex::new(SnapshotState::default()),
            max_entries: MAX_SNAPSHOT_ENTRIES,
        }
    }
}

impl CodingSnapshotRegistry {
    fn lock_state(&self) -> MutexGuard<'_, SnapshotState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Record the content tag observed for `virtual_path` under `scope`
    /// (computed by the caller via [`super::hashline::compute_file_hash`]).
    pub(crate) fn record(
        &self,
        scope: &CodingScopeKey,
        virtual_path: &str,
        tag: &str,
        fingerprint: [u8; 32],
    ) {
        let key = (scope.clone(), virtual_path.to_string());
        let mut state = self.lock_state();
        if !state.entries.contains_key(&key) {
            if state.entries.len() >= self.max_entries
                && let Some(evicted) = state.order.pop_front()
            {
                // Evict the oldest recorded entry; the evicted path just
                // requires a fresh read before its next edit.
                state.entries.remove(&evicted);
            }
            state.order.push_back(key.clone());
        }
        state.entries.insert(
            key,
            SnapshotEntry {
                tag: tag.to_string(),
                fingerprint,
            },
        );
    }

    /// Whether a tag was ever recorded for this path in this scope+run —
    /// the `hashRecognized` input to the stale-anchor message split.
    pub(crate) fn tag_recognized(
        &self,
        scope: &CodingScopeKey,
        virtual_path: &str,
        tag: &str,
    ) -> bool {
        let state = self.lock_state();
        state
            .entries
            .get(&(scope.clone(), virtual_path.to_string()))
            .is_some_and(|entry| entry.tag == tag)
    }

    /// Verify that the current full normalized content is the exact snapshot
    /// recorded with the model-visible tag. The four-hex tag remains the pinned
    /// wire contract; this collision-resistant fingerprint is host-internal.
    pub(crate) fn snapshot_matches(
        &self,
        scope: &CodingScopeKey,
        virtual_path: &str,
        tag: &str,
        normalized: &str,
    ) -> bool {
        let fingerprint = *blake3::hash(normalized.as_bytes()).as_bytes();
        let state = self.lock_state();
        state
            .entries
            .get(&(scope.clone(), virtual_path.to_string()))
            .is_some_and(|entry| entry.tag == tag && entry.fingerprint == fingerprint)
    }

    /// Drop the snapshot for a deleted path (REM).
    pub(crate) fn invalidate(&self, scope: &CodingScopeKey, virtual_path: &str) {
        let key = (scope.clone(), virtual_path.to_string());
        let mut state = self.lock_state();
        state.entries.remove(&key);
        state.order.retain(|candidate| candidate != &key);
    }
}

/// Convenience: stale-anchor kind for a recognized vs unrecognized tag.
pub(crate) fn stale_anchor_kind(recognized: bool) -> CodingEngineErrorKind {
    if recognized {
        CodingEngineErrorKind::StaleAnchorHashRecognized
    } else {
        CodingEngineErrorKind::StaleAnchorHashUnrecognized
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::ids::{InvocationId, UserId};

    fn scope(run: Option<RunId>) -> CodingScopeKey {
        let scope =
            ResourceScope::local_default(UserId::new("u1").expect("user id"), InvocationId::new())
                .expect("scope");
        CodingScopeKey::from_scope(&scope, run)
    }

    fn recorded(
        registry: &CodingSnapshotRegistry,
        scope: &CodingScopeKey,
        virtual_path: &str,
    ) -> Option<String> {
        registry
            .lock_state()
            .entries
            .get(&(scope.clone(), virtual_path.to_string()))
            .map(|entry| entry.tag.clone())
    }

    #[test]
    fn record_and_lookup_round_trip() {
        let registry = CodingSnapshotRegistry::default();
        let scope = scope(None);
        registry.record(&scope, "/projects/workspace/foo.ts", "1A2B", [1; 32]);
        assert_eq!(
            recorded(&registry, &scope, "/projects/workspace/foo.ts"),
            Some("1A2B".to_string())
        );
        assert!(registry.tag_recognized(&scope, "/projects/workspace/foo.ts", "1A2B"));
        assert!(!registry.tag_recognized(&scope, "/projects/workspace/foo.ts", "3C4D"));
        // A different scope never sees the entry.
        let other = CodingScopeKey {
            tenant_id: "other".to_string(),
            ..scope.clone()
        };
        assert!(recorded(&registry, &other, "/projects/workspace/foo.ts").is_none());
    }

    #[test]
    fn run_bound_reads_never_authorize_later_runs() {
        let registry = CodingSnapshotRegistry::default();
        let run_a = scope(Some(RunId::new()));
        let run_b = scope(Some(RunId::new()));
        registry.record(&run_a, "/projects/workspace/foo.ts", "1A2B", [1; 32]);
        assert!(recorded(&registry, &run_b, "/projects/workspace/foo.ts").is_none());
        assert!(!registry.tag_recognized(&run_b, "/projects/workspace/foo.ts", "1A2B"));
    }

    #[test]
    fn successful_edit_refreshes_the_tag() {
        let registry = CodingSnapshotRegistry::default();
        let scope = scope(None);
        let path = "/projects/workspace/foo.ts";
        registry.record(&scope, path, "1A2B", [1; 32]);
        registry.record(&scope, path, "3C4D", [2; 32]);
        assert_eq!(recorded(&registry, &scope, path), Some("3C4D".to_string()));
        assert!(registry.tag_recognized(&scope, path, "3C4D"));
        assert!(!registry.tag_recognized(&scope, path, "1A2B"));
    }

    #[test]
    fn bounded_registry_evicts_oldest() {
        let registry = CodingSnapshotRegistry {
            state: Mutex::new(SnapshotState::default()),
            max_entries: 2,
        };
        let scope = scope(None);
        registry.record(&scope, "/p/a", "AAAA", [1; 32]);
        registry.record(&scope, "/p/b", "BBBB", [2; 32]);
        registry.record(&scope, "/p/c", "CCCC", [3; 32]);
        assert!(recorded(&registry, &scope, "/p/a").is_none());
        assert_eq!(
            recorded(&registry, &scope, "/p/b"),
            Some("BBBB".to_string())
        );
        assert_eq!(
            recorded(&registry, &scope, "/p/c"),
            Some("CCCC".to_string())
        );
    }

    #[test]
    fn invalidate_drops_entry_and_order() {
        let registry = CodingSnapshotRegistry::default();
        let scope = scope(None);
        registry.record(&scope, "/p/a", "AAAA", [1; 32]);
        registry.invalidate(&scope, "/p/a");
        assert!(recorded(&registry, &scope, "/p/a").is_none());
        registry.record(&scope, "/p/a", "BBBB", [2; 32]);
        assert_eq!(
            recorded(&registry, &scope, "/p/a"),
            Some("BBBB".to_string())
        );
    }
}
