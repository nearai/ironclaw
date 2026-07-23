use axum::body::Body;
use http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
use ironclaw_host_api::{TenantId, UserId};
use ironclaw_webui::{
    empty_webui_v2_auth_providers_mount, signed_session_store, signed_session_webui_v2_auth_mount,
};
use secrecy::{ExposeSecret, SecretString};
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

#[tokio::test]
async fn signed_session_mount_exposes_logout_and_revokes_the_bearer() {
    let tenant = TenantId::new("logout-tenant").expect("tenant");
    let store = signed_session_store(&SecretString::from("logout-secret".to_string()), &tenant);
    let token = store
        .create_session(
            tenant,
            UserId::new("logout-user").expect("user"),
            chrono::Duration::hours(1),
            false,
        )
        .await
        .expect("session");
    let mount = signed_session_webui_v2_auth_mount(store.clone());
    assert_eq!(mount.descriptors.len(), 2);
    assert_eq!(
        mount.descriptors[1].route_pattern().as_str(),
        "/auth/logout"
    );

    let logout = mount
        .router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/logout")
                .header("authorization", format!("Bearer {}", token.expose_secret()))
                .body(Body::empty())
                .expect("logout request"),
        )
        .await
        .expect("logout oneshot");
    assert_eq!(logout.status(), StatusCode::NO_CONTENT);
    assert!(
        store
            .lookup(token.expose_secret())
            .await
            .expect("lookup")
            .is_none(),
        "logout must revoke the signed session"
    );
}

#[tokio::test]
async fn logout_keeps_a_reusable_auth_token_valid_for_the_next_login() {
    let tenant = TenantId::new("auth-token-tenant").expect("tenant");
    let store = signed_session_store(
        &SecretString::from("auth-token-secret".to_string()),
        &tenant,
    );
    let token = store
        .create_reusable_auth_token(
            tenant,
            UserId::new("returning-user").expect("user"),
            chrono::Duration::hours(1),
        )
        .await
        .expect("auth token");
    let mount = signed_session_webui_v2_auth_mount(store.clone());

    let logout = mount
        .router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/logout")
                .header("authorization", format!("Bearer {}", token.expose_secret()))
                .body(Body::empty())
                .expect("logout request"),
        )
        .await
        .expect("logout oneshot");
    assert_eq!(logout.status(), StatusCode::NO_CONTENT);
    assert!(
        store
            .lookup(token.expose_secret())
            .await
            .expect("lookup")
            .is_some(),
        "the reusable auth token must work again after logout"
    );
}
