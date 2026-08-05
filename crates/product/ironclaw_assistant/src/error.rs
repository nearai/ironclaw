//! Workflow-layer error vocabulary.
//!
//! [`ProductSurfaceFailure`] is the internal error type used within the workflow
//! crate. It converts to [`ProductAdapterError`] at the service boundary so
//! adapters never see host-layer details.
//!
//! It is the **superset** half of a two-part vocabulary, and the claim above is
//! now enforced rather than asserted. The boundary half —
//! [`ProductOperationFailure`], the six string-or-nothing variants a port
//! implementor below product may produce — lives in
//! `ironclaw_product_contracts::error` so `ironclaw_extension_host` and its
//! successors never depend on this crate to describe their own failures. What
//! stays here is what is genuinely workflow-internal: the turn-coordinator
//! variants carrying [`TurnError`], the interaction rejection kinds, the
//! idempotency replay, and the inbound-attachment and policy failures. The
//! [`From<ProductOperationFailure>`] below is total and 1:1, so a port failure
//! reaching product through `?` keeps its exact discriminant.
//!
//! See `docs/reborn/target-architecture/PROPOSAL.md` §6.1.3 for the recorded
//! ownership decision and the alternatives it beat.

use crate::{ProductAdapterError, ProductSurfaceRejectionKind, RedactedString};
use ironclaw_host_api::error::HostApiError;
use ironclaw_product_contracts::error::ProductOperationFailure;
use ironclaw_product_contracts::surface::ProductSurfaceError;
use ironclaw_turns::{TurnError, TurnErrorCategory};
use thiserror::Error;

use crate::approval_interaction::ApprovalInteractionRejectionKind;
use crate::auth_interaction::AuthInteractionRejectionKind;

/// Stable reasons for rejecting an auth continuation before or during turn resume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthContinuationRejectionKind {
    NotTurnGateResume,
    MissingThreadScope,
    InvalidTurnRunRef,
    InvalidGateRef,
    InvalidIdempotencyKey,
    InvalidBindingRef,
    UnauthorizedBlockedGate,
}

impl AuthContinuationRejectionKind {
    pub fn sanitized_reason(self) -> &'static str {
        match self {
            Self::NotTurnGateResume => "auth continuation is not a turn-gate resume",
            Self::MissingThreadScope => "invalid auth continuation scope",
            Self::InvalidTurnRunRef => "invalid auth continuation run reference",
            Self::InvalidGateRef => "invalid auth continuation gate reference",
            Self::InvalidIdempotencyKey => "invalid auth continuation idempotency key",
            Self::InvalidBindingRef => "invalid auth continuation binding ref",
            Self::UnauthorizedBlockedGate => {
                "auth continuation does not match an authorized blocked auth gate"
            }
        }
    }
}

/// Internal error type for the product workflow service.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProductSurfaceFailure {
    /// The adapter installation is not mapped to a tenant.
    #[error("unknown adapter installation")]
    UnknownInstallation,

    /// The conversation binding could not be resolved for the given external refs.
    #[error("binding resolution failed: {reason}")]
    BindingResolutionFailed { reason: String },

    /// The external actor has no trusted binding to a canonical user.
    #[error("binding required: {reason}")]
    BindingRequired { reason: String },

    /// The actor or route is not allowed to use the resolved thread.
    #[error("binding access denied")]
    BindingAccessDenied,

    /// The binding request is invalid and should not be retried unchanged.
    #[error("invalid binding request: {reason}")]
    InvalidBindingRequest { reason: String },

    /// A provider's OPERATOR-level instance configuration (e.g. no OAuth
    /// backend registered on this build at all) is missing entirely — a
    /// static, build-time fact distinct from a per-user missing/expired
    /// credential account. `reason` is a plain, host-authored `String` (not a
    /// composition type — this crate sits below `composition` in the
    /// dependency graph) carrying the exact remediation text (e.g. the
    /// `ironclaw config set` commands to run). Two independent consumers read
    /// this variant differently: the WebUI service
    /// (`lifecycle_product_surface_error`) discards `reason` and maps
    /// the DISCRIMINANT alone to a sanitized 400 (no free text crosses the
    /// wire contract); the LLM tool path
    /// (`extension_lifecycle_capabilities::lifecycle_error`) forwards
    /// `reason` verbatim onto the diagnostic-detail channel so the exact
    /// `config set` commands reach the model.
    #[error("provider instance not configured: {reason}")]
    ProviderInstanceNotConfigured { reason: String },

    /// Turn coordinator rejected the submission before typed turn errors were available.
    #[error("turn submission rejected: {reason}")]
    TurnSubmissionRejected { reason: String },

    /// Turn coordinator rejected the submission with typed category/status information.
    #[error("turn submission failed: {error}")]
    TurnSubmissionFailed { error: TurnError },

    /// Turn coordinator resume rejected.
    #[error("turn resume rejected: {reason}")]
    TurnResumeRejected { reason: String },

    /// Auth continuation was rejected with a stable sanitized reason.
    #[error("auth continuation rejected: {kind:?}")]
    AuthContinuationRejected { kind: AuthContinuationRejectionKind },

    /// Approval interaction was rejected with a stable sanitized reason.
    #[error("approval interaction rejected: {kind:?}")]
    ApprovalInteractionRejected {
        kind: ApprovalInteractionRejectionKind,
    },

    /// Auth interaction was rejected with a stable sanitized reason.
    #[error("auth interaction rejected: {kind:?}")]
    AuthInteractionRejected { kind: AuthInteractionRejectionKind },

    /// Turn coordinator rejected a resume with typed category/status information.
    #[error("turn resume denied: {error}")]
    TurnResumeDenied { error: TurnError },

    /// A transient store or service failure.
    #[error("transient workflow failure: {reason}")]
    Transient { reason: String },

    /// Before-inbound policy failed before it could produce an allow/rewrite/reject outcome.
    #[error("before-inbound policy failed: {reason}")]
    BeforeInboundPolicyFailed { reason: String, permanent: bool },

    /// Deferred channel attachment transfer failed before message acceptance.
    #[error("inbound attachment transfer failed: {reason}")]
    InboundAttachmentFailed { reason: String, retryable: bool },

    /// The action was identified as a duplicate and the prior outcome should be replayed.
    #[error("duplicate action")]
    DuplicateAction {
        prior_outcome: crate::ProductInboundAck,
    },

    /// The requested action kind is not supported by this workflow version.
    #[error("unsupported action kind: {kind}")]
    UnsupportedActionKind { kind: String },

    /// The resolved outbound target is not a direct message, but the payload
    /// requires a DM-only target (e.g. carries an OAuth authorization_url).
    #[error("outbound target is not a direct message but the payload requires one")]
    OutboundTargetNotDirectMessage,
}

impl From<HostApiError> for ProductSurfaceFailure {
    fn from(error: HostApiError) -> Self {
        ProductSurfaceFailure::InvalidBindingRequest {
            reason: error.to_string(),
        }
    }
}

/// Absorb a port implementor's failure into the workflow vocabulary.
///
/// Total and 1:1 — every [`ProductOperationFailure`] discriminant has exactly
/// one image here and carries its payload unchanged, so `?` at a product call
/// site over a `product_contracts` port loses nothing. This direction is the
/// only one that exists: product never narrows a workflow failure back down,
/// because the turn-coordinator and interaction variants have no boundary
/// image and inventing one would flatten them into `Transient`.
impl From<ProductOperationFailure> for ProductSurfaceFailure {
    fn from(value: ProductOperationFailure) -> Self {
        match value {
            ProductOperationFailure::BindingResolutionFailed { reason } => {
                ProductSurfaceFailure::BindingResolutionFailed { reason }
            }
            ProductOperationFailure::BindingRequired { reason } => {
                ProductSurfaceFailure::BindingRequired { reason }
            }
            ProductOperationFailure::BindingAccessDenied => {
                ProductSurfaceFailure::BindingAccessDenied
            }
            ProductOperationFailure::UnknownInstallation => {
                ProductSurfaceFailure::UnknownInstallation
            }
            ProductOperationFailure::TurnSubmissionRejected { reason } => {
                ProductSurfaceFailure::TurnSubmissionRejected { reason }
            }
            ProductOperationFailure::InvalidBindingRequest { reason } => {
                ProductSurfaceFailure::InvalidBindingRequest { reason }
            }
            ProductOperationFailure::ProviderInstanceNotConfigured { reason } => {
                ProductSurfaceFailure::ProviderInstanceNotConfigured { reason }
            }
            ProductOperationFailure::UnsupportedActionKind { kind } => {
                ProductSurfaceFailure::UnsupportedActionKind { kind }
            }
            ProductOperationFailure::Transient { reason } => {
                ProductSurfaceFailure::Transient { reason }
            }
        }
    }
}

fn surface_rejection_kind(category: TurnErrorCategory) -> ProductSurfaceRejectionKind {
    match category {
        TurnErrorCategory::ThreadBusy => ProductSurfaceRejectionKind::ThreadBusy,
        TurnErrorCategory::AdmissionRejected => ProductSurfaceRejectionKind::AdmissionRejected,
        TurnErrorCategory::ScopeNotFound => ProductSurfaceRejectionKind::ScopeNotFound,
        TurnErrorCategory::Unauthorized => ProductSurfaceRejectionKind::Unauthorized,
        TurnErrorCategory::InvalidRequest => ProductSurfaceRejectionKind::InvalidRequest,
        TurnErrorCategory::Unavailable => ProductSurfaceRejectionKind::Unavailable,
        TurnErrorCategory::CapacityExceeded => ProductSurfaceRejectionKind::AdmissionRejected,
        TurnErrorCategory::Conflict => ProductSurfaceRejectionKind::Conflict,
    }
}

/// Project a lifecycle failure onto the sanitized product-surface error.
///
/// The six discriminants product shares with its port implementors delegate to
/// `ProductOperationFailure`'s own projection rather than repeating the status
/// choices, so this path and the one `ironclaw_extension_host` takes cannot
/// drift apart. What stays here is the logging (contracts may not log) and the
/// workflow-only variants, which have no boundary image at all.
pub fn lifecycle_product_surface_error(error: ProductSurfaceFailure) -> ProductSurfaceError {
    match error {
        ProductSurfaceFailure::InvalidBindingRequest { reason } => {
            ProductOperationFailure::InvalidBindingRequest { reason }.into()
        }
        ProductSurfaceFailure::UnsupportedActionKind { kind } => {
            ProductOperationFailure::UnsupportedActionKind { kind }.into()
        }
        ProductSurfaceFailure::ProviderInstanceNotConfigured { reason } => {
            ProductOperationFailure::ProviderInstanceNotConfigured { reason }.into()
        }
        ProductSurfaceFailure::BindingAccessDenied => {
            ProductOperationFailure::BindingAccessDenied.into()
        }
        ProductSurfaceFailure::Transient { reason } => {
            // The 503 body is sanitized; without this line the cause is
            // dropped entirely and the failure is diagnosable from logs.
            tracing::warn!(reason = %reason, "lifecycle action failed with a transient error");
            ProductOperationFailure::Transient { reason }.into()
        }
        ProductSurfaceFailure::BindingResolutionFailed { reason } => {
            ProductOperationFailure::BindingResolutionFailed { reason }.into()
        }
        ProductSurfaceFailure::BindingRequired { .. }
        | ProductSurfaceFailure::TurnSubmissionRejected { .. }
        | ProductSurfaceFailure::TurnSubmissionFailed { .. }
        | ProductSurfaceFailure::TurnResumeRejected { .. }
        | ProductSurfaceFailure::TurnResumeDenied { .. }
        | ProductSurfaceFailure::ApprovalInteractionRejected { .. }
        | ProductSurfaceFailure::AuthInteractionRejected { .. }
        | ProductSurfaceFailure::AuthContinuationRejected { .. }
        | ProductSurfaceFailure::BeforeInboundPolicyFailed { .. }
        | ProductSurfaceFailure::InboundAttachmentFailed { .. }
        | ProductSurfaceFailure::DuplicateAction { .. }
        | ProductSurfaceFailure::OutboundTargetNotDirectMessage
        | ProductSurfaceFailure::UnknownInstallation => ProductSurfaceError::internal_invariant(),
    }
}

impl From<ProductSurfaceFailure> for ProductAdapterError {
    fn from(value: ProductSurfaceFailure) -> Self {
        match value {
            ProductSurfaceFailure::UnknownInstallation => ProductAdapterError::SurfaceRejected {
                kind: ProductSurfaceRejectionKind::Unauthorized,
                status_code: 403,
                retryable: false,
                reason: RedactedString::new("unknown adapter installation"),
            },
            ProductSurfaceFailure::BindingResolutionFailed { reason } => {
                ProductAdapterError::Internal {
                    detail: RedactedString::new(reason),
                }
            }
            ProductSurfaceFailure::BindingRequired { reason } => {
                ProductAdapterError::SurfaceRejected {
                    kind: ProductSurfaceRejectionKind::ScopeNotFound,
                    status_code: 404,
                    retryable: false,
                    reason: RedactedString::new(reason),
                }
            }
            ProductSurfaceFailure::BindingAccessDenied => ProductAdapterError::SurfaceRejected {
                kind: ProductSurfaceRejectionKind::Unauthorized,
                status_code: 403,
                retryable: false,
                reason: RedactedString::new("binding access denied"),
            },
            ProductSurfaceFailure::InvalidBindingRequest { reason } => {
                ProductAdapterError::SurfaceRejected {
                    kind: ProductSurfaceRejectionKind::InvalidRequest,
                    status_code: 400,
                    retryable: false,
                    reason: RedactedString::new(reason),
                }
            }
            ProductSurfaceFailure::ProviderInstanceNotConfigured { reason } => {
                ProductAdapterError::SurfaceRejected {
                    kind: ProductSurfaceRejectionKind::InvalidRequest,
                    status_code: 400,
                    retryable: false,
                    reason: RedactedString::new(reason),
                }
            }
            ProductSurfaceFailure::TurnSubmissionRejected { reason } => {
                ProductAdapterError::Internal {
                    detail: RedactedString::new(reason),
                }
            }
            ProductSurfaceFailure::TurnSubmissionFailed { error } => {
                let status_code = error.adapter_status_code();
                ProductAdapterError::SurfaceRejected {
                    kind: surface_rejection_kind(error.category()),
                    status_code,
                    retryable: matches!(status_code, 429 | 503),
                    reason: RedactedString::new(error.to_string()),
                }
            }
            ProductSurfaceFailure::TurnResumeRejected { reason } => ProductAdapterError::Internal {
                detail: RedactedString::new(reason),
            },
            ProductSurfaceFailure::AuthContinuationRejected { kind } => {
                ProductAdapterError::SurfaceRejected {
                    kind: ProductSurfaceRejectionKind::InvalidRequest,
                    status_code: 400,
                    retryable: false,
                    reason: RedactedString::new(kind.sanitized_reason()),
                }
            }
            ProductSurfaceFailure::ApprovalInteractionRejected { kind } => {
                ProductAdapterError::SurfaceRejected {
                    kind: kind.surface_rejection_kind(),
                    status_code: kind.status_code(),
                    retryable: kind.retryable(),
                    reason: RedactedString::new(kind.sanitized_reason()),
                }
            }
            ProductSurfaceFailure::AuthInteractionRejected { kind } => {
                ProductAdapterError::SurfaceRejected {
                    kind: kind.surface_rejection_kind(),
                    status_code: kind.status_code(),
                    retryable: kind.retryable(),
                    reason: RedactedString::new(kind.sanitized_reason()),
                }
            }
            ProductSurfaceFailure::TurnResumeDenied { error } => {
                let status_code = error.adapter_status_code();
                ProductAdapterError::SurfaceRejected {
                    kind: surface_rejection_kind(error.category()),
                    status_code,
                    retryable: matches!(status_code, 429 | 503),
                    reason: RedactedString::new(error.to_string()),
                }
            }
            ProductSurfaceFailure::Transient { reason } => ProductAdapterError::SurfaceTransient {
                reason: RedactedString::new(reason),
            },
            ProductSurfaceFailure::BeforeInboundPolicyFailed { reason, permanent } => {
                // Adapter error surfaces wrap the reason in RedactedString, so
                // diagnostics remain available internally without leaking to
                // public protocol output.
                if permanent {
                    ProductAdapterError::SurfaceRejected {
                        kind: ProductSurfaceRejectionKind::AdmissionRejected,
                        status_code: 403,
                        retryable: false,
                        reason: RedactedString::new(reason),
                    }
                } else {
                    ProductAdapterError::SurfaceTransient {
                        reason: RedactedString::new(reason),
                    }
                }
            }
            ProductSurfaceFailure::InboundAttachmentFailed { reason, retryable } => {
                if retryable {
                    ProductAdapterError::SurfaceTransient {
                        reason: RedactedString::new(reason),
                    }
                } else {
                    ProductAdapterError::SurfaceRejected {
                        kind: ProductSurfaceRejectionKind::InvalidRequest,
                        status_code: 400,
                        retryable: false,
                        reason: RedactedString::new(reason),
                    }
                }
            }
            ProductSurfaceFailure::DuplicateAction { .. } => ProductAdapterError::Internal {
                detail: RedactedString::new("duplicate action escaped workflow layer"),
            },
            ProductSurfaceFailure::UnsupportedActionKind { kind } => {
                ProductAdapterError::Internal {
                    detail: RedactedString::new(format!("unsupported action kind: {kind}")),
                }
            }
            ProductSurfaceFailure::OutboundTargetNotDirectMessage => {
                ProductAdapterError::SurfaceRejected {
                    kind: ProductSurfaceRejectionKind::Unauthorized,
                    status_code: 403,
                    retryable: false,
                    reason: RedactedString::new(
                        "outbound target is not a direct message but the payload requires one",
                    ),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The absorption is total and payload-preserving: a port failure that
    /// reaches product through `?` must arrive as the same discriminant with
    /// the same text, or an extension-host diagnostic silently changes meaning
    /// on the way into the workflow.
    #[test]
    fn every_operation_failure_absorbs_into_its_matching_workflow_variant() {
        let cases = [
            (
                ProductOperationFailure::BindingResolutionFailed {
                    reason: "no tenant".into(),
                },
                ProductSurfaceFailure::BindingResolutionFailed {
                    reason: "no tenant".into(),
                },
            ),
            (
                ProductOperationFailure::BindingAccessDenied,
                ProductSurfaceFailure::BindingAccessDenied,
            ),
            (
                ProductOperationFailure::InvalidBindingRequest {
                    reason: "bad ref".into(),
                },
                ProductSurfaceFailure::InvalidBindingRequest {
                    reason: "bad ref".into(),
                },
            ),
            (
                ProductOperationFailure::ProviderInstanceNotConfigured {
                    reason: "ironclaw config set google.client_id <id>".into(),
                },
                ProductSurfaceFailure::ProviderInstanceNotConfigured {
                    reason: "ironclaw config set google.client_id <id>".into(),
                },
            ),
            (
                ProductOperationFailure::UnsupportedActionKind {
                    kind: "unknown".into(),
                },
                ProductSurfaceFailure::UnsupportedActionKind {
                    kind: "unknown".into(),
                },
            ),
            (
                ProductOperationFailure::Transient {
                    reason: "db timeout".into(),
                },
                ProductSurfaceFailure::Transient {
                    reason: "db timeout".into(),
                },
            ),
        ];

        for (boundary, expected) in cases {
            let absorbed: ProductSurfaceFailure = boundary.clone().into();
            assert_eq!(absorbed, expected, "absorbing {boundary:?}");
        }
    }

    /// Product's lifecycle projection and the contract's own projection are the
    /// same table for the six shared discriminants. Without this, a status
    /// change made in one crate would silently apply to only half the callers —
    /// the WebUI would answer 400 through product's lifecycle service and 503
    /// through the extension host's, for the identical failure.
    #[test]
    fn lifecycle_projection_agrees_with_the_contract_projection_on_shared_variants() {
        for boundary in [
            ProductOperationFailure::BindingResolutionFailed {
                reason: "no tenant".into(),
            },
            ProductOperationFailure::BindingAccessDenied,
            ProductOperationFailure::InvalidBindingRequest {
                reason: "bad ref".into(),
            },
            ProductOperationFailure::ProviderInstanceNotConfigured {
                reason: "run ironclaw config set".into(),
            },
            ProductOperationFailure::UnsupportedActionKind {
                kind: "unknown".into(),
            },
            ProductOperationFailure::Transient {
                reason: "db timeout".into(),
            },
        ] {
            let through_contract: ProductSurfaceError = boundary.clone().into();
            let through_product =
                lifecycle_product_surface_error(ProductSurfaceFailure::from(boundary.clone()));
            assert_eq!(
                through_product, through_contract,
                "projections disagree for {boundary:?}"
            );
        }
    }

    #[test]
    fn transient_maps_to_retryable() {
        let err: ProductAdapterError = ProductSurfaceFailure::Transient {
            reason: "db timeout".into(),
        }
        .into();
        assert!(err.is_retryable());
    }

    #[test]
    fn binding_failure_maps_to_internal() {
        let err: ProductAdapterError = ProductSurfaceFailure::BindingResolutionFailed {
            reason: "no tenant".into(),
        }
        .into();
        assert!(!err.is_retryable());
    }

    #[test]
    fn permanent_before_inbound_policy_failure_maps_to_rejection() {
        let err: ProductAdapterError = ProductSurfaceFailure::BeforeInboundPolicyFailed {
            reason: "classifier misconfigured".into(),
            permanent: true,
        }
        .into();
        assert!(!err.is_retryable());
        assert!(matches!(err, ProductAdapterError::SurfaceRejected { .. }));
    }

    #[test]
    fn attachment_failures_preserve_retryability_without_exposing_provider_detail() {
        let retryable: ProductAdapterError = ProductSurfaceFailure::InboundAttachmentFailed {
            reason: "channel attachment transfer failed".into(),
            retryable: true,
        }
        .into();
        assert!(retryable.is_retryable());

        let permanent: ProductAdapterError = ProductSurfaceFailure::InboundAttachmentFailed {
            reason: "attachment exceeds the per-file byte limit".into(),
            retryable: false,
        }
        .into();
        assert!(!permanent.is_retryable());
        assert!(matches!(
            permanent,
            ProductAdapterError::SurfaceRejected {
                kind: ProductSurfaceRejectionKind::InvalidRequest,
                status_code: 400,
                retryable: false,
                ..
            }
        ));
    }

    #[test]
    fn turn_resume_denied_maps_to_workflow_rejected() {
        for (error, expected_kind, expected_status, expected_retryable) in [
            (
                TurnError::Unauthorized,
                ProductSurfaceRejectionKind::Unauthorized,
                403,
                false,
            ),
            (
                TurnError::ScopeNotFound,
                ProductSurfaceRejectionKind::ScopeNotFound,
                404,
                false,
            ),
            (
                TurnError::Unavailable {
                    reason: "turn store offline".to_string(),
                },
                ProductSurfaceRejectionKind::Unavailable,
                503,
                true,
            ),
            (
                TurnError::capacity_exceeded(
                    ironclaw_turns::TurnCapacityResource::SpawnTreeDescendants,
                    3,
                ),
                ProductSurfaceRejectionKind::AdmissionRejected,
                429,
                true,
            ),
        ] {
            let err: ProductAdapterError = ProductSurfaceFailure::TurnResumeDenied { error }.into();

            match err {
                ProductAdapterError::SurfaceRejected {
                    kind,
                    status_code,
                    retryable,
                    ..
                } => {
                    assert_eq!(kind, expected_kind);
                    assert_eq!(status_code, expected_status);
                    assert_eq!(retryable, expected_retryable);
                }
                other => panic!("expected typed workflow rejection, got {other:?}"),
            }
        }
    }

    #[test]
    fn provider_instance_not_configured_maps_to_workflow_rejected() {
        let reason =
            "ironclaw config set google.client_id <id>.apps.googleusercontent.com".to_string();
        let err: ProductAdapterError = ProductSurfaceFailure::ProviderInstanceNotConfigured {
            reason: reason.clone(),
        }
        .into();
        match err {
            ProductAdapterError::SurfaceRejected {
                kind,
                status_code,
                retryable,
                reason: mapped_reason,
            } => {
                assert_eq!(kind, ProductSurfaceRejectionKind::InvalidRequest);
                assert_eq!(status_code, 400);
                assert!(!retryable);
                assert_eq!(mapped_reason, RedactedString::new(reason));
            }
            other => panic!("expected typed workflow rejection, got {other:?}"),
        }
    }

    #[test]
    fn outbound_target_not_direct_message_maps_to_workflow_rejected() {
        let err: ProductAdapterError = ProductSurfaceFailure::OutboundTargetNotDirectMessage.into();
        match err {
            ProductAdapterError::SurfaceRejected {
                kind,
                status_code,
                retryable,
                ..
            } => {
                assert_eq!(kind, ProductSurfaceRejectionKind::Unauthorized);
                assert_eq!(status_code, 403);
                assert!(!retryable);
            }
            other => panic!("expected typed workflow rejection, got {other:?}"),
        }
    }
}
