//! Capability invocation host contracts for IronClaw Reborn.
//!
//! `ironclaw_capabilities` is the caller-facing capability invocation service.
//! It coordinates authorization, approval resume, run-state transitions, and
//! neutral runtime dispatch without depending on concrete runtime crates.
#![warn(unreachable_pub)]

mod conformance;
mod dispatch;
mod error;
mod helpers;
mod host;
mod obligations;
mod ports;
mod process_authorization;
mod registry;
mod replay_payload;
mod requests;
mod trust;

pub use conformance::{
    CapabilityProfileClaim, CapabilityProfileClaimedOperation, CapabilityProfileConformanceFinding,
    CapabilityProfileConformanceFindingKind, CapabilityProfileConformanceReport,
    evaluate_profile_conformance,
};
pub use dispatch::{
    BoundCapabilityAdapter, ChainToolResolver, ResolvedCapability, RuntimeAdapterResult,
    RuntimeDispatcher, ToolResolver,
};
pub use error::{CapabilityInvocationError, ResumeContextMismatchKind};
pub use host::CapabilityHost;
pub use ironclaw_host_api::{
    Authorized, CapabilityDispatchRequest, CapabilityDispatchResult, CapabilityDispatcher,
    CapabilityDisplayOutputPreview, DispatchError, DispatchFailureDetail, RuntimeDispatchErrorKind,
};
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
pub use registry::{CapabilityDispatchRegistry, CapabilityRegistrationError};
pub use replay_payload::{
    FilesystemReplayPayloadStore, ReplayPayload, ReplayPayloadStore, ReplayPayloadStoreError,
};
pub use requests::{CapabilityInvocationResult, CapabilitySpawnRequest, CapabilitySpawnResult};
