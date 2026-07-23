pub use ironclaw_host_api::{
    AcceptedMessageRef, CapabilityActivityId, IdempotencyKey, LoopDiagnosticRef, LoopExitId,
    LoopGateRef, LoopMessageRef, LoopResultRef, ReplyTargetBindingRef, RunProfileId,
    RunProfileRequest, RunProfileVersion, SourceBindingRef, TurnCheckpointId, TurnId,
    TurnLeaseToken, TurnRunId, TurnRunnerId,
};

pub type GateRef = ironclaw_host_api::TurnGateRef;
