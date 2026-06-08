#![cfg(feature = "openai-compat-beta")]

use axum::body::Body;
use http::Request;
use http_body_util::BodyExt;
use ironclaw_reborn_openai_compat::openai_compat_router;
use tower::ServiceExt;

#[tokio::test]
async fn mounted_routes_fail_closed_until_product_workflow_is_wired() {
    let cases = [
        ("POST", "/v1/chat/completions"),
        ("POST", "/api/v1/responses"),
        ("POST", "/v1/responses"),
        ("GET", "/api/v1/responses/resp_123"),
        ("GET", "/v1/responses/resp_123"),
        ("POST", "/api/v1/responses/resp_123/cancel"),
        ("POST", "/v1/responses/resp_123/cancel"),
    ];

    for (method, path) in cases {
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

        assert_eq!(
            response.status(),
            http::StatusCode::NOT_IMPLEMENTED,
            "{path}"
        );
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&bytes).expect("json body");
        assert_eq!(body["error"]["code"], "unsupported", "{path}");
        assert_eq!(
            body["error"]["message"], "This OpenAI-compatible Reborn route is not wired yet.",
            "{path}"
        );
    }
}
