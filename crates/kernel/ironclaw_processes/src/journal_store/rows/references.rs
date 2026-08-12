//! Row references a batch of journal commands needs loaded.

use ironclaw_host_api::{ids::ProcessId, resource::ResourceScope};

use crate::{
    ClaimProcessesRequest, ProcessCheckpointId, ProcessConcurrencyLimits, ProcessKind,
    RecoverExpiredProcessLeasesRequest,
};

/// Rows one or more commands need loaded before they can be applied.
///
/// Every field is plural because the group-commit funnel merges the references
/// of a whole batch and loads their union once: global metadata and the shared
/// idempotency-order row are then read once per transaction rather than once
/// per command.
#[derive(Default, Clone)]
pub(in crate::journal_store) struct LoadReferences {
    pub(in crate::journal_store) process_ids: Vec<ProcessId>,
    pub(in crate::journal_store) tree_roots: Vec<ProcessId>,
    pub(in crate::journal_store) dependencies: Vec<(ProcessId, ProcessId)>,
    pub(in crate::journal_store) checkpoints: Vec<ProcessCheckpointId>,
    pub(in crate::journal_store) submission_idempotency_keys: Vec<String>,
    pub(in crate::journal_store) control_idempotency_keys: Vec<String>,
    pub(in crate::journal_store) active_conflicts: Vec<(ResourceScope, ProcessKind)>,
    pub(in crate::journal_store) claims: Vec<(ClaimProcessesRequest, ProcessConcurrencyLimits)>,
    pub(in crate::journal_store) recover_expired: Vec<RecoverExpiredProcessLeasesRequest>,
}

impl LoadReferences {
    /// Fold another command's references into this batch-wide set.
    /// Append `other`'s references. The caller normalizes once after folding
    /// the whole batch — normalizing per merge would re-sort collections that
    /// the previous fold already sorted.
    pub(in crate::journal_store) fn merge_from(&mut self, other: &Self) {
        self.process_ids.extend(other.process_ids.iter().copied());
        self.tree_roots.extend(other.tree_roots.iter().copied());
        self.dependencies.extend(other.dependencies.iter().copied());
        self.checkpoints.extend(other.checkpoints.iter().cloned());
        self.submission_idempotency_keys
            .extend(other.submission_idempotency_keys.iter().cloned());
        self.control_idempotency_keys
            .extend(other.control_idempotency_keys.iter().cloned());
        self.active_conflicts
            .extend(other.active_conflicts.iter().cloned());
        self.claims.extend(other.claims.iter().cloned());
        self.recover_expired
            .extend(other.recover_expired.iter().cloned());
    }

    /// Sort and dedupe the reference sets that address individual rows so a
    /// merged batch never reads the same row twice.
    pub(in crate::journal_store) fn normalize(&mut self) {
        self.process_ids.sort_by_key(ProcessId::as_uuid);
        self.process_ids.dedup();
        self.tree_roots.sort_by_key(ProcessId::as_uuid);
        self.tree_roots.dedup();
        self.dependencies
            .sort_by_key(|(dependent, dependency)| (dependent.as_uuid(), dependency.as_uuid()));
        self.dependencies.dedup();
        self.checkpoints
            .sort_by(|left, right| left.as_str().cmp(right.as_str()));
        self.checkpoints.dedup();
        self.submission_idempotency_keys.sort();
        self.submission_idempotency_keys.dedup();
        self.control_idempotency_keys.sort();
        self.control_idempotency_keys.dedup();
    }
}
