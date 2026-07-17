//! Whole-journey Telegram scenario through the PRODUCTION composition
//! (`build_reborn_runtime` + `build_telegram_host_runtime_mounts` +
//! `build_webui_services_with_telegram_host_mounts`), asserting at every
//! seam the contract names (`docs/reborn/contracts/telegram-v2.md`):
//!
//! 1. **Admin setup** — the operator PUTs the bot token to the real
//!    protected route; the save pipeline's `getMe` + `setWebhook` are
//!    captured at the network boundary (the URL carries the SUBSTITUTED
//!    `/bot<token>/` segment — the placeholder never leaks), the registered
//!    webhook URL derives from the public base, and `GET` returns the
//!    redacted status.
//! 2. **In-chat activation parks** — a WebChat turn calls
//!    `builtin.extension_install` + `builtin.extension_activate` for
//!    `telegram`; the unpaired caller parks the run as
//!    `TurnStatus::BlockedAuth` (the pairing gate).
//! 3. **Pairing consume resumes** — the pairing route mints a code; the
//!    webhook (verified `X-Telegram-Bot-Api-Secret-Token`, read from the
//!    captured `setWebhook` body exactly where Telegram would hold it)
//!    delivers `/start <CODE>`; consume binds the account (pairing status
//!    facade flips to connected over the durable binding), records the DM
//!    target (the production outbound-target provider lists the
//!    `telegram:dm:…` entry), replies with the paired confirmation, and
//!    dispatches the auth continuation — the parked run RESUMES to
//!    `Completed` and the post-resume model reply lands on the WebChat
//!    timeline.
//! 4. **DM turn renders through the revision workflow** — a subsequent DM
//!    webhook produces a real turn whose final reply is rendered by the
//!    per-revision adapter and egresses as `sendMessage` to the DM chat,
//!    captured at the network boundary with the substituted bot path.
//!
//! Model scripting preserves the single-fake-at-the-vendor-SDK-seam
//! invariant: a scripted `TraceLlm` sits under the REAL
//! `provider_chain_over` + `LlmProviderModelGateway`, routed uniformly to
//! every scope by a `resolve_for_scope` adapter (`scope_gateway.rs`'s
//! pattern for runtimes whose thread scopes are minted at bind time).
//!
//! Manual-QA catalog rows this scenario covers (coverage map:
//! `docs/qa/telegram-coverage-map.md`): qa-telegram admin-setup happy path,
//! unpaired-activation pairing gate, `/start <CODE>` consume + blocked-run
//! resume, paired-DM turn + outbound render, and webhook secret
//! verification on the live route.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode};
use ironclaw_host_api::{AgentId, TenantId, UserId};
use ironclaw_llm::LlmProvider;
use ironclaw_loop_host::{
    HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
    HostManagedModelRequest, HostManagedModelResponse,
};
use ironclaw_network::{
    NetworkHttpEgress, NetworkHttpError, NetworkHttpRequest, NetworkHttpResponse, NetworkUsage,
};
use ironclaw_product_workflow::WebUiAuthenticatedCaller;
use ironclaw_reborn_composition::{
    RebornBuildInput, RebornRuntimeIdentity, RebornRuntimeInput, TelegramHostRuntimeConfig,
    build_reborn_runtime, build_telegram_host_runtime_mounts,
    build_webui_services_with_telegram_host_mounts, local_dev_runtime_policy,
};
use ironclaw_runner::model_gateway::{LlmModelProfilePolicy, LlmProviderModelGateway};
use ironclaw_turns::run_profile::ModelProfileId;
use ironclaw_turns::{GetRunStateRequest, TurnRunId, TurnScope, TurnStatus};
use reborn_support::reply::RebornScriptedReply;
use reborn_support::scripted_provider::{SCRIPTED_MODEL_NAME, scripted_trace_llm};
use serde_json::{Value, json};
use tempfile::tempdir;
use tower::ServiceExt;

const TENANT: &str = "tg-journey-tenant";
const AGENT: &str = "tg-journey-agent";
const USER: &str = "tg-journey-user";
const PUBLIC_BASE: &str = "https://tg-journey.example";
const BOT_ID: i64 = 777000111;
const BOT_USERNAME: &str = "ironclaw_journey_bot";
const BOT_TOKEN: &str = "777000111:journey-bot-token";
const TG_USER_ID: i64 = 9001;
const TG_CHAT_ID: i64 = 555;
const SECRET_HEADER: &str = "X-Telegram-Bot-Api-Secret-Token";
const INTERACTIVE_MODEL_PROFILE: &str = "interactive_model";

/// Scripted Telegram Bot API at the NETWORK boundary: the real
/// `HostEgressTelegramBotApi` / `TelegramProtocolHttpEgress` chains (policy
/// checks, credential-handle resolution, `PathPlaceholder` substitution) run
/// above it; this fake only answers the wire.
#[derive(Debug, Default)]
struct ScriptedTelegramNetwork {
    requests: Mutex<Vec<NetworkHttpRequest>>,
    sent_messages: Mutex<u32>,
}

impl ScriptedTelegramNetwork {
    fn requests(&self) -> Vec<NetworkHttpRequest> {
        self.requests.lock().expect("network requests lock").clone()
    }

    fn request_bodies_for(&self, url_substr: &str) -> Vec<Value> {
        self.requests()
            .iter()
            .filter(|request| request.url.contains(url_substr))
            .map(|request| serde_json::from_slice(&request.body).unwrap_or(Value::Null))
            .collect()
    }
}

fn json_response(status: u16, body: Value) -> NetworkHttpResponse {
    let body = body.to_string().into_bytes();
    NetworkHttpResponse {
        status,
        headers: Vec::new(),
        usage: NetworkUsage {
            request_bytes: 0,
            response_bytes: body.len() as u64,
            resolved_ip: None,
        },
        body,
    }
}

#[async_trait]
impl NetworkHttpEgress for ScriptedTelegramNetwork {
    async fn execute(
        &self,
        request: NetworkHttpRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        let url = request.url.clone();
        self.requests
            .lock()
            .expect("network requests lock")
            .push(request);
        let response = if url.ends_with("/getMe") {
            json_response(
                200,
                json!({"ok": true, "result": {"id": BOT_ID, "is_bot": true, "first_name": "IronClaw", "username": BOT_USERNAME}}),
            )
        } else if url.ends_with("/setWebhook") || url.ends_with("/deleteWebhook") {
            json_response(200, json!({"ok": true, "result": true}))
        } else if url.ends_with("/sendMessage") {
            let mut sent = self.sent_messages.lock().expect("sent counter lock");
            *sent += 1;
            json_response(200, json!({"ok": true, "result": {"message_id": *sent}}))
        } else {
            json_response(
                404,
                json!({"ok": false, "description": "unscripted method"}),
            )
        };
        Ok(response)
    }
}

/// Routes EVERY scope to one real-chain gateway over the scripted `TraceLlm`.
/// The journey's Telegram conversation thread scope is minted at bind time
/// (webhook consume), so per-scope pre-registration (the group harness's
/// `ScopeRegistryGateway`) cannot know it up front; uniform routing keeps the
/// vendor-SDK fake invariant with deterministic FIFO scripting because the
/// journey drives turns strictly in sequence.
struct UniformScopeGateway {
    chain: Arc<dyn HostManagedModelGateway>,
}

#[async_trait]
impl HostManagedModelGateway for UniformScopeGateway {
    async fn stream_model(
        &self,
        _request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::ConfigurationError,
            "UniformScopeGateway only routes via resolve_for_scope".to_string(),
        ))
    }

    fn resolve_for_scope(&self, _scope: &TurnScope) -> Option<Arc<dyn HostManagedModelGateway>> {
        Some(Arc::clone(&self.chain))
    }
}

/// The real `ironclaw_llm` chain over a scripted `TraceLlm` (FIFO across all
/// turns), packaged as a `HostManagedModelGateway` for
/// `with_model_gateway_override`.
async fn scripted_chain_gateway(
    session_dir: &std::path::Path,
    replies: impl IntoIterator<Item = RebornScriptedReply>,
) -> Arc<dyn HostManagedModelGateway> {
    let raw: Arc<dyn LlmProvider> = Arc::new(scripted_trace_llm(replies));
    let session = ironclaw_llm::create_session_manager(ironclaw_llm::SessionConfig {
        session_path: session_dir.join("telegram-journey.session.json"),
        ..ironclaw_llm::SessionConfig::default()
    })
    .await;
    let llm_config = ironclaw_llm::testing::nearai_test_config(SCRIPTED_MODEL_NAME);
    let provider = ironclaw_llm::testing::provider_chain_over(raw, &llm_config, session)
        .await
        .expect("provider chain builds");
    let policy = LlmModelProfilePolicy::new().allow_model_profile(
        ModelProfileId::new(INTERACTIVE_MODEL_PROFILE).expect("model profile id"),
        None,
    );
    Arc::new(UniformScopeGateway {
        chain: Arc::new(LlmProviderModelGateway::new(provider, policy)),
    })
}

fn journey_caller() -> WebUiAuthenticatedCaller {
    WebUiAuthenticatedCaller::new(
        TenantId::new(TENANT).expect("tenant"),
        UserId::new(USER).expect("user"),
        Some(AgentId::new(AGENT).expect("agent")),
        None,
    )
    .with_operator_webui_config(true)
}

/// Drive `router` with `caller` injected the way the production bearer
/// middleware would, returning status + parsed JSON body.
async fn call_route(
    router: axum::Router,
    method: Method,
    path: &str,
    caller: Option<WebUiAuthenticatedCaller>,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(path);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    let mut request = builder
        .body(match &body {
            Some(value) => Body::from(value.to_string()),
            None => Body::empty(),
        })
        .expect("request builds");
    if let Some(caller) = caller {
        request.extensions_mut().insert(caller);
    }
    let response = router.oneshot(request).await.expect("router responds");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("body bytes");
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

/// A private-chat Telegram `message` update.
fn dm_update(update_id: i64, text: &str) -> Value {
    json!({
        "update_id": update_id,
        "message": {
            "message_id": update_id,
            "date": 1_700_000_000,
            "chat": {"id": TG_CHAT_ID, "type": "private"},
            "from": {"id": TG_USER_ID, "is_bot": false, "first_name": "Journey"},
            "text": text,
        }
    })
}

async fn wait_for_run_status(
    coordinator: &Arc<dyn ironclaw_turns::TurnCoordinator>,
    scope: &TurnScope,
    run_id: TurnRunId,
    expected: TurnStatus,
) -> Result<(), String> {
    for _ in 0..200 {
        let state = coordinator
            .get_run_state(GetRunStateRequest {
                scope: scope.clone(),
                run_id,
            })
            .await
            .map_err(|error| format!("run state read failed: {error}"))?;
        if state.status == expected {
            return Ok(());
        }
        if state.status.is_terminal() && state.status != expected {
            return Err(format!(
                "run reached terminal status {:?} while waiting for {expected:?}",
                state.status
            ));
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    Err(format!("run never reached {expected:?}"))
}

#[tokio::test]
async fn telegram_whole_journey_setup_pair_resume_and_dm_reply() {
    let root = tempdir().expect("runtime storage tempdir");
    let storage_root = root.path().join("local-dev");
    let network = Arc::new(ScriptedTelegramNetwork::default());

    // Turn A (WebChat): install + activate telegram; activation parks on the
    // pairing gate and, once resumed, the model reacts to the completed
    // activation with the final text. Turn B (Telegram DM): plain reply.
    let gateway = scripted_chain_gateway(
        root.path(),
        [
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "telegram"}),
            ),
            RebornScriptedReply::tool_call(
                "builtin.extension_activate",
                json!({"extension_id": "telegram"}),
            ),
            RebornScriptedReply::text("telegram is ready"),
            RebornScriptedReply::text("hello from your ironclaw bot"),
        ],
    )
    .await;

    let input = RebornBuildInput::local_dev(USER, storage_root.clone())
        .with_local_runtime_identity(
            TenantId::new(TENANT).expect("tenant"),
            AgentId::new(AGENT).expect("agent"),
        )
        .with_runtime_policy(local_dev_runtime_policy().expect("local-dev policy"))
        .with_network_http_egress_for_test(Arc::clone(&network) as Arc<dyn NetworkHttpEgress>);
    let runtime = build_reborn_runtime(
        RebornRuntimeInput::from_services(input)
            .with_identity(RebornRuntimeIdentity {
                tenant_id: TENANT.to_string(),
                agent_id: AGENT.to_string(),
                source_binding_id: "tg-journey-source".to_string(),
                reply_target_binding_id: "tg-journey-reply".to_string(),
            })
            .with_model_gateway_override(gateway),
    )
    .await
    .expect("production Reborn runtime builds");

    let mounts = build_telegram_host_runtime_mounts(
        &runtime,
        TelegramHostRuntimeConfig::new(
            TenantId::new(TENANT).expect("tenant"),
            AgentId::new(AGENT).expect("agent"),
            None,
            UserId::new(USER).expect("user"),
            Some(PUBLIC_BASE.to_string()),
        ),
    )
    .await
    .expect("telegram host mounts build");
    let webui = build_webui_services_with_telegram_host_mounts(&runtime, None, Some(&mounts))
        .expect("webui bundle builds over the telegram facades");
    let caller = journey_caller();
    let protected = mounts.protected_routes();

    // ── Seam 1: admin save → getMe + setWebhook at the network boundary ────
    let (status, body) = call_route(
        protected.router.clone(),
        Method::PUT,
        "/api/webchat/v2/channels/telegram/setup",
        Some(caller.clone()),
        &[],
        Some(json!({"bot_token": BOT_TOKEN})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "admin save response: {body}");

    let get_me_urls: Vec<String> = network
        .requests()
        .iter()
        .filter(|request| request.url.ends_with("/getMe"))
        .map(|request| request.url.clone())
        .collect();
    assert_eq!(get_me_urls.len(), 1, "save validates the token via getMe");
    assert!(
        get_me_urls[0].contains(&format!("/bot{BOT_TOKEN}/")),
        "the dispatched getMe URL carries the SUBSTITUTED token (never the placeholder): {}",
        get_me_urls[0]
    );
    let set_webhook_bodies = network.request_bodies_for("/setWebhook");
    assert_eq!(set_webhook_bodies.len(), 1, "save registers the webhook");
    assert_eq!(
        set_webhook_bodies[0]["url"],
        format!("{PUBLIC_BASE}/webhooks/extensions/telegram/updates"),
        "the registered URL derives from the deployment public base"
    );
    let webhook_secret = set_webhook_bodies[0]["secret_token"]
        .as_str()
        .expect("setWebhook carries the minted secret")
        .to_string();
    assert!(!webhook_secret.is_empty(), "webhook secret is minted");

    let (status, body) = call_route(
        protected.router.clone(),
        Method::GET,
        "/api/webchat/v2/channels/telegram/setup",
        Some(caller.clone()),
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["configured"], true, "redacted status: {body}");
    assert_eq!(body["bot_username"], BOT_USERNAME);
    assert!(
        body.get("bot_token").is_none(),
        "raw secrets are never echoed: {body}"
    );

    // ── Seam 2: in-chat activation parks BlockedAuth (the pairing gate) ────
    let webui_router = || {
        reborn_support::webui_mount::mount_webui_v2_router(Arc::clone(&webui.api), caller.clone())
    };
    let (status, body) = call_route(
        webui_router(),
        Method::POST,
        "/api/webchat/v2/threads",
        None,
        &[],
        Some(json!({"client_action_id": "journey-thread"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "thread create: {body}");
    let thread_id = body
        .get("thread_id")
        .or_else(|| {
            body.get("thread")
                .and_then(|thread| thread.get("thread_id"))
        })
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("thread id in response: {body}"))
        .to_string();

    let (status, body) = call_route(
        webui_router(),
        Method::POST,
        &format!("/api/webchat/v2/threads/{thread_id}/messages"),
        None,
        &[],
        Some(json!({"client_action_id": "journey-activate", "content": "set up telegram for me"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "send message: {body}");
    let run_id: TurnRunId = body["run_id"]
        .as_str()
        .expect("submit response carries run_id")
        .parse()
        .expect("run id parses");

    let coordinator = runtime.webui_turn_coordinator_for_test();
    let webchat_scope = TurnScope::new_with_owner(
        TenantId::new(TENANT).expect("tenant"),
        Some(AgentId::new(AGENT).expect("agent")),
        None,
        ironclaw_host_api::ThreadId::from_trusted(thread_id.clone()),
        Some(UserId::new(USER).expect("user")),
    );
    wait_for_run_status(
        &coordinator,
        &webchat_scope,
        run_id,
        TurnStatus::BlockedAuth,
    )
    .await
    .expect("unpaired telegram activation parks the run on the pairing gate");

    // ── Seam 3: pairing code → verified /start webhook → binding + resume ──
    let (status, body) = call_route(
        protected.router.clone(),
        Method::POST,
        "/api/webchat/v2/channels/telegram/pairing",
        Some(caller.clone()),
        &[],
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "pairing issue: {body}");
    let code = body["code"].as_str().expect("pairing code").to_string();
    assert_eq!(
        body["deep_link"],
        format!("https://t.me/{BOT_USERNAME}?start={code}"),
        "deep link carries the bot username and code"
    );

    // Forged-secret probe first: the verified route fails closed.
    let (status, _body) = call_route(
        mounts.events.router.clone(),
        Method::POST,
        "/webhooks/extensions/telegram/updates",
        None,
        &[(SECRET_HEADER, "forged-secret")],
        Some(dm_update(1, &format!("/start {code}"))),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "a forged webhook secret must be rejected before any dispatch"
    );

    let (status, _body) = call_route(
        mounts.events.router.clone(),
        Method::POST,
        "/webhooks/extensions/telegram/updates",
        None,
        &[(SECRET_HEADER, webhook_secret.as_str())],
        Some(dm_update(2, &format!("/start {code}"))),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "verified /start consume acks 200");
    if let Some(drain) = mounts.events.drain.as_ref() {
        drain.drain().await;
    }

    // Binding row: the production channel-connection facade (over the durable
    // binding store) flips to connected.
    let (status, body) = call_route(
        protected.router.clone(),
        Method::GET,
        "/api/webchat/v2/channels/telegram/pairing",
        Some(caller.clone()),
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["connected"], true,
        "consume must bind the telegram account: {body}"
    );

    // Paired confirmation reply into the DM.
    let confirmations = network.request_bodies_for("/sendMessage");
    assert!(
        confirmations
            .iter()
            .any(|body| body["chat_id"] == TG_CHAT_ID
                && body["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("Paired"))),
        "consume replies with the paired confirmation in the DM: {confirmations:?}"
    );

    // Continuation dispatch resumed the parked run to completion.
    wait_for_run_status(&coordinator, &webchat_scope, run_id, TurnStatus::Completed)
        .await
        .expect("pairing consume must resume the parked activation run");
    let (status, body) = call_route(
        webui_router(),
        Method::GET,
        &format!("/api/webchat/v2/threads/{thread_id}/timeline"),
        None,
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body["messages"].as_array().is_some_and(|messages| {
            messages.iter().any(|message| {
                message["content"]
                    .as_str()
                    .is_some_and(|content| content.contains("telegram is ready"))
            })
        }),
        "the resumed run finalizes the post-activation reply: {body}"
    );

    // DM target: the production outbound-target provider (registered on the
    // runtime by the mounts build) lists the paired DM through the REAL
    // WebUI facade surface.
    let installation = format!("tg-bot-{BOT_ID}");
    let targets = webui
        .api
        .list_outbound_delivery_targets(caller.clone())
        .await
        .expect("outbound targets list");
    let targets_json = serde_json::to_value(&targets).expect("targets serialize");
    assert!(
        targets_json
            .to_string()
            .contains(&format!("telegram:dm:{installation}:{USER}")),
        "pairing must record the DM delivery target: {targets_json}"
    );

    // ── Seam 4: paired DM turn renders the reply through the revision
    //            workflow into the DM ─────────────────────────────────────
    let (status, _body) = call_route(
        mounts.events.router.clone(),
        Method::POST,
        "/webhooks/extensions/telegram/updates",
        None,
        &[(SECRET_HEADER, webhook_secret.as_str())],
        Some(dm_update(3, "hi ironclaw")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "paired DM webhook acks 200");
    if let Some(drain) = mounts.events.drain.as_ref() {
        drain.drain().await;
    }

    let mut rendered = None;
    for _ in 0..200 {
        let sends = network.request_bodies_for("/sendMessage");
        if let Some(body) = sends.iter().find(|body| {
            body["chat_id"] == TG_CHAT_ID
                && body["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("hello from your ironclaw bot"))
        }) {
            rendered = Some(body.clone());
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let rendered = rendered.expect("the DM turn's final reply renders into the chat");
    assert_eq!(rendered["chat_id"], TG_CHAT_ID);
    let reply_urls: Vec<String> = network
        .requests()
        .iter()
        .filter(|request| request.url.ends_with("/sendMessage"))
        .map(|request| request.url.clone())
        .collect();
    assert!(
        reply_urls
            .iter()
            .all(|url| url.contains(&format!("/bot{BOT_TOKEN}/"))),
        "every outbound send carries the substituted bot path: {reply_urls:?}"
    );

    drop(webui);
    runtime.shutdown().await.expect("runtime shuts down");
}
