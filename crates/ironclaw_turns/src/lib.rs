//! Host-layer turn coordination contracts for IronClaw Reborn.
//!
//! `ironclaw_turns` sits above the Reborn kernel facade. Product adapters use
//! the adapter-safe [`TurnCoordinator`] API with canonical refs resolved by the
//! binding/session layer. Trusted workers use [`runner`] explicitly; runner
//! transition APIs are intentionally not re-exported from this crate prelude.

pub mod coordinator;
pub mod events;
pub mod ids;
pub mod memory;
pub mod request;
pub mod response;
pub mod runner;
pub mod scope;
pub mod status;
pub mod store;

pub use coordinator::{
    AllowAllTurnAdmissionPolicy, DefaultTurnCoordinator, TurnAdmissionPolicy, TurnCoordinator,
};
pub use events::{InMemoryTurnEventSink, TurnEventKind, TurnEventSink, TurnLifecycleEvent};
pub use ids::{
    AcceptedMessageRef, GateRef, IdempotencyKey, ReplyTargetBindingRef, SourceBindingRef,
    TurnCheckpointId, TurnId, TurnLeaseToken, TurnRunId, TurnRunnerId,
};
pub use memory::InMemoryTurnStateStore;
pub use request::{CancelRunRequest, GetRunStateRequest, ResumeTurnRequest, SubmitTurnRequest};
pub use response::{CancelRunResponse, ResumeTurnResponse, SubmitTurnResponse, ThreadBusy};
pub use scope::{TurnActor, TurnScope};
pub use status::{
    AdmissionRejection, BlockedReason, SanitizedCancelReason, SanitizedFailure, TurnError,
    TurnRunProfile, TurnRunState, TurnStatus,
};
pub use store::TurnStateStore;
