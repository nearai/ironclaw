use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use hmac::{Hmac, Mac};
use http_body_util::BodyExt;
use ironclaw_host_api::{RuntimeHttpEgress, RuntimeHttpEgressRequest, RuntimeHttpEgressResponse};
use ironclaw_host_runtime::HostRuntimeHttpEgressPort;
use ironclaw_loop_support::{
    HostManagedModelError, HostManagedModelGateway, HostManagedModelRequest,
    HostManagedModelResponse,
};
use ironclaw_product_workflow::{
    ProductActorUserResolutionRequest, ProductWorkflowError, WebUiAuthenticatedCaller,
};
use ironclaw_threads::{ListThreadsForScopeRequest, ThreadHistoryRequest, ThreadScope};
use ironclaw_turns::run_profile::LoopCapabilityPort;
use secrecy::ExposeSecret;
use tower::ServiceExt;

use super::*;
use crate::slack_personal_binding_pairing_serve::{
    WEBUI_V2_EXTENSION_PAIRING_REDEEM_PATH, slack_personal_binding_pairing_route_mount,
};
use crate::{
    RebornBuildInput, RebornRuntimeIdentity, RebornRuntimeInput, SLACK_EVENTS_PATH,
    build_reborn_runtime, local_dev_runtime_policy,
};

const TENANT: &str = "tenant:slack-host";
const AGENT: &str = "agent:slack-host";
const PROJECT: &str = "project:slack-host";
const USER: &str = "user:slack-host";
const INSTALLATION: &str = "install_host_beta";
const TEAM: &str = "THOST";
const API_APP: &str = "AHOST";
const SLACK_USER: &str = "UHOST";
const SECRET: &str = "host-signing-secret";

type HmacSha256 = Hmac<sha2::Sha256>;

#[tokio::test]
async fn build_slack_events_route_mount_builds_signed_route_from_reborn_runtime() {
    let (runtime, _root) = runtime().await;

    let mount = build_slack_events_route_mount(&runtime, config())
        .await
        .expect("route builds");
    assert_eq!(mount.descriptors.len(), 1);
    assert!(mount.drain.is_some());

    let body = r#"{"type":"url_verification","challenge":"reborn-slack-ok"}"#;
    let timestamp = current_unix_timestamp();
    let response = mount
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(SLACK_EVENTS_PATH)
                .header(SLACK_TIMESTAMP_HEADER, timestamp.to_string())
                .header(SLACK_SIGNATURE_HEADER, slack_signature(timestamp, body))
                .body(Body::from(body))
                .expect("request builds"),
        )
        .await
        .expect("router responds");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body collects")
        .to_bytes();
    assert!(String::from_utf8_lossy(&bytes).contains("reborn-slack-ok"));

    runtime.shutdown().await.expect("runtime shuts down");
}

#[tokio::test]
async fn custom_actor_user_resolver_routes_inbound_slack_event() {
    let (runtime, _root) = runtime().await;
    let resolver = Arc::new(RecordingProductActorUserResolver::new(
        UserId::new(USER).expect("user"),
    ));
    let mount = build_slack_events_route_mount_with_actor_user_resolver(
        &runtime,
        config(),
        resolver.clone(),
    )
    .await
    .expect("route builds");

    let body = dm_event_body();
    let timestamp = current_unix_timestamp();
    let response = mount
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(SLACK_EVENTS_PATH)
                .header(SLACK_TIMESTAMP_HEADER, timestamp.to_string())
                .header(SLACK_SIGNATURE_HEADER, slack_signature(timestamp, body))
                .body(Body::from(body))
                .expect("request builds"),
        )
        .await
        .expect("router responds");

    assert_eq!(response.status(), StatusCode::OK);
    let calls = wait_for_resolver_calls(&resolver, 1).await;
    assert!(!calls.is_empty());
    assert_eq!(calls[0].adapter_id.as_str(), SLACK_V2_ADAPTER_ID);
    assert_eq!(calls[0].installation_id.as_str(), INSTALLATION);
    assert_eq!(calls[0].external_actor_ref.kind(), SLACK_USER_ACTOR_KIND);
    assert_eq!(calls[0].external_actor_ref.id(), SLACK_USER);

    runtime.shutdown().await.expect("runtime shuts down");
}

#[tokio::test]
async fn build_slack_events_route_mount_fails_when_runtime_http_egress_unavailable() {
    let (runtime, _root) = runtime_without_host_egress().await;

    let error = match build_slack_events_route_mount(&runtime, config()).await {
        Ok(_) => panic!("Slack route requires runtime HTTP egress"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        SlackHostBetaBuildError::RuntimeHttpEgressUnavailable
    ));
    runtime.shutdown().await.expect("runtime shuts down");
}

#[tokio::test]
async fn build_slack_host_beta_mounts_fails_when_durable_host_state_unavailable() {
    let (mut runtime, _root) = runtime().await;
    runtime.clear_local_runtime_for_test();

    let error = match build_slack_host_beta_mounts(&runtime, config()).await {
        Ok(_) => panic!("Slack host-beta route requires durable host state"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        SlackHostBetaBuildError::DurableHostStateUnavailable
    ));
    runtime.shutdown().await.expect("runtime shuts down");
}

#[tokio::test]
async fn build_slack_events_route_mount_dispatches_signed_event_callback() {
    let egress = Arc::new(RecordingRuntimeHttpEgress::default());
    let (runtime, _root) = runtime_with_recording_egress(egress.clone()).await;
    let mount = build_slack_events_route_mount(&runtime, config())
        .await
        .expect("route builds");
    let body = r#"{
            "type":"event_callback",
            "team_id":"THOST",
            "api_app_id":"AHOST",
            "event_id":"Ev-host-beta-dispatch",
            "event":{"type":"message","channel_type":"im","user":"UHOST","channel":"DHOST","text":"hello","ts":"1710000000.000010"}
        }"#;
    let timestamp = current_unix_timestamp();

    let response = mount
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(SLACK_EVENTS_PATH)
                .header(SLACK_TIMESTAMP_HEADER, timestamp.to_string())
                .header(SLACK_SIGNATURE_HEADER, slack_signature(timestamp, body))
                .body(Body::from(body))
                .expect("request builds"),
        )
        .await
        .expect("router responds");

    assert_eq!(response.status(), StatusCode::OK);
    if let Some(drain) = mount.drain.as_ref() {
        drain.drain().await;
    }
    let history = wait_for_slack_thread_history_with_messages(&runtime, 2).await;
    assert_eq!(history.messages.len(), 2);
    assert_eq!(history.messages[0].content.as_deref(), Some("hello"));
    assert_eq!(history.messages[1].content.as_deref(), Some("ok"));
    assert_eq!(
        history.messages[0].source_binding_id.as_deref(),
        Some(
            "adapter:8:slack_v2;installation:17:install_host_beta;agent:16:agent:slack-host;project:18:project:slack-host;space:5:THOST;conversation:5:DHOST;topic:0:;"
        )
    );
    wait_for_slack_posted_text(&egress, "ok").await;

    runtime.shutdown().await.expect("runtime shuts down");
}

#[tokio::test]
async fn duplicate_slack_event_replays_after_runtime_reopen_without_duplicate_reply() {
    let root = tempfile::tempdir().expect("tempdir");
    let runtime_root = root.path().join("local-dev");
    let first_egress = Arc::new(RecordingRuntimeHttpEgress::default());
    let (first_runtime, _first_root) =
        runtime_at_with_recording_egress(&runtime_root, first_egress.clone()).await;
    let first_mount = build_slack_events_route_mount(&first_runtime, config())
        .await
        .expect("first route builds");
    let body = dm_event_body_with(
        "Ev-host-beta-restart-replay",
        "durable hello",
        "1710000000.000040",
    );

    post_signed_slack_event(&first_mount, &body).await;
    if let Some(drain) = first_mount.drain.as_ref() {
        drain.drain().await;
    }
    wait_for_slack_posted_text(&first_egress, "ok").await;
    let first_history = wait_for_slack_thread_history_with_messages(&first_runtime, 2).await;
    assert_eq!(first_history.messages.len(), 2);
    first_runtime
        .shutdown()
        .await
        .expect("first runtime shuts down");

    let second_egress = Arc::new(RecordingRuntimeHttpEgress::default());
    let (second_runtime, _second_root) =
        runtime_at_with_recording_egress(&runtime_root, second_egress.clone()).await;
    let second_mount = build_slack_events_route_mount(&second_runtime, config())
        .await
        .expect("second route builds");

    post_signed_slack_event(&second_mount, &body).await;
    if let Some(drain) = second_mount.drain.as_ref() {
        drain.drain().await;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert!(
        second_egress.posted_slack_texts().is_empty(),
        "duplicate replay after restart must not post a second Slack reply"
    );
    let second_history = wait_for_slack_thread_history_with_messages(&second_runtime, 2).await;
    assert_eq!(second_history.messages.len(), 2);

    second_runtime
        .shutdown()
        .await
        .expect("second runtime shuts down");
}

#[tokio::test]
async fn build_slack_host_beta_mounts_exposes_events_and_pairing_redeem_route() {
    let root = tempfile::tempdir().expect("tempdir");
    let runtime = build_reborn_runtime(
        RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("slack-host-beta-owner", root.path().join("local-dev"))
                .with_runtime_policy(local_dev_runtime_policy().expect("local policy")),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: TENANT.to_string(),
            agent_id: AGENT.to_string(),
            source_binding_id: "slack-host-source".to_string(),
            reply_target_binding_id: "slack-host-reply".to_string(),
        })
        .with_model_gateway_override(Arc::new(StaticGateway)),
    )
    .await
    .expect("runtime builds");

    let mounts = build_slack_host_beta_mounts(&runtime, config())
        .await
        .expect("mounts build");
    let pairing_mount = slack_personal_binding_pairing_route_mount(mounts.personal_binding_pairing);

    assert_eq!(mounts.events.descriptors.len(), 1);
    assert!(
        pairing_mount
            .descriptors
            .iter()
            .any(|descriptor| descriptor.route_pattern().as_str()
                == WEBUI_V2_EXTENSION_PAIRING_REDEEM_PATH)
    );

    runtime.shutdown().await.expect("runtime shuts down");
}

#[tokio::test]
async fn build_slack_host_beta_mounts_pairs_unknown_slack_actor_then_routes_bound_event() {
    let egress = Arc::new(RecordingRuntimeHttpEgress::default());
    let (runtime, _root) = runtime_with_recording_egress(egress.clone()).await;
    let mounts = build_slack_host_beta_mounts(&runtime, config_without_legacy_actor())
        .await
        .expect("mounts");

    let first_body =
        dm_event_body_with("Ev-host-beta-pairing-first", "pair me", "1710000000.000020");
    post_signed_slack_event(&mounts.events, &first_body).await;
    if let Some(drain) = mounts.events.drain.as_ref() {
        drain.drain().await;
    }
    let pairing_code = wait_for_pairing_code(&egress).await;

    let pairing_mount = slack_personal_binding_pairing_route_mount(mounts.personal_binding_pairing);
    let redeem_body = format!(r#"{{"channel":"slack","code":"{pairing_code}"}}"#);
    let redeem_response = pairing_mount
        .protected
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(WEBUI_V2_EXTENSION_PAIRING_REDEEM_PATH)
                .header("content-type", "application/json")
                .extension(WebUiAuthenticatedCaller {
                    tenant_id: TenantId::new(TENANT).expect("tenant"),
                    user_id: UserId::new(USER).expect("user"),
                    agent_id: Some(AgentId::new(AGENT).expect("agent")),
                    project_id: Some(ProjectId::new(PROJECT).expect("project")),
                })
                .body(Body::from(redeem_body))
                .expect("redeem request builds"),
        )
        .await
        .expect("redeem route responds");

    assert_eq!(redeem_response.status(), StatusCode::OK);

    let second_body = dm_event_body_with(
        "Ev-host-beta-pairing-second",
        "after pairing",
        "1710000000.000030",
    );
    post_signed_slack_event(&mounts.events, &second_body).await;
    if let Some(drain) = mounts.events.drain.as_ref() {
        drain.drain().await;
    }

    let history = wait_for_slack_thread_history_with_messages(&runtime, 2).await;
    assert_eq!(history.messages.len(), 2);
    assert_eq!(
        history.messages[0].content.as_deref(),
        Some("after pairing")
    );
    assert_eq!(history.messages[1].content.as_deref(), Some("ok"));
    wait_for_slack_posted_text(&egress, "ok").await;

    runtime.shutdown().await.expect("runtime shuts down");
}

#[tokio::test]
async fn build_slack_host_beta_mounts_rejects_team_only_selector_for_pairing() {
    let root = tempfile::tempdir().expect("tempdir");
    let runtime = build_reborn_runtime(
        RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("slack-host-beta-owner", root.path().join("local-dev"))
                .with_runtime_policy(local_dev_runtime_policy().expect("local policy")),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: TENANT.to_string(),
            agent_id: AGENT.to_string(),
            source_binding_id: "slack-host-source".to_string(),
            reply_target_binding_id: "slack-host-reply".to_string(),
        })
        .with_model_gateway_override(Arc::new(StaticGateway)),
    )
    .await
    .expect("runtime builds");
    let team_only_config = SlackHostBetaConfig::new(SlackHostBetaConfigInput {
        tenant_id: TenantId::new(TENANT).expect("tenant"),
        agent_id: AgentId::new(AGENT).expect("agent"),
        project_id: Some(ProjectId::new(PROJECT).expect("project")),
        installation_id: INSTALLATION.to_string(),
        team_id: TEAM.to_string(),
        api_app_id: None,
        slack_user_id: Some(SLACK_USER.to_string()),
        user_id: UserId::new(USER).expect("user"),
        signing_secret: SecretString::from(SECRET),
        bot_token: SecretString::from("xoxb-host-token"),
    })
    .expect("team-only config still parses");

    let error = match build_slack_host_beta_mounts(&runtime, team_only_config).await {
        Ok(_) => panic!("pairing requires tenant app selector"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        SlackHostBetaBuildError::TenantAppSelectorRequired
    ));
    runtime.shutdown().await.expect("runtime shuts down");
}

#[test]
fn slack_host_beta_config_keeps_optional_legacy_slack_actor() {
    let config = config();

    assert_eq!(config.installation_id.as_str(), INSTALLATION);
    let slack_actor = config.slack_actor.as_ref().expect("legacy actor");
    assert_eq!(slack_actor.kind(), SLACK_USER_ACTOR_KIND);
    assert_eq!(slack_actor.id(), SLACK_USER);
    assert_eq!(config.user_id, UserId::new(USER).expect("user id"));
    assert_eq!(config.signing_secret.expose_secret(), SECRET);
    assert_eq!(config.bot_token.expose_secret(), "xoxb-host-token");
}

#[test]
fn slack_egress_scope_uses_configured_tenant_agent_and_project() {
    let config = config();

    let scope = slack_egress_scope(&config);

    assert_eq!(scope.tenant_id, TenantId::new(TENANT).expect("tenant"));
    assert_eq!(scope.user_id, UserId::new(USER).expect("user"));
    assert_eq!(scope.agent_id, Some(AgentId::new(AGENT).expect("agent")));
    assert_eq!(
        scope.project_id,
        Some(ProjectId::new(PROJECT).expect("project"))
    );
}

#[tokio::test]
async fn layered_resolver_preserves_configured_legacy_slack_actor_mapping() {
    let resolver = SlackHostBetaActorUserResolver::new(
        AdapterInstallationId::new(INSTALLATION).expect("installation"),
        Some(
            ExternalActorRef::new(SLACK_USER_ACTOR_KIND, SLACK_USER, None::<String>)
                .expect("actor"),
        ),
        UserId::new(USER).expect("user"),
        Arc::new(FailingProductActorUserResolver),
        Arc::new(FailingProductActorUserResolver),
    );
    let request = ProductActorUserResolutionRequest::new(
        ProductAdapterId::new(SLACK_V2_ADAPTER_ID).expect("adapter"),
        AdapterInstallationId::new(INSTALLATION).expect("installation"),
        ExternalActorRef::new(SLACK_USER_ACTOR_KIND, SLACK_USER, None::<String>).expect("actor"),
    );

    let resolved = resolver
        .resolve_product_actor_user(request)
        .await
        .expect("resolver succeeds");

    assert_eq!(resolved, Some(UserId::new(USER).expect("user")));
}

fn config() -> SlackHostBetaConfig {
    SlackHostBetaConfig::new(SlackHostBetaConfigInput {
        tenant_id: TenantId::new(TENANT).expect("tenant"),
        agent_id: AgentId::new(AGENT).expect("agent"),
        project_id: Some(ProjectId::new(PROJECT).expect("project")),
        installation_id: INSTALLATION.to_string(),
        team_id: TEAM.to_string(),
        api_app_id: Some(API_APP.to_string()),
        slack_user_id: Some(SLACK_USER.to_string()),
        user_id: UserId::new(USER).expect("user"),
        signing_secret: SecretString::from(SECRET),
        bot_token: SecretString::from("xoxb-host-token"),
    })
    .expect("valid config")
}

fn config_without_legacy_actor() -> SlackHostBetaConfig {
    SlackHostBetaConfig::new(SlackHostBetaConfigInput {
        tenant_id: TenantId::new(TENANT).expect("tenant"),
        agent_id: AgentId::new(AGENT).expect("agent"),
        project_id: Some(ProjectId::new(PROJECT).expect("project")),
        installation_id: INSTALLATION.to_string(),
        team_id: TEAM.to_string(),
        api_app_id: Some(API_APP.to_string()),
        slack_user_id: None,
        user_id: UserId::new(USER).expect("user"),
        signing_secret: SecretString::from(SECRET),
        bot_token: SecretString::from("xoxb-host-token"),
    })
    .expect("valid config")
}

async fn post_signed_slack_event(mount: &PublicRouteMount, body: &str) {
    let timestamp = current_unix_timestamp();
    let response = mount
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(SLACK_EVENTS_PATH)
                .header(SLACK_TIMESTAMP_HEADER, timestamp.to_string())
                .header(SLACK_SIGNATURE_HEADER, slack_signature(timestamp, body))
                .body(Body::from(body.to_string()))
                .expect("request builds"),
        )
        .await
        .expect("router responds");

    assert_eq!(response.status(), StatusCode::OK);
}

async fn runtime() -> (RebornRuntime, tempfile::TempDir) {
    let root = tempfile::tempdir().expect("tempdir");
    let runtime = runtime_at(root.path().join("local-dev")).await.0;
    (runtime, root)
}

async fn runtime_at(root: impl AsRef<Path>) -> (RebornRuntime, ()) {
    runtime_at_with_host_egress_override(root, None).await
}

async fn runtime_without_host_egress() -> (RebornRuntime, tempfile::TempDir) {
    let root = tempfile::tempdir().expect("tempdir");
    let runtime = runtime_at_with_host_egress_override(root.path().join("local-dev"), Some(None))
        .await
        .0;
    (runtime, root)
}

async fn runtime_with_recording_egress(
    egress: Arc<RecordingRuntimeHttpEgress>,
) -> (RebornRuntime, tempfile::TempDir) {
    let root = tempfile::tempdir().expect("tempdir");
    let runtime = runtime_at_with_recording_egress(root.path().join("local-dev"), egress)
        .await
        .0;
    (runtime, root)
}

async fn runtime_at_with_recording_egress(
    root: impl AsRef<Path>,
    egress: Arc<RecordingRuntimeHttpEgress>,
) -> (RebornRuntime, ()) {
    let runtime_egress: Arc<dyn RuntimeHttpEgress> = egress;
    runtime_at_with_host_egress_override(
        root,
        Some(Some(
            HostRuntimeHttpEgressPort::test_support_with_allow_all_obligations(runtime_egress),
        )),
    )
    .await
}

async fn runtime_at_with_host_egress_override(
    root: impl AsRef<Path>,
    host_egress_override: Option<Option<HostRuntimeHttpEgressPort>>,
) -> (RebornRuntime, ()) {
    let mut build_input = RebornBuildInput::local_dev(USER, root.as_ref().to_path_buf())
        .with_runtime_policy(local_dev_runtime_policy().expect("local policy"));
    if let Some(host_egress_override) = host_egress_override {
        build_input = build_input.with_host_runtime_http_egress_for_test(host_egress_override);
    }
    let runtime = build_reborn_runtime(
        RebornRuntimeInput::from_services(build_input)
            .with_identity(RebornRuntimeIdentity {
                tenant_id: TENANT.to_string(),
                agent_id: AGENT.to_string(),
                source_binding_id: "slack-host-source".to_string(),
                reply_target_binding_id: "slack-host-reply".to_string(),
            })
            .with_model_gateway_override(Arc::new(StaticGateway)),
    )
    .await
    .expect("runtime builds");
    (runtime, ())
}

async fn wait_for_slack_thread_history_with_messages(
    runtime: &RebornRuntime,
    expected_messages: usize,
) -> ironclaw_threads::ThreadHistory {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let thread_service = runtime.webui_thread_service();
    let scope = ThreadScope {
        tenant_id: TenantId::new(TENANT).expect("tenant"),
        agent_id: AgentId::new(AGENT).expect("agent"),
        project_id: Some(ProjectId::new(PROJECT).expect("project")),
        owner_user_id: Some(UserId::new(USER).expect("user")),
        mission_id: None,
    };
    loop {
        let threads = thread_service
            .list_threads_for_scope(ListThreadsForScopeRequest {
                scope: scope.clone(),
                limit: Some(1),
                cursor: None,
            })
            .await
            .expect("list Slack-created threads");
        if let Some(thread) = threads.threads.first() {
            let history = thread_service
                .list_thread_history(ThreadHistoryRequest {
                    scope: scope.clone(),
                    thread_id: thread.thread_id.clone(),
                })
                .await
                .expect("read Slack-created thread history");
            if history.messages.len() >= expected_messages {
                return history;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "signed Slack event did not create {expected_messages} messages; {}",
                turn_run_debug_summary(runtime).await
            );
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn turn_run_debug_summary(runtime: &RebornRuntime) -> String {
    let Some(snapshot) = runtime.turn_persistence_snapshot_for_test().await else {
        return "turn snapshot unavailable".to_string();
    };
    if snapshot.runs.is_empty() {
        return "turn snapshot has no runs".to_string();
    }
    let run_summary = snapshot
        .runs
        .iter()
        .map(|run| {
            format!(
                "run={} status={:?} failure={:?} project={:?}",
                run.run_id,
                run.status,
                run.failure,
                run.scope
                    .project_id
                    .as_ref()
                    .map(|project| project.as_str())
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    let event_summary = snapshot
        .events
        .iter()
        .map(|event| {
            format!(
                "event_run={} status={:?} kind={:?} reason={:?}",
                event.run_id, event.status, event.kind, event.sanitized_reason
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!("{run_summary}; events=[{event_summary}]")
}

fn current_unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after Unix epoch")
        .as_secs()
}

fn slack_signature(timestamp: u64, body: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(SECRET.as_bytes()).expect("HMAC accepts any key size");
    mac.update(format!("v0:{timestamp}:").as_bytes());
    mac.update(body.as_bytes());
    format!("v0={:x}", mac.finalize().into_bytes())
}

fn dm_event_body() -> &'static str {
    r#"{
          "type":"event_callback",
          "team_id":"THOST",
          "api_app_id":"AHOST",
          "event_id":"Ev-host-beta-custom-resolver",
          "event":{
            "type":"message",
            "channel_type":"im",
            "user":"UHOST",
            "channel":"DHOST",
            "text":"hello",
            "ts":"1710000000.000001"
          }
        }"#
}

fn dm_event_body_with(event_id: &str, text: &str, ts: &str) -> String {
    serde_json::json!({
        "type": "event_callback",
        "team_id": TEAM,
        "api_app_id": API_APP,
        "event_id": event_id,
        "event": {
            "type": "message",
            "channel_type": "im",
            "user": SLACK_USER,
            "channel": "DHOST",
            "text": text,
            "ts": ts
        }
    })
    .to_string()
}

async fn wait_for_resolver_calls(
    resolver: &RecordingProductActorUserResolver,
    expected_len: usize,
) -> Vec<ProductActorUserResolutionRequest> {
    for _ in 0..40 {
        let calls = resolver.calls();
        if calls.len() >= expected_len {
            return calls;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    resolver.calls()
}

async fn wait_for_pairing_code(egress: &RecordingRuntimeHttpEgress) -> String {
    for _ in 0..40 {
        if let Some(code) = egress.pairing_code() {
            return code;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("Slack pairing notifier did not post a pairing code");
}

async fn wait_for_slack_posted_text(egress: &RecordingRuntimeHttpEgress, expected: &str) {
    for _ in 0..80 {
        if egress
            .posted_slack_texts()
            .iter()
            .any(|text| text == expected)
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!(
        "Slack final reply was not posted with expected text {expected:?}; posted texts: {:?}",
        egress.posted_slack_texts()
    );
}

#[derive(Debug)]
struct RecordingProductActorUserResolver {
    user_id: UserId,
    calls: Mutex<Vec<ProductActorUserResolutionRequest>>,
}

impl RecordingProductActorUserResolver {
    fn new(user_id: UserId) -> Self {
        Self {
            user_id,
            calls: Mutex::default(),
        }
    }

    fn calls(&self) -> Vec<ProductActorUserResolutionRequest> {
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[async_trait::async_trait]
impl ProductActorUserResolver for RecordingProductActorUserResolver {
    async fn resolve_product_actor_user(
        &self,
        request: ProductActorUserResolutionRequest,
    ) -> Result<Option<UserId>, ProductWorkflowError> {
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(request);
        Ok(Some(self.user_id.clone()))
    }
}

#[derive(Debug)]
struct FailingProductActorUserResolver;

#[async_trait::async_trait]
impl ProductActorUserResolver for FailingProductActorUserResolver {
    async fn resolve_product_actor_user(
        &self,
        _request: ProductActorUserResolutionRequest,
    ) -> Result<Option<UserId>, ProductWorkflowError> {
        Err(ProductWorkflowError::BindingResolutionFailed {
            reason: "fallback should not be called".into(),
        })
    }
}

#[derive(Debug)]
struct StaticGateway;

#[async_trait::async_trait]
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

#[derive(Default)]
struct RecordingRuntimeHttpEgress {
    requests: std::sync::Mutex<Vec<RuntimeHttpEgressRequest>>,
}

#[async_trait]
impl RuntimeHttpEgress for RecordingRuntimeHttpEgress {
    async fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, ironclaw_host_api::RuntimeHttpEgressError> {
        let response = if request.url.contains("/api/conversations.open") {
            br#"{"ok":true,"channel":{"id":"DHOST"}}"#.to_vec()
        } else {
            br#"{"ok":true}"#.to_vec()
        };
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(request);
        Ok(RuntimeHttpEgressResponse {
            status: 200,
            headers: Vec::new(),
            body: response,
            saved_body: None,
            request_bytes: 0,
            response_bytes: 0,
            redaction_applied: false,
        })
    }
}

impl RecordingRuntimeHttpEgress {
    fn posted_slack_texts(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .filter(|request| request.url.contains("/api/chat.postMessage"))
            .filter_map(|request| serde_json::from_slice::<serde_json::Value>(&request.body).ok())
            .filter_map(|body| body["text"].as_str().map(str::to_string))
            .collect()
    }

    fn pairing_code(&self) -> Option<String> {
        self.posted_slack_texts().into_iter().find_map(|text| {
            text.split(" code ")
                .nth(1)
                .and_then(|suffix| suffix.split(" in WebChat").next())
                .map(str::to_string)
        })
    }
}
