//! Neutral capability dispatch port contracts.
//!
//! These types describe an already-authorized capability dispatch request and
//! normalized runtime result. Concrete dispatcher/runtime crates implement the
//! behavior; caller-facing workflow crates depend only on this neutral port.

use std::fmt;

use async_trait::async_trait;
use serde_json::Value;
use thiserror::Error;

use crate::{
    CapabilityId, ExtensionId, MountView, ResourceEstimate, ResourceReceipt, ResourceReservation,
    ResourceScope, ResourceUsage, RuntimeKind, SecretHandle,
};

/// Request for one already-authorized declared capability dispatch.
#[derive(Debug, Clone, PartialEq)]
pub struct CapabilityDispatchRequest {
    pub capability_id: CapabilityId,
    pub scope: ResourceScope,
    pub estimate: ResourceEstimate,
    pub mounts: Option<MountView>,
    pub resource_reservation: Option<ResourceReservation>,
    pub input: Value,
}

/// Normalized dispatch result returned by a runtime dispatcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityDispatchResult {
    pub capability_id: CapabilityId,
    pub provider: ExtensionId,
    pub runtime: RuntimeKind,
    pub output: Value,
    pub usage: ResourceUsage,
    pub receipt: ResourceReceipt,
}

/// Stable, redacted runtime failure categories surfaced through the dispatch port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeDispatchErrorKind {
    Backend,
    Client,
    Executor,
    ExitFailure,
    ExtensionRuntimeMismatch,
    FilesystemDenied,
    Guest,
    InputEncode,
    InvalidResult,
    Manifest,
    Memory,
    MethodMissing,
    NetworkDenied,
    OperationFailed,
    OutputDecode,
    OutputTooLarge,
    PolicyDenied,
    Resource,
    SecretDenied,
    UndeclaredCapability,
    UnsupportedRunner,
    Unknown,
}

impl RuntimeDispatchErrorKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Backend => "Backend",
            Self::Client => "Client",
            Self::Executor => "Executor",
            Self::ExitFailure => "ExitFailure",
            Self::ExtensionRuntimeMismatch => "ExtensionRuntimeMismatch",
            Self::FilesystemDenied => "FilesystemDenied",
            Self::Guest => "Guest",
            Self::InputEncode => "InputEncode",
            Self::InvalidResult => "InvalidResult",
            Self::Manifest => "Manifest",
            Self::Memory => "Memory",
            Self::MethodMissing => "MethodMissing",
            Self::NetworkDenied => "NetworkDenied",
            Self::OperationFailed => "OperationFailed",
            Self::OutputDecode => "OutputDecode",
            Self::OutputTooLarge => "OutputTooLarge",
            Self::PolicyDenied => "PolicyDenied",
            Self::Resource => "Resource",
            Self::SecretDenied => "SecretDenied",
            Self::UndeclaredCapability => "UndeclaredCapability",
            Self::UnsupportedRunner => "UnsupportedRunner",
            Self::Unknown => "Unknown",
        }
    }

    /// Sanitizer-compatible event/audit token for this redacted failure kind.
    pub const fn event_kind(self) -> &'static str {
        match self {
            Self::Backend => "backend",
            Self::Client => "client",
            Self::Executor => "executor",
            Self::ExitFailure => "exit_failure",
            Self::ExtensionRuntimeMismatch => "extension.runtime_mismatch",
            Self::FilesystemDenied => "filesystem_denied",
            Self::Guest => "guest",
            Self::InputEncode => "input_encode",
            Self::InvalidResult => "invalid_result",
            Self::Manifest => "manifest",
            Self::Memory => "memory",
            Self::MethodMissing => "method_missing",
            Self::NetworkDenied => "network_denied",
            Self::OperationFailed => "operation_failed",
            Self::OutputDecode => "output_decode",
            Self::OutputTooLarge => "output_too_large",
            Self::PolicyDenied => "policy_denied",
            Self::Resource => "resource",
            Self::SecretDenied => "secret_denied",
            Self::UndeclaredCapability => "undeclared_capability",
            Self::UnsupportedRunner => "unsupported_runner",
            Self::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for RuntimeDispatchErrorKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Stable, redacted dispatch failure categories surfaced above the neutral dispatch port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchFailureKind {
    UnknownCapability,
    UnknownProvider,
    RuntimeMismatch,
    MissingRuntimeBackend,
    UnsupportedRuntime,
    AuthRequired,
    Runtime(RuntimeDispatchErrorKind),
}

impl DispatchFailureKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnknownCapability => "UnknownCapability",
            Self::UnknownProvider => "UnknownProvider",
            Self::RuntimeMismatch => "RuntimeMismatch",
            Self::MissingRuntimeBackend => "MissingRuntimeBackend",
            Self::UnsupportedRuntime => "UnsupportedRuntime",
            Self::AuthRequired => "AuthRequired",
            Self::Runtime(kind) => kind.as_str(),
        }
    }
}

impl std::fmt::Display for DispatchFailureKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Runtime dispatch failures surfaced through the neutral host API port.
#[derive(Error)]
pub enum DispatchError {
    #[error("unknown capability {capability}")]
    UnknownCapability { capability: CapabilityId },
    #[error("capability {capability} provider {provider} is not registered")]
    UnknownProvider {
        capability: CapabilityId,
        provider: ExtensionId,
    },
    #[error(
        "capability {capability} descriptor runtime {descriptor_runtime:?} does not match package runtime {package_runtime:?}"
    )]
    RuntimeMismatch {
        capability: CapabilityId,
        descriptor_runtime: RuntimeKind,
        package_runtime: RuntimeKind,
    },
    #[error("runtime backend {runtime:?} is not configured")]
    MissingRuntimeBackend { runtime: RuntimeKind },
    #[error(
        "runtime {runtime:?} is recognized but not supported by this dispatcher yet for capability {capability}"
    )]
    UnsupportedRuntime {
        capability: CapabilityId,
        runtime: RuntimeKind,
    },
    /// Authentication is required to dispatch this capability.
    ///
    /// `required_secrets` names the credentials the caller must stage.  The
    /// field is intentionally absent from the `Debug` output to avoid leaking
    /// secret-handle identifiers into logs.
    #[error("capability {capability} dispatch requires authentication")]
    AuthRequired {
        capability: CapabilityId,
        required_secrets: Vec<SecretHandle>,
    },
    #[error("MCP dispatch failed: {kind}")]
    Mcp { kind: RuntimeDispatchErrorKind },
    #[error("script dispatch failed: {kind}")]
    Script { kind: RuntimeDispatchErrorKind },
    #[error("WASM dispatch failed: {kind}")]
    Wasm { kind: RuntimeDispatchErrorKind },
    #[error("first-party dispatch failed: {kind}")]
    FirstParty { kind: RuntimeDispatchErrorKind },
}

impl fmt::Debug for DispatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownCapability { capability } => f
                .debug_struct("UnknownCapability")
                .field("capability", capability)
                .finish(),
            Self::UnknownProvider {
                capability,
                provider,
            } => f
                .debug_struct("UnknownProvider")
                .field("capability", capability)
                .field("provider", provider)
                .finish(),
            Self::RuntimeMismatch {
                capability,
                descriptor_runtime,
                package_runtime,
            } => f
                .debug_struct("RuntimeMismatch")
                .field("capability", capability)
                .field("descriptor_runtime", descriptor_runtime)
                .field("package_runtime", package_runtime)
                .finish(),
            Self::MissingRuntimeBackend { runtime } => f
                .debug_struct("MissingRuntimeBackend")
                .field("runtime", runtime)
                .finish(),
            Self::UnsupportedRuntime {
                capability,
                runtime,
            } => f
                .debug_struct("UnsupportedRuntime")
                .field("capability", capability)
                .field("runtime", runtime)
                .finish(),
            // `required_secrets` handle names are omitted from Debug output to
            // prevent leaking secret identifiers into logs and error chains.
            Self::AuthRequired {
                capability,
                required_secrets,
            } => f
                .debug_struct("AuthRequired")
                .field("capability", capability)
                .field(
                    "required_secrets",
                    &format!("[{} handle(s) redacted]", required_secrets.len()),
                )
                .finish(),
            Self::Mcp { kind } => f.debug_struct("Mcp").field("kind", kind).finish(),
            Self::Script { kind } => f.debug_struct("Script").field("kind", kind).finish(),
            Self::Wasm { kind } => f.debug_struct("Wasm").field("kind", kind).finish(),
            Self::FirstParty { kind } => f.debug_struct("FirstParty").field("kind", kind).finish(),
        }
    }
}

/// Stable two-variant error for staged credential operations.
///
/// Both the host-runtime staging layer (`ProductAuthCredentialStageError`) and the
/// per-extension staging traits (e.g. `GsuiteCredentialStageError`) map 1:1 to this
/// type so that no mechanical conversion glue is needed across crate boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialStageError {
    /// Credential is missing, expired, or revoked — user must re-authenticate.
    AuthRequired,
    /// Internal staging failure not attributable to the user's credentials.
    Backend,
}

impl DispatchError {
    pub const fn failure_kind(&self) -> DispatchFailureKind {
        match self {
            Self::UnknownCapability { .. } => DispatchFailureKind::UnknownCapability,
            Self::UnknownProvider { .. } => DispatchFailureKind::UnknownProvider,
            Self::RuntimeMismatch { .. } => DispatchFailureKind::RuntimeMismatch,
            Self::MissingRuntimeBackend { .. } => DispatchFailureKind::MissingRuntimeBackend,
            Self::UnsupportedRuntime { .. } => DispatchFailureKind::UnsupportedRuntime,
            Self::AuthRequired { .. } => DispatchFailureKind::AuthRequired,
            Self::Mcp { kind }
            | Self::Script { kind }
            | Self::Wasm { kind }
            | Self::FirstParty { kind } => DispatchFailureKind::Runtime(*kind),
        }
    }

    /// Stable event-token string for the error, suitable for telemetry and structured logging.
    ///
    /// This is the single canonical source for dispatch error event tokens; crates should
    /// call this method rather than maintaining a parallel local `match` over `DispatchError`.
    pub fn event_kind(&self) -> &'static str {
        match self {
            Self::UnknownCapability { .. } => "unknown_capability",
            Self::UnknownProvider { .. } => "unknown_provider",
            Self::RuntimeMismatch { .. } => "runtime_mismatch",
            Self::MissingRuntimeBackend { .. } => "missing_runtime_backend",
            Self::UnsupportedRuntime { .. } => "unsupported_runtime",
            Self::AuthRequired { .. } => "auth_required",
            Self::Mcp { kind }
            | Self::Script { kind }
            | Self::Wasm { kind }
            | Self::FirstParty { kind } => kind.event_kind(),
        }
    }
}

/// Interface for already-authorized runtime dispatch.
#[async_trait]
pub trait CapabilityDispatcher: Send + Sync {
    /// Dispatches one already-authorized JSON capability request and must not perform caller-facing authorization or approval resolution.
    async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<CapabilityDispatchResult, DispatchError>;
}
