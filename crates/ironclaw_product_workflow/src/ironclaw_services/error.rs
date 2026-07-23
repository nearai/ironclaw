use serde::{Deserialize, Serialize};

use crate::{WebUiInboundValidationCode, WebUiInboundValidationError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IronClawServicesErrorCode {
    InvalidRequest,
    Unauthenticated,
    Forbidden,
    NotFound,
    Conflict,
    RateLimited,
    Unavailable,
    Internal,
}

/// Stable user-facing error family for WebUI rendering.
///
/// Keep this vocabulary independent from backend error type names. The HTTP-ish
/// [`IronClawServicesErrorCode`] and `status_code` fields describe transport
/// shape; this kind describes what M1 may render without parsing backend text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IronClawServicesErrorKind {
    Validation,
    Duplicate,
    Busy,
    ParticipantDenied,
    BlockedApproval,
    BlockedAuthentication,
    BlockedResource,
    ReplayUnavailable,
    TimelineUnavailable,
    ServiceUnavailable,
    NotFound,
    Conflict,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("IronClaw WebUI service error: {code:?}")]
pub struct IronClawServicesError {
    pub code: IronClawServicesErrorCode,
    pub kind: IronClawServicesErrorKind,
    pub status_code: u16,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_code: Option<WebUiInboundValidationCode>,
}

impl IronClawServicesError {
    pub(super) fn validation(error: WebUiInboundValidationError) -> Self {
        Self {
            code: IronClawServicesErrorCode::InvalidRequest,
            kind: IronClawServicesErrorKind::Validation,
            status_code: 400,
            retryable: false,
            field: Some(error.field),
            validation_code: Some(error.code),
        }
    }

    pub(super) fn from_status(
        code: IronClawServicesErrorCode,
        status_code: u16,
        retryable: bool,
    ) -> Self {
        Self::from_status_kind(code, default_kind_for_code(code), status_code, retryable)
    }

    pub(super) fn from_status_kind(
        code: IronClawServicesErrorCode,
        kind: IronClawServicesErrorKind,
        status_code: u16,
        retryable: bool,
    ) -> Self {
        Self {
            code,
            kind,
            status_code,
            retryable,
            field: None,
            validation_code: None,
        }
    }

    pub(super) fn internal_invariant() -> Self {
        Self::from_status(IronClawServicesErrorCode::Internal, 500, false)
    }

    /// Sanitized internal (500) error for host-composition adapters that
    /// implement workflow ports from outside this crate. The struct fields are
    /// public, but new construction sites should route through one constructor
    /// rather than hand-rolling the status/kind pairing.
    pub fn internal() -> Self {
        Self::from_status(IronClawServicesErrorCode::Internal, 500, false)
    }

    /// Sanitized not-found (404) error for host-composition adapters: the
    /// requested entity/identifier is not known to any mounted facade. Client
    /// input, not a host fault — never map this shape to `internal()`.
    pub fn not_found() -> Self {
        Self::from_status(IronClawServicesErrorCode::NotFound, 404, false)
    }

    /// Build a sanitized internal (500) error from a backend cause, logging the
    /// cause so a 500 can never leave a boundary silently.
    ///
    /// Use this anywhere a backend error is converted into the user-facing
    /// surface and the cause would otherwise be dropped — i.e. instead of the
    /// `.map_err(|_| IronClawServicesError::…internal…)` footgun, which discards
    /// the error binding and produces an undiagnosable 500. The wire response
    /// stays sanitized (the cause is logged, never serialized to the client);
    /// the gateway boundary additionally logs the HTTP fact for every 5xx.
    pub fn internal_from(source: impl std::fmt::Display) -> Self {
        tracing::error!(error = %source, "internal service error");
        Self::from_status(IronClawServicesErrorCode::Internal, 500, false)
    }

    pub(super) fn service_unavailable(retryable: bool) -> Self {
        Self::from_status_kind(
            IronClawServicesErrorCode::Unavailable,
            IronClawServicesErrorKind::ServiceUnavailable,
            503,
            retryable,
        )
    }
}

impl From<WebUiInboundValidationError> for IronClawServicesError {
    fn from(value: WebUiInboundValidationError) -> Self {
        Self::validation(value)
    }
}

fn default_kind_for_code(code: IronClawServicesErrorCode) -> IronClawServicesErrorKind {
    match code {
        IronClawServicesErrorCode::InvalidRequest => IronClawServicesErrorKind::Validation,
        IronClawServicesErrorCode::Unauthenticated | IronClawServicesErrorCode::Forbidden => {
            IronClawServicesErrorKind::ParticipantDenied
        }
        IronClawServicesErrorCode::NotFound => IronClawServicesErrorKind::NotFound,
        IronClawServicesErrorCode::Conflict => IronClawServicesErrorKind::Conflict,
        IronClawServicesErrorCode::RateLimited => IronClawServicesErrorKind::Busy,
        IronClawServicesErrorCode::Unavailable => IronClawServicesErrorKind::ServiceUnavailable,
        IronClawServicesErrorCode::Internal => IronClawServicesErrorKind::Internal,
    }
}
