//! Capability invocation host contracts for IronClaw Reborn.
//!
//! `ironclaw_capabilities` is the caller-facing capability invocation service.
//! It coordinates authorization, approval resume, run-state transitions, and
//! neutral runtime dispatch without depending on concrete runtime crates.
#![warn(unreachable_pub)]

mod conformance;
mod error;
mod helpers;
mod host;
mod obligations;
mod ports;
mod process_authorization;
mod replay_payload;
mod requests;
mod trust;

pub use conformance::{
    CapabilityProfileClaim, CapabilityProfileClaimedOperation, CapabilityProfileConformanceFinding,
    CapabilityProfileConformanceFindingKind, CapabilityProfileConformanceReport,
    evaluate_profile_conformance,
};
pub use error::{CapabilityInvocationError, ResumeContextMismatchKind};
pub use host::CapabilityHost;
pub use obligations::{
    CapabilityObligationAbortRequest, CapabilityObligationCompletionRequest,
    CapabilityObligationError, CapabilityObligationFailureKind, CapabilityObligationHandler,
    CapabilityObligationOutcome, CapabilityObligationPhase, CapabilityObligationRequest,
};
pub use ports::{CredentialPresence, HostPolicyFacts, PolicyAction};
pub use process_authorization::{
    ProcessAuthorizationRemintError, ProcessAuthorizationRemintPort,
    process_authorization_remint_port,
};
pub use replay_payload::{
    FilesystemReplayPayloadStore, ReplayPayload, ReplayPayloadStore, ReplayPayloadStoreError,
};
pub use requests::{
    CapabilityAuthResumeRequest, CapabilityInvocationRequest, CapabilityInvocationResult,
    CapabilityResumeRequest, CapabilitySpawnRequest, CapabilitySpawnResult,
};
