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
