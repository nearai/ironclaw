#![cfg(feature = "slack-v2-host-beta")]
//! Slack host-beta routes composed into the (host-owned) `webui_v2_app`.
//!
//! `webui_v2_app` was hoisted out of `ironclaw_reborn_composition` into this
//! ingress crate during the WebUI host-stack merge, so these tests — which drive
//! Slack channel-route admin + connectable-channel surfaces through the fully
//! composed app over HTTP — live here, where `webui_v2_app` is a crate-local
//! symbol and composition is a normal dependency (single crate copy, no
//! dev-dependency cycle). They build a local-dev runtime and mount the Slack
//! host-beta surface through composition's public builders, exactly as
//! `ironclaw-reborn serve` does. Relocated from
//! `ironclaw_reborn_composition::slack::slack_host_beta` unit tests.

use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_loop_host::{
    HostManagedModelError, HostManagedModelGateway, HostManagedModelRequest,
    HostManagedModelResponse,
};
use ironclaw_product_workflow::{RebornChannelConnectStrategy, WebUiAuthenticatedCaller};
use ironclaw_reborn_composition::{
    RebornBuildInput, RebornRuntime, RebornRuntimeIdentity, RebornRuntimeInput,
    SlackHostBetaConfig, SlackHostBetaConfigInput, SlackOperatorRouteVisibility, SlackTeamId,
    WEBUI_V2_CHANNELS_SLACK_ALLOWED_PATH, WEBUI_V2_CHANNELS_SLACK_ROUTES_PATH,
    build_reborn_runtime, build_slack_host_beta_mounts,
    build_webui_services_with_slack_host_beta_mounts, local_dev_runtime_policy,
};
use ironclaw_turns::run_profile::LoopCapabilityPort;
use ironclaw_webui::{WebuiAuthentication, WebuiAuthenticator, WebuiServeConfig, webui_v2_app};
use secrecy::SecretString;
use tower::ServiceExt;

const TENANT: &str = "tenant:slack-host";
const AGENT: &str = "agent:slack-host";
const PROJECT: &str = "project:slack-host";
const USER: &str = "user:slack-host";
const SHARED_SUBJECT: &str = "user:slack-shared-subject";
const INSTALLATION: &str = "install_host_beta";
const TEAM: &str = "T0HOST";
const API_APP: &str = "A0HOST";
const SECRET: &str = "host-signing-secret";

#[derive(Debug)]
struct StaticGateway;

#[async_trait]
impl HostManagedModelGateway for StaticGateway {
    async fn stream_model(
        &self,
        _request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        Ok(HostManagedModelResponse::assistant_reply("ok"))
    }

    async fn stream_model_with_capabilities(
        &self,
        request: HostManagedModelRequest,
        _capabilities: Arc<dyn LoopCapabilityPort>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.stream_model(request).await
    }
}

async fn runtime() -> (RebornRuntime, tempfile::TempDir) {
    let root = tempfile::tempdir().expect("tempdir");
    let build_input = RebornBuildInput::local_dev(USER, root.path().join("local-dev"))
        .with_runtime_policy(local_dev_runtime_policy().expect("local policy"));
    let runtime = build_reborn_runtime(
        RebornRuntimeInput::from_services(build_input)
            .with_identity(RebornRuntimeIdentity {
                tenant_id: TENANT.to_string(),
                agent_id: AGENT.to_string(),
                source_binding_id: "slack-host-source".to_string(),
                reply_target_binding_id: "slack-host-reply".to_string(),
            })
            .with_default_project_id(ProjectId::new(PROJECT).expect("project"))
            .with_model_gateway_override(Arc::new(StaticGateway)),
    )
    .await
    .expect("runtime builds");
    (runtime, root)
}

struct OperatorTokenAuthenticator;

#[async_trait]
impl WebuiAuthenticator for OperatorTokenAuthenticator {
    async fn authenticate(&self, token: &str) -> Option<WebuiAuthentication> {
        if token == "operator-token" {
            Some(WebuiAuthentication::operator(
                UserId::new(USER).expect("user"),
            ))
        } else {
            None
        }
    }

    fn mounts_operator_webui_config_routes(&self) -> bool {
        true
    }
}

struct MixedSessionAndOperatorAuthenticator;

#[async_trait]
impl WebuiAuthenticator for MixedSessionAndOperatorAuthenticator {
    async fn authenticate(&self, token: &str) -> Option<WebuiAuthentication> {
        match token {
            "session-token" => Some(WebuiAuthentication::user(UserId::new(USER).expect("user"))),
            "operator-token" => Some(WebuiAuthentication::operator(
                UserId::new(USER).expect("user"),
            )),
            _ => None,
        }
    }

    fn mounts_operator_webui_config_routes(&self) -> bool {
        true
    }
}

struct HiddenOperatorRouteAuthenticator;

#[async_trait]
impl WebuiAuthenticator for HiddenOperatorRouteAuthenticator {
    async fn authenticate(&self, token: &str) -> Option<WebuiAuthentication> {
        if token == "operator-token" {
            Some(WebuiAuthentication::operator(
                UserId::new(USER).expect("user"),
            ))
        } else {
            None
        }
    }
}

fn config_without_channel_routes() -> SlackHostBetaConfig {
    SlackHostBetaConfig::new(SlackHostBetaConfigInput {
        tenant_id: TenantId::new(TENANT).expect("tenant"),
        agent_id: AgentId::new(AGENT).expect("agent"),
        project_id: Some(ProjectId::new(PROJECT).expect("project")),
        installation_id: INSTALLATION.to_string(),
        team_id: SlackTeamId::new(TEAM),
        api_app_id: Some(API_APP.to_string()),
        user_id: UserId::new(USER).expect("user"),
        shared_subject_user_id: Some(UserId::new(SHARED_SUBJECT).expect("shared subject")),
        channel_routes: Vec::new(),
        signing_secret: SecretString::from(SECRET),
        bot_token: SecretString::from("xoxb-host-token"),
    })
    .expect("valid config")
}

#[tokio::test]
async fn slack_allowed_channels_are_reachable_through_webui_v2_app() {
    let (runtime, _root) = runtime().await;
    let mounts =
        build_slack_host_beta_mounts(&runtime, config_without_channel_routes()).expect("mounts");
    let bundle = build_webui_services_with_slack_host_beta_mounts(
        &runtime,
        None,
        Some(&mounts),
        SlackOperatorRouteVisibility::Visible,
    )
    .expect("webui bundle");
    let app = webui_v2_app(
        bundle,
        WebuiServeConfig::new(
            TenantId::new(TENANT).expect("tenant"),
            Arc::new(OperatorTokenAuthenticator),
            Vec::new(),
        )
        .with_default_agent_id(AgentId::new(AGENT).expect("agent"))
        .with_default_project_id(ProjectId::new(PROJECT).expect("project"))
        .with_slack_channel_routes(mounts.channel_routes),
    )
    .expect("webui app");

    let save = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(WEBUI_V2_CHANNELS_SLACK_ALLOWED_PATH)
                .header("authorization", "Bearer operator-token")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"channel_ids":["C0HOST","C0OPS"]}"#))
                .expect("save request builds"),
        )
        .await
        .expect("save route responds");
    assert_eq!(save.status(), StatusCode::OK);
    let save_body = axum::body::to_bytes(save.into_body(), 64 * 1024)
        .await
        .expect("save body");
    let save_body: serde_json::Value = serde_json::from_slice(&save_body).expect("save json");
    assert_eq!(save_body["channels"].as_array().expect("channels").len(), 2);
    assert_ne!(
        save_body["channels"][0]["subject_user_id"], save_body["channels"][1]["subject_user_id"],
        "allowed API should assign one tenant-scoped subject per channel"
    );

    let list = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(WEBUI_V2_CHANNELS_SLACK_ALLOWED_PATH)
                .header("authorization", "Bearer operator-token")
                .body(Body::empty())
                .expect("list request builds"),
        )
        .await
        .expect("list route responds");
    assert_eq!(list.status(), StatusCode::OK);
    let body = axum::body::to_bytes(list.into_body(), 64 * 1024)
        .await
        .expect("list body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("list json");
    assert_eq!(
        body["channels"],
        serde_json::json!([
            {
                "channel_id":"C0HOST",
                "subject_user_id": save_body["channels"][0]["subject_user_id"].clone(),
                "subject_display_name": save_body["channels"][0]["subject_display_name"].clone()
            },
            {
                "channel_id":"C0OPS",
                "subject_user_id": save_body["channels"][1]["subject_user_id"].clone(),
                "subject_display_name": save_body["channels"][1]["subject_display_name"].clone()
            }
        ])
    );

    runtime.shutdown().await.expect("runtime shuts down");
}

#[tokio::test]
async fn slack_connectable_channels_advertise_admin_action_to_operator_token() {
    let (runtime, _root) = runtime().await;
    let mounts =
        build_slack_host_beta_mounts(&runtime, config_without_channel_routes()).expect("mounts");
    let bundle = build_webui_services_with_slack_host_beta_mounts(
        &runtime,
        None,
        Some(&mounts),
        SlackOperatorRouteVisibility::Visible,
    )
    .expect("webui bundle");
    let app = webui_v2_app(
        bundle,
        WebuiServeConfig::new(
            TenantId::new(TENANT).expect("tenant"),
            Arc::new(OperatorTokenAuthenticator),
            Vec::new(),
        )
        .with_default_agent_id(AgentId::new(AGENT).expect("agent"))
        .with_default_project_id(ProjectId::new(PROJECT).expect("project"))
        .with_slack_channel_routes(mounts.channel_routes),
    )
    .expect("webui app");

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/webchat/v2/channels/connectable")
                .header("authorization", "Bearer operator-token")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("route responds");

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("response body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("response json");
    let strategies: Vec<_> = body["channels"]
        .as_array()
        .expect("channels")
        .iter()
        .map(|channel| channel["strategy"].as_str().expect("strategy"))
        .collect();
    assert!(
        strategies.contains(&"admin_managed_channels"),
        "operator token should see Slack admin channel setup: {body}"
    );
    assert!(
        strategies.contains(&"oauth"),
        "operator token should see personal Slack OAuth connection: {body}"
    );

    runtime.shutdown().await.expect("runtime shuts down");
}

#[tokio::test]
async fn slack_connectable_channels_hide_admin_action_from_sso_session_token() {
    let (runtime, _root) = runtime().await;
    let mounts =
        build_slack_host_beta_mounts(&runtime, config_without_channel_routes()).expect("mounts");
    let bundle = build_webui_services_with_slack_host_beta_mounts(
        &runtime,
        None,
        Some(&mounts),
        SlackOperatorRouteVisibility::Visible,
    )
    .expect("webui bundle");
    let app = webui_v2_app(
        bundle,
        WebuiServeConfig::new(
            TenantId::new(TENANT).expect("tenant"),
            Arc::new(MixedSessionAndOperatorAuthenticator),
            Vec::new(),
        )
        .with_default_agent_id(AgentId::new(AGENT).expect("agent"))
        .with_default_project_id(ProjectId::new(PROJECT).expect("project"))
        .with_slack_channel_routes(mounts.channel_routes),
    )
    .expect("webui app");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/webchat/v2/channels/connectable")
                .header("authorization", "Bearer session-token")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("route responds");

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("response body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("response json");
    let session_strategies: Vec<_> = body["channels"]
        .as_array()
        .expect("channels")
        .iter()
        .map(|channel| channel["strategy"].as_str().expect("strategy"))
        .collect();
    assert!(
        !session_strategies.contains(&"admin_managed_channels"),
        "SSO session token should not see Slack admin setup: {body}"
    );
    assert!(
        session_strategies.contains(&"oauth"),
        "SSO session token should see personal Slack OAuth connection: {body}"
    );

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/webchat/v2/channels/connectable")
                .header("authorization", "Bearer operator-token")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("route responds");

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("response body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("response json");
    let operator_strategies: Vec<_> = body["channels"]
        .as_array()
        .expect("channels")
        .iter()
        .map(|channel| channel["strategy"].as_str().expect("strategy"))
        .collect();
    assert!(
        operator_strategies.contains(&"admin_managed_channels"),
        "operator token should see Slack admin setup: {body}"
    );

    runtime.shutdown().await.expect("runtime shuts down");
}

#[tokio::test]
async fn slack_channel_route_admin_is_reachable_through_webui_v2_app() {
    let (runtime, _root) = runtime().await;
    let mounts =
        build_slack_host_beta_mounts(&runtime, config_without_channel_routes()).expect("mounts");
    let bundle = build_webui_services_with_slack_host_beta_mounts(
        &runtime,
        None,
        Some(&mounts),
        SlackOperatorRouteVisibility::Visible,
    )
    .expect("webui bundle");
    let app = webui_v2_app(
        bundle,
        WebuiServeConfig::new(
            TenantId::new(TENANT).expect("tenant"),
            Arc::new(OperatorTokenAuthenticator),
            Vec::new(),
        )
        .with_default_agent_id(AgentId::new(AGENT).expect("agent"))
        .with_default_project_id(ProjectId::new(PROJECT).expect("project"))
        .with_slack_channel_routes(mounts.channel_routes),
    )
    .expect("webui app");

    let unauthenticated = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(WEBUI_V2_CHANNELS_SLACK_ROUTES_PATH)
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    r#"{{"channel_id":"C0HOST","subject_user_id":"{SHARED_SUBJECT}"}}"#
                )))
                .expect("unauthenticated request builds"),
        )
        .await
        .expect("unauthenticated route responds");
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);

    let empty_list = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(WEBUI_V2_CHANNELS_SLACK_ROUTES_PATH)
                .header("authorization", "Bearer operator-token")
                .body(Body::empty())
                .expect("empty list request builds"),
        )
        .await
        .expect("empty list route responds");
    assert_eq!(empty_list.status(), StatusCode::OK);
    let body = axum::body::to_bytes(empty_list.into_body(), 64 * 1024)
        .await
        .expect("empty list body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("empty list json");
    assert_eq!(body["routes"], serde_json::json!([]));
    assert_eq!(body["next_cursor"], serde_json::Value::Null);

    let upsert = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(WEBUI_V2_CHANNELS_SLACK_ROUTES_PATH)
                .header("authorization", "Bearer operator-token")
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    r#"{{"channel_id":"C0HOST","subject_user_id":"{SHARED_SUBJECT}"}}"#
                )))
                .expect("upsert request builds"),
        )
        .await
        .expect("upsert route responds");
    assert_eq!(upsert.status(), StatusCode::OK);

    let list = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(WEBUI_V2_CHANNELS_SLACK_ROUTES_PATH)
                .header("authorization", "Bearer operator-token")
                .body(Body::empty())
                .expect("list request builds"),
        )
        .await
        .expect("list route responds");
    assert_eq!(list.status(), StatusCode::OK);
    let body = axum::body::to_bytes(list.into_body(), 64 * 1024)
        .await
        .expect("list body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("list json");
    assert_eq!(body["routes"][0]["channel_id"], "C0HOST");
    assert_eq!(body["routes"][0]["subject_user_id"], SHARED_SUBJECT);

    let delete = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(WEBUI_V2_CHANNELS_SLACK_ROUTES_PATH)
                .header("authorization", "Bearer operator-token")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"channel_id":"C0HOST"}"#))
                .expect("delete request builds"),
        )
        .await
        .expect("delete route responds");
    assert_eq!(delete.status(), StatusCode::OK);
    let body = axum::body::to_bytes(delete.into_body(), 64 * 1024)
        .await
        .expect("delete body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("delete json");
    assert_eq!(body["deleted"], true);

    let list_after_delete = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(WEBUI_V2_CHANNELS_SLACK_ROUTES_PATH)
                .header("authorization", "Bearer operator-token")
                .body(Body::empty())
                .expect("list request builds"),
        )
        .await
        .expect("list route responds");
    assert_eq!(list_after_delete.status(), StatusCode::OK);
    let body = axum::body::to_bytes(list_after_delete.into_body(), 64 * 1024)
        .await
        .expect("list body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("list json");
    assert_eq!(body["routes"], serde_json::json!([]));

    runtime.shutdown().await.expect("runtime shuts down");
}

#[tokio::test]
async fn slack_channel_routes_mount_for_sso_operator_authenticator() {
    let (runtime, _root) = runtime().await;
    let mounts =
        build_slack_host_beta_mounts(&runtime, config_without_channel_routes()).expect("mounts");
    let bundle = build_webui_services_with_slack_host_beta_mounts(
        &runtime,
        None,
        Some(&mounts),
        SlackOperatorRouteVisibility::Hidden,
    )
    .expect("webui bundle");
    let caller = WebUiAuthenticatedCaller::new(
        TenantId::new(TENANT).expect("tenant"),
        UserId::new(USER).expect("user"),
        Some(AgentId::new(AGENT).expect("agent")),
        Some(ProjectId::new(PROJECT).expect("project")),
    );
    let connectable = bundle
        .api
        .list_connectable_channels(caller)
        .await
        .expect("connectable channels");
    assert!(
        connectable
            .channels
            .iter()
            .any(|channel| channel.strategy == RebornChannelConnectStrategy::OAuth),
        "non-operator WebUI should still advertise personal Slack OAuth"
    );
    assert!(
        connectable
            .channels
            .iter()
            .all(|channel| channel.strategy != RebornChannelConnectStrategy::AdminManagedChannels),
        "non-operator WebUI must not advertise Slack admin channel management"
    );
    let app = webui_v2_app(
        bundle,
        WebuiServeConfig::new(
            TenantId::new(TENANT).expect("tenant"),
            Arc::new(OperatorTokenAuthenticator),
            Vec::new(),
        )
        .with_default_agent_id(AgentId::new(AGENT).expect("agent"))
        .with_default_project_id(ProjectId::new(PROJECT).expect("project"))
        .with_slack_channel_routes(mounts.channel_routes),
    )
    .expect("webui app");

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(WEBUI_V2_CHANNELS_SLACK_ROUTES_PATH)
                .header("authorization", "Bearer operator-token")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("route responds");
    assert_eq!(response.status(), StatusCode::OK);

    runtime.shutdown().await.expect("runtime shuts down");
}

#[tokio::test]
async fn slack_channel_routes_not_mounted_when_operator_route_visibility_is_hidden() {
    let (runtime, _root) = runtime().await;
    let mounts =
        build_slack_host_beta_mounts(&runtime, config_without_channel_routes()).expect("mounts");
    let bundle = build_webui_services_with_slack_host_beta_mounts(
        &runtime,
        None,
        Some(&mounts),
        SlackOperatorRouteVisibility::Hidden,
    )
    .expect("webui bundle");
    let app = webui_v2_app(
        bundle,
        WebuiServeConfig::new(
            TenantId::new(TENANT).expect("tenant"),
            Arc::new(HiddenOperatorRouteAuthenticator),
            Vec::new(),
        )
        .with_default_agent_id(AgentId::new(AGENT).expect("agent"))
        .with_default_project_id(ProjectId::new(PROJECT).expect("project"))
        .with_slack_channel_routes(mounts.channel_routes),
    )
    .expect("webui app");

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(WEBUI_V2_CHANNELS_SLACK_ROUTES_PATH)
                .header("authorization", "Bearer operator-token")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("route responds");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    runtime.shutdown().await.expect("runtime shuts down");
}

#[tokio::test]
async fn slack_host_beta_admin_routes_feed_outbound_target_provider() {
    let (runtime, _root) = runtime().await;
    let mounts =
        build_slack_host_beta_mounts(&runtime, config_without_channel_routes()).expect("mounts");
    let bundle = build_webui_services_with_slack_host_beta_mounts(
        &runtime,
        None,
        Some(&mounts),
        SlackOperatorRouteVisibility::Visible,
    )
    .expect("webui bundle");
    let app = webui_v2_app(
        bundle.clone(),
        WebuiServeConfig::new(
            TenantId::new(TENANT).expect("tenant"),
            Arc::new(OperatorTokenAuthenticator),
            Vec::new(),
        )
        .with_default_agent_id(AgentId::new(AGENT).expect("agent"))
        .with_default_project_id(ProjectId::new(PROJECT).expect("project"))
        .with_slack_channel_routes(mounts.channel_routes),
    )
    .expect("webui app");

    let save = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(WEBUI_V2_CHANNELS_SLACK_ALLOWED_PATH)
                .header("authorization", "Bearer operator-token")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"channel_ids":["C0DYNAMIC"]}"#))
                .expect("save request builds"),
        )
        .await
        .expect("save route responds");
    assert_eq!(save.status(), StatusCode::OK);
    let body = axum::body::to_bytes(save.into_body(), 64 * 1024)
        .await
        .expect("save body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("save json");
    let subject_user_id = body["channels"][0]["subject_user_id"]
        .as_str()
        .expect("assigned subject");
    let caller = WebUiAuthenticatedCaller::new(
        TenantId::new(TENANT).expect("tenant"),
        UserId::new(subject_user_id).expect("subject user"),
        Some(AgentId::new(AGENT).expect("agent")),
        Some(ProjectId::new(PROJECT).expect("project")),
    );

    let targets = bundle
        .api
        .list_outbound_delivery_targets(caller)
        .await
        .expect("dynamic route target list");

    assert_eq!(targets.targets.len(), 1);
    assert_eq!(
        targets.targets[0].target.target_id.as_str(),
        "slack:shared-channel:T0HOST:C0DYNAMIC"
    );
    assert_eq!(
        targets.targets[0].target.display_name.as_str(),
        "Slack channel C0DYNAMIC"
    );

    runtime.shutdown().await.expect("runtime shuts down");
}
