#![cfg(feature = "openai-compat-beta")]

use axum::body::Body;
use http::Request;
use http_body_util::BodyExt;
use ironclaw_reborn_openai_compat::openai_compat_router;
use tower::ServiceExt;

#[tokio::test]
async fn mounted_routes_fail_closed_until_product_workflow_is_wired() {
    let cases = [
        (
            "POST",
            "/v1/chat/completions",
            http::StatusCode::UNAUTHORIZED,
        ),
        (
            "POST",
            "/api/v1/responses",
            http::StatusCode::NOT_IMPLEMENTED,
        ),
        ("POST", "/v1/responses", http::StatusCode::NOT_IMPLEMENTED),
        (
            "GET",
            "/api/v1/responses/resp_123",
            http::StatusCode::NOT_IMPLEMENTED,
        ),
        (
            "GET",
            "/v1/responses/resp_123",
            http::StatusCode::NOT_IMPLEMENTED,
        ),
        (
            "POST",
            "/api/v1/responses/resp_123/cancel",
            http::StatusCode::NOT_IMPLEMENTED,
        ),
        (
            "POST",
            "/v1/responses/resp_123/cancel",
            http::StatusCode::NOT_IMPLEMENTED,
        ),
    ];

    for (method, path, expected_status) in cases {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .expect("request");
        let response = openai_compat_router()
            .oneshot(request)
            .await
            .expect("route response");

        assert_eq!(response.status(), expected_status, "{path}");
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&bytes).expect("json body");
        if expected_status == http::StatusCode::UNAUTHORIZED {
            assert_eq!(body["error"]["code"], "authentication_required", "{path}");
        } else {
            assert_eq!(body["error"]["code"], "unsupported", "{path}");
            assert_eq!(
                body["error"]["message"], "This OpenAI-compatible Reborn route is not wired yet.",
                "{path}"
            );
        }
    }
}
