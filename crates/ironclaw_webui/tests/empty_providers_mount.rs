use axum::body::Body;
use http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
use ironclaw_webui::empty_webui_v2_auth_providers_mount;
use tower::ServiceExt;

#[tokio::test]
async fn empty_provider_mount_only_serves_provider_discovery() {
    let mount = empty_webui_v2_auth_providers_mount();
    assert_eq!(mount.descriptors.len(), 1);
    assert_eq!(
        mount.descriptors[0].route_id().as_str(),
        "webui.sso.providers"
    );
    assert_eq!(
        mount.descriptors[0].route_pattern().as_str(),
        "/auth/providers"
    );

    let providers = mount
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/auth/providers")
                .body(Body::empty())
                .expect("providers request"),
        )
        .await
        .expect("providers oneshot");
    assert_eq!(providers.status(), StatusCode::OK);
    let bytes = providers
        .into_body()
        .collect()
        .await
        .expect("providers body")
        .to_bytes();
    assert_eq!(bytes.as_ref(), br#"{"providers":[]}"#);

    let logout = mount
        .router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/logout")
                .header("authorization", "Bearer env-token")
                .body(Body::empty())
                .expect("logout request"),
        )
        .await
        .expect("logout oneshot");
    assert_eq!(logout.status(), StatusCode::NOT_FOUND);
}
