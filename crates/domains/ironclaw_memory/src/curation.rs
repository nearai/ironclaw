//! The memory-curation trigger contract (the "dreaming" seam, issue #7276).
//!
//! Memory only ever grows: writes accumulate, nothing prunes, and the standing
//! document has a byte budget, so redundancy crowds out what matters. No human
//! reads the file, so the decay is invisible. A periodic curation pass fixes
//! that — but the tier that NOTICES a turn finished (loop execution) and the
//! tier that owns product orchestration are deliberately not allowed to depend
//! on each other, so the vocabulary between them needs a shared home.
//!
//! That home is here, with the rest of the memory contract, because this is
//! memory vocabulary: "curation" means nothing outside memory, and the signal
//! exists only to decide whether a user's memory needs tidying. Both tiers
//! already depend on this crate — the runner for after-turn recording, the
//! product tier for the memory service — so nothing new is pulled in by putting
//! it where it belongs.
//!
//! The loop tier derives an [`AfterTurnCurationSignal`] from a terminal run and
//! reports it; the product tier implements [`AfterTurnCurationPort`] and owns
//! every policy decision (whether to curate, how often, what a pass may do).

use async_trait::async_trait;
use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, UserId};

/// The scope a completed user turn ran under — everything the product tier
/// needs to decide on, and then perform, a curation pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AfterTurnCurationSignal {
    pub tenant_id: TenantId,
    /// The human whose memory would be curated. A pass acts AS this user:
    /// memory is per-owner, so a pass acting as anything else would read and
    /// write the wrong scope.
    pub user_id: UserId,
    pub agent_id: Option<AgentId>,
    pub project_id: Option<ProjectId>,
}

/// Product-tier port notified after each completed ordinary user turn.
///
/// The caller treats every outcome as best-effort: this is invoked after the
/// run is ALREADY terminal, so an implementation must never fail, delay, or
/// otherwise affect it, and must not log above `debug!` (it runs on a
/// background path where `info!`/`warn!` corrupts the REPL).
///
/// Implementations must assume they are called for ORDINARY turns only. The
/// caller is responsible for never reporting an unbound run — a curation pass
/// is itself unbound, so triggering on one would let each pass schedule its own
/// successor forever.
#[async_trait]
pub trait AfterTurnCurationPort: Send + Sync {
    async fn on_completed_turn(&self, signal: AfterTurnCurationSignal);
}
