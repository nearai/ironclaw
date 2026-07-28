//! HTTP error vocabulary for the WebChat v2 routes.
//!
//! Maps the redacted [`ProductSurfaceError`] surface into the wire-stable
//! shape browser handlers return to clients. The handler layer must never
//! leak backend identifiers, scopes, or transport error details — the only
//! information that crosses this boundary is what `ProductSurfaceError`
//! already exposes (`code`, `kind`, `status_code`, `retryable`, and the typed
//! validation hint).

use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use ironclaw_host_api::{
    ProductSurfaceError, ProductSurfaceErrorCode, ProductSurfaceErrorKind,
    ProductSurfaceValidationCode,
};
use serde::Serialize;

/// HTTP-shaped error response wrapping a [`ProductSurfaceError`].
///
/// `IntoResponse` is the only construction path: callers convert via
/// `From<ProductSurfaceError>` and `?`, never by hand-building a status
/// code, so the mapping stays consistent across handlers.
#[derive(Debug)]
pub struct WebUiV2HttpError(ProductSurfaceError);

impl WebUiV2HttpError {
    pub fn into_response_parts(self) -> (StatusCode, WebUiV2HttpErrorBody) {
        let parts = self.0.into_http_parts();
        let status = StatusCode::from_u16(parts.status_code).unwrap_or_else(|_| {
            // Defensive: every call site in `ironclaw_product` builds
            // status codes from a fixed table (400/401/403/404/409/429/500/503).
            // If a future variant introduces a non-HTTP code, log loudly and
            // fall back to 500 rather than poisoning the response.
            tracing::error!(
                target = "ironclaw_webui_v2::error",
                status_code = parts.status_code,
                "ProductSurfaceError carried a non-HTTP status code; coercing to 500"
            );
            StatusCode::INTERNAL_SERVER_ERROR
        });
        // Single choke point for every WebChat v2 error response: a sanitized
        // 5xx must never leave the gateway without a server-side log line, or an
        // operator sees a bare "Internal" 500 with no trail. This is the safety
        // net that makes surfacing internal errors a property of the boundary,
        // not of each call site remembering to log before it maps. The
        // underlying cause (when a backend error was converted) is logged at the
        // construction site — see `ProductSurfaceError::internal_from`.
        if status.is_server_error() {
            tracing::error!(
                status = status.as_u16(),
                code = ?parts.code,
                kind = ?parts.kind,
                retryable = parts.retryable,
                "WebChat v2 handler returned a server error",
            );
        }
        let body = WebUiV2HttpErrorBody {
            error: parts.code,
            kind: parts.kind,
            retryable: parts.retryable,
            field: parts.field,
            validation_code: parts.validation_code,
        };
        (status, body)
    }
}

impl From<ProductSurfaceError> for WebUiV2HttpError {
    fn from(error: ProductSurfaceError) -> Self {
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
/// `validation_code` is the typed enum from `ironclaw_product`; it
/// serializes as snake_case (e.g. `"missing_field"`) via its own `Serialize`
/// impl, so this struct does not perform any fallible string conversion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WebUiV2HttpErrorBody {
    pub error: ProductSurfaceErrorCode,
    pub kind: ProductSurfaceErrorKind,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_code: Option<ProductSurfaceValidationCode>,
}
