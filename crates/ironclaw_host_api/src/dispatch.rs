//! Neutral capability dispatch port contracts.
//!
//! These types describe an already-authorized capability dispatch request and
//! normalized runtime result. Concrete dispatcher/runtime crates implement the
//! behavior; caller-facing workflow crates depend only on this neutral port.

use async_trait::async_trait;
use serde_json::Value;
use thiserror::Error;

use crate::{
    CapabilityId, ExtensionId, MountView, ResourceEstimate, ResourceReceipt, ResourceReservation,
    ResourceScope, ResourceUsage, RuntimeKind,
};

/// Raw request data for one declared capability dispatch.
///
/// This payload is not sufficient authority to invoke a runtime. Dispatcher
/// ports accept only [`AuthorizedDispatchRequest`], which is minted after the
/// capability host has completed authorization, obligation preparation, and
/// resource reservation.
#[derive(Debug, Clone, PartialEq)]
pub struct CapabilityDispatchRequest {
    pub capability_id: CapabilityId,
    pub scope: ResourceScope,
    pub estimate: ResourceEstimate,
    pub mounts: Option<MountView>,
    pub resource_reservation: Option<ResourceReservation>,
    pub input: Value,
}

/// Opaque proof that a dispatch request came through an approved authority path.
///
/// Rust cannot restrict a constructor to one sibling crate, so production code
/// must keep minting at the capability-host boundary and architecture tests
/// guard that invariant. The source is intentionally diagnostic only; it does
/// not leave the sealed request as a reusable authority handle.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct DispatchAuthorityProof {
    source: DispatchAuthoritySource,
    _private: (),
}

impl std::fmt::Debug for DispatchAuthorityProof {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DispatchAuthorityProof").finish()
    }
}

impl DispatchAuthorityProof {
    /// Mint proof after capability-host authorization and obligation checks.
    ///
    /// Use only at the capability-host boundary after authorization,
    /// obligation preparation, and resource reservation have completed.
    #[doc(hidden)]
    pub const fn capability_host() -> Self {
        Self {
            source: DispatchAuthoritySource::CapabilityHost,
            _private: (),
        }
    }

    /// Mint proof for host-owned process execution of previously authorized work.
    ///
    /// Use only inside the host process-executor boundary for work that has
    /// already passed dispatch authorization.
    #[doc(hidden)]
    pub const fn host_process_executor() -> Self {
        Self {
            source: DispatchAuthoritySource::HostProcessExecutor,
            _private: (),
        }
    }

    /// Mint proof for contract tests that exercise dispatcher behavior directly.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub const fn test() -> Self {
        Self {
            source: DispatchAuthoritySource::Test,
            _private: (),
        }
    }

    pub const fn source(self) -> DispatchAuthoritySource {
        self.source
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchAuthoritySource {
    CapabilityHost,
    HostProcessExecutor,
    #[cfg(any(test, feature = "test-support"))]
    Test,
}

/// Capability dispatch request sealed with authority proof.
#[derive(Debug, Clone, PartialEq)]
pub struct AuthorizedDispatchRequest {
    request: CapabilityDispatchRequest,
    authority: DispatchAuthorityProof,
}

impl AuthorizedDispatchRequest {
    /// Seal raw request data after the caller has proved dispatch authority.
    pub fn new(request: CapabilityDispatchRequest, authority: DispatchAuthorityProof) -> Self {
        Self { request, authority }
    }

    pub fn capability_id(&self) -> &CapabilityId {
        &self.request.capability_id
    }

    pub fn scope(&self) -> &ResourceScope {
        &self.request.scope
    }

    pub fn estimate(&self) -> &ResourceEstimate {
        &self.request.estimate
    }

    pub fn mounts(&self) -> Option<&MountView> {
        self.request.mounts.as_ref()
    }

    pub fn resource_reservation(&self) -> Option<&ResourceReservation> {
        self.request.resource_reservation.as_ref()
    }

    pub fn input(&self) -> &Value {
        &self.request.input
    }

    pub const fn authority_source(&self) -> DispatchAuthoritySource {
        self.authority.source()
    }
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
    OutputDecode,
    OutputTooLarge,
    Resource,
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
            Self::OutputDecode => "OutputDecode",
            Self::OutputTooLarge => "OutputTooLarge",
            Self::Resource => "Resource",
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
            Self::OutputDecode => "output_decode",
            Self::OutputTooLarge => "output_too_large",
            Self::Resource => "resource",
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
#[derive(Debug, Error)]
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
    #[error("MCP dispatch failed: {kind}")]
    Mcp { kind: RuntimeDispatchErrorKind },
    #[error("script dispatch failed: {kind}")]
    Script { kind: RuntimeDispatchErrorKind },
    #[error("WASM dispatch failed: {kind}")]
    Wasm { kind: RuntimeDispatchErrorKind },
    #[error("first-party dispatch failed: {kind}")]
    FirstParty { kind: RuntimeDispatchErrorKind },
}

impl DispatchError {
    pub const fn failure_kind(&self) -> DispatchFailureKind {
        match self {
            Self::UnknownCapability { .. } => DispatchFailureKind::UnknownCapability,
            Self::UnknownProvider { .. } => DispatchFailureKind::UnknownProvider,
            Self::RuntimeMismatch { .. } => DispatchFailureKind::RuntimeMismatch,
            Self::MissingRuntimeBackend { .. } => DispatchFailureKind::MissingRuntimeBackend,
            Self::UnsupportedRuntime { .. } => DispatchFailureKind::UnsupportedRuntime,
            Self::Mcp { kind }
            | Self::Script { kind }
            | Self::Wasm { kind }
            | Self::FirstParty { kind } => DispatchFailureKind::Runtime(*kind),
        }
    }
}

/// Interface for already-authorized runtime dispatch.
#[async_trait]
pub trait CapabilityDispatcher: Send + Sync {
    /// Dispatches one sealed, already-authorized JSON capability request and must not perform caller-facing authorization or approval resolution.
    async fn dispatch_json(
        &self,
        request: AuthorizedDispatchRequest,
    ) -> Result<CapabilityDispatchResult, DispatchError>;
}
