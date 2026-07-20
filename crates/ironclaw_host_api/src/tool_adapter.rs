//! The extension **tool adapter** contract.
//!
//! One adapter instance per extension, one method: given validated input for
//! a declared (or MCP-discovered) capability, do the work
//! (`docs/reborn/extension-runtime/overview.md` §4.1). Everything else —
//! what tools exist, listing, validation, authorization, approvals,
//! obligations, resource reservation, credential injection, events, audit —
//! is manifest data or the host dispatcher pipeline. Adapters never report
//! metadata, and discovery is never part of this ABI.
//!
//! This module is call vocabulary, not wire vocabulary: a [`ToolCall`] is an
//! in-process envelope the dispatcher builds per invocation; nothing here
//! serializes.

use async_trait::async_trait;

use crate::{
    CapabilityDisplayOutputPreview, CapabilityId, MountView, NetworkMethod, ResourceEstimate,
    ResourceReservation, ResourceScope, RuntimeCredentialAuthRequirement, RuntimeDispatchErrorKind,
    SecretHandle, Timestamp,
};

/// One invocation of one declared capability.
#[derive(Debug)]
pub struct ToolCall {
    pub capability_id: CapabilityId,
    /// Actor/turn authority scope for this invocation (carries the
    /// invocation identity).
    pub scope: ResourceScope,
    /// Schema-validated input.
    pub input: serde_json::Value,
    /// Host-imposed completion deadline, when bounded.
    pub deadline: Option<Timestamp>,
    /// Host resource bookkeeping prepared by the obligation pipeline; the
    /// invoking lane reconciles or releases it (same legs as today's
    /// runtime adapters).
    pub resources: ToolCallResources,
}

/// Obligation-prepared resource context carried alongside a call.
#[derive(Debug, Default)]
pub struct ToolCallResources {
    pub estimate: ResourceEstimate,
    pub mounts: Option<MountView>,
    pub reservation: Option<ResourceReservation>,
}

/// Successful invocation output. Behavior only — resource usage, the
/// reservation receipt, events, and audit are the host's, produced by the
/// loader/dispatcher pipeline that wraps `invoke`, never by the adapter.
#[derive(Debug)]
pub struct ToolResult {
    pub output: serde_json::Value,
    pub display_preview: Option<CapabilityDisplayOutputPreview>,
    /// The adapter's own count of the output payload bytes (the host
    /// re-measures for enforcement; this is advisory).
    pub output_bytes: u64,
}

/// Typed invocation failures. The host maps these onto the dispatch port's
/// redacted failure categories; `AuthRequired` maps to the generic re-auth
/// gate and resumes through the standard blocked-turn flow.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ToolError {
    #[error("invalid tool input: {reason}")]
    InvalidInput { reason: String },
    #[error("tool invocation requires authorization")]
    AuthRequired {
        required_secrets: Vec<SecretHandle>,
        credential_requirements: Vec<RuntimeCredentialAuthRequirement>,
    },
    #[error("tool invocation failed ({kind:?})")]
    Failed {
        kind: RuntimeDispatchErrorKind,
        /// Fixed, host-authored text only — never interpolated payload data.
        safe_summary: Option<String>,
    },
}

/// Host ports available to an adapter during one invocation — derived from
/// the resolved contract, nothing wider. A port is `None` exactly when the
/// declaration grants it nothing (no declared egress ⇒ no egress port), so
/// an adapter cannot reach authority its manifest never named.
pub struct ToolPorts<'a> {
    pub egress: Option<&'a dyn RestrictedEgress>,
}

/// Invoke one declared (or MCP-discovered) capability.
///
/// There is **one adapter instance per extension, not per tool**: the call
/// carries the capability id and the adapter routes internally.
#[async_trait]
pub trait ToolAdapter: Send + Sync {
    async fn invoke(&self, call: ToolCall, ports: &ToolPorts<'_>) -> Result<ToolResult, ToolError>;
}

/// Host-mediated outbound HTTP for adapters: scheme/host/method allowlists
/// come from the resolved contract, credentials are injected host-side by
/// declared handle, responses are size-capped, and cross-host redirects and
/// private-IP targets are denied. Adapters never see secret bytes.
#[async_trait]
pub trait RestrictedEgress: Send + Sync {
    async fn send(
        &self,
        request: RestrictedEgressRequest,
    ) -> Result<RestrictedEgressResponse, RestrictedEgressError>;
}

/// One outbound request an adapter asks the host to perform.
#[derive(Debug, Clone)]
pub struct RestrictedEgressRequest {
    pub method: NetworkMethod,
    /// Full `https` URL; the host rejects hosts outside the declared
    /// allowlist before any network activity.
    pub url: String,
    /// Additional request headers. Host-owned headers (`authorization`
    /// where injection is declared, `host`, hop-by-hop) are rejected.
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    /// Declared credential handle to inject, if the call needs one. An
    /// undeclared handle is rejected before any network activity.
    pub credential: Option<SecretHandle>,
    /// Declared body-credential handles to inject into the JSON body at
    /// their manifest-declared RFC 6901 pointers (`[[channel.egress]]
    /// body_credentials`). A handle without a declared binding for the
    /// matched target is rejected before any network activity; the adapter
    /// names handles only and never sees secret bytes.
    pub body_credentials: Vec<SecretHandle>,
}

/// Status and size-capped body; response headers are deliberately not
/// exposed to adapters.
#[derive(Debug, Clone)]
pub struct RestrictedEgressResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// Typed restricted-egress failures, all raised before or at the network
/// boundary.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RestrictedEgressError {
    #[error("egress host is not declared by the extension contract: {host}")]
    UndeclaredHost { host: String },
    #[error("egress method is not declared for this host")]
    UndeclaredMethod,
    #[error("egress header is host-owned and cannot be supplied by an adapter: {name}")]
    HostOwnedHeader { name: String },
    #[error("egress credential handle is not declared by the extension contract: {handle}")]
    UndeclaredCredential { handle: String },
    #[error("egress credential is not available")]
    AuthRequired {
        required_secrets: Vec<SecretHandle>,
        credential_requirements: Vec<RuntimeCredentialAuthRequirement>,
    },
    #[error("egress request was rejected by host network policy")]
    PolicyDenied,
    #[error("egress response exceeded the host size cap")]
    ResponseTooLarge,
    #[error("egress transport failed: {reason}")]
    Transport { reason: String },
    #[error("egress deadline exceeded")]
    DeadlineExceeded,
}

impl ToolCall {
    /// Convenience constructor for the common shape; resource bookkeeping
    /// defaults to empty and is filled by the dispatcher.
    pub fn new(
        capability_id: CapabilityId,
        scope: ResourceScope,
        input: serde_json::Value,
    ) -> Self {
        Self {
            capability_id,
            scope,
            input,
            deadline: None,
            resources: ToolCallResources::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_error_display_stays_redacted() {
        let error = ToolError::Failed {
            kind: RuntimeDispatchErrorKind::Backend,
            safe_summary: Some("vendor API unavailable".to_string()),
        };
        let rendered = error.to_string();
        assert!(rendered.contains("Backend"), "{rendered}");
        assert!(!rendered.contains("token"), "{rendered}");
    }

    #[test]
    fn restricted_egress_errors_name_the_denied_authority() {
        let error = RestrictedEgressError::UndeclaredHost {
            host: "evil.example".to_string(),
        };
        assert!(error.to_string().contains("evil.example"));
    }
}
