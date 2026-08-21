//! Capability invocation host contracts for IronClaw Reborn.
//!
//! `ironclaw_capabilities` is the caller-facing capability invocation service.
//! It coordinates authorization, approval resume, process invocation transitions, and
//! neutral runtime dispatch without depending on concrete runtime crates.
#![warn(unreachable_pub)]

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

pub use dispatch::{
    BoundCapabilityAdapter, ChainToolResolver, ResolvedCapability, RuntimeAdapterResult,
    RuntimeDispatcher, ToolResolver,
};
pub use error::{CapabilityInvocationError, ResumeContextMismatchKind};
pub use host::CapabilityHost;
pub use ironclaw_host_api::{
    authorized::Authorized,
    dispatch::{
        CapabilityDispatchRequest, CapabilityDispatchResult, CapabilityDispatcher,
        CapabilityDisplayOutputPreview, DispatchError, DispatchFailureDetail, DispatchFailureKind,
        ProviderDiagnostic, RuntimeDispatchErrorKind, UntrustedProviderMessage,
    },
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
    ReplayPayload, ReplayPayloadStore, ReplayPayloadStoreError, ReplayPayloadStorePort,
};
pub use requests::{CapabilitySpawnRequest, CapabilitySpawnResult};
