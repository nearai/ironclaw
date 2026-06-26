use ironclaw_product_adapters::{ProductAdapterError, ProductWorkflowRejectionKind};
#[cfg(feature = "openai-compat-beta")]
use ironclaw_product_adapters::{ProductRejection, ProductRejectionKind};
use serde::{Deserialize, Serialize};

use crate::OpenAiCompatRefError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenAiCompatErrorKind {
    Validation,
    Authentication,
    PermissionDenied,
    NotFound,
    Conflict,
    RateLimited,
    ServiceUnavailable,
    Unsupported,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenAiCompatErrorType {
    InvalidRequestError,
    AuthenticationError,
    PermissionError,
    NotFoundError,
    ConflictError,
    RateLimitError,
    ServerError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenAiCompatErrorCode {
    InvalidRequest,
    AuthenticationRequired,
    PermissionDenied,
    NotFound,
    Conflict,
    RateLimited,
    ServiceUnavailable,
    Unsupported,
    InternalError,
}

impl OpenAiCompatErrorCode {
    pub fn sanitized_message(self) -> &'static str {
        match self {
            Self::InvalidRequest => "The request is invalid.",
            Self::AuthenticationRequired => "Authentication is required.",
            Self::PermissionDenied => "The caller is not allowed to access this resource.",
            Self::NotFound => "The requested resource was not found.",
            Self::Conflict => "The request conflicts with the current resource state.",
            Self::RateLimited => "The request is temporarily rate limited.",
            Self::ServiceUnavailable => "The service is temporarily unavailable.",
            Self::Unsupported => "This OpenAI-compatible Reborn route is not wired yet.",
            Self::InternalError => "An internal error occurred.",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenAiCompatErrorResponse {
    pub error: OpenAiCompatError,
}

impl OpenAiCompatErrorResponse {
    pub fn new(error: OpenAiCompatError) -> Self {
        Self { error }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenAiCompatError {
    message: String,
    #[serde(rename = "type")]
    error_type: OpenAiCompatErrorType,
    param: Option<String>,
    code: Option<OpenAiCompatErrorCode>,
}

impl OpenAiCompatError {
    pub fn from_kind(kind: OpenAiCompatErrorKind, param: Option<String>) -> Self {
        let spec = ErrorSpec::for_kind(kind);
        Self {
            message: spec.message.to_string(),
            error_type: spec.error_type,
            param: clean_param(param),
            code: Some(spec.code),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn error_type(&self) -> OpenAiCompatErrorType {
        self.error_type
    }

    pub fn param(&self) -> Option<&str> {
        self.param.as_deref()
    }

    pub fn code(&self) -> Option<OpenAiCompatErrorCode> {
        self.code
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiCompatHttpError {
    status_code: u16,
    retryable: bool,
    body: OpenAiCompatErrorResponse,
}

impl OpenAiCompatHttpError {
    pub fn from_kind(
        status_code: u16,
        retryable: bool,
        kind: OpenAiCompatErrorKind,
        param: Option<String>,
    ) -> Self {
        Self {
            status_code: sanitize_status_code(status_code),
            retryable,
            body: OpenAiCompatErrorResponse::new(OpenAiCompatError::from_kind(kind, param)),
        }
    }

    pub fn invalid_request(param: Option<String>) -> Self {
        Self::from_kind(400, false, OpenAiCompatErrorKind::Validation, param)
    }

    pub fn not_found(param: Option<String>) -> Self {
        Self::from_kind(404, false, OpenAiCompatErrorKind::NotFound, param)
    }

    pub fn conflict(param: Option<String>) -> Self {
        Self::from_kind(409, false, OpenAiCompatErrorKind::Conflict, param)
    }

    pub fn not_wired() -> Self {
        Self::from_kind(501, false, OpenAiCompatErrorKind::Unsupported, None)
    }

    pub fn from_workflow_rejection(
        kind: ProductWorkflowRejectionKind,
        status_code: u16,
        retryable: bool,
        param: Option<String>,
    ) -> Self {
        let error_kind = match kind {
            ProductWorkflowRejectionKind::ThreadBusy
            | ProductWorkflowRejectionKind::AdmissionRejected => OpenAiCompatErrorKind::RateLimited,
            ProductWorkflowRejectionKind::ScopeNotFound => OpenAiCompatErrorKind::NotFound,
            ProductWorkflowRejectionKind::Unauthorized => OpenAiCompatErrorKind::PermissionDenied,
            ProductWorkflowRejectionKind::InvalidRequest => OpenAiCompatErrorKind::Validation,
            ProductWorkflowRejectionKind::Unavailable => OpenAiCompatErrorKind::ServiceUnavailable,
            ProductWorkflowRejectionKind::Conflict | ProductWorkflowRejectionKind::Ambiguous => {
                OpenAiCompatErrorKind::Conflict
            }
        };
        Self::from_kind(status_code, retryable, error_kind, param)
    }

    pub fn from_product_adapter_error(error: ProductAdapterError) -> Self {
        match error {
            ProductAdapterError::InvalidIdentifier { .. }
            | ProductAdapterError::MalformedInboundPayload { .. } => Self::invalid_request(None),
            ProductAdapterError::Authentication(_) => {
                Self::from_kind(401, false, OpenAiCompatErrorKind::Authentication, None)
            }
            ProductAdapterError::WorkflowRejected {
                kind,
                status_code,
                retryable,
                ..
            } => Self::from_workflow_rejection(kind, status_code, retryable, None),
            ProductAdapterError::WorkflowTransient { .. }
            | ProductAdapterError::EgressTransient { .. } => {
                Self::from_kind(503, true, OpenAiCompatErrorKind::ServiceUnavailable, None)
            }
            ProductAdapterError::EgressDenied { .. }
            | ProductAdapterError::EgressUndeclaredHost { .. }
            | ProductAdapterError::Internal { .. } => {
                Self::from_kind(500, false, OpenAiCompatErrorKind::Internal, None)
            }
        }
    }

    pub fn internal() -> Self {
        Self::from_kind(500, false, OpenAiCompatErrorKind::Internal, None)
    }

    pub fn status_code(&self) -> u16 {
        self.status_code
    }

    pub fn retryable(&self) -> bool {
        self.retryable
    }

    pub fn body(&self) -> &OpenAiCompatErrorResponse {
        &self.body
    }
}

impl From<ProductAdapterError> for OpenAiCompatHttpError {
    fn from(error: ProductAdapterError) -> Self {
        Self::from_product_adapter_error(error)
    }
}

impl From<OpenAiCompatRefError> for OpenAiCompatHttpError {
    fn from(error: OpenAiCompatRefError) -> Self {
        match error {
            OpenAiCompatRefError::InvalidIdentifier { .. } => Self::invalid_request(None),
            OpenAiCompatRefError::StoreUnavailable => {
                Self::from_kind(503, true, OpenAiCompatErrorKind::ServiceUnavailable, None)
            }
            OpenAiCompatRefError::CorruptMapping => Self::internal(),
        }
    }
}

/// Translates a [`ProductRejection`] into an [`OpenAiCompatHttpError`].
///
/// `param` carries the surface-specific field name for `BindingRequired` and
/// `InvalidRequest` rejections. Chat passes `Some("messages")`; Responses
/// passes `Some("input")`.
#[cfg(feature = "openai-compat-beta")]
pub(crate) fn product_rejection_to_openai_error(
    rejection: &ProductRejection,
    param: Option<&str>,
) -> OpenAiCompatHttpError {
    match rejection.kind {
        ProductRejectionKind::BindingRequired => {
            OpenAiCompatHttpError::not_found(param.map(str::to_owned))
        }
        ProductRejectionKind::AccessDenied | ProductRejectionKind::PolicyDenied => {
            OpenAiCompatHttpError::from_workflow_rejection(
                ProductWorkflowRejectionKind::Unauthorized,
                403,
                false,
                None,
            )
        }
        ProductRejectionKind::UnknownInstallation => OpenAiCompatHttpError::from_kind(
            503,
            true,
            OpenAiCompatErrorKind::ServiceUnavailable,
            None,
        ),
        ProductRejectionKind::InvalidRequest => {
            OpenAiCompatHttpError::invalid_request(param.map(str::to_owned))
        }
        ProductRejectionKind::AmbiguousResolution => {
            OpenAiCompatHttpError::from_workflow_rejection(
                ProductWorkflowRejectionKind::Ambiguous,
                409,
                false,
                None,
            )
        }
        // The gate already resolved (approved/denied) — the resolution conflicts
        // with the now-settled state, so surface a 409 Conflict.
        ProductRejectionKind::StaleGate => OpenAiCompatHttpError::from_workflow_rejection(
            ProductWorkflowRejectionKind::Conflict,
            409,
            false,
            None,
        ),
    }
}

#[cfg(feature = "openai-compat-beta")]
impl axum::response::IntoResponse for OpenAiCompatHttpError {
    fn into_response(self) -> axum::response::Response {
        use axum::Json;
        use axum::http::StatusCode;

        let status = StatusCode::from_u16(self.status_code).unwrap_or_else(|_| {
            tracing::error!(
                target = "ironclaw_reborn_openai_compat::error",
                status_code = self.status_code,
                "OpenAI-compatible error carried a non-HTTP status; coercing to 500"
            );
            StatusCode::INTERNAL_SERVER_ERROR
        });
        (status, Json(self.body)).into_response()
    }
}

#[derive(Debug, Clone, Copy)]
struct ErrorSpec {
    message: &'static str,
    error_type: OpenAiCompatErrorType,
    code: OpenAiCompatErrorCode,
}

impl ErrorSpec {
    fn for_kind(kind: OpenAiCompatErrorKind) -> Self {
        match kind {
            OpenAiCompatErrorKind::Validation => Self {
                message: OpenAiCompatErrorCode::InvalidRequest.sanitized_message(),
                error_type: OpenAiCompatErrorType::InvalidRequestError,
                code: OpenAiCompatErrorCode::InvalidRequest,
            },
            OpenAiCompatErrorKind::Authentication => Self {
                message: OpenAiCompatErrorCode::AuthenticationRequired.sanitized_message(),
                error_type: OpenAiCompatErrorType::AuthenticationError,
                code: OpenAiCompatErrorCode::AuthenticationRequired,
            },
            OpenAiCompatErrorKind::PermissionDenied => Self {
                message: OpenAiCompatErrorCode::PermissionDenied.sanitized_message(),
                error_type: OpenAiCompatErrorType::PermissionError,
                code: OpenAiCompatErrorCode::PermissionDenied,
            },
            OpenAiCompatErrorKind::NotFound => Self {
                message: OpenAiCompatErrorCode::NotFound.sanitized_message(),
                error_type: OpenAiCompatErrorType::NotFoundError,
                code: OpenAiCompatErrorCode::NotFound,
            },
            OpenAiCompatErrorKind::Conflict => Self {
                message: OpenAiCompatErrorCode::Conflict.sanitized_message(),
                error_type: OpenAiCompatErrorType::ConflictError,
                code: OpenAiCompatErrorCode::Conflict,
            },
            OpenAiCompatErrorKind::RateLimited => Self {
                message: OpenAiCompatErrorCode::RateLimited.sanitized_message(),
                error_type: OpenAiCompatErrorType::RateLimitError,
                code: OpenAiCompatErrorCode::RateLimited,
            },
            OpenAiCompatErrorKind::ServiceUnavailable => Self {
                message: OpenAiCompatErrorCode::ServiceUnavailable.sanitized_message(),
                error_type: OpenAiCompatErrorType::ServerError,
                code: OpenAiCompatErrorCode::ServiceUnavailable,
            },
            OpenAiCompatErrorKind::Unsupported => Self {
                message: OpenAiCompatErrorCode::Unsupported.sanitized_message(),
                error_type: OpenAiCompatErrorType::InvalidRequestError,
                code: OpenAiCompatErrorCode::Unsupported,
            },
            OpenAiCompatErrorKind::Internal => Self {
                message: OpenAiCompatErrorCode::InternalError.sanitized_message(),
                error_type: OpenAiCompatErrorType::ServerError,
                code: OpenAiCompatErrorCode::InternalError,
            },
        }
    }
}

fn sanitize_status_code(status_code: u16) -> u16 {
    match status_code {
        400..=499 | 500 | 501 | 503 => status_code,
        _ => 503,
    }
}

fn clean_param(param: Option<String>) -> Option<String> {
    let value = param?;
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed != value
        || trimmed.len() > 128
        || trimmed.chars().any(|ch| ch == '\0' || ch.is_control())
        || contains_no_exposure_sentinel(trimmed)
        || !is_allowed_param_path(trimmed)
    {
        return None;
    }
    Some(trimmed.to_string())
}

fn is_allowed_param_path(value: &str) -> bool {
    let mut segments = value.split('.');
    let Some(first) = segments.next() else {
        return false;
    };
    if !is_allowed_param_root(first) {
        return false;
    }
    segments.all(is_allowed_param_segment)
}

fn is_allowed_param_root(segment: &str) -> bool {
    let (field, index) = split_param_segment(segment);
    matches!(
        field,
        "body"
            | "idempotency_key"
            | "input"
            | "messages"
            | "metadata"
            | "model"
            | "previous_response_id"
            | "response_id"
            | "stream"
            | "tool_choice"
            | "tools"
    ) && index.is_none_or(is_ascii_digits)
        && (index.is_none() || matches!(field, "input" | "messages" | "tools"))
}

fn is_allowed_param_segment(segment: &str) -> bool {
    let (field, index) = split_param_segment(segment);
    is_ascii_snake_field(field) && index.is_none_or(is_ascii_digits)
}

fn split_param_segment(segment: &str) -> (&str, Option<&str>) {
    let Some(open) = segment.find('[') else {
        return (segment, None);
    };
    let Some(index) = segment
        .strip_suffix(']')
        .and_then(|_| segment.get(open + 1..segment.len() - 1))
    else {
        return (segment, Some(""));
    };
    (&segment[..open], Some(index))
}

fn is_ascii_snake_field(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|ch| ch == '_' || ch.is_ascii_lowercase() || ch.is_ascii_digit())
        && value
            .chars()
            .next()
            .is_some_and(|ch| ch == '_' || ch.is_ascii_lowercase())
}

fn is_ascii_digits(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|ch| ch.is_ascii_digit())
}

fn contains_no_exposure_sentinel(value: &str) -> bool {
    const NO_EXPOSURE_SENTINELS: &[&str] = &[
        "RAW_PROMPT_SENTINEL",
        "SECRET_SENTINEL",
        "secret-token",
        "sk-live",
        "/host/path",
        "/Users/",
    ];
    NO_EXPOSURE_SENTINELS
        .iter()
        .any(|sentinel| value.contains(sentinel))
}

#[cfg(all(test, feature = "openai-compat-beta"))]
mod tests {
    use ironclaw_product_adapters::{ProductRejection, ProductRejectionKind};

    use super::product_rejection_to_openai_error;

    /// `BindingRequired` maps to 404 Not Found (scope lookup failed).
    #[test]
    fn binding_required_maps_to_404() {
        let rejection =
            ProductRejection::permanent(ProductRejectionKind::BindingRequired, "no binding");
        let err = product_rejection_to_openai_error(&rejection, Some("messages"));
        assert_eq!(err.status_code(), 404);
        assert!(!err.retryable());
    }

    /// `AccessDenied` maps to 403 Forbidden, non-retryable.
    #[test]
    fn access_denied_maps_to_403() {
        let rejection = ProductRejection::permanent(ProductRejectionKind::AccessDenied, "denied");
        let err = product_rejection_to_openai_error(&rejection, None);
        assert_eq!(err.status_code(), 403);
        assert!(!err.retryable());
    }

    /// `PolicyDenied` maps to 403 Forbidden, non-retryable.
    #[test]
    fn policy_denied_maps_to_403() {
        let rejection = ProductRejection::permanent(ProductRejectionKind::PolicyDenied, "policy");
        let err = product_rejection_to_openai_error(&rejection, None);
        assert_eq!(err.status_code(), 403);
        assert!(!err.retryable());
    }

    /// `UnknownInstallation` maps to 503 Service Unavailable, retryable.
    #[test]
    fn unknown_installation_maps_to_503_retryable() {
        let rejection =
            ProductRejection::retryable(ProductRejectionKind::UnknownInstallation, "unknown");
        let err = product_rejection_to_openai_error(&rejection, None);
        assert_eq!(err.status_code(), 503);
        assert!(err.retryable());
    }

    /// `InvalidRequest` maps to 400 Bad Request, non-retryable.
    #[test]
    fn invalid_request_maps_to_400() {
        let rejection =
            ProductRejection::permanent(ProductRejectionKind::InvalidRequest, "bad input");
        let err = product_rejection_to_openai_error(&rejection, Some("input"));
        assert_eq!(err.status_code(), 400);
        assert!(!err.retryable());
    }

    /// `AmbiguousResolution` maps to 409 Conflict, non-retryable.
    #[test]
    fn ambiguous_resolution_maps_to_409() {
        let rejection =
            ProductRejection::permanent(ProductRejectionKind::AmbiguousResolution, "ambiguous");
        let err = product_rejection_to_openai_error(&rejection, None);
        assert_eq!(err.status_code(), 409);
        assert!(!err.retryable());
    }

    /// `StaleGate` maps to 409 Conflict, non-retryable.
    ///
    /// The gate already resolved (approved/denied) — the resolution conflicts
    /// with the now-settled state. Must NOT be retryable (the client must not
    /// replay the same approval/denial against an already-resolved gate).
    #[test]
    fn stale_gate_maps_to_409_conflict_not_retryable() {
        let rejection =
            ProductRejection::permanent(ProductRejectionKind::StaleGate, "gate already resolved");
        let err = product_rejection_to_openai_error(&rejection, None);
        assert_eq!(
            err.status_code(),
            409,
            "StaleGate must surface as 409 Conflict, got {}",
            err.status_code()
        );
        assert!(
            !err.retryable(),
            "StaleGate is a terminal conflict — must not be retryable"
        );
    }
}
