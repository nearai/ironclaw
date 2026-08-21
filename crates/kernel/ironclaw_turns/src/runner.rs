use serde::{Deserialize, Serialize};

use crate::{
    BlockedReason, CapabilityActivityId, SanitizedFailure, TurnCheckpointId, TurnLeaseToken,
    TurnRunState, TurnRunnerId,
};
use ironclaw_loop_contracts::{LoopCheckpointStateRef, ResolvedRunProfile};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimedTurnRun {
    pub state: TurnRunState,
    pub resolved_run_profile: ResolvedRunProfile,
    #[serde(default)]
    pub subagent_depth: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawn_tree_descendant_cap: Option<u32>,
    /// Carried so a terminal metadata rewrite can restore it. `from_state`
    /// rebuilds the metadata envelope without lineage, and the loop-exit path
    /// writes that envelope on every terminal transition — without this the
    /// provenance is erased the moment a run completes, and the derived
    /// activation-streak caps read every historical run as untagged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_activation_provenance: Option<crate::ActivationProvenance>,
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
