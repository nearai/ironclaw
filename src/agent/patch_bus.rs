//! Patch-bus coordination for isolated agent workspaces.
//!
//! Isolated agents should not write directly into the coordinator's canonical
//! workspace. They should submit a typed patch envelope instead. The coordinator
//! can then queue independent patches, flag stale-base or overlapping-file
//! conflicts, and hand conflicted work to an integrator.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Component;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The coarse role an isolated agent should play in a routed task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    /// Read-only codebase or workspace investigation.
    Explorer,
    /// Bounded implementation work that may produce a patch.
    Worker,
    /// Read-only validation, test execution, and regression review.
    Verifier,
    /// Conflict resolution and final merge preparation.
    Integrator,
}

impl AgentRole {
    /// Whether this role is expected to submit file changes.
    pub fn allows_patch(self) -> bool {
        matches!(self, Self::Worker | Self::Integrator)
    }
}

/// A deterministic router input for picking an isolated-agent role.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoutingRequest {
    /// User or coordinator prompt for the subtask.
    pub prompt: String,
    /// Files or paths already known to be in scope.
    pub files: Vec<String>,
    /// Whether the task explicitly requires file writes.
    pub requires_write: bool,
    /// Number of unresolved patch conflicts in the current patch bus.
    pub unresolved_conflicts: usize,
}

/// Deterministic multi-agent router.
#[derive(Debug, Clone, Default)]
pub struct MultiAgentRouter;

impl MultiAgentRouter {
    /// Route a task to the least-powerful role that can plausibly handle it.
    pub fn route(&self, request: &RoutingRequest) -> AgentRole {
        let prompt = request.prompt.to_ascii_lowercase();

        if request.unresolved_conflicts > 0
            || contains_any(&prompt, &["conflict", "merge", "integrate", "rebase"])
        {
            return AgentRole::Integrator;
        }

        if !request.requires_write
            && contains_any(
                &prompt,
                &["review", "verify", "validate", "test", "regression", "ci"],
            )
        {
            return AgentRole::Verifier;
        }

        if request.requires_write
            || !request.files.is_empty()
            || contains_any(
                &prompt,
                &[
                    "build",
                    "fix",
                    "implement",
                    "ship",
                    "write",
                    "edit",
                    "refactor",
                ],
            )
        {
            return AgentRole::Worker;
        }

        AgentRole::Explorer
    }
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

/// Patch data returned by an isolated agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatchEnvelope {
    /// Stable proposal ID assigned by the coordinator or submitting agent.
    pub id: Uuid,
    /// Agent identity that produced this patch.
    pub agent_id: Uuid,
    /// Optional scheduler/container job that produced this patch.
    pub job_id: Option<Uuid>,
    /// Role the agent was routed to.
    pub role: AgentRole,
    /// Canonical snapshot SHA the agent received before starting work.
    pub base_sha: String,
    /// Files touched by the patch, relative to the canonical workspace root.
    pub files_touched: BTreeSet<String>,
    /// Unified diff or equivalent patch payload.
    pub diff: String,
    /// Test/check commands the agent ran.
    pub tests_run: Vec<String>,
    /// Human-readable implementation notes or reviewer context.
    pub notes: Option<String>,
}

impl PatchEnvelope {
    /// Build a patch envelope with generated agent/proposal IDs.
    pub fn new(
        role: AgentRole,
        base_sha: impl Into<String>,
        files_touched: impl IntoIterator<Item = impl Into<String>>,
        diff: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            agent_id: Uuid::new_v4(),
            job_id: None,
            role,
            base_sha: base_sha.into(),
            files_touched: files_touched
                .into_iter()
                .map(Into::into)
                .collect::<BTreeSet<_>>(),
            diff: diff.into(),
            tests_run: Vec::new(),
            notes: None,
        }
    }
}

/// Current state of a submitted patch proposal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchStatus {
    /// Waiting to be applied by the coordinator.
    Queued,
    /// Marked conflicted; needs an integrator or resubmission.
    Conflicted,
    /// Applied to the canonical workspace by the coordinator.
    Accepted,
    /// Rejected without being applied.
    Rejected,
}

/// Why a proposal cannot be queued/applied automatically.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchConflict {
    /// The patch came from a role that should not write files.
    ReadOnlyRole { role: AgentRole },
    /// The patch was generated from a snapshot other than this bus epoch.
    StaleBase { expected: String, actual: String },
    /// No touched files were declared.
    EmptyFileSet,
    /// Patch payload is empty.
    EmptyDiff,
    /// A touched path is absolute, escapes the root, or is otherwise unsafe.
    InvalidPath { path: String, reason: String },
    /// Another queued or accepted proposal touches at least one same file.
    OverlappingFiles {
        proposal_id: Uuid,
        files: BTreeSet<String>,
    },
}

/// A patch proposal plus coordinator-owned state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatchRecord {
    pub envelope: PatchEnvelope,
    pub status: PatchStatus,
    pub conflicts: Vec<PatchConflict>,
    pub rejection_reason: Option<String>,
}

impl PatchRecord {
    fn queued(envelope: PatchEnvelope) -> Self {
        Self {
            envelope,
            status: PatchStatus::Queued,
            conflicts: Vec::new(),
            rejection_reason: None,
        }
    }

    fn conflicted(envelope: PatchEnvelope, conflicts: Vec<PatchConflict>) -> Self {
        Self {
            envelope,
            status: PatchStatus::Conflicted,
            conflicts,
            rejection_reason: None,
        }
    }
}

/// Result of submitting a patch envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchDecision {
    /// The proposal is queued for coordinator application.
    Queued { proposal_id: Uuid },
    /// The proposal was stored but requires integration work.
    Conflicted {
        proposal_id: Uuid,
        conflicts: Vec<PatchConflict>,
    },
}

/// Patch-bus operation errors.
#[derive(Debug, thiserror::Error)]
pub enum PatchBusError {
    #[error("patch proposal {id} already exists")]
    DuplicateProposal { id: Uuid },
    #[error("patch proposal {id} was not found")]
    ProposalNotFound { id: Uuid },
    #[error("patch proposal {id} is {status:?}, expected queued")]
    ProposalNotQueued { id: Uuid, status: PatchStatus },
}

/// Coordinator-owned patch queue for one canonical workspace snapshot epoch.
#[derive(Debug, Clone)]
pub struct PatchBus {
    base_sha: String,
    head_sha: String,
    records: BTreeMap<Uuid, PatchRecord>,
    queue: VecDeque<Uuid>,
    claimed_files: BTreeMap<String, Uuid>,
}

impl PatchBus {
    /// Start a bus epoch from the canonical snapshot handed to agents.
    pub fn new(base_sha: impl Into<String>) -> Self {
        let base_sha = base_sha.into();
        Self {
            head_sha: base_sha.clone(),
            base_sha,
            records: BTreeMap::new(),
            queue: VecDeque::new(),
            claimed_files: BTreeMap::new(),
        }
    }

    /// Snapshot SHA that isolated agents must use for this epoch.
    pub fn base_sha(&self) -> &str {
        &self.base_sha
    }

    /// Current canonical head after accepted proposals.
    pub fn head_sha(&self) -> &str {
        &self.head_sha
    }

    /// Submit a patch envelope into the bus.
    pub fn submit(&mut self, envelope: PatchEnvelope) -> Result<PatchDecision, PatchBusError> {
        if self.records.contains_key(&envelope.id) {
            return Err(PatchBusError::DuplicateProposal { id: envelope.id });
        }

        let mut envelope = envelope;
        let conflicts = self.validate_envelope(&mut envelope);
        let id = envelope.id;

        if conflicts.is_empty() {
            for path in &envelope.files_touched {
                self.claimed_files.insert(path.clone(), id);
            }
            self.queue.push_back(id);
            self.records.insert(id, PatchRecord::queued(envelope));
            Ok(PatchDecision::Queued { proposal_id: id })
        } else {
            self.records
                .insert(id, PatchRecord::conflicted(envelope, conflicts.clone()));
            Ok(PatchDecision::Conflicted {
                proposal_id: id,
                conflicts,
            })
        }
    }

    /// Return the next queued proposal, without mutating the queue.
    pub fn next_ready(&self) -> Option<&PatchRecord> {
        self.queue.front().and_then(|id| self.records.get(id))
    }

    /// Mark a queued proposal as applied and advance the bus head SHA.
    pub fn mark_applied(
        &mut self,
        proposal_id: Uuid,
        new_head_sha: impl Into<String>,
    ) -> Result<(), PatchBusError> {
        let record = self
            .records
            .get_mut(&proposal_id)
            .ok_or(PatchBusError::ProposalNotFound { id: proposal_id })?;
        if record.status != PatchStatus::Queued {
            return Err(PatchBusError::ProposalNotQueued {
                id: proposal_id,
                status: record.status,
            });
        }

        record.status = PatchStatus::Accepted;
        self.head_sha = new_head_sha.into();
        self.queue.retain(|id| *id != proposal_id);
        Ok(())
    }

    /// Reject a proposal and release any queued file claims it held.
    pub fn reject(
        &mut self,
        proposal_id: Uuid,
        reason: impl Into<String>,
    ) -> Result<(), PatchBusError> {
        let record = self
            .records
            .get_mut(&proposal_id)
            .ok_or(PatchBusError::ProposalNotFound { id: proposal_id })?;
        let was_queued = record.status == PatchStatus::Queued;
        record.status = PatchStatus::Rejected;
        record.rejection_reason = Some(reason.into());
        self.queue.retain(|id| *id != proposal_id);
        if was_queued {
            self.release_claims(proposal_id);
        }
        Ok(())
    }

    /// Return a proposal by ID.
    pub fn record(&self, proposal_id: Uuid) -> Option<&PatchRecord> {
        self.records.get(&proposal_id)
    }

    /// Count proposals by status for dashboards/logging.
    pub fn status_counts(&self) -> BTreeMap<PatchStatus, usize> {
        let mut out = BTreeMap::new();
        for record in self.records.values() {
            *out.entry(record.status).or_insert(0) += 1;
        }
        out
    }

    fn validate_envelope(&self, envelope: &mut PatchEnvelope) -> Vec<PatchConflict> {
        let mut conflicts = Vec::new();

        if !envelope.role.allows_patch() {
            conflicts.push(PatchConflict::ReadOnlyRole {
                role: envelope.role,
            });
        }

        if envelope.base_sha != self.base_sha {
            conflicts.push(PatchConflict::StaleBase {
                expected: self.base_sha.clone(),
                actual: envelope.base_sha.clone(),
            });
        }

        if envelope.files_touched.is_empty() {
            conflicts.push(PatchConflict::EmptyFileSet);
        }

        if envelope.diff.trim().is_empty() {
            conflicts.push(PatchConflict::EmptyDiff);
        }

        let mut normalized = BTreeSet::new();
        for path in &envelope.files_touched {
            match normalize_patch_path(path) {
                Ok(path) => {
                    normalized.insert(path);
                }
                Err(reason) => conflicts.push(PatchConflict::InvalidPath {
                    path: path.clone(),
                    reason,
                }),
            }
        }
        envelope.files_touched = normalized;

        let mut overlaps_by_proposal: BTreeMap<Uuid, BTreeSet<String>> = BTreeMap::new();
        for path in &envelope.files_touched {
            if let Some(existing_id) = self.claimed_files.get(path) {
                overlaps_by_proposal
                    .entry(*existing_id)
                    .or_default()
                    .insert(path.clone());
            }
        }
        conflicts.extend(
            overlaps_by_proposal
                .into_iter()
                .map(|(proposal_id, files)| PatchConflict::OverlappingFiles { proposal_id, files }),
        );

        conflicts
    }

    fn release_claims(&mut self, proposal_id: Uuid) {
        self.claimed_files
            .retain(|_, owner_id| *owner_id != proposal_id);
    }
}

fn normalize_patch_path(path: &str) -> Result<String, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("path is empty".to_string());
    }

    let raw = std::path::Path::new(trimmed);
    if raw.is_absolute() {
        return Err("absolute paths are not allowed".to_string());
    }

    let mut parts = Vec::new();
    for component in raw.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().to_string()),
            Component::CurDir => {}
            Component::ParentDir => return Err("`..` components are not allowed".to_string()),
            Component::RootDir | Component::Prefix(_) => {
                return Err("root or prefix components are not allowed".to_string());
            }
        }
    }

    if parts.is_empty() {
        return Err("path does not name a file".to_string());
    }

    Ok(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(id: Uuid, base: &str, files: &[&str]) -> PatchEnvelope {
        PatchEnvelope {
            id,
            agent_id: Uuid::new_v4(),
            job_id: None,
            role: AgentRole::Worker,
            base_sha: base.to_string(),
            files_touched: files.iter().map(|path| (*path).to_string()).collect(),
            diff: "diff --git a/file b/file".to_string(),
            tests_run: vec!["cargo test patch_bus".to_string()],
            notes: None,
        }
    }

    #[test]
    fn router_prefers_integrator_when_conflicts_exist() {
        let router = MultiAgentRouter;
        let request = RoutingRequest {
            prompt: "ship this autonomously".to_string(),
            unresolved_conflicts: 1,
            ..Default::default()
        };

        assert_eq!(router.route(&request), AgentRole::Integrator);
    }

    #[test]
    fn router_prefers_verifier_for_read_only_test_tasks() {
        let router = MultiAgentRouter;
        let request = RoutingRequest {
            prompt: "verify the regression tests".to_string(),
            ..Default::default()
        };

        assert_eq!(router.route(&request), AgentRole::Verifier);
    }

    #[test]
    fn router_prefers_worker_for_write_tasks() {
        let router = MultiAgentRouter;
        let request = RoutingRequest {
            prompt: "implement the patch bus".to_string(),
            requires_write: true,
            ..Default::default()
        };

        assert_eq!(router.route(&request), AgentRole::Worker);
    }

    #[test]
    fn independent_patches_queue_and_apply() {
        let mut bus = PatchBus::new("base");
        let first_id = Uuid::new_v4();
        let second_id = Uuid::new_v4();

        let first = bus.submit(envelope(first_id, "base", &["src/a.rs"]));
        let second = bus.submit(envelope(second_id, "base", &["src/b.rs"]));

        assert_eq!(
            first.unwrap(),
            PatchDecision::Queued {
                proposal_id: first_id
            }
        );
        assert_eq!(
            second.unwrap(),
            PatchDecision::Queued {
                proposal_id: second_id
            }
        );
        assert_eq!(bus.next_ready().map(|r| r.envelope.id), Some(first_id));

        bus.mark_applied(first_id, "head-1").unwrap();

        assert_eq!(bus.head_sha(), "head-1");
        assert_eq!(bus.next_ready().map(|r| r.envelope.id), Some(second_id));
    }

    #[test]
    fn overlapping_patch_is_conflicted() {
        let mut bus = PatchBus::new("base");
        let first_id = Uuid::new_v4();
        let second_id = Uuid::new_v4();

        bus.submit(envelope(first_id, "base", &["src/a.rs"]))
            .unwrap();
        let decision = bus
            .submit(envelope(second_id, "base", &["./src/a.rs"]))
            .unwrap();

        assert_eq!(
            decision,
            PatchDecision::Conflicted {
                proposal_id: second_id,
                conflicts: vec![PatchConflict::OverlappingFiles {
                    proposal_id: first_id,
                    files: BTreeSet::from(["src/a.rs".to_string()])
                }]
            }
        );
    }

    #[test]
    fn stale_base_is_conflicted() {
        let mut bus = PatchBus::new("base");
        let id = Uuid::new_v4();

        let decision = bus
            .submit(envelope(id, "different-base", &["src/a.rs"]))
            .unwrap();

        assert_eq!(
            decision,
            PatchDecision::Conflicted {
                proposal_id: id,
                conflicts: vec![PatchConflict::StaleBase {
                    expected: "base".to_string(),
                    actual: "different-base".to_string()
                }]
            }
        );
    }

    #[test]
    fn read_only_roles_cannot_submit_patches() {
        let mut bus = PatchBus::new("base");
        let id = Uuid::new_v4();
        let mut patch = envelope(id, "base", &["src/a.rs"]);
        patch.role = AgentRole::Explorer;

        let decision = bus.submit(patch).unwrap();

        assert_eq!(
            decision,
            PatchDecision::Conflicted {
                proposal_id: id,
                conflicts: vec![PatchConflict::ReadOnlyRole {
                    role: AgentRole::Explorer
                }]
            }
        );
    }

    #[test]
    fn unsafe_paths_are_conflicted() {
        let mut bus = PatchBus::new("base");
        let id = Uuid::new_v4();

        let decision = bus
            .submit(envelope(id, "base", &["../outside.rs"]))
            .unwrap();

        assert!(matches!(
            decision,
            PatchDecision::Conflicted { conflicts, .. }
                if conflicts == vec![PatchConflict::InvalidPath {
                    path: "../outside.rs".to_string(),
                    reason: "`..` components are not allowed".to_string()
                }]
        ));
    }
}
