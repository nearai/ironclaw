use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ironclaw_host_api::{TenantId, UserId};
use ironclaw_product_adapters::AdapterInstallationId;
use tower::ServiceExt;

use super::*;
use crate::slack_channel_routes::{
    DEFAULT_LIST_LIMIT, InMemorySlackChannelRouteStore, SlackChannelRouteError,
    SlackChannelRouteKey, SlackChannelRouteListPage, SlackChannelRouteStore,
    slack_channel_route_admin_route_mount,
};

const TENANT: &str = "tenant:slack-routes";
const INSTALLATION: &str = "install_slack_routes";
const TEAM: &str = "T0ROUTES";

#[tokio::test]
async fn allowed_channel_admin_saves_replaces_and_lists_channel_routes() {
    let store = Arc::new(InMemorySlackChannelRouteStore::new());
    let mount = slack_channel_route_admin_route_mount(route_config(store.clone()));

    let save_response = mount
        .protected
        .clone()
        .oneshot(request(
            "PUT",
            r#"{"channel_ids":["C0OPS"," C0ENG ","C0OPS"]}"#,
            TENANT,
        ))
        .await
        .expect("save responds");
    assert_eq!(save_response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(save_response.into_body(), 64 * 1024)
        .await
        .expect("body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(body["success"], true);
    assert_eq!(body["team_id"], TEAM);
    assert_eq!(body["channels"].as_array().expect("channels").len(), 2);
    assert_eq!(body["channels"][0]["channel_id"], "C0ENG");
    assert_eq!(body["channels"][1]["channel_id"], "C0OPS");
    assert_ne!(
        body["channels"][0]["subject_user_id"], body["channels"][1]["subject_user_id"],
        "each managed Slack channel must get its own tenant-scoped subject"
    );

    let routes = store
        .list_routes(
            &TenantId::new(TENANT).expect("tenant"),
            &AdapterInstallationId::new(INSTALLATION).expect("installation"),
            TEAM,
            0,
            DEFAULT_LIST_LIMIT,
        )
        .await
        .expect("routes list");
    assert_eq!(routes.routes.len(), 2);
    assert_ne!(
        routes.routes[0].subject_user_id, routes.routes[1].subject_user_id,
        "persisted routes should keep per-channel subjects"
    );

    let replace_response = mount
        .protected
        .clone()
        .oneshot(request("PUT", r#"{"channel_ids":["C0OPS"]}"#, TENANT))
        .await
        .expect("replace responds");
    assert_eq!(replace_response.status(), StatusCode::OK);
    let replace_body = axum::body::to_bytes(replace_response.into_body(), 64 * 1024)
        .await
        .expect("replace body");
    let replace_body: serde_json::Value =
        serde_json::from_slice(&replace_body).expect("replace json");

    let list_response = mount
        .protected
        .oneshot(request("GET", "", TENANT))
        .await
        .expect("list responds");
    assert_eq!(list_response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(list_response.into_body(), 64 * 1024)
        .await
        .expect("body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(
        body["channels"],
        serde_json::json!([
            {
                "channel_id": "C0OPS",
                "subject_user_id": replace_body["channels"][0]["subject_user_id"].clone(),
                "subject_display_name": replace_body["channels"][0]["subject_display_name"].clone()
            }
        ])
    );
}

#[tokio::test]
async fn allowed_channel_admin_replaces_with_selected_team_subjects() {
    let store = Arc::new(InMemorySlackChannelRouteStore::new());
    let mount = slack_channel_route_admin_route_mount(route_config(store.clone()));

    let save_response = mount
        .protected
        .clone()
        .oneshot(request(
            "PUT",
            r#"{"channels":[{"channel_id":"C0PRODUCT","subject_user_id":"user:eng-team-agent"}]}"#,
            TENANT,
        ))
        .await
        .expect("save responds");

    assert_eq!(save_response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(save_response.into_body(), 64 * 1024)
        .await
        .expect("body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(
        body["channels"],
        serde_json::json!([
            {
                "channel_id": "C0PRODUCT",
                "subject_user_id": "user:eng-team-agent",
                "subject_display_name": "Eng"
            }
        ])
    );
    let routes = store
        .list_routes(
            &TenantId::new(TENANT).expect("tenant"),
            &AdapterInstallationId::new(INSTALLATION).expect("installation"),
            TEAM,
            0,
            DEFAULT_LIST_LIMIT,
        )
        .await
        .expect("routes list");
    assert_eq!(routes.routes[0].subject_user_id, "user:eng-team-agent");
}

#[tokio::test]
async fn allowed_channel_admin_rejects_unknown_selected_team_subject() {
    let store = Arc::new(InMemorySlackChannelRouteStore::new());
    let mount = slack_channel_route_admin_route_mount(route_config(store.clone()));

    let response = mount
            .protected
            .oneshot(request(
                "PUT",
                r#"{"channels":[{"channel_id":"C0PRODUCT","subject_user_id":"user:finance-team-agent"}]}"#,
                TENANT,
            ))
            .await
            .expect("save responds");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(
        store
            .list_routes(
                &TenantId::new(TENANT).expect("tenant"),
                &AdapterInstallationId::new(INSTALLATION).expect("installation"),
                TEAM,
                0,
                DEFAULT_LIST_LIMIT,
            )
            .await
            .expect("routes list")
            .routes
            .is_empty()
    );
}

#[tokio::test]
async fn allowed_channel_admin_rejects_mixed_save_shapes_without_mutating_store() {
    let store = Arc::new(InMemorySlackChannelRouteStore::new());
    let mount = slack_channel_route_admin_route_mount(route_config(store.clone()));

    let response = mount
        .protected
        .oneshot(request(
            "PUT",
            r#"{"channel_ids":["C0OPS"],"channels":[{"channel_id":"C0PRODUCT","subject_user_id":"user:eng-team-agent"}]}"#,
            TENANT,
        ))
        .await
        .expect("save responds");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        store
            .list_routes(
                &TenantId::new(TENANT).expect("tenant"),
                &AdapterInstallationId::new(INSTALLATION).expect("installation"),
                TEAM,
                0,
                DEFAULT_LIST_LIMIT,
            )
            .await
            .expect("routes list")
            .routes
            .is_empty()
    );
}

#[tokio::test]
async fn allowed_channel_admin_preserves_matching_generated_subjects() {
    let store = Arc::new(InMemorySlackChannelRouteStore::new());
    let mount = slack_channel_route_admin_route_mount(route_config(store.clone()));

    let initial_save = mount
        .protected
        .clone()
        .oneshot(request("PUT", r#"{"channel_ids":["C0OPS"]}"#, TENANT))
        .await
        .expect("initial save responds");
    assert_eq!(initial_save.status(), StatusCode::OK);
    let body = axum::body::to_bytes(initial_save.into_body(), 64 * 1024)
        .await
        .expect("body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("json");
    let generated_subject = body["channels"][0]["subject_user_id"]
        .as_str()
        .expect("generated subject");

    let explicit_save = mount
        .protected
        .clone()
        .oneshot(request(
            "PUT",
            &serde_json::json!({
                "channels": [
                    {
                        "channel_id": "C0OPS",
                        "subject_user_id": generated_subject
                    }
                ]
            })
            .to_string(),
            TENANT,
        ))
        .await
        .expect("explicit save responds");

    assert_eq!(explicit_save.status(), StatusCode::OK);
    let body = axum::body::to_bytes(explicit_save.into_body(), 64 * 1024)
        .await
        .expect("body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(body["channels"][0]["subject_user_id"], generated_subject);
}

#[tokio::test]
async fn allowed_channel_admin_lists_and_replaces_existing_unmanaged_routes() {
    let store = Arc::new(InMemorySlackChannelRouteStore::new());
    let mount = slack_channel_route_admin_route_mount(route_config(store.clone()));
    let tenant_id = TenantId::new(TENANT).expect("tenant");
    let installation_id = AdapterInstallationId::new(INSTALLATION).expect("installation");
    store
        .upsert_route(
            SlackChannelRouteKey::new(
                tenant_id.clone(),
                installation_id.clone(),
                TEAM.to_string(),
                "C0RAW".to_string(),
            )
            .expect("raw key"),
            UserId::new("user:raw-route-subject").expect("raw subject"),
        )
        .await
        .expect("seed raw route");

    let list_response = mount
        .protected
        .clone()
        .oneshot(request("GET", "", TENANT))
        .await
        .expect("list responds");
    assert_eq!(list_response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(list_response.into_body(), 64 * 1024)
        .await
        .expect("body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(body["channels"][0]["channel_id"], "C0RAW");
    assert_eq!(
        body["channels"][0]["subject_user_id"],
        "user:raw-route-subject"
    );

    let replace_response = mount
        .protected
        .oneshot(request("PUT", r#"{"channel_ids":["C0ENG"]}"#, TENANT))
        .await
        .expect("replace responds");
    assert_eq!(replace_response.status(), StatusCode::OK);
    assert_eq!(
        store
            .resolve_subject_user_id(
                &SlackChannelRouteKey::new(
                    tenant_id,
                    installation_id,
                    TEAM.to_string(),
                    "C0RAW".to_string(),
                )
                .expect("raw key"),
            )
            .await
            .expect("resolve raw route"),
        None
    );
}

#[tokio::test]
async fn allowed_channel_admin_preserves_existing_unmanaged_subject_for_same_channel() {
    let store = Arc::new(InMemorySlackChannelRouteStore::new());
    let mount = slack_channel_route_admin_route_mount(route_config(store.clone()));
    let tenant_id = TenantId::new(TENANT).expect("tenant");
    let installation_id = AdapterInstallationId::new(INSTALLATION).expect("installation");
    store
        .upsert_route(
            SlackChannelRouteKey::new(
                tenant_id,
                installation_id,
                TEAM.to_string(),
                "C0RAW".to_string(),
            )
            .expect("raw key"),
            UserId::new("user:raw-route-subject").expect("raw subject"),
        )
        .await
        .expect("seed raw route");

    let response = mount
            .protected
            .oneshot(request(
                "PUT",
                r#"{"channels":[{"channel_id":"C0RAW","subject_user_id":"user:raw-route-subject"},{"channel_id":"C0ENG","subject_user_id":"user:eng-team-agent"}]}"#,
                TENANT,
            ))
            .await
            .expect("save responds");

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(
        body["channels"],
        serde_json::json!([
            {
                "channel_id": "C0ENG",
                "subject_user_id": "user:eng-team-agent",
                "subject_display_name": "Eng"
            },
            {
                "channel_id": "C0RAW",
                "subject_user_id": "user:raw-route-subject",
                "subject_display_name": "Raw Route Subject"
            }
        ])
    );
}

#[tokio::test]
async fn allowed_channel_admin_generates_only_missing_explicit_subjects() {
    let store = Arc::new(InMemorySlackChannelRouteStore::new());
    let mount = slack_channel_route_admin_route_mount(route_config(store.clone()));
    let tenant_id = TenantId::new(TENANT).expect("tenant");
    let installation_id = AdapterInstallationId::new(INSTALLATION).expect("installation");
    store
        .upsert_route(
            SlackChannelRouteKey::new(
                tenant_id,
                installation_id,
                TEAM.to_string(),
                "C0RAW".to_string(),
            )
            .expect("raw key"),
            UserId::new("user:raw-route-subject").expect("raw subject"),
        )
        .await
        .expect("seed raw route");

    let response = mount
            .protected
            .oneshot(request(
                "PUT",
                r#"{"channels":[{"channel_id":"C0RAW","subject_user_id":"user:raw-route-subject"},{"channel_id":"C0NEW"}]}"#,
                TENANT,
            ))
            .await
            .expect("save responds");

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(
        body["channels"][1],
        serde_json::json!({
            "channel_id": "C0RAW",
            "subject_user_id": "user:raw-route-subject",
            "subject_display_name": "Raw Route Subject"
        })
    );
    let generated_subject = body["channels"][0]["subject_user_id"]
        .as_str()
        .expect("generated subject");
    assert_eq!(body["channels"][0]["channel_id"], "C0NEW");
    assert_ne!(generated_subject, "user:raw-route-subject");
    assert!(
        generated_subject.starts_with("user:slack-channel:"),
        "{generated_subject}"
    );
}

#[tokio::test]
async fn allowed_channel_admin_rejects_existing_unmanaged_subject_for_other_channel() {
    let store = Arc::new(InMemorySlackChannelRouteStore::new());
    let mount = slack_channel_route_admin_route_mount(route_config(store.clone()));
    let tenant_id = TenantId::new(TENANT).expect("tenant");
    let installation_id = AdapterInstallationId::new(INSTALLATION).expect("installation");
    let raw_key = SlackChannelRouteKey::new(
        tenant_id.clone(),
        installation_id.clone(),
        TEAM.to_string(),
        "C0RAW".to_string(),
    )
    .expect("raw key");
    store
        .upsert_route(
            raw_key.clone(),
            UserId::new("user:raw-route-subject").expect("raw subject"),
        )
        .await
        .expect("seed raw route");

    let response = mount
        .protected
        .oneshot(request(
            "PUT",
            r#"{"channels":[{"channel_id":"C0ENG","subject_user_id":"user:raw-route-subject"}]}"#,
            TENANT,
        ))
        .await
        .expect("save responds");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        store
            .resolve_subject_user_id(&raw_key)
            .await
            .expect("resolve raw route"),
        Some(UserId::new("user:raw-route-subject").expect("raw subject"))
    );
}

#[tokio::test]
async fn allowed_channel_admin_assigns_distinct_subjects_for_same_channel_across_scopes() {
    let store = Arc::new(InMemorySlackChannelRouteStore::new());
    let base = slack_channel_route_admin_route_mount(route_config(store.clone()));
    let other_tenant = slack_channel_route_admin_route_mount(route_config_for(
        store.clone(),
        "tenant:other-slack-routes",
        TEAM,
    ));
    let other_team =
        slack_channel_route_admin_route_mount(route_config_for(store, TENANT, "T0OTHER"));

    let base_subject = save_single_channel_subject(&base, TENANT, "C0SHARED").await;
    let other_tenant_subject =
        save_single_channel_subject(&other_tenant, "tenant:other-slack-routes", "C0SHARED").await;
    let other_team_subject = save_single_channel_subject(&other_team, TENANT, "C0SHARED").await;

    assert_ne!(base_subject, other_tenant_subject);
    assert_ne!(base_subject, other_team_subject);
    assert_ne!(other_tenant_subject, other_team_subject);
}

#[tokio::test]
async fn allowed_channel_admin_rejects_invalid_channel_without_mutating_store() {
    let store = Arc::new(InMemorySlackChannelRouteStore::new());
    let mount = slack_channel_route_admin_route_mount(route_config(store.clone()));

    let response = mount
        .protected
        .oneshot(request("PUT", r#"{"channel_ids":["C0ENG",""]}"#, TENANT))
        .await
        .expect("save responds");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        store
            .list_routes(
                &TenantId::new(TENANT).expect("tenant"),
                &AdapterInstallationId::new(INSTALLATION).expect("installation"),
                TEAM,
                0,
                DEFAULT_LIST_LIMIT,
            )
            .await
            .expect("routes list")
            .routes
            .is_empty()
    );
}

#[tokio::test]
async fn allowed_channel_admin_rejects_more_than_max_allowed_channels_without_mutating_store() {
    let store = Arc::new(InMemorySlackChannelRouteStore::new());
    let mount = slack_channel_route_admin_route_mount(route_config(store.clone()));
    let channel_ids = (0..=MAX_ALLOWED_CHANNELS)
        .map(|index| format!("C{index:08}"))
        .collect::<Vec<_>>();
    let body = serde_json::json!({ "channel_ids": channel_ids }).to_string();

    let response = mount
        .protected
        .oneshot(request("PUT", &body, TENANT))
        .await
        .expect("save responds");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        store
            .list_routes(
                &TenantId::new(TENANT).expect("tenant"),
                &AdapterInstallationId::new(INSTALLATION).expect("installation"),
                TEAM,
                0,
                DEFAULT_LIST_LIMIT,
            )
            .await
            .expect("routes list")
            .routes
            .is_empty()
    );
}

#[tokio::test]
async fn allowed_channel_admin_rejects_cross_tenant_and_non_operator_callers() {
    let store = Arc::new(InMemorySlackChannelRouteStore::new());
    let mount = slack_channel_route_admin_route_mount(route_config(store));

    for (method, body, tenant_id, user_id, expected_status) in [
        (
            "GET",
            "",
            "tenant:other",
            "user:admin",
            StatusCode::NOT_FOUND,
        ),
        (
            "PUT",
            r#"{"channel_ids":["C0ENG"]}"#,
            "tenant:other",
            "user:admin",
            StatusCode::NOT_FOUND,
        ),
        ("GET", "", TENANT, "user:not-admin", StatusCode::FORBIDDEN),
        (
            "PUT",
            r#"{"channel_ids":["C0ENG"]}"#,
            TENANT,
            "user:not-admin",
            StatusCode::FORBIDDEN,
        ),
    ] {
        let response = mount
            .protected
            .clone()
            .oneshot(request_for_caller(method, body, tenant_id, user_id))
            .await
            .expect("route responds");
        assert_eq!(response.status(), expected_status, "{method} {user_id}");
    }
}

#[tokio::test]
async fn allowed_channel_admin_returns_503_when_store_unavailable() {
    let mount =
        slack_channel_route_admin_route_mount(route_config(Arc::new(UnavailableAllowedRouteStore)));

    for (method, body) in [("GET", ""), ("PUT", r#"{"channel_ids":["C0ENG"]}"#)] {
        let response = mount
            .protected
            .clone()
            .oneshot(request(method, body, TENANT))
            .await
            .expect("route responds");
        assert_eq!(
            response.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "{method}"
        );
    }
}

#[tokio::test]
async fn allowed_channel_admin_empty_save_clears_all_channel_routes() {
    let store = Arc::new(InMemorySlackChannelRouteStore::new());
    let mount = slack_channel_route_admin_route_mount(route_config(store.clone()));

    let seed = mount
        .protected
        .clone()
        .oneshot(request(
            "PUT",
            r#"{"channel_ids":["C0OPS","C0ENG"]}"#,
            TENANT,
        ))
        .await
        .expect("seed responds");
    assert_eq!(seed.status(), StatusCode::OK);

    let clear = mount
        .protected
        .oneshot(request("PUT", r#"{"channel_ids":[]}"#, TENANT))
        .await
        .expect("clear responds");
    assert_eq!(clear.status(), StatusCode::OK);
    let body = axum::body::to_bytes(clear.into_body(), 64 * 1024)
        .await
        .expect("body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(body["channels"], serde_json::json!([]));
    assert!(
        store
            .list_routes(
                &TenantId::new(TENANT).expect("tenant"),
                &AdapterInstallationId::new(INSTALLATION).expect("installation"),
                TEAM,
                0,
                DEFAULT_LIST_LIMIT,
            )
            .await
            .expect("routes list")
            .routes
            .is_empty()
    );
}

fn route_config(store: Arc<dyn SlackChannelRouteStore>) -> SlackChannelRouteAdminRouteConfig {
    route_config_for(store, TENANT, TEAM)
}

fn route_config_for(
    store: Arc<dyn SlackChannelRouteStore>,
    tenant_id: &str,
    team_id: &str,
) -> SlackChannelRouteAdminRouteConfig {
    SlackChannelRouteAdminRouteConfig::new(
        TenantId::new(tenant_id).expect("tenant"),
        AdapterInstallationId::new(INSTALLATION).expect("installation"),
        team_id.to_string(),
        UserId::new("user:admin").expect("operator user"),
        store,
    )
    .with_allowed_subject_user_ids([UserId::new("user:eng-team-agent").expect("subject user")])
}

async fn save_single_channel_subject(
    mount: &crate::slack_channel_routes::SlackChannelRouteAdminRouteMount,
    tenant_id: &str,
    channel_id: &str,
) -> String {
    let response = mount
        .protected
        .clone()
        .oneshot(request(
            "PUT",
            &serde_json::json!({ "channel_ids": [channel_id] }).to_string(),
            tenant_id,
        ))
        .await
        .expect("save responds");
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("json");
    body["channels"][0]["subject_user_id"]
        .as_str()
        .expect("subject")
        .to_string()
}

fn request(method: &str, body: &str, tenant_id: &str) -> Request<Body> {
    request_for_caller(method, body, tenant_id, "user:admin")
}

fn request_for_caller(method: &str, body: &str, tenant_id: &str, user_id: &str) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(WEBUI_V2_CHANNELS_SLACK_ALLOWED_PATH)
        .header("content-type", "application/json")
        .extension(WebUiAuthenticatedCaller {
            tenant_id: TenantId::new(tenant_id).expect("tenant"),
            user_id: UserId::new(user_id).expect("user"),
            agent_id: None,
            project_id: None,
        });
    if method == "GET" {
        builder = builder.header("content-length", "0");
    }
    builder
        .body(Body::from(body.to_string()))
        .expect("request builds")
}

#[derive(Debug)]
struct UnavailableAllowedRouteStore;

#[async_trait::async_trait]
impl SlackChannelRouteStore for UnavailableAllowedRouteStore {
    async fn list_routes(
        &self,
        _tenant_id: &TenantId,
        _installation_id: &AdapterInstallationId,
        _team_id: &str,
        _cursor: usize,
        _limit: usize,
    ) -> Result<SlackChannelRouteListPage, SlackChannelRouteError> {
        Err(SlackChannelRouteError::StoreUnavailable)
    }

    async fn upsert_route(
        &self,
        _key: SlackChannelRouteKey,
        _subject_user_id: UserId,
    ) -> Result<SlackChannelRoute, SlackChannelRouteError> {
        Err(SlackChannelRouteError::StoreUnavailable)
    }

    async fn delete_route(
        &self,
        _key: &SlackChannelRouteKey,
    ) -> Result<bool, SlackChannelRouteError> {
        Err(SlackChannelRouteError::StoreUnavailable)
    }

    async fn replace_managed_routes(
        &self,
        _tenant_id: &TenantId,
        _installation_id: &AdapterInstallationId,
        _team_id: &str,
        _assignments: Vec<crate::slack_channel_routes::SlackChannelRouteAssignment>,
    ) -> Result<Vec<SlackChannelRoute>, SlackChannelRouteError> {
        Err(SlackChannelRouteError::StoreUnavailable)
    }

    async fn resolve_subject_user_id(
        &self,
        _key: &SlackChannelRouteKey,
    ) -> Result<Option<UserId>, SlackChannelRouteError> {
        Err(SlackChannelRouteError::StoreUnavailable)
    }
}
