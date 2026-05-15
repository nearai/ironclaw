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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("Reborn WebUI service error: {code:?}")]
pub struct RebornServicesError {
    pub code: RebornServicesErrorCode,
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
        Self {
            code,
            status_code,
            retryable,
            field: None,
            validation_code: None,
        }
    }

    pub(super) fn internal_invariant() -> Self {
        Self::from_status(RebornServicesErrorCode::Internal, 500, false)
    }

    pub(super) fn service_unavailable(retryable: bool) -> Self {
        Self::from_status(RebornServicesErrorCode::Unavailable, 503, retryable)
    }
}

impl From<WebUiInboundValidationError> for RebornServicesError {
    fn from(value: WebUiInboundValidationError) -> Self {
        Self::validation(value)
    }
}
