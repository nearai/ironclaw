//! HTTP error vocabulary for the WebChat v2 routes.
//!
//! Maps the redacted [`RebornServicesError`] surface into the wire-stable
//! shape browser handlers return to clients. The handler layer must never
//! leak backend identifiers, scopes, or transport error details — the only
//! information that crosses this boundary is what `RebornServicesError`
//! already exposes (`code`, `kind`, `status_code`, `retryable`, and the typed
//! validation hint).

use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use ironclaw_product_workflow::{
    RebornServicesError, RebornServicesErrorCode, RebornServicesErrorKind,
    WebUiInboundValidationCode,
};
use serde::Serialize;

/// HTTP-shaped error response wrapping a [`RebornServicesError`].
///
/// `IntoResponse` is the only construction path: callers convert via
/// `From<RebornServicesError>` and `?`, never by hand-building a status
/// code, so the mapping stays consistent across handlers.
#[derive(Debug)]
pub struct WebUiV2HttpError(RebornServicesError);

impl WebUiV2HttpError {
    pub fn into_response_parts(self) -> (StatusCode, WebUiV2HttpErrorBody) {
        let status = StatusCode::from_u16(self.0.status_code).unwrap_or_else(|_| {
            // Defensive: every call site in `ironclaw_product_workflow` builds
            // status codes from a fixed table (400/401/403/404/409/429/500/503).
            // If a future variant introduces a non-HTTP code, log loudly and
            // fall back to 500 rather than poisoning the response.
            tracing::error!(
                target = "ironclaw_webui_v2::error",
                status_code = self.0.status_code,
                "RebornServicesError carried a non-HTTP status code; coercing to 500"
            );
            StatusCode::INTERNAL_SERVER_ERROR
        });
        let body = WebUiV2HttpErrorBody {
            error: self.0.code,
            kind: self.0.kind,
            retryable: self.0.retryable,
            field: self.0.field.clone(),
            validation_code: self.0.validation_code,
        };
        (status, body)
    }
}

impl From<RebornServicesError> for WebUiV2HttpError {
    fn from(error: RebornServicesError) -> Self {
        Self(error)
    }
}

impl IntoResponse for WebUiV2HttpError {
    fn into_response(self) -> Response {
        let (status, body) = self.into_response_parts();
        (status, Json(body)).into_response()
    }
}

/// Wire shape of an HTTP error returned by a WebChat v2 handler.
///
/// `validation_code` is the typed enum from `ironclaw_product_workflow`; it
/// serializes as snake_case (e.g. `"missing_field"`) via its own `Serialize`
/// impl, so this struct does not perform any fallible string conversion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WebUiV2HttpErrorBody {
    pub error: RebornServicesErrorCode,
    pub kind: RebornServicesErrorKind,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_code: Option<WebUiInboundValidationCode>,
}
