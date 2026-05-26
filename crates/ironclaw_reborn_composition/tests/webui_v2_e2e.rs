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
//! (`builtin.echo`) → durable transcript → timeline endpoint — works
//! end-to-end without anything mocked above the LLM boundary.
//!
//! SSE wire-shape coverage (asserting `capability_activity` /
//! `final_reply` SSE frames and `Last-Event-ID` resume behavior over
//! a real listener) is intentionally out of scope here — the v2 SSE
//! handler closes on every `replay_unavailable retryable=true`
//! transient from the projection adapter and expects the browser's
//! `EventSource` auto-reconnect to resume, which doesn't fit cleanly
//! inside the descriptor's per-route stream rate-limit (12 opens / 60s)
//! when driven from a test loop. That coverage belongs in a follow-up
//! PR with proper investigation of the projection adapter's drain
//! semantics.

#![cfg(all(feature = "webui-v2-beta", feature = "test-support"))]

use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderValue, Method, Request, StatusCode, header};
use ironclaw_host_api::runtime_policy::{
    ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
    NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
};
use ironclaw_host_api::{AgentId, CapabilityId, TenantId, UserId};
use ironclaw_loop_support::{
    HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
    HostManagedModelMessageRole, HostManagedModelRequest, HostManagedModelResponse,
};
use ironclaw_reborn_composition::{
    PollSettings, RebornBuildInput, RebornRuntime, RebornRuntimeIdentity, RebornRuntimeInput,
    WebuiAuthenticator, WebuiServeConfig, build_reborn_runtime, build_webui_services, webui_v2_app,
};
use ironclaw_turns::run_profile::{LoopCapabilityPort, ProviderToolCall};
use serde_json::{Value, json};
use tower::ServiceExt;

// ─── identities ───────────────────────────────────────────────────────

const VALID_TOKEN: &str = "valid-e2e-token";
const TENANT: &str = "e2e-tenant";
const USER: &str = "e2e-owner";
const AGENT: &str = "e2e-agent";

// ─── auth stub ────────────────────────────────────────────────────────

struct OnlyValidToken;

#[async_trait]
impl WebuiAuthenticator for OnlyValidToken {
    async fn authenticate(&self, token: &str) -> Option<UserId> {
        if token == VALID_TOKEN {
            Some(UserId::new(USER).expect("user id"))
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
///    with arguments `{"message": "hello from e2e tool"}` and return
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
                arguments: json!({"message": "hello from e2e tool"}),
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
    _root: tempfile::TempDir,
}

async fn build_harness() -> Harness {
    let root = tempfile::tempdir().expect("tempdir");
    let gateway = Arc::new(ToolCallingGateway::default());
    let input = RebornRuntimeInput::from_services(
        RebornBuildInput::local_dev(USER, root.path().join("local-dev"))
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
    Request::builder()
        .method(Method::GET)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
        .body(Body::empty())
        .expect("bearer GET request")
}

// ─── tests ────────────────────────────────────────────────────────────

/// Step 2 of Lane 7: drive `create_thread` → `submit_turn` → poll
/// `timeline` through the composed v2 HTTP surface, against a real
/// local-dev runtime whose LLM gateway is scripted to call
/// `builtin.echo` once and then return a final assistant reply.
///
/// Locks the issue's exit criteria that this PR owns:
///   - "WebUI can create a thread and submit a message through
///     `RebornServicesApi`"
///   - "Reborn AgentLoop runs and returns final assistant reply"
///   - "First-party capability call is visible through timeline
///     integration" — verified two ways: (a) the assistant reply text
///     can only come from the gateway's second call, which is gated
///     on the tool result being hydrated; (b) the timeline includes a
///     `tool_result_reference` message wired to the capability output.
///   - "WebUI handlers do not directly access runtime internals or
///     stores" — already enforced by the architecture boundary test;
///     this test additionally proves the same path *works* end-to-end.
#[tokio::test]
async fn webui_v2_http_happy_path_with_builtin_tool_call() {
    let harness = build_harness().await;

    // 1. Create a thread over HTTP.
    let create = harness
        .router
        .clone()
        .oneshot(bearer_post(
            "/api/webchat/v2/threads",
            json!({"client_action_id": "e2e-create-1"}),
        ))
        .await
        .expect("create_thread oneshot");
    assert_eq!(
        create.status(),
        StatusCode::OK,
        "create_thread must succeed against the real bundle"
    );
    let create_body = read_json(create).await;
    let thread_id = create_body["thread"]["thread_id"]
        .as_str()
        .expect("create_thread response must carry thread.thread_id")
        .to_string();

    // 2. Submit a user message over HTTP. The submit return is
    //    fire-and-forget at the HTTP layer; the worker picks the
    //    queued turn up off the durable store and drives the agent
    //    loop in the background.
    let send = harness
        .router
        .clone()
        .oneshot(bearer_post(
            &format!("/api/webchat/v2/threads/{thread_id}/messages"),
            json!({
                "client_action_id": "e2e-send-1",
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

    // 3. Poll the timeline until the assistant final reply lands.
    //    `e2e tool ok` only comes from the gateway's second call,
    //    which only fires AFTER the tool result has been hydrated
    //    back into the request transcript — so a successful match
    //    is also proof the capability dispatch path ran.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut final_reply: Option<String> = None;
    while Instant::now() < deadline {
        let response = harness
            .router
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
        if let Some(text) = messages.iter().find_map(extract_assistant_text)
            && text.contains("e2e tool ok")
        {
            final_reply = Some(text);
            // Lock the issue's "first-party capability call visible
            // through timeline integration" exit criterion: the
            // assistant reply alone proves the loop completed, but
            // the durable transcript must ALSO carry a
            // tool_result_reference message linking back to the
            // capability invocation. A regression that lost the
            // tool-result hop (or surfaced the assistant reply
            // without recording the tool execution) would still let
            // the assistant-text assertion pass.
            let tool_result_seen = messages.iter().any(|message| {
                message.get("kind").and_then(Value::as_str) == Some("tool_result_reference")
                    && message
                        .get("tool_result_ref")
                        .and_then(Value::as_str)
                        .is_some_and(|reference| reference.starts_with("result:"))
            });
            assert!(
                tool_result_seen,
                "timeline must include a tool_result_reference message for the builtin.echo \
                 invocation, but the messages array was: {messages:#?}",
            );
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    assert!(
        final_reply.is_some(),
        "timeline never surfaced an assistant message containing 'e2e tool ok' \
         within 10s — the agent loop did not complete the tool round-trip",
    );

    harness
        .runtime
        .shutdown()
        .await
        .expect("runtime shutdown clean");
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

