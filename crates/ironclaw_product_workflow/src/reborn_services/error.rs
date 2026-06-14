use serde::{Deserialize, Serialize};

use crate::{WebUiInboundValidationCode, WebUiInboundValidationError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebornServicesErrorCode {
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
/// [`RebornServicesErrorCode`] and `status_code` fields describe transport
/// shape; this kind describes what M1 may render without parsing backend text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebornServicesErrorKind {
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
#[error("Reborn WebUI service error: {code:?}")]
pub struct RebornServicesError {
    pub code: RebornServicesErrorCode,
    pub kind: RebornServicesErrorKind,
    pub status_code: u16,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_code: Option<WebUiInboundValidationCode>,
}

impl RebornServicesError {
    pub(super) fn validation(error: WebUiInboundValidationError) -> Self {
        Self {
            code: RebornServicesErrorCode::InvalidRequest,
            kind: RebornServicesErrorKind::Validation,
            status_code: 400,
            retryable: false,
            field: Some(error.field),
            validation_code: Some(error.code),
        }
    }

    pub(super) fn from_status(
        code: RebornServicesErrorCode,
        status_code: u16,
        retryable: bool,
    ) -> Self {
        Self::from_status_kind(code, default_kind_for_code(code), status_code, retryable)
    }

    pub(super) fn from_status_kind(
        code: RebornServicesErrorCode,
        kind: RebornServicesErrorKind,
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
        Self::from_status(RebornServicesErrorCode::Internal, 500, false)
    }

    /// Sanitized internal (500) error for host-composition adapters that
    /// implement workflow ports from outside this crate. The struct fields are
    /// public, but new construction sites should route through one constructor
    /// rather than hand-rolling the status/kind pairing.
    pub fn internal() -> Self {
        Self::from_status(RebornServicesErrorCode::Internal, 500, false)
    }

    pub(super) fn service_unavailable(retryable: bool) -> Self {
        Self::from_status_kind(
            RebornServicesErrorCode::Unavailable,
            RebornServicesErrorKind::ServiceUnavailable,
            503,
            retryable,
        )
    }
}

impl From<WebUiInboundValidationError> for RebornServicesError {
    fn from(value: WebUiInboundValidationError) -> Self {
        Self::validation(value)
    }
}

fn default_kind_for_code(code: RebornServicesErrorCode) -> RebornServicesErrorKind {
    match code {
        RebornServicesErrorCode::InvalidRequest => RebornServicesErrorKind::Validation,
        RebornServicesErrorCode::Unauthenticated | RebornServicesErrorCode::Forbidden => {
            RebornServicesErrorKind::ParticipantDenied
        }
        RebornServicesErrorCode::NotFound => RebornServicesErrorKind::NotFound,
        RebornServicesErrorCode::Conflict => RebornServicesErrorKind::Conflict,
        RebornServicesErrorCode::RateLimited => RebornServicesErrorKind::Busy,
        RebornServicesErrorCode::Unavailable => RebornServicesErrorKind::ServiceUnavailable,
        RebornServicesErrorCode::Internal => RebornServicesErrorKind::Internal,
    }
}
