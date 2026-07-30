use serde::{Deserialize, Serialize};

use crate::{
    BlockedReason, CapabilityActivityId, ResolvedRunProfile, SanitizedFailure, TurnCheckpointId,
    TurnLeaseToken, TurnRunState, TurnRunnerId, run_profile::LoopCheckpointStateRef,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimedTurnRun {
    pub state: TurnRunState,
    pub resolved_run_profile: ResolvedRunProfile,
    #[serde(default)]
    pub subagent_depth: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawn_tree_descendant_cap: Option<u32>,
    pub runner_id: TurnRunnerId,
    pub lease_token: TurnLeaseToken,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnRunnerOutcome {
    Completed,
    Cancelled,
    Blocked {
        checkpoint_id: TurnCheckpointId,
        state_ref: LoopCheckpointStateRef,
        reason: BlockedReason,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        blocked_activity_id: Option<CapabilityActivityId>,
    },
    Failed {
        failure: SanitizedFailure,
    },
}
