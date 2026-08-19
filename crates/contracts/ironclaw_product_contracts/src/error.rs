//! The error vocabulary of the product-side ports.
//!
//! [`ProductOperationFailure`] is what a crate *below or beside* product may
//! say when a product-side port it implements fails. It is deliberately the
//! smaller of the two product error types:
//!
//! - **`ProductOperationFailure` (here)** — the boundary vocabulary. Every
//!   variant carries a plain `String` or nothing at all, so the type names no
//!   kernel, domain, or workflow type and satisfies this crate's dependency
//!   ceiling (`ironclaw_host_api` + `ironclaw_extension_contracts`, §6.1.3).
//!   A port implementor never needs more than this to describe its own
//!   failure.
//! - **`ironclaw_assistant::ProductSurfaceFailure`** — the workflow vocabulary.
//!   It is a strict superset: it adds the turn-coordinator, interaction, and
//!   idempotency variants whose payloads are kernel types
//!   (`ironclaw_turns::TurnError`) or product-internal rejection kinds, which
//!   only the workflow crate produces and only the workflow crate reads.
//!   `ironclaw_assistant` absorbs this type into that one with a total,
//!   information-preserving `From`, so `?` at a product call site keeps
//!   working unchanged.
//!
//! The split is where the two vocabularies actually diverge, measured rather
//! than chosen: `ironclaw_extension_host` constructs exactly the six variants
//! below across its 19 production files, and none of the kernel-typed ones.
//! See `docs/internal/reborn/target-architecture/PROPOSAL.md` §6.1.3 for the recorded
//! decision and the alternatives it beat.

use ironclaw_host_api::error::HostApiError;
use thiserror::Error;

use crate::surface::{ProductSurfaceError, ProductSurfaceErrorCode};

/// A product-side port implementation failed.
///
/// Constructed by whichever crate implements the port — the extension host,
/// the extension manager, the operator surface, or composition — and either
/// projected to [`ProductSurfaceError`] for a caller across the membrane, or
/// absorbed into `ironclaw_assistant::ProductSurfaceFailure` when product itself
/// is the caller.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProductOperationFailure {
    /// The conversation binding could not be resolved for the given external
    /// refs.
    #[error("binding resolution failed: {reason}")]
    BindingResolutionFailed { reason: String },

    /// The external actor has no trusted binding to a canonical user.
    ///
    /// **Distinct from [`Self::BindingResolutionFailed`], and the difference is
    /// user-visible.** `BindingResolutionFailed` is an internal invariant (500);
    /// this one is the fail-closed "you are not paired yet" answer the workflow
    /// turns into a `ProductRejectionKind::BindingRequired` acknowledgement with
    /// its own onboarding hint. Collapsing the two would change what an unpaired
    /// external actor is told.
    #[error("binding required: {reason}")]
    BindingRequired { reason: String },

    /// The actor or route is not allowed to use the resolved thread.
    #[error("binding access denied")]
    BindingAccessDenied,

    /// The adapter installation is not mapped to a tenant.
    #[error("unknown adapter installation")]
    UnknownInstallation,

    /// The request is invalid and should not be retried unchanged.
    #[error("invalid binding request: {reason}")]
    InvalidBindingRequest { reason: String },

    /// A provider's OPERATOR-level instance configuration (e.g. no OAuth
    /// backend registered on this build at all) is missing entirely — a
    /// static, build-time fact distinct from a per-user missing or expired
    /// credential account. `reason` is plain, host-authored text carrying the
    /// exact remediation (e.g. the `ironclaw config set` commands to run).
    ///
    /// Two independent consumers read this variant differently, and both
    /// behaviours are load-bearing: the WebUI projection discards `reason` and
    /// maps the DISCRIMINANT alone to a sanitized 400 (no free text crosses
    /// the wire contract), while the LLM tool path forwards `reason` verbatim
    /// onto the diagnostic-detail channel so the exact `config set` commands
    /// reach the model.
    #[error("provider instance not configured: {reason}")]
    ProviderInstanceNotConfigured { reason: String },

    /// The requested action kind is not supported by this port implementation.
    #[error("unsupported action kind: {kind}")]
    UnsupportedActionKind { kind: String },

    /// The turn coordinator rejected a submission before typed turn errors
    /// were available, carrying only the rendered cause.
    ///
    /// The untyped half of `ironclaw_assistant::ProductSurfaceFailure`'s
    /// submission pair: the typed half (`TurnSubmissionFailed { error:
    /// TurnError }`) is minted only by product's own turn path and stays
    /// there, because `TurnError` is a kernel type this crate's ceiling
    /// forbids. A port implementor reaches this variant when the conversation
    /// binding store surfaces a submission rejection it can only render.
    #[error("turn submission rejected: {reason}")]
    TurnSubmissionRejected { reason: String },

    /// A transient store or service failure.
    #[error("transient workflow failure: {reason}")]
    Transient { reason: String },
}

impl From<HostApiError> for ProductOperationFailure {
    fn from(error: HostApiError) -> Self {
        ProductOperationFailure::InvalidBindingRequest {
            reason: error.to_string(),
        }
    }
}

/// The membrane projection: how a port failure is reported to a caller that
/// only speaks [`ProductSurfaceError`].
///
/// This is the *single* mapping table for these six discriminants —
/// `ironclaw_assistant`'s `lifecycle_product_surface_error` delegates its
/// matching arms here rather than repeating the status choices, so the two
/// paths cannot drift. It is a projection between two types this crate owns,
/// not workflow classification: no reason text crosses (the wire contract has
/// no free-text field) and nothing is logged here — a caller that wants the
/// cause in its logs emits it before projecting, which is why product keeps
/// its `Transient` warning and the extension host keeps its own.
impl From<ProductOperationFailure> for ProductSurfaceError {
    fn from(value: ProductOperationFailure) -> Self {
        match value {
            ProductOperationFailure::InvalidBindingRequest { .. }
            | ProductOperationFailure::UnsupportedActionKind { .. }
            // WebUI gets a plain 400 with no free text: the exact `config set`
            // remediation reaches users via the LLM tool path and
            // `ironclaw status`; bespoke WebUI messaging is a deliberately
            // deferred scope-cut.
            | ProductOperationFailure::ProviderInstanceNotConfigured { .. } => {
                ProductSurfaceError::from_status(ProductSurfaceErrorCode::InvalidRequest, 400, false)
            }
            ProductOperationFailure::BindingAccessDenied => {
                ProductSurfaceError::from_status(ProductSurfaceErrorCode::Forbidden, 403, false)
            }
            ProductOperationFailure::Transient { .. } => {
                ProductSurfaceError::service_unavailable(true)
            }
            // No boundary image: the membrane renders these as an internal
            // invariant and the caller-facing answer is produced upstream —
            // `BindingRequired` and `UnknownInstallation` become typed
            // `ProductRejectionKind` acknowledgements on the inbound path, and
            // a rendered submission rejection is never a client's fault.
            ProductOperationFailure::BindingResolutionFailed { .. }
            | ProductOperationFailure::BindingRequired { .. }
            | ProductOperationFailure::UnknownInstallation
            | ProductOperationFailure::TurnSubmissionRejected { .. } => {
                ProductSurfaceError::internal_invariant()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::surface::ProductSurfaceErrorKind;

    /// Every discriminant is pinned, so a new variant cannot land without a
    /// deliberate status choice and a reviewer seeing it.
    #[test]
    fn every_operation_failure_projects_to_its_frozen_surface_status() {
        let cases = [
            (
                ProductOperationFailure::InvalidBindingRequest {
                    reason: "bad ref".into(),
                },
                ProductSurfaceErrorCode::InvalidRequest,
                400,
                false,
            ),
            (
                ProductOperationFailure::UnsupportedActionKind {
                    kind: "unknown".into(),
                },
                ProductSurfaceErrorCode::InvalidRequest,
                400,
                false,
            ),
            (
                ProductOperationFailure::ProviderInstanceNotConfigured {
                    reason: "ironclaw config set google.client_id <id>".into(),
                },
                ProductSurfaceErrorCode::InvalidRequest,
                400,
                false,
            ),
            (
                ProductOperationFailure::BindingAccessDenied,
                ProductSurfaceErrorCode::Forbidden,
                403,
                false,
            ),
            (
                ProductOperationFailure::Transient {
                    reason: "db timeout".into(),
                },
                ProductSurfaceErrorCode::Unavailable,
                503,
                true,
            ),
            (
                ProductOperationFailure::BindingResolutionFailed {
                    reason: "no tenant".into(),
                },
                ProductSurfaceErrorCode::Internal,
                500,
                false,
            ),
            (
                ProductOperationFailure::BindingRequired {
                    reason: "actor is not paired".into(),
                },
                ProductSurfaceErrorCode::Internal,
                500,
                false,
            ),
            (
                ProductOperationFailure::UnknownInstallation,
                ProductSurfaceErrorCode::Internal,
                500,
                false,
            ),
            (
                ProductOperationFailure::TurnSubmissionRejected {
                    reason: "thread busy".into(),
                },
                ProductSurfaceErrorCode::Internal,
                500,
                false,
            ),
        ];

        for (failure, expected_code, expected_status, expected_retryable) in cases {
            let projected: ProductSurfaceError = failure.clone().into();
            assert_eq!(projected.code, expected_code, "code for {failure:?}");
            assert_eq!(
                projected.status_code, expected_status,
                "status for {failure:?}"
            );
            assert_eq!(
                projected.retryable, expected_retryable,
                "retryable for {failure:?}"
            );
        }
    }

    /// The wire contract has no free-text field; the projection must not
    /// invent one, or the remediation string in
    /// `ProviderInstanceNotConfigured` would start crossing to the browser.
    #[test]
    fn projection_carries_no_reason_text_across_the_membrane() {
        let projected: ProductSurfaceError =
            ProductOperationFailure::ProviderInstanceNotConfigured {
                reason: "ironclaw config set google.client_secret <secret>".into(),
            }
            .into();
        assert_eq!(projected.field, None);
        assert_eq!(projected.validation_code, None);
        assert_eq!(projected.kind, ProductSurfaceErrorKind::Validation);
    }

    /// `Display` is not decoration here: the LLM tool path forwards the
    /// rendered failure onto the diagnostic-detail channel, so a variant that
    /// swallowed its `reason` would silently drop the remediation text a user
    /// needs (the exact `config set` commands). Every variant is rendered, and
    /// every one that carries text is asserted to include it.
    #[test]
    fn display_renders_every_variant_and_keeps_the_text_the_tool_path_forwards() {
        let cases = [
            (
                ProductOperationFailure::BindingResolutionFailed {
                    reason: "no tenant".into(),
                },
                Some("no tenant"),
            ),
            (
                ProductOperationFailure::BindingRequired {
                    reason: "actor is not paired".into(),
                },
                Some("actor is not paired"),
            ),
            (ProductOperationFailure::BindingAccessDenied, None),
            (ProductOperationFailure::UnknownInstallation, None),
            (
                ProductOperationFailure::TurnSubmissionRejected {
                    reason: "thread busy".into(),
                },
                Some("thread busy"),
            ),
            (
                ProductOperationFailure::InvalidBindingRequest {
                    reason: "bad ref".into(),
                },
                Some("bad ref"),
            ),
            (
                ProductOperationFailure::ProviderInstanceNotConfigured {
                    reason: "ironclaw config set google.client_id <id>".into(),
                },
                Some("ironclaw config set google.client_id <id>"),
            ),
            (
                ProductOperationFailure::UnsupportedActionKind {
                    kind: "teleport".into(),
                },
                Some("teleport"),
            ),
            (
                ProductOperationFailure::Transient {
                    reason: "db timeout".into(),
                },
                Some("db timeout"),
            ),
        ];

        for (failure, expected_fragment) in cases {
            let rendered = failure.to_string();
            assert!(!rendered.is_empty(), "every variant renders: {failure:?}");
            if let Some(fragment) = expected_fragment {
                assert!(
                    rendered.contains(fragment),
                    "{failure:?} rendered as {rendered:?}, dropping {fragment:?}"
                );
            }
        }
    }

    /// Discriminant name of a `HostApiError`, by an exhaustive `match`.
    ///
    /// The exhaustiveness is the point: a new `HostApiError` variant stops
    /// compiling here, which forces whoever adds it to decide how it reaches a
    /// caller instead of inheriting the blanket mapping silently.
    fn host_api_error_tag(error: &HostApiError) -> &'static str {
        match error {
            HostApiError::InvalidId { .. } => "InvalidId",
            HostApiError::InvalidPath { .. } => "InvalidPath",
            HostApiError::InvalidCapability { .. } => "InvalidCapability",
            HostApiError::InvalidMount { .. } => "InvalidMount",
            HostApiError::InvalidNetworkTarget { .. } => "InvalidNetworkTarget",
            HostApiError::InvalidRuntimeCredentialTarget { .. } => "InvalidRuntimeCredentialTarget",
            HostApiError::InvalidSafeSummary { .. } => "InvalidSafeSummary",
            HostApiError::InvalidModelDiagnostic { .. } => "InvalidModelDiagnostic",
            HostApiError::InvalidHostRemediation { .. } => "InvalidHostRemediation",
            HostApiError::InvalidOutputContract { .. } => "InvalidOutputContract",
            HostApiError::InvariantViolation { .. } => "InvariantViolation",
        }
    }

    /// `HostApiError` is validation feedback, so it lands on the caller's side
    /// of the boundary rather than reading as a service outage.
    ///
    /// **Every variant is enumerated**, `InvariantViolation` included. That one
    /// is an internal protocol mismatch rather than caller input, so its 400
    /// arguably overstates client responsibility — but the classification is
    /// not this slice's: `ironclaw_assistant`'s `From<HostApiError> for
    /// ProductSurfaceFailure` has always been the same blanket mapping, and the
    /// impl under test mirrors it so the two absorption paths cannot disagree.
    /// It is pinned here rather than changed so that reclassifying it is a
    /// deliberate edit with a failing test, not silent drift. (It is also
    /// unreachable on this path today: the only construction outside
    /// `ironclaw_host_api` itself is in a gsuite handler, which does not reach
    /// a product-side port.)
    #[test]
    fn host_api_errors_become_invalid_requests_not_transient_failures() {
        let cases = [
            HostApiError::InvalidId {
                kind: "extension",
                value: String::new(),
                reason: "blank".to_string(),
            },
            HostApiError::InvalidPath {
                value: "../escape".to_string(),
                reason: "dot-dot".to_string(),
            },
            HostApiError::InvalidCapability {
                value: "teleport".to_string(),
                reason: "unknown".to_string(),
            },
            HostApiError::InvalidMount {
                value: "/nowhere".to_string(),
                reason: "unmounted".to_string(),
            },
            HostApiError::InvalidNetworkTarget {
                value: "10.0.0.1".to_string(),
                reason: "private range".to_string(),
            },
            HostApiError::InvalidRuntimeCredentialTarget {
                value: "google".to_string(),
                reason: "not declared".to_string(),
            },
            HostApiError::InvalidSafeSummary {
                reason: "too long".to_string(),
            },
            HostApiError::InvalidModelDiagnostic {
                reason: "empty".to_string(),
            },
            HostApiError::InvalidHostRemediation {
                reason: "empty".to_string(),
            },
            HostApiError::InvalidOutputContract {
                reason: "schema name is invalid".to_string(),
            },
            HostApiError::InvariantViolation {
                reason: "port answered for an unrequested id".to_string(),
            },
        ];

        let covered: BTreeSet<&'static str> = cases.iter().map(host_api_error_tag).collect();
        assert_eq!(
            covered.len(),
            cases.len(),
            "each variant is enumerated exactly once: {covered:?}"
        );
        assert_eq!(
            covered.len(),
            11,
            "a HostApiError variant was added without a case here: {covered:?}"
        );

        for error in cases {
            let tag = host_api_error_tag(&error);
            let rendered = error.to_string();
            let failure: ProductOperationFailure = error.into();
            let ProductOperationFailure::InvalidBindingRequest { reason } = &failure else {
                panic!("{tag} projected to {failure:?}, not InvalidBindingRequest");
            };
            assert_eq!(
                reason, &rendered,
                "{tag} must carry its own rendering, or the cause is lost at the boundary"
            );

            // The consequence a caller actually sees. Pinned alongside the
            // discriminant so a status change cannot slip through by editing
            // only the projection table above.
            let projected: ProductSurfaceError = failure.into();
            assert_eq!(
                projected.code,
                ProductSurfaceErrorCode::InvalidRequest,
                "{tag}"
            );
            assert_eq!(projected.status_code, 400, "{tag}");
            assert!(!projected.retryable, "{tag}");
        }
    }
}
