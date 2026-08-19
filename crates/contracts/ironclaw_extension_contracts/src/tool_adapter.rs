//! The extension **tool adapter** contract.
//!
//! One adapter instance per extension, one method: given validated input for
//! a declared (or MCP-discovered) capability, do the work
//! (`docs/internal/reborn/extension-runtime/overview.md` §4.1). Everything else —
//! what tools exist, listing, validation, authorization, approvals,
//! obligations, resource reservation, credential injection, events, audit —
//! is manifest data or the host dispatcher pipeline. Adapters never report
//! metadata, and discovery is never part of this ABI.
//!
//! This module is call vocabulary, not wire vocabulary: a [`ToolCall`] is an
//! in-process envelope the dispatcher builds per invocation; nothing here
//! serializes.

use async_trait::async_trait;

use ironclaw_host_api::{
    Timestamp,
    action::NetworkMethod,
    decision::RuntimeCredentialAuthRequirement,
    dispatch::{
        CapabilityDisplayOutputPreview, DispatchFailureDetail, DispatchFailureKind,
        ProviderDiagnostic, RuntimeDispatchErrorKind,
    },
    ids::{CapabilityId, SecretHandle},
    mount::MountView,
    resource::{ResourceEstimate, ResourceReservation, ResourceScope},
    runtime::RuntimeKind,
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
#[derive(Debug, Clone, thiserror::Error)]
pub enum ToolError {
    #[error("tool invocation requires authorization")]
    AuthRequired {
        required_secrets: Vec<SecretHandle>,
        credential_requirements: Vec<RuntimeCredentialAuthRequirement>,
        /// Raw provider rejection retained for the host's downstream
        /// model-diagnostic scrub/fence seam. Its `Debug` representation is
        /// redacted; never log or render it directly.
        model_visible_cause: Option<ProviderDiagnostic>,
    },
    #[error("tool provider rejected invocation ({kind})")]
    Rejected {
        runtime: Option<RuntimeKind>,
        kind: DispatchFailureKind,
        /// Provider-authored metadata remains typed so its `Debug` output is
        /// redacted until the host's model-diagnostic scrub/fence seam.
        diagnostic: Option<ProviderDiagnostic>,
        detail: Option<DispatchFailureDetail>,
    },
    #[error("tool invocation failed ({kind:?})")]
    Failed {
        kind: RuntimeDispatchErrorKind,
        /// Fixed, host-authored text only — never interpolated payload data.
        safe_summary: Option<String>,
        /// Raw-or-better failure cause for the model-visible Diagnostic seam
        /// (#5965). Carried across the tool ABI verbatim and scrubbed
        /// downstream (`scrub_model_visible_detail`) — NOT display-safe here;
        /// never log or render it directly.
        model_visible_cause: Option<String>,
    },
}

impl PartialEq for ToolError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::AuthRequired {
                    required_secrets: left_secrets,
                    credential_requirements: left_requirements,
                    ..
                },
                Self::AuthRequired {
                    required_secrets: right_secrets,
                    credential_requirements: right_requirements,
                    ..
                },
            ) => left_secrets == right_secrets && left_requirements == right_requirements,
            (
                Self::Failed {
                    kind: left_kind,
                    safe_summary: left_summary,
                    model_visible_cause: left_cause,
                },
                Self::Failed {
                    kind: right_kind,
                    safe_summary: right_summary,
                    model_visible_cause: right_cause,
                },
            ) => {
                left_kind == right_kind
                    && left_summary == right_summary
                    && left_cause == right_cause
            }
            (
                Self::Rejected {
                    runtime: left_runtime,
                    kind: left_kind,
                    diagnostic: left_diagnostic,
                    detail: left_detail,
                },
                Self::Rejected {
                    runtime: right_runtime,
                    kind: right_kind,
                    diagnostic: right_diagnostic,
                    detail: right_detail,
                },
            ) => {
                left_runtime == right_runtime
                    && left_kind == right_kind
                    && left_diagnostic == right_diagnostic
                    && left_detail == right_detail
            }
            _ => false,
        }
    }
}

impl Eq for ToolError {}

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
    use ironclaw_host_api::dispatch::{DispatchInputIssue, DispatchInputIssueCode};

    #[test]
    fn tool_error_display_stays_redacted() {
        let error = ToolError::Failed {
            kind: RuntimeDispatchErrorKind::Backend,
            safe_summary: Some("vendor API unavailable".to_string()),
            model_visible_cause: None,
        };
        let rendered = error.to_string();
        assert!(rendered.contains("Backend"), "{rendered}");
        assert!(!rendered.contains("token"), "{rendered}");
    }

    #[test]
    fn rejected_debug_redacts_nested_diagnostic_detail_but_keeps_input_detail() {
        let marker = "TOOL_DIAGNOSTIC_SECRET_MARKER";
        let error = ToolError::Rejected {
            runtime: Some(RuntimeKind::Wasm),
            kind: DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::Guest),
            diagnostic: None,
            detail: Some(DispatchFailureDetail::Diagnostic {
                text: marker.to_string(),
            }),
        };
        let debug = format!("{error:?}");
        assert!(!debug.contains(marker), "diagnostic text leaked: {debug}");
        assert!(
            debug.contains("Diagnostic"),
            "detail kind was lost: {debug}"
        );
        assert!(
            debug.contains("<redacted>"),
            "redaction is not visible: {debug}"
        );

        let useful = ToolError::Rejected {
            runtime: Some(RuntimeKind::Wasm),
            kind: DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::Guest),
            diagnostic: None,
            detail: Some(DispatchFailureDetail::InvalidInput {
                issues: vec![
                    DispatchInputIssue::new("/title", DispatchInputIssueCode::TypeMismatch)
                        .expected("string")
                        .received("integer"),
                ],
            }),
        };
        let useful_debug = format!("{useful:?}");
        assert!(
            useful_debug.contains("/title"),
            "input path was lost: {useful_debug}"
        );
        assert!(
            useful_debug.contains("string"),
            "input detail was lost: {useful_debug}"
        );
    }

    #[test]
    fn restricted_egress_errors_name_the_denied_authority() {
        let error = RestrictedEgressError::UndeclaredHost {
            host: "evil.example".to_string(),
        };
        assert!(error.to_string().contains("evil.example"));
    }

    #[test]
    fn auth_required_equality_ignores_model_visible_cause() {
        let secrets = vec![SecretHandle::new("notion-token").unwrap()];
        let left = ToolError::AuthRequired {
            required_secrets: secrets.clone(),
            credential_requirements: Vec::new(),
            model_visible_cause: Some(ProviderDiagnostic {
                code: None,
                message: None,
                retry_after: None,
            }),
        };
        let right = ToolError::AuthRequired {
            required_secrets: secrets,
            credential_requirements: Vec::new(),
            model_visible_cause: None,
        };

        assert_eq!(left, right);
    }

    #[test]
    fn auth_required_equality_still_compares_required_secrets() {
        let left = ToolError::AuthRequired {
            required_secrets: vec![SecretHandle::new("notion-token").unwrap()],
            credential_requirements: Vec::new(),
            model_visible_cause: None,
        };
        let right = ToolError::AuthRequired {
            required_secrets: vec![SecretHandle::new("slack-token").unwrap()],
            credential_requirements: Vec::new(),
            model_visible_cause: None,
        };

        assert_ne!(left, right);
    }

    fn rejected(kind: DispatchFailureKind, diagnostic: Option<ProviderDiagnostic>) -> ToolError {
        ToolError::Rejected {
            runtime: Some(RuntimeKind::FirstParty),
            kind,
            diagnostic,
            detail: None,
        }
    }

    #[test]
    fn rejected_equality_compares_kind_and_diagnostic() {
        let denied = DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::PolicyDenied);
        let network_denied = DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::NetworkDenied);

        assert_eq!(
            rejected(denied, None),
            rejected(denied, None),
            "identical Rejected values must be equal"
        );
        assert_ne!(
            rejected(denied, None),
            rejected(network_denied, None),
            "differing kind must break equality"
        );
        assert_ne!(
            rejected(
                denied,
                Some(ProviderDiagnostic {
                    code: None,
                    message: None,
                    retry_after: None,
                })
            ),
            rejected(denied, None),
            "differing diagnostic presence must break equality"
        );
    }

    #[test]
    fn tool_error_of_different_variants_are_never_equal() {
        let auth_required = ToolError::AuthRequired {
            required_secrets: Vec::new(),
            credential_requirements: Vec::new(),
            model_visible_cause: None,
        };
        let failed = ToolError::Failed {
            kind: RuntimeDispatchErrorKind::Backend,
            safe_summary: None,
            model_visible_cause: None,
        };

        assert_ne!(auth_required, failed);
    }
}
