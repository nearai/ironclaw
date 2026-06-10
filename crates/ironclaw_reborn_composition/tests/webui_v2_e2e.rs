//! Lane 7 end-to-end coverage for the WebChat v2 HTTP surface.
//!
//! Unlike [`webui_v2_serve`], which drives the composed router against a
//! stub `RebornServicesApi`, this test stands up a real local-dev
//! `RebornRuntime`, overrides its LLM gateway with a scripted
//! tool-calling fake, composes the v2 router through
//! [`build_webui_services`] + [`webui_v2_app`], and exercises it from
//! the browser side over HTTP (`tower::ServiceExt::oneshot`).
//!
//! The point is to prove the full chain — bearer auth → caller scope →
//! product workflow → turn coordinator → agent loop → capability host
//! (`builtin.echo`) → durable transcript → WebChat v2 SSE/timeline
//! endpoints — works end-to-end without anything mocked above the LLM
//! boundary.

#![cfg(all(feature = "webui-v2-beta", feature = "test-support"))]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderValue, Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use ironclaw_auth::{
    AuthProductScope, AuthSurface, CredentialAccountLabel, CredentialAccountStatus,
    CredentialOwnership, NewCredentialAccount, ProviderScope,
};
use ironclaw_host_api::runtime_policy::{
    ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
    NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
};
use ironclaw_host_api::{
    AgentId, CapabilityId, InvocationId, ResourceScope, SecretHandle, TenantId, UserId,
};
use ironclaw_loop_support::{
    HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
    HostManagedModelMessageRole, HostManagedModelRequest, HostManagedModelResponse,
};
use ironclaw_reborn_composition::{
    PollSettings, RebornBuildInput, RebornRuntime, RebornRuntimeIdentity, RebornRuntimeInput,
    WebuiAuthentication, WebuiAuthenticator, WebuiServeConfig, build_reborn_runtime,
    build_webui_services, webui_v2_app,
};
use ironclaw_turns::run_profile::{LoopCapabilityPort, ProviderToolCall};
use serde_json::{Value, json};
use tower::ServiceExt;

// ─── identities ───────────────────────────────────────────────────────

const VALID_TOKEN: &str = "valid-e2e-token";
const TENANT: &str = "e2e-tenant";
const USER: &str = "e2e-owner";
const AGENT: &str = "e2e-agent";
const SENSITIVE_TOOL_SENTINEL: &str = "sk-e2e-progress-secret";

// ─── auth stub ────────────────────────────────────────────────────────

struct OnlyValidToken;

#[async_trait]
impl WebuiAuthenticator for OnlyValidToken {
    async fn authenticate(&self, token: &str) -> Option<WebuiAuthentication> {
        if token == VALID_TOKEN {
            Some(WebuiAuthentication::user(
                UserId::new(USER).expect("user id"),
            ))
        } else {
            None
        }
    }
}

// ─── runtime policy ───────────────────────────────────────────────────

fn local_dev_effective_policy() -> EffectiveRuntimePolicy {
    // Mirrors the policy the in-mod runtime tests use. Avoids the
    // public `local_dev_runtime_policy()` helper because that returns a
    // `ResolvedRuntimePolicy` shape; `RebornBuildInput::with_runtime_policy`
    // takes the `EffectiveRuntimePolicy` shape and the two are not
    // interchangeable in this direction yet.
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::LocalSingleUser,
        requested_profile: RuntimeProfile::LocalDev,
        resolved_profile: RuntimeProfile::LocalDev,
        filesystem_backend: FilesystemBackendKind::HostWorkspace,
        process_backend: ProcessBackendKind::LocalHost,
        network_mode: NetworkMode::DirectLogged,
        secret_mode: SecretMode::ScrubbedEnv,
        approval_policy: ApprovalPolicy::AskDestructive,
        audit_mode: AuditMode::LocalMinimal,
    }
}

// ─── scripted tool-calling gateway ────────────────────────────────────

/// Two-step LLM stand-in:
///
/// 1. First call: register a provider tool call against `builtin.echo`
///    with arguments containing a secret-like sentinel and return
///    that as a `CapabilityCalls` response so the agent loop dispatches
///    the tool.
/// 2. Second call (after tool execution): assert the tool result is
///    visible in the request transcript, then return a plain assistant
///    reply that the timeline endpoint will surface as the final user-
///    visible message.
#[derive(Debug, Default)]
struct ToolCallingGateway {
    call_count: StdMutex<usize>,
}

#[async_trait]
impl HostManagedModelGateway for ToolCallingGateway {
    async fn stream_model(
        &self,
        _request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        // The capability-aware entrypoint is the right one for this
        // flow; the bare `stream_model` exists for non-tool-calling
        // gateways and should never be reached here. Surfacing an
        // explicit error makes a routing regression fail loudly.
        Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::InvalidRequest,
            "ToolCallingGateway requires the capability-aware model path",
        ))
    }

    async fn stream_model_with_capabilities(
        &self,
        request: HostManagedModelRequest,
        capabilities: Arc<dyn LoopCapabilityPort>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        let call_index = {
            let mut count = self
                .call_count
                .lock()
                .expect("tool gateway call lock poisoned");
            let index = *count;
            *count += 1;
            index
        };

        if call_index > 0 {
            let tool_result = request
                .messages
                .iter()
                .find(|m| m.role == HostManagedModelMessageRole::ToolResult)
                .expect("follow-up model call must include a tool_result message");
            assert!(
                tool_result.content.contains("hello from e2e tool"),
                "follow-up model call should see hydrated echo output, got: {}",
                tool_result.content,
            );
            return Ok(HostManagedModelResponse::assistant_reply("e2e tool ok"));
        }

        let echo_id = CapabilityId::new("builtin.echo").expect("echo capability id");
        let echo_tool = capabilities
            .tool_definitions()
            .map_err(|err| {
                HostManagedModelError::safe(
                    HostManagedModelErrorKind::InvalidRequest,
                    format!("tool_definitions failed: {err}"),
                )
            })?
            .into_iter()
            .find(|def| def.capability_id == echo_id)
            .expect("builtin.echo must be visible in local-dev capability surface");

        let candidate = capabilities
            .register_provider_tool_call(ProviderToolCall {
                provider_id: "e2e-provider".to_string(),
                provider_model_id: "e2e-model".to_string(),
                turn_id: Some("e2e-turn-1".to_string()),
                id: "e2e-call-1".to_string(),
                name: echo_tool.name,
                arguments: json!({"message": format!("hello from e2e tool {SENSITIVE_TOOL_SENTINEL}")}),
                response_reasoning: None,
                reasoning: None,
                signature: None,
            })
            .await
            .map_err(|err| {
                HostManagedModelError::safe(
                    HostManagedModelErrorKind::InvalidRequest,
                    format!("register_provider_tool_call failed: {err}"),
                )
            })?;

        Ok(HostManagedModelResponse::capability_calls(
            vec![candidate],
            "",
        ))
    }
}

// ─── harness ──────────────────────────────────────────────────────────

struct Harness {
    runtime: RebornRuntime,
    router: axum::Router,
    _root: Option<tempfile::TempDir>,
}

async fn build_harness() -> Harness {
    let root = tempfile::tempdir().expect("tempdir");
    let storage_root = root.path().join("local-dev");
    build_harness_at(storage_root, Some(root)).await
}

async fn build_harness_on_storage(storage_root: impl AsRef<Path>) -> Harness {
    build_harness_at(storage_root.as_ref().to_path_buf(), None).await
}

async fn build_harness_at(storage_root: PathBuf, root: Option<tempfile::TempDir>) -> Harness {
    let gateway = Arc::new(ToolCallingGateway::default());
    let input = RebornRuntimeInput::from_services(
        RebornBuildInput::local_dev(USER, storage_root)
            .with_runtime_policy(local_dev_effective_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: TENANT.to_string(),
        agent_id: AGENT.to_string(),
        source_binding_id: "e2e-source".to_string(),
        reply_target_binding_id: "e2e-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: Duration::from_secs(10),
    })
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let bundle = build_webui_services(&runtime, None).expect("webui bundle");
    let config = WebuiServeConfig::new(
        TenantId::new(TENANT).expect("tenant"),
        Arc::new(OnlyValidToken),
        // CORS allowlist is unused in oneshot tests (no Origin header
        // is set), but the WebuiServeConfig constructor rejects an
        // empty Vec to keep production deployments fail-closed. Any
        // throwaway origin satisfies the type without affecting these
        // tests.
        vec![HeaderValue::from_static("http://localhost:0")],
    )
    .with_default_agent_id(AgentId::new(AGENT).expect("agent"));
    let router = webui_v2_app(bundle, config).expect("webui v2 app");

    Harness {
        runtime,
        router,
        _root: root,
    }
}

async fn read_json(response: axum::response::Response) -> Value {
    let bytes = to_bytes(response.into_body(), 256 * 1024)
        .await
        .expect("response body within 256 KiB cap");
    serde_json::from_slice(&bytes).expect("response body is valid JSON")
}

fn bearer_post(uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("bearer POST request")
}

fn bearer_get(uri: &str) -> Request<Body> {
    bearer_get_with_last_event_id(uri, None)
}

fn bearer_get_with_last_event_id(uri: &str, last_event_id: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"));
    if let Some(last_event_id) = last_event_id {
        builder = builder.header("Last-Event-ID", last_event_id);
    }
    builder.body(Body::empty()).expect("bearer GET request")
}

#[derive(Default, Debug)]
struct ParsedSseEvent {
    event: Option<String>,
    id: Option<String>,
    data: Option<String>,
}

fn parse_sse_events(bytes: &[u8]) -> Vec<ParsedSseEvent> {
    let text = String::from_utf8_lossy(bytes);
    let mut events = Vec::new();
    let mut parsed = ParsedSseEvent::default();
    let mut has_fields = false;
    for line in text.lines() {
        if line.is_empty() {
            if has_fields {
                events.push(parsed);
                parsed = ParsedSseEvent::default();
                has_fields = false;
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("event:") {
            parsed.event = Some(rest.trim_start().to_string());
            has_fields = true;
        } else if let Some(rest) = line.strip_prefix("id:") {
            parsed.id = Some(rest.trim_start().to_string());
            has_fields = true;
        } else if let Some(rest) = line.strip_prefix("data:") {
            parsed.data = Some(rest.trim_start().to_string());
            has_fields = true;
        }
    }
    events
}

async fn collect_sse_until<F>(body: &mut Body, timeout: Duration, mut done: F) -> Vec<u8>
where
    F: FnMut(&[u8]) -> bool,
{
    let deadline = Instant::now() + timeout;
    let mut buf = Vec::new();
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(remaining, body.frame()).await {
            Ok(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    buf.extend_from_slice(data.as_ref());
                    if done(&buf) {
                        return buf;
                    }
                }
            }
            Ok(_) | Err(_) => return buf,
        }
    }
    buf
}

async fn open_sse(
    router: &axum::Router,
    thread_id: &str,
    last_event_id: Option<&str>,
) -> axum::response::Response {
    let response = router
        .clone()
        .oneshot(bearer_get_with_last_event_id(
            &format!("/api/webchat/v2/threads/{thread_id}/events"),
            last_event_id,
        ))
        .await
        .expect("SSE oneshot");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "SSE stream must open successfully"
    );
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    assert!(
        content_type.starts_with("text/event-stream"),
        "SSE content type expected, got: {content_type}"
    );
    response
}

async fn create_thread(router: &axum::Router, client_action_id: &str) -> String {
    let create = router
        .clone()
        .oneshot(bearer_post(
            "/api/webchat/v2/threads",
            json!({"client_action_id": client_action_id}),
        ))
        .await
        .expect("create_thread oneshot");
    assert_eq!(
        create.status(),
        StatusCode::OK,
        "create_thread must succeed against the real bundle"
    );
    let create_body = read_json(create).await;
    create_body["thread"]["thread_id"]
        .as_str()
        .expect("create_thread response must carry thread.thread_id")
        .to_string()
}

async fn send_message(router: &axum::Router, thread_id: &str, client_action_id: &str) {
    let send = router
        .clone()
        .oneshot(bearer_post(
            &format!("/api/webchat/v2/threads/{thread_id}/messages"),
            json!({
                "client_action_id": client_action_id,
                "content": "please call the echo tool",
            }),
        ))
        .await
        .expect("send_message oneshot");
    assert_eq!(
        send.status(),
        StatusCode::OK,
        "send_message must accept the queued turn"
    );
}

async fn wait_for_final_timeline(router: &axum::Router, thread_id: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        let response = router
            .clone()
            .oneshot(bearer_get(&format!(
                "/api/webchat/v2/threads/{thread_id}/timeline"
            )))
            .await
            .expect("timeline oneshot");
        assert_eq!(response.status(), StatusCode::OK);
        let timeline = read_json(response).await;
        let messages = timeline["messages"]
            .as_array()
            .expect("timeline.messages must be an array");
        if messages.iter().any(|message| {
            extract_assistant_text(message).is_some_and(|text| text.contains("e2e tool ok"))
        }) {
            return timeline;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timeline never surfaced an assistant message containing 'e2e tool ok' within 10s");
}

fn assert_timeline_has_tool_result_reference(timeline: &Value) {
    let messages = timeline["messages"]
        .as_array()
        .expect("timeline.messages must be an array");
    let tool_result_seen = messages.iter().any(|message| {
        message.get("kind").and_then(Value::as_str) == Some("tool_result_reference")
            && message
                .get("tool_result_ref")
                .and_then(Value::as_str)
                .is_some_and(|reference| reference.starts_with("result:"))
    });
    assert!(
        tool_result_seen,
        "timeline must include a tool_result_reference message for the builtin.echo invocation, \
         but the messages array was: {messages:#?}",
    );
}

fn assert_no_sensitive_payload(label: &str, bytes_or_json: impl AsRef<[u8]>) {
    let text = String::from_utf8_lossy(bytes_or_json.as_ref());
    assert!(
        !text.contains(SENSITIVE_TOOL_SENTINEL),
        "{label} leaked sensitive tool input sentinel: {text}"
    );
}

fn event_ids(events: &[ParsedSseEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| event.id.clone())
        .collect::<Vec<_>>()
}

fn has_browser_visible_progress(events: &[ParsedSseEvent]) -> bool {
    events.iter().any(|event| {
        matches!(
            event.event.as_deref(),
            Some("accepted")
                | Some("running")
                | Some("capability_progress")
                | Some("capability_activity")
                | Some("capability_display_preview")
                | Some("projection_snapshot")
                | Some("projection_update")
                | Some("final_reply")
        )
    })
}

fn events_include_error(bytes: &[u8]) -> bool {
    parse_sse_events(bytes)
        .iter()
        .any(|event| event.event.as_deref() == Some("error"))
}

fn events_include_final_reply(bytes: &[u8]) -> bool {
    parse_sse_events(bytes)
        .iter()
        .any(|event| event.event.as_deref() == Some("final_reply"))
}

fn cursor_scopes_thread(cursor: &str, thread_id: &str) -> bool {
    let Ok(cursor) = serde_json::from_str::<Value>(cursor) else {
        return false;
    };
    let cursor = match cursor.as_str() {
        Some(encoded) => match serde_json::from_str::<Value>(encoded) {
            Ok(decoded) => decoded,
            Err(_) => return false,
        },
        None => cursor,
    };
    cursor["runtime"]["scope"]["read_scope"]["thread_id"].as_str() == Some(thread_id)
        || cursor["live"]["scope"]["read_scope"]["thread_id"].as_str() == Some(thread_id)
        || cursor["turn"]["scope"]["thread_id"].as_str() == Some(thread_id)
}

fn first_scoped_cursor_before_later_id(events: &[ParsedSseEvent], thread_id: &str) -> String {
    events
        .iter()
        .enumerate()
        .find_map(|(index, event)| {
            let id = event.id.as_deref()?;
            if !cursor_scopes_thread(id, thread_id) {
                return None;
            }
            events[index + 1..]
                .iter()
                .any(|later| later.id.is_some())
                .then(|| id.to_string())
        })
        .unwrap_or_else(|| {
            panic!(
                "SSE stream must include a scoped cursor with replayable events after it, got ids: {:?}",
                event_ids(events)
            )
        })
}

fn assert_only_fail_closed_error(label: &str, events: &[ParsedSseEvent]) -> Value {
    assert_eq!(
        events.len(),
        1,
        "{label} must fail closed before emitting replay/projection frames, got: {events:?}"
    );
    let error_event = events
        .first()
        .expect("event count checked")
        .event
        .as_deref();
    assert_eq!(
        error_event,
        Some("error"),
        "{label} must emit only an error event, got: {events:?}"
    );
    event_payload_json(events.first().expect("event count checked"))
}

fn event_payload_json(event: &ParsedSseEvent) -> Value {
    serde_json::from_str(event.data.as_deref().expect("SSE event data is present"))
        .expect("SSE data is JSON")
}

fn serialize_json(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).expect("JSON value serializes")
}

fn webui_extension_setup_scope(extension_id: &str) -> AuthProductScope {
    let seed = format!("webui-v2-extension-setup:{TENANT}:{USER}:{AGENT}::{extension_id}");
    let resource = ResourceScope {
        tenant_id: TenantId::new(TENANT).expect("tenant"),
        user_id: UserId::new(USER).expect("user"),
        agent_id: Some(AgentId::new(AGENT).expect("agent")),
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::from_uuid(uuid::Uuid::new_v5(
            &uuid::Uuid::NAMESPACE_OID,
            seed.as_bytes(),
        )),
    };
    AuthProductScope::new(resource, AuthSurface::Callback)
}

// ─── tests ────────────────────────────────────────────────────────────

#[tokio::test]
async fn webui_v2_http_list_automations_uses_composed_runtime_facade() {
    let harness = build_harness().await;

    let response = harness
        .router
        .clone()
        .oneshot(bearer_get("/api/webchat/v2/automations?limit=10"))
        .await
        .expect("list automations oneshot");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "list_automations must be wired through the real composed bundle"
    );
    let body = read_json(response).await;
    assert!(
        body["automations"].as_array().is_some(),
        "list_automations response must include an automations array, got: {body:#?}"
    );

    harness
        .runtime
        .shutdown()
        .await
        .expect("runtime shutdown clean");
}

/// Beta scoreboard acceptance for issue #3613: drive the WebUI/WebChat
/// v2 API from the browser side, stream live Reborn projections over
/// SSE, replay with `Last-Event-ID`, verify final durable timeline
/// state, reject a cross-thread cursor as a redacted SSE error, and
/// reopen the same local-dev stores to prove the transcript survives
/// runtime restart.
#[tokio::test]
async fn webui_v2_beta_acceptance_stream_replay_restart_and_redaction() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage_root = root.path().join("local-dev");
    let harness = build_harness_on_storage(&storage_root).await;

    let thread_id = create_thread(&harness.router, "e2e-create-1").await;
    let response = open_sse(&harness.router, &thread_id, None).await;
    let mut body = response.into_body();

    send_message(&harness.router, &thread_id, "e2e-send-1").await;

    let sse_bytes = collect_sse_until(
        &mut body,
        Duration::from_secs(10),
        events_include_final_reply,
    )
    .await;
    drop(body);

    assert_no_sensitive_payload("live SSE stream", &sse_bytes);
    let events = parse_sse_events(&sse_bytes);
    assert!(
        has_browser_visible_progress(&events),
        "SSE stream must surface browser-visible progress, got: {events:?}; raw: {}",
        String::from_utf8_lossy(&sse_bytes)
    );
    let ids = event_ids(&events);
    assert!(
        ids.len() >= 2,
        "SSE stream must emit at least two cursor ids for replay coverage, got: {events:?}; raw: {}",
        String::from_utf8_lossy(&sse_bytes)
    );

    let replay_from = first_scoped_cursor_before_later_id(&events, &thread_id);
    let replay_response = open_sse(&harness.router, &thread_id, Some(&replay_from)).await;
    let mut replay_body = replay_response.into_body();
    let replay_bytes = collect_sse_until(
        &mut replay_body,
        Duration::from_secs(5),
        events_include_final_reply,
    )
    .await;
    drop(replay_body);

    assert_no_sensitive_payload("replayed SSE stream", &replay_bytes);
    let replay_events = parse_sse_events(&replay_bytes);
    let replay_ids = event_ids(&replay_events);
    assert!(
        !replay_ids.is_empty(),
        "Last-Event-ID replay must return cursor-addressed events after {replay_from}, got: {replay_events:?}; raw: {}",
        String::from_utf8_lossy(&replay_bytes)
    );
    assert!(
        replay_ids.iter().all(|id| id != &replay_from),
        "Last-Event-ID replay must resume after the provided cursor, got: {replay_ids:?}"
    );
    assert!(
        replay_ids
            .iter()
            .any(|id| ids.iter().skip(1).any(|seen| seen == id)),
        "Last-Event-ID replay should include an event already observed after the first cursor; \
         original ids: {ids:?}, replay ids: {replay_ids:?}"
    );

    let timeline = wait_for_final_timeline(&harness.router, &thread_id).await;
    assert_timeline_has_tool_result_reference(&timeline);
    assert_no_sensitive_payload("durable timeline", serialize_json(&timeline));

    let other_thread_id = create_thread(&harness.router, "e2e-create-foreign-cursor").await;
    let foreign_response = open_sse(&harness.router, &other_thread_id, Some(&replay_from)).await;
    let mut foreign_body = foreign_response.into_body();
    let foreign_bytes = collect_sse_until(
        &mut foreign_body,
        Duration::from_secs(5),
        events_include_error,
    )
    .await;
    drop(foreign_body);

    assert_no_sensitive_payload("foreign-cursor SSE error", &foreign_bytes);
    let foreign_events = parse_sse_events(&foreign_bytes);
    let error_json = assert_only_fail_closed_error("foreign cursor", &foreign_events);
    assert_eq!(
        error_json["error"], "invalid_request",
        "foreign cursor error should be redacted and non-successful: {error_json}"
    );
    assert_eq!(error_json["retryable"], false);

    harness
        .runtime
        .shutdown()
        .await
        .expect("runtime shutdown clean");

    let reopened = build_harness_on_storage(&storage_root).await;
    let reopened_timeline = wait_for_final_timeline(&reopened.router, &thread_id).await;
    assert_timeline_has_tool_result_reference(&reopened_timeline);
    assert_no_sensitive_payload(
        "reopened durable timeline",
        serialize_json(&reopened_timeline),
    );

    reopened
        .runtime
        .shutdown()
        .await
        .expect("reopened runtime shutdown clean");
}

#[tokio::test]
async fn webui_v2_gmail_oauth_setup_complete_allows_activation() {
    let harness = build_harness().await;
    let product_auth = harness
        .runtime
        .services()
        .product_auth
        .as_ref()
        .expect("local-dev runtime wires product auth");
    product_auth
        .credential_account_service()
        .create_account(NewCredentialAccount {
            scope: webui_extension_setup_scope("gmail"),
            provider: ironclaw_first_party_extensions::google_provider_id()
                .expect("google provider id"),
            label: CredentialAccountLabel::new("work google").expect("label"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("google-access").expect("secret handle")),
            refresh_secret: None,
            scopes: vec![
                ProviderScope::new("https://www.googleapis.com/auth/gmail.readonly")
                    .expect("gmail scope"),
                ProviderScope::new("https://www.googleapis.com/auth/gmail.send")
                    .expect("gmail scope"),
                ProviderScope::new("https://www.googleapis.com/auth/gmail.modify")
                    .expect("gmail scope"),
            ],
        })
        .await
        .expect("seed callback-surface Google OAuth account");

    let package_ref = json!({"kind": "extension", "id": "gmail"});
    let install = harness
        .router
        .clone()
        .oneshot(bearer_post(
            "/api/webchat/v2/extensions/install",
            json!({"package_ref": package_ref}),
        ))
        .await
        .expect("install Gmail oneshot");
    assert_eq!(install.status(), StatusCode::OK);
    let install_body = read_json(install).await;
    assert_eq!(
        install_body["success"], true,
        "install body: {install_body}"
    );

    let setup = harness
        .router
        .clone()
        .oneshot(bearer_get("/api/webchat/v2/extensions/gmail/setup"))
        .await
        .expect("setup Gmail oneshot");
    assert_eq!(setup.status(), StatusCode::OK);
    let setup_body = read_json(setup).await;
    assert_eq!(
        setup_body["secrets"][0]["provided"], true,
        "setup should see the completed Google OAuth account so the UI can offer Activate: {setup_body}"
    );

    let activate = harness
        .router
        .clone()
        .oneshot(bearer_post(
            "/api/webchat/v2/extensions/gmail/activate",
            json!({}),
        ))
        .await
        .expect("activate Gmail oneshot");
    assert_eq!(activate.status(), StatusCode::OK);
    let activate_body = read_json(activate).await;
    assert_eq!(
        activate_body["success"], true,
        "activation should succeed after setup completion: {activate_body}"
    );
    assert_eq!(activate_body["activated"], true);
}

#[tokio::test]
async fn webui_v2_google_docs_setup_projects_oauth_before_install() {
    let harness = build_harness().await;

    let setup = harness
        .router
        .clone()
        .oneshot(bearer_get("/api/webchat/v2/extensions/google-docs/setup"))
        .await
        .expect("setup Google Docs oneshot");
    assert_eq!(setup.status(), StatusCode::OK);
    let setup_body = read_json(setup).await;
    assert_eq!(setup_body["package_ref"]["id"], "google-docs");
    assert_eq!(setup_body["phase"], "discovered");

    let secrets = setup_body["secrets"]
        .as_array()
        .expect("setup secrets should be an array");
    let google_oauth_scopes = secrets
        .iter()
        .filter(|secret| secret["provider"] == "google")
        .map(|secret| {
            let setup = &secret["setup"];
            assert_eq!(setup["kind"], "oauth", "secret should be OAuth: {secret}");
            setup["scopes"]
                .as_array()
                .expect("OAuth scopes should be an array")
                .iter()
                .map(|scope| scope.as_str().expect("scope string").to_string())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    assert_eq!(
        google_oauth_scopes,
        vec![
            vec!["https://www.googleapis.com/auth/documents".to_string()],
            vec!["https://www.googleapis.com/auth/documents.readonly".to_string()],
        ],
        "Google Docs setup should expose WASM-declared OAuth scopes before install: {setup_body}"
    );

    harness
        .runtime
        .shutdown()
        .await
        .expect("runtime shutdown clean");
}

#[tokio::test]
async fn webui_v2_google_drive_oauth_setup_projects_distinct_operation_scopes() {
    let harness = build_harness().await;

    let package_ref = json!({"kind": "extension", "id": "google-drive"});
    let install = harness
        .router
        .clone()
        .oneshot(bearer_post(
            "/api/webchat/v2/extensions/install",
            json!({"package_ref": package_ref}),
        ))
        .await
        .expect("install Google Drive oneshot");
    assert_eq!(install.status(), StatusCode::OK);

    let setup = harness
        .router
        .clone()
        .oneshot(bearer_get("/api/webchat/v2/extensions/google-drive/setup"))
        .await
        .expect("setup Google Drive oneshot");
    assert_eq!(setup.status(), StatusCode::OK);
    let setup_body = read_json(setup).await;
    let secrets = setup_body["secrets"]
        .as_array()
        .expect("setup secrets should be an array");
    let google_oauth_setups = secrets
        .iter()
        .filter(|secret| secret["provider"] == "google")
        .map(|secret| {
            let setup = &secret["setup"];
            assert_eq!(setup["kind"], "oauth", "secret should be OAuth: {secret}");
            (
                setup["account_label"]
                    .as_str()
                    .expect("OAuth account label")
                    .to_string(),
                setup["scopes"]
                    .as_array()
                    .expect("OAuth scopes should be an array")
                    .iter()
                    .map(|scope| scope.as_str().expect("scope string").to_string())
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        google_oauth_setups
            .iter()
            .map(|(_, scopes)| scopes.clone())
            .collect::<Vec<_>>(),
        vec![
            vec!["https://www.googleapis.com/auth/drive.readonly".to_string()],
            vec!["https://www.googleapis.com/auth/drive".to_string()],
        ],
        "Google Drive setup should keep read-only and write OAuth scopes separate: {setup_body}"
    );
    assert_ne!(
        google_oauth_setups[0].0, google_oauth_setups[1].0,
        "split Google Drive setup credentials should have distinct account labels"
    );

    harness
        .runtime
        .shutdown()
        .await
        .expect("runtime shutdown clean");
}

/// SECURITY (review on #4673): a browser request body must not be able to set
/// the caller scope. The v2 request DTOs carry no `tenant_id`/`user_id`/`scope`
/// field and the caller scope is host-minted, so injecting the reserved system
/// sentinel into the JSON body is inert — the thread is created under the
/// authenticated caller and stays readable by them. If the injected sentinel
/// had diverted creation to the system scope (or another tenant), this caller
/// could not read the resulting thread's timeline.
#[tokio::test]
async fn untrusted_request_body_cannot_inject_system_scope() {
    let harness = build_harness().await;
    let sentinel = "\u{1f}SYSTEM\u{1f}";

    let malicious = json!({
        "client_action_id": "inject-scope-1",
        "tenant_id": sentinel,
        "user_id": sentinel,
        "scope": { "tenant_id": sentinel, "user_id": sentinel },
    });
    let create = harness
        .router
        .clone()
        .oneshot(bearer_post("/api/webchat/v2/threads", malicious))
        .await
        .expect("create oneshot");
    assert_eq!(
        create.status(),
        StatusCode::OK,
        "injected scope fields must be ignored (unknown fields), not honored or errored"
    );
    let body = read_json(create).await;
    let thread_id = body["thread"]["thread_id"]
        .as_str()
        .expect("thread_id")
        .to_string();

    let timeline = harness
        .router
        .clone()
        .oneshot(bearer_get(&format!(
            "/api/webchat/v2/threads/{thread_id}/timeline"
        )))
        .await
        .expect("timeline oneshot");
    assert_eq!(
        timeline.status(),
        StatusCode::OK,
        "the thread must belong to the authenticated caller — the body could not set scope"
    );

    harness
        .runtime
        .shutdown()
        .await
        .expect("runtime shutdown clean");
}

// ─── operator LLM-config smoke (issue #4673) ──────────────────────────
//
// Stands up the same real local-dev runtime as the chat e2e, but with a boot
// config (so the WebUI facade composes the operator LLM-config service) and an
// operator-scoped authenticator (so the `/api/webchat/v2/llm/providers` routes
// mount). Saving the built-in NEAR AI provider stores its API key under the
// system scope; the regression was that the system-scoped secret serialized but
// failed to deserialize, so the very next read-back (snapshot metadata, or the
// previous-key read on a second save) returned `service_unavailable`.

#[cfg(feature = "root-llm-provider")]
mod operator_llm_config {
    use super::*;
    use ironclaw_reborn_config::{RebornBootConfig, RebornHome, RebornProfile};

    struct OperatorToken;

    #[async_trait]
    impl WebuiAuthenticator for OperatorToken {
        async fn authenticate(&self, token: &str) -> Option<WebuiAuthentication> {
            if token == VALID_TOKEN {
                Some(WebuiAuthentication::operator(
                    UserId::new(USER).expect("user id"),
                ))
            } else {
                None
            }
        }

        fn mounts_operator_webui_config_routes(&self) -> bool {
            true
        }
    }

    async fn build_operator_harness() -> Harness {
        let root = tempfile::tempdir().expect("tempdir");
        let storage_root = root.path().join("local-dev");
        let home = RebornHome::resolve_from_env_parts(
            Some(root.path().join("reborn-home").into_os_string()),
            None,
            None,
        )
        .expect("valid reborn home");
        let boot = RebornBootConfig::new(home, RebornProfile::LocalDev);

        let gateway = Arc::new(ToolCallingGateway::default());
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev(USER, storage_root)
                .with_runtime_policy(local_dev_effective_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: TENANT.to_string(),
            agent_id: AGENT.to_string(),
            source_binding_id: "e2e-source".to_string(),
            reply_target_binding_id: "e2e-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(10),
        })
        .with_model_gateway_override(gateway)
        .with_boot_config(boot);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let bundle = build_webui_services(&runtime, None).expect("webui bundle");
        let config = WebuiServeConfig::new(
            TenantId::new(TENANT).expect("tenant"),
            Arc::new(OperatorToken),
            vec![HeaderValue::from_static("http://localhost:0")],
        )
        .with_default_agent_id(AgentId::new(AGENT).expect("agent"));
        let router = webui_v2_app(bundle, config).expect("webui v2 app");

        Harness {
            runtime,
            router,
            _root: Some(root),
        }
    }

    fn nearai_save_payload() -> Value {
        json!({
            "id": "nearai",
            "name": "NEAR AI",
            "adapter": "near_ai",
            "base_url": "https://cloud-api.near.ai",
            "default_model": "deepseek-ai/DeepSeek-V4-Flash",
            "api_key": "sk-e2e-operator-key",
            "set_active": true,
            "model": "deepseek-ai/DeepSeek-V4-Flash"
        })
    }

    fn find_nearai(snapshot: &Value) -> &Value {
        snapshot["providers"]
            .as_array()
            .expect("providers array")
            .iter()
            .find(|provider| provider["id"] == "nearai")
            .expect("nearai provider in snapshot")
    }

    #[tokio::test]
    async fn nearai_provider_save_persists_key_and_survives_resave() {
        let harness = build_operator_harness().await;

        // First save: persists the operator's NEAR AI key under the system scope
        // and selects it active.
        let response = harness
            .router
            .clone()
            .oneshot(bearer_post(
                "/api/webchat/v2/llm/providers",
                nearai_save_payload(),
            ))
            .await
            .expect("save request");
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "saving the NEAR AI provider must succeed"
        );
        let body = read_json(response).await;
        assert_eq!(body["active"]["provider_id"], "nearai");
        assert_eq!(body["active"]["model"], "deepseek-ai/DeepSeek-V4-Flash");
        assert_eq!(
            find_nearai(&body)["api_key_set"],
            true,
            "the stored system-scope key must read back (regression #4673)"
        );

        // Re-saving reads the previous key back first — the exact op that
        // returned service_unavailable before the system-scope deserialize fix.
        let resave = harness
            .router
            .clone()
            .oneshot(bearer_post(
                "/api/webchat/v2/llm/providers",
                nearai_save_payload(),
            ))
            .await
            .expect("resave request");
        assert_eq!(
            resave.status(),
            StatusCode::OK,
            "re-saving an already-configured provider must not 503"
        );

        // The welcome-screen read-back path: GET must report NEAR AI as active
        // with its key set, not as still requiring setup.
        let get = harness
            .router
            .clone()
            .oneshot(bearer_get("/api/webchat/v2/llm/providers"))
            .await
            .expect("get request");
        assert_eq!(get.status(), StatusCode::OK);
        let snapshot = read_json(get).await;
        assert_eq!(snapshot["active"]["provider_id"], "nearai");
        assert_eq!(find_nearai(&snapshot)["api_key_set"], true);

        harness
            .runtime
            .shutdown()
            .await
            .expect("runtime shutdown clean");
    }
}

/// Walks a `ThreadMessageRecord` JSON object and returns the rendered
/// text if it is an assistant reply with content. Done as a free
/// function so the polling loop above can stay readable.
fn extract_assistant_text(message: &Value) -> Option<String> {
    let kind = message.get("kind")?.as_str()?;
    if kind != "assistant" {
        return None;
    }
    message
        .get("content")?
        .as_str()
        .map(std::string::ToString::to_string)
}
