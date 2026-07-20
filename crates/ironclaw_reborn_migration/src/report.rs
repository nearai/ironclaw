//! Migration outcome accounting.
//!
//! Two shapes: [`MigrationStats`] counts what was converted per domain, and
//! [`LossyItem`] records every source field/entity that could **not** be
//! represented in Reborn. Together they form the [`MigrationReport`], which is
//! JSON-serializable so an operator (or a follow-up in-process migration step)
//! can inspect exactly what carried over and what was dropped. Nothing is ever
//! silently lost: a value that has no Reborn home lands here as a `LossyItem`.

use serde::{Deserialize, Serialize};

#[cfg(feature = "full-migration")]
use ironclaw_host_api::UserId;

/// The domain a converted item or a loss belongs to. Keeps report entries
/// grouped and greppable rather than free-text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Domain {
    Thread,
    Message,
    Routine,
    Mission,
    Job,
    Memory,
    Secret,
    Extension,
    Identity,
    Heartbeat,
    Setting,
}

/// Why a source value did not fully carry over into Reborn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LossReason {
    /// The target concept does not exist in Reborn at all (e.g. a durable
    /// mission entity, event/webhook trigger sources).
    NoTargetConcept,
    /// The target type exists but has no field for this value (e.g. routine
    /// guardrails, notify config, run counters).
    NoTargetField,
    /// The value was degraded onto a coarser target (e.g. a `Failed` routine
    /// mapped onto `Paused` because Reborn has no failed trigger state).
    Degraded,
    /// The source value was malformed / unparseable and skipped.
    Unparseable,
}

/// One thing that could not be losslessly represented in Reborn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LossyItem {
    pub domain: Domain,
    /// Source identifier (v1 row id, routine name, mission slug, settings key…).
    pub source_id: String,
    /// The specific field/aspect that was lost, or `"*"` for the whole entity.
    pub field: String,
    pub reason: LossReason,
    /// Human-readable explanation, ideally naming the Reborn gap.
    pub detail: String,
}

/// Per-domain counts of successfully converted source entities.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationStats {
    pub threads: usize,
    pub messages: usize,
    pub routines: usize,
    pub missions: usize,
    pub trigger_runs: usize,
    pub jobs: usize,
    pub memory_documents: usize,
    pub secrets: usize,
    pub extensions: usize,
    pub identities: usize,
    pub heartbeats: usize,
    pub settings: usize,
}

/// The full outcome of a migration run: what converted, and everything that
/// could not be represented in Reborn.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationReport {
    /// True when the run was a dry run (nothing written to the Reborn store).
    pub dry_run: bool,
    pub stats: MigrationStats,
    pub lossy: Vec<LossyItem>,
}

impl MigrationReport {
    pub fn new(dry_run: bool) -> Self {
        Self {
            dry_run,
            ..Default::default()
        }
    }

    /// Record a value that could not be losslessly represented in Reborn.
    pub fn record_loss(
        &mut self,
        domain: Domain,
        source_id: impl Into<String>,
        field: impl Into<String>,
        reason: LossReason,
        detail: impl Into<String>,
    ) {
        self.lossy.push(LossyItem {
            domain,
            source_id: source_id.into(),
            field: field.into(),
            reason,
            detail: detail.into(),
        });
    }

    /// Validate a source user id, returning the Reborn [`UserId`] or recording an
    /// [`LossReason::Unparseable`] loss and returning `None` when it is invalid.
    ///
    /// Shared by every converter that scopes a record to a per-user `UserId`
    /// (threads owner, secrets, memory docs, identities, routines, missions) so
    /// the "validate → skip + record" shape has a single definition and cannot
    /// drift between copies.
    #[cfg(feature = "full-migration")]
    pub(crate) fn valid_user_id(
        &mut self,
        domain: Domain,
        source_id: impl Into<String>,
        field: &'static str,
        raw_user_id: &str,
    ) -> Option<UserId> {
        match UserId::new(raw_user_id) {
            Ok(user) => Some(user),
            Err(e) => {
                self.record_loss(
                    domain,
                    source_id,
                    field,
                    LossReason::Unparseable,
                    format!("source {field} is not a valid Reborn UserId (record skipped): {e}"),
                );
                None
            }
        }
    }

    /// Count of losses for a given domain — used by tests and summaries.
    pub fn losses_in(&self, domain: Domain) -> usize {
        self.lossy
            .iter()
            .filter(|item| item.domain == domain)
            .count()
    }

    /// Pretty JSON for `--report <path>` / stdout.
    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }
}
