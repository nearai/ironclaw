use ironclaw_product_adapters::{
    ProductAdapterError, ProductWorkflowRejectionKind, ProtocolAuthFailure, RedactedString,
};
use ironclaw_reborn_openai_compat::{
    OpenAiCompatErrorCode, OpenAiCompatErrorKind, OpenAiCompatErrorResponse, OpenAiCompatErrorType,
    OpenAiCompatHttpError,
};
use serde_json::json;

#[test]
fn workflow_rejection_maps_to_stable_openai_error_envelope() {
    let error = OpenAiCompatHttpError::from_workflow_rejection(
        ProductWorkflowRejectionKind::Unauthorized,
        403,
        false,
        Some("response_id".to_string()),
    );

    assert_eq!(error.status_code(), 403);
    assert!(!error.retryable());
    assert_eq!(
        error.body().error.error_type(),
        OpenAiCompatErrorType::PermissionError
    );
    assert_eq!(
        error.body().error.code(),
        Some(OpenAiCompatErrorCode::PermissionDenied)
    );
    assert_eq!(error.body().error.param(), Some("response_id"));

    let serialized = serde_json::to_value(error.body()).expect("serialize error");
    assert_eq!(serialized["error"]["type"], "permission_error");
    assert_eq!(serialized["error"]["code"], "permission_denied");
}

#[test]
fn busy_and_transient_failures_keep_retryable_status_mapping() {
    let busy = OpenAiCompatHttpError::from_workflow_rejection(
        ProductWorkflowRejectionKind::ThreadBusy,
        429,
        true,
        None,
    );
    assert_eq!(busy.status_code(), 429);
    assert!(busy.retryable());
    assert_eq!(
        busy.body().error.code(),
        Some(OpenAiCompatErrorCode::RateLimited)
    );

    let transient =
        OpenAiCompatHttpError::from_product_adapter_error(ProductAdapterError::WorkflowTransient {
            reason: RedactedString::new("store down /host/path secret-token"),
        });
    assert_eq!(transient.status_code(), 503);
    assert!(transient.retryable());
    assert_eq!(
        transient.body().error.code(),
        Some(OpenAiCompatErrorCode::ServiceUnavailable)
    );
}

#[test]
fn server_status_codes_preserve_stable_server_contracts() {
    for status in [500, 501, 503] {
        let error =
            OpenAiCompatHttpError::from_kind(status, false, OpenAiCompatErrorKind::Internal, None);
        assert_eq!(error.status_code(), status, "{status}");
    }

    for status in [200, 502, 504, 599] {
        let error = OpenAiCompatHttpError::from_kind(
            status,
            true,
            OpenAiCompatErrorKind::ServiceUnavailable,
            None,
        );
        assert_eq!(error.status_code(), 503, "{status}");
    }
}

#[test]
fn error_mapping_does_not_serialize_backend_or_secret_details() {
    let error = OpenAiCompatHttpError::from_product_adapter_error(ProductAdapterError::Internal {
        detail: RedactedString::new(
            "RAW_PROMPT_SENTINEL provider stack /host/path /Users/alice secret-token sk-live",
        ),
    });
    let rendered = serde_json::to_string(error.body()).expect("serialize error");

    for forbidden in [
        "RAW_PROMPT_SENTINEL",
        "provider stack",
        "/host/path",
        "/Users/alice",
        "secret-token",
        "sk-live",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "error body leaked forbidden detail {forbidden:?}: {rendered}"
        );
    }
}

#[test]
fn suspicious_error_params_are_dropped_instead_of_normalized() {
    let mut invalid = vec![
        "".to_string(),
        " response_id".to_string(),
        "response_id ".to_string(),
        "messages[0].content\n".to_string(),
        "RAW_PROMPT_SENTINEL".to_string(),
        "secret-token".to_string(),
        "/host/path".to_string(),
        "/Users/alice".to_string(),
        "sk-live".to_string(),
        "secret_token".to_string(),
        "messages[-1].content".to_string(),
        "messages[].content".to_string(),
        "model[0]".to_string(),
        "messages[0].Content".to_string(),
    ];
    invalid.push("x".repeat(129));
    for param in invalid {
        let error = OpenAiCompatHttpError::from_kind(
            400,
            false,
            OpenAiCompatErrorKind::Validation,
            Some(param.clone()),
        );
        assert_eq!(error.body().error.param(), None, "{param:?}");
    }

    for param in [
        "body",
        "model",
        "messages",
        "messages[0].content",
        "input[12].content",
        "response_id",
        "idempotency_key",
    ] {
        let error = OpenAiCompatHttpError::from_kind(
            400,
            false,
            OpenAiCompatErrorKind::Validation,
            Some(param.to_string()),
        );
        assert_eq!(error.body().error.param(), Some(param), "{param:?}");
    }
}

#[test]
fn workflow_rejection_maps_each_kind_to_stable_openai_error() {
    use ProductWorkflowRejectionKind as K;

    let cases = [
        (
            K::ThreadBusy,
            429,
            true,
            OpenAiCompatErrorType::RateLimitError,
            OpenAiCompatErrorCode::RateLimited,
        ),
        (
            K::AdmissionRejected,
            429,
            true,
            OpenAiCompatErrorType::RateLimitError,
            OpenAiCompatErrorCode::RateLimited,
        ),
        (
            K::ScopeNotFound,
            404,
            false,
            OpenAiCompatErrorType::NotFoundError,
            OpenAiCompatErrorCode::NotFound,
        ),
        (
            K::Unauthorized,
            403,
            false,
            OpenAiCompatErrorType::PermissionError,
            OpenAiCompatErrorCode::PermissionDenied,
        ),
        (
            K::InvalidRequest,
            400,
            false,
            OpenAiCompatErrorType::InvalidRequestError,
            OpenAiCompatErrorCode::InvalidRequest,
        ),
        (
            K::Unavailable,
            503,
            true,
            OpenAiCompatErrorType::ServerError,
            OpenAiCompatErrorCode::ServiceUnavailable,
        ),
        (
            K::Conflict,
            409,
            false,
            OpenAiCompatErrorType::ConflictError,
            OpenAiCompatErrorCode::Conflict,
        ),
        (
            K::Ambiguous,
            409,
            false,
            OpenAiCompatErrorType::ConflictError,
            OpenAiCompatErrorCode::Conflict,
        ),
    ];

    for (kind, status, retryable, error_type, code) in cases {
        let error = OpenAiCompatHttpError::from_workflow_rejection(
            kind,
            status,
            retryable,
            Some("messages[0].content".to_string()),
        );
        assert_eq!(error.status_code(), status, "{kind:?}");
        assert_eq!(error.retryable(), retryable, "{kind:?}");
        assert_eq!(error.body().error.error_type(), error_type, "{kind:?}");
        assert_eq!(error.body().error.code(), Some(code), "{kind:?}");
        assert_eq!(error.body().error.param(), Some("messages[0].content"));
    }
}

#[test]
fn product_adapter_error_variants_map_to_sanitized_openai_errors() {
    let cases = [
        (
            ProductAdapterError::InvalidIdentifier {
                kind: "adapter",
                reason: "bad /Users/alice secret-token".to_string(),
            },
            400,
            false,
            OpenAiCompatErrorCode::InvalidRequest,
        ),
        (
            ProductAdapterError::MalformedInboundPayload {
                reason: RedactedString::new("bad RAW_PROMPT_SENTINEL"),
            },
            400,
            false,
            OpenAiCompatErrorCode::InvalidRequest,
        ),
        (
            ProductAdapterError::Authentication(ProtocolAuthFailure::Missing),
            401,
            false,
            OpenAiCompatErrorCode::AuthenticationRequired,
        ),
        (
            ProductAdapterError::WorkflowRejected {
                kind: ProductWorkflowRejectionKind::Conflict,
                status_code: 409,
                retryable: false,
                reason: RedactedString::new("duplicate"),
            },
            409,
            false,
            OpenAiCompatErrorCode::Conflict,
        ),
        (
            ProductAdapterError::WorkflowTransient {
                reason: RedactedString::new("store down"),
            },
            503,
            true,
            OpenAiCompatErrorCode::ServiceUnavailable,
        ),
        (
            ProductAdapterError::EgressTransient {
                reason: RedactedString::new("timeout"),
            },
            503,
            true,
            OpenAiCompatErrorCode::ServiceUnavailable,
        ),
        (
            ProductAdapterError::EgressDenied {
                reason: RedactedString::new("blocked secret-token"),
            },
            500,
            false,
            OpenAiCompatErrorCode::InternalError,
        ),
        (
            ProductAdapterError::EgressUndeclaredHost {
                host: "metadata.local".to_string(),
            },
            500,
            false,
            OpenAiCompatErrorCode::InternalError,
        ),
        (
            ProductAdapterError::Internal {
                detail: RedactedString::new("stack /host/path"),
            },
            500,
            false,
            OpenAiCompatErrorCode::InternalError,
        ),
    ];

    for (adapter_error, status, retryable, code) in cases {
        let error = OpenAiCompatHttpError::from_product_adapter_error(adapter_error);
        assert_eq!(error.status_code(), status, "{code:?}");
        assert_eq!(error.retryable(), retryable, "{code:?}");
        assert_eq!(error.body().error.code(), Some(code), "{code:?}");
    }
}

#[test]
fn error_envelope_rejects_unknown_fields() {
    let err = serde_json::from_value::<OpenAiCompatErrorResponse>(json!({
        "error": {
            "message": "The request is invalid.",
            "type": "invalid_request_error",
            "param": null,
            "code": "invalid_request",
            "debug": "must reject"
        }
    }))
    .expect_err("unknown fields must reject");
    assert!(err.to_string().contains("unknown field"));
}
