//! Neutral capability dispatch port contracts.
//!
//! These types describe the host-facing request/result/error shape for already
//! authorized capability dispatch. Runtime-specific dispatcher implementations
//! live outside this crate.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::{
    CapabilityId, ErrorKind, ExtensionId, ResourceEstimate, ResourceReceipt, ResourceScope,
    ResourceUsage, RuntimeKind,
};

/// Already-authorized request to dispatch one declared capability.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityDispatchRequest {
    pub capability_id: CapabilityId,
    pub scope: ResourceScope,
    pub estimate: ResourceEstimate,
    pub input: Value,
}

/// Normalized capability dispatch result returned by a runtime port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityDispatchResult {
    pub capability_id: CapabilityId,
    pub provider: ExtensionId,
    pub runtime: RuntimeKind,
    pub output: Value,
    pub usage: ResourceUsage,
    pub receipt: ResourceReceipt,
}

/// Host-safe dispatch failure classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityDispatchFailureKind {
    UnknownCapability,
    UnknownProvider,
    RuntimeMismatch,
    MissingRuntimeBackend,
    UnsupportedRuntime,
    Wasm,
    Script,
    Mcp,
}

impl CapabilityDispatchFailureKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnknownCapability => "UnknownCapability",
            Self::UnknownProvider => "UnknownProvider",
            Self::RuntimeMismatch => "RuntimeMismatch",
            Self::MissingRuntimeBackend => "MissingRuntimeBackend",
            Self::UnsupportedRuntime => "UnsupportedRuntime",
            Self::Wasm => "Wasm",
            Self::Script => "Script",
            Self::Mcp => "Mcp",
        }
    }
}

/// Host-safe dispatch error returned across the capability dispatch port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Error)]
#[error("capability dispatch failed: {kind:?}")]
pub struct CapabilityDispatchError {
    pub kind: CapabilityDispatchFailureKind,
    pub capability_id: CapabilityId,
    pub provider: Option<ExtensionId>,
    pub runtime: Option<RuntimeKind>,
}

impl CapabilityDispatchError {
    pub fn new(
        kind: CapabilityDispatchFailureKind,
        capability_id: CapabilityId,
        provider: Option<ExtensionId>,
        runtime: Option<RuntimeKind>,
    ) -> Self {
        Self {
            kind,
            capability_id,
            provider,
            runtime,
        }
    }

    pub fn error_kind(&self) -> ErrorKind {
        ErrorKind::new(self.kind.as_str())
    }
}

/// Runtime port for already-authorized capability dispatch.
#[async_trait]
pub trait CapabilityDispatcher: Send + Sync {
    /// Dispatches one already-authorized JSON capability request and must not perform caller-facing authorization or approval resolution.
    async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<CapabilityDispatchResult, CapabilityDispatchError>;
}
