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
//! Manual-QA catalog rows this bin covers (coverage map:
//! `docs/qa/telegram-coverage-map.md`): qa-telegram admin-setup happy path,
//! unpaired-activation pairing gate, `/start <CODE>` consume + blocked-run
//! resume, paired-DM turn + outbound render, webhook secret verification on
//! the live route, and the in-DM extension-install gate feedback regression
//! (see `telegram_dm_slack_install_gates_with_action_needed_notice_not_silence`).

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
    /// Every `/sendMessage` attempt with the HTTP status this fake answered —
    /// the authoritative provider-outcome log (a captured request alone does
    /// not prove delivery; see the F3 assertions).
    send_outcomes: Mutex<Vec<(Value, u16)>>,
    sent_messages: Mutex<u32>,
    /// When set, the first `/sendMessage` whose text contains the needle
    /// answers with this HTTP status and an `ok:false` envelope (then
    /// clears) — the F3 blocked-recipient shape, targeted so unrelated
    /// status posts (e.g. the working message) keep succeeding.
    fail_matching_send: Mutex<Option<(String, u16)>>,
}

impl ScriptedTelegramNetwork {
    fn requests(&self) -> Vec<NetworkHttpRequest> {
        self.requests.lock().expect("network requests lock").clone()
    }

    fn fail_send_containing(&self, needle: &str, status: u16) {
        *self.fail_matching_send.lock().expect("fail toggle lock") =
            Some((needle.to_string(), status));
    }

    fn request_bodies_for(&self, url_substr: &str) -> Vec<Value> {
        self.requests()
            .iter()
            .filter(|request| request.url.contains(url_substr))
            .map(|request| serde_json::from_slice(&request.body).unwrap_or(Value::Null))
            .collect()
    }

    /// `(body, answered_status)` for every `/sendMessage` attempt, in order.
    fn send_outcomes(&self) -> Vec<(Value, u16)> {
        self.send_outcomes
            .lock()
            .expect("send outcomes lock")
            .clone()
    }

    /// Count of DELIVERED (2xx-answered) sendMessage bodies whose text
    /// contains `needle`.
    fn delivered_sends_containing(&self, needle: &str) -> usize {
        self.send_outcomes()
            .iter()
            .filter(|(body, status)| {
                (200..300).contains(status)
                    && body["text"]
                        .as_str()
                        .is_some_and(|text| text.contains(needle))
            })
            .count()
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
        // Parse THIS call's body before publishing the request to the shared
        // log: `requests().last()` after the push races concurrent egress
        // calls and could pair this URL with another call's body,
        // misdirecting the failure toggle.
        let request_body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
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
            let matched_failure = {
                let mut toggle = self.fail_matching_send.lock().expect("fail toggle lock");
                match toggle.as_ref() {
                    Some((needle, status))
                        if request_body["text"]
                            .as_str()
                            .is_some_and(|text| text.contains(needle.as_str())) =>
                    {
                        let status = *status;
                        *toggle = None;
                        Some(status)
                    }
                    _ => None,
                }
            };
            if let Some(status) = matched_failure {
                self.send_outcomes
                    .lock()
                    .expect("send outcomes lock")
                    .push((request_body, status));
                return Ok(json_response(
                    status,
                    json!({"ok": false, "description": "scripted send failure"}),
                ));
            }
            let mut sent = self.sent_messages.lock().expect("sent counter lock");
            *sent += 1;
            self.send_outcomes
                .lock()
                .expect("send outcomes lock")
                .push((request_body, 200));
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
    dm_update_from(update_id, TG_USER_ID, TG_CHAT_ID, text)
}

fn dm_update_from(update_id: i64, tg_user: i64, chat_id: i64, text: &str) -> Value {
    json!({
        "update_id": update_id,
        "message": {
            "message_id": update_id,
            "date": 1_700_000_000,
            "chat": {"id": chat_id, "type": "private"},
            "from": {"id": tg_user, "is_bot": false, "first_name": "Journey"},
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

/// The composed production stack every scenario in this bin drives: runtime,
/// telegram host mounts, WebUI bundle over the telegram facades, and the
/// scripted network. `_root` keeps the storage tempdir alive.
struct JourneyStack {
    _root: tempfile::TempDir,
    network: Arc<ScriptedTelegramNetwork>,
    runtime: ironclaw_reborn_composition::RebornRuntime,
    mounts: ironclaw_reborn_composition::TelegramHostMounts,
    webui: ironclaw_reborn_composition::RebornWebuiBundle,
    caller: WebUiAuthenticatedCaller,
}

impl JourneyStack {
    /// Deliver a verified private-chat webhook update and drain the
    /// immediate-ack dispatch tasks so its turn/consume settles.
    async fn webhook_dm(&self, secret: &str, update_id: i64, text: &str) -> StatusCode {
        self.webhook_update(secret, dm_update(update_id, text))
            .await
    }

    async fn webhook_update(&self, secret: &str, update: Value) -> StatusCode {
        let (status, _body) = call_route(
            self.mounts.events.router.clone(),
            Method::POST,
            "/webhooks/extensions/telegram/updates",
            None,
            &[(SECRET_HEADER, secret)],
            Some(update),
        )
        .await;
        if let Some(drain) = self.mounts.events.drain.as_ref() {
            drain.drain().await;
        }
        status
    }

    /// Poll the scripted network for a `sendMessage` into the DM chat whose
    /// text matches `predicate`.
    async fn wait_for_dm_send(&self, predicate: impl Fn(&str) -> bool) -> Result<Value, String> {
        self.wait_for_send_in(TG_CHAT_ID, predicate).await
    }

    async fn wait_for_send_in(
        &self,
        chat_id: i64,
        predicate: impl Fn(&str) -> bool,
    ) -> Result<Value, String> {
        for _ in 0..200 {
            let sends = self.network.request_bodies_for("/sendMessage");
            if let Some(body) = sends.iter().find(|body| {
                body["chat_id"] == chat_id && body["text"].as_str().is_some_and(&predicate)
            }) {
                return Ok(body.clone());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        Err(format!(
            "no matching sendMessage for chat {chat_id}; captured: {:?}",
            self.network.request_bodies_for("/sendMessage")
        ))
    }
}

/// Build the production stack with the given model script (FIFO across every
/// turn the scenario drives).
async fn build_journey_stack(
    replies: impl IntoIterator<Item = RebornScriptedReply>,
) -> JourneyStack {
    build_journey_stack_customized(replies, |input| input).await
}

/// Journey stack with a Google OAuth provider client configured through the
/// production `with_google_oauth_backend` seam — the same wiring `serve`
/// applies from env. With a provider configured, a run that parks
/// `BlockedAuth` on a `google` credential requirement gets a link-shaped
/// challenge (authorization URL), driving the delivery driver's link-prompt
/// arm instead of the credential-entry deny arm.
async fn build_journey_stack_with_google_oauth(
    replies: impl IntoIterator<Item = RebornScriptedReply>,
) -> JourneyStack {
    build_journey_stack_customized(replies, |input| {
        let google = ironclaw_reborn_composition::OAuthClientConfig::new(
            "journey-google-client",
            "http://127.0.0.1:8745/api/reborn/product-auth/oauth/callback",
            None,
        )
        .expect("test google oauth client config");
        input.with_google_oauth_backend(google)
    })
    .await
}

async fn build_journey_stack_customized(
    replies: impl IntoIterator<Item = RebornScriptedReply>,
    customize: impl FnOnce(RebornBuildInput) -> RebornBuildInput,
) -> JourneyStack {
    let root = tempdir().expect("runtime storage tempdir");
    let storage_root = root.path().join("local-dev");
    let network = Arc::new(ScriptedTelegramNetwork::default());
    let gateway = scripted_chain_gateway(root.path(), replies).await;

    let input = RebornBuildInput::local_dev(USER, storage_root.clone())
        .with_local_runtime_identity(
            TenantId::new(TENANT).expect("tenant"),
            AgentId::new(AGENT).expect("agent"),
        )
        .with_runtime_policy(local_dev_runtime_policy().expect("local-dev policy"))
        .with_network_http_egress_for_test(Arc::clone(&network) as Arc<dyn NetworkHttpEgress>);
    let input = customize(input);
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
    JourneyStack {
        _root: root,
        network,
        runtime,
        mounts,
        webui,
        caller: journey_caller(),
    }
}

/// Admin-save the bot token through the real protected route and return the
/// webhook secret captured from the `setWebhook` body (exactly where Telegram
/// would hold it).
async fn admin_save(stack: &JourneyStack) -> String {
    let (status, body) = call_route(
        stack.mounts.protected_routes().router,
        Method::PUT,
        "/api/webchat/v2/channels/telegram/setup",
        Some(stack.caller.clone()),
        &[],
        Some(json!({"bot_token": BOT_TOKEN})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "admin save response: {body}");
    let set_webhook_bodies = stack.network.request_bodies_for("/setWebhook");
    set_webhook_bodies
        .last()
        .and_then(|body| body["secret_token"].as_str())
        .expect("setWebhook carries the minted secret")
        .to_string()
}

/// Mint a pairing code via the real pairing route and consume it over the
/// verified webhook; asserts the pairing facade flips to connected.
async fn pair_via_webhook(stack: &JourneyStack, secret: &str, update_id: i64) {
    pair_user_via_webhook(
        stack,
        secret,
        &stack.caller.clone(),
        TG_USER_ID,
        TG_CHAT_ID,
        update_id,
    )
    .await;
}

/// Multi-user variant: mint the code AS `caller` and consume it from the
/// given telegram identity.
async fn pair_user_via_webhook(
    stack: &JourneyStack,
    secret: &str,
    caller: &WebUiAuthenticatedCaller,
    tg_user: i64,
    chat_id: i64,
    update_id: i64,
) {
    let code = issue_pairing_code(stack, caller).await;
    let status = stack
        .webhook_update(
            secret,
            dm_update_from(update_id, tg_user, chat_id, &format!("/start {code}")),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "verified /start consume acks 200");
    assert!(
        pairing_connected(stack, caller).await,
        "consume must bind the telegram account"
    );
}

async fn issue_pairing_code(stack: &JourneyStack, caller: &WebUiAuthenticatedCaller) -> String {
    let (status, body) = call_route(
        stack.mounts.protected_routes().router,
        Method::POST,
        "/api/webchat/v2/channels/telegram/pairing",
        Some(caller.clone()),
        &[],
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "pairing issue: {body}");
    body["code"].as_str().expect("pairing code").to_string()
}

async fn pairing_connected(stack: &JourneyStack, caller: &WebUiAuthenticatedCaller) -> bool {
    let (status, body) = call_route(
        stack.mounts.protected_routes().router,
        Method::GET,
        "/api/webchat/v2/channels/telegram/pairing",
        Some(caller.clone()),
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    body["connected"] == true
}

#[tokio::test]
async fn telegram_whole_journey_setup_pair_resume_and_dm_reply() {
    // Turn A (WebChat): install + activate telegram; activation parks on the
    // pairing gate and, once resumed, the model reacts to the completed
    // activation with the final text. Turn B (Telegram DM): plain reply.
    let stack = build_journey_stack([
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
    ])
    .await;
    let JourneyStack {
        _root,
        network,
        runtime,
        mounts,
        webui,
        caller,
    } = stack;
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
    let expected_target_id = format!("telegram:dm:{installation}:{USER}");
    let dm_target = targets
        .targets
        .iter()
        .find(|option| option.target.target_id.as_str() == expected_target_id)
        .unwrap_or_else(|| {
            panic!(
                "pairing must record the DM delivery target {expected_target_id}; got: {:?}",
                targets
                    .targets
                    .iter()
                    .map(|option| option.target.target_id.as_str())
                    .collect::<Vec<_>>()
            )
        });
    assert_eq!(
        dm_target.target.channel.as_str(),
        "telegram",
        "the DM target is a telegram-channel entry"
    );
    assert!(
        dm_target.capabilities.final_replies,
        "the paired DM target must accept final replies"
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

/// Ben's regression (2026-07-16): a paired user DMed the bot "can you
/// install slack" and the conversation HUNG — the run parked on slack's
/// OAuth setup gate, and with `TelegramDeliveryProtocol::post_status_message`
/// unwired in v1 every host-authored notice (working message, blocked-run
/// action-needed prompt, busy hints) silently failed, so the DM saw nothing
/// and follow-up messages produced nothing. This pins the fixed contract:
///
/// 1. a paired DM asking for a slack install runs the REAL
///    `builtin.extension_install` + `builtin.extension_activate` capabilities;
///    slack's activation gates on its personal-OAuth credential requirement;
/// 2. the DM RECEIVES host-authored feedback about the gate (via the wired
///    `sendMessage` status path — never silence);
/// 3. the channel is not wedged: a follow-up DM still gets host feedback.
///
/// Covers (docs/qa/telegram-coverage-map.md): the cross-extension in-DM
/// install/gate rows of qa-telegram Conversations + the deferred-busy
/// feedback rows of Telegram Failure and Recovery.
#[tokio::test]
async fn telegram_dm_slack_install_gates_with_action_needed_notice_not_silence() {
    // FIFO: the DM turn calls install then activate; activate parks on the
    // slack OAuth gate, so the post-resume entry and the follow-up-turn entry
    // may stay unconsumed depending on how the gate resolves — both are
    // scripted so neither outcome can starve the trace.
    let stack = build_journey_stack([
        RebornScriptedReply::tool_call(
            "builtin.extension_install",
            json!({"extension_id": "slack"}),
        ),
        RebornScriptedReply::tool_call(
            "builtin.extension_activate",
            json!({"extension_id": "slack"}),
        ),
        RebornScriptedReply::text("slack is connected"),
        RebornScriptedReply::text("still here"),
    ])
    .await;

    let secret = admin_save(&stack).await;
    pair_via_webhook(&stack, &secret, 1).await;
    let sends_after_pairing = stack.network.request_bodies_for("/sendMessage").len();

    // The regression trigger: a paired DM asking for a slack install.
    let status = stack
        .webhook_dm(&secret, 2, "can you install slack for me?")
        .await;
    assert_eq!(status, StatusCode::OK, "paired DM webhook acks 200");

    // The working message posts first ("Ironclaw is thinking...") — the
    // observer's placeholder, riding the wired status path. Its silent
    // failure was half of the "hung" experience.
    stack
        .wait_for_dm_send(|text| text.contains("Ironclaw is thinking"))
        .await
        .expect("the DM turn must post the working message, not silence");

    // Slack's activation parks on its personal-OAuth credential requirement.
    // This stack has no slack OAuth client config, so the challenge is
    // credential-entry shaped and the observer takes the deny arm: it cancels
    // the blocked run and posts the host-authored "set this up in the web
    // app" notice — via the SAME wired status path that used to fail
    // silently. The OAuth-configured link-prompt arm is pinned separately by
    // `telegram_dm_gated_install_posts_oauth_authorization_link_not_silence`.
    let notice = stack
        .wait_for_dm_send(|text| text.contains("credential-based connections can only be set up"))
        .await
        .expect("the gated slack install must produce the action-needed DM notice, not silence");
    assert_eq!(notice["chat_id"], TG_CHAT_ID);
    assert!(
        notice["text"]
            .as_str()
            .is_some_and(|text| text.contains("ask me again here")),
        "the notice tells the user how to recover: {notice}"
    );

    // Not wedged: a follow-up DM still produces host feedback (a busy hint
    // for the parked run, or a fresh reply if the gate auto-resolved).
    let before_follow_up = stack.network.request_bodies_for("/sendMessage").len();
    assert!(
        before_follow_up > sends_after_pairing,
        "the gate notice itself must be a new send"
    );
    let status = stack.webhook_dm(&secret, 3, "are you still there?").await;
    assert_eq!(status, StatusCode::OK, "follow-up DM webhook acks 200");
    for _ in 0..200 {
        if stack.network.request_bodies_for("/sendMessage").len() > before_follow_up {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let all_sends = stack.network.request_bodies_for("/sendMessage");
    assert!(
        all_sends.len() > before_follow_up,
        "a follow-up DM to a gated conversation must still get host feedback \
         (busy hint or reply), got no new sends: {all_sends:?}"
    );

    stack.runtime.shutdown().await.expect("runtime shuts down");
}

/// Ben's second regression (2026-07-17): the OAuth-CONFIGURED sibling of the
/// scenario above. With a provider client configured, a gated install parks
/// `BlockedAuth` with a link-shaped challenge and the delivery driver takes
/// the link-prompt arm — building an `AuthPrompt` whose authorization URL the
/// adapter must deliver to the DM. The Telegram adapter used to record that
/// prompt `Deferred` and render nothing, so the DM watched "thinking…" get
/// deleted and then SILENCE while the driver waited for an authorization
/// nobody was ever offered. Pins: the auth prompt reaches the chat as a
/// `sendMessage` carrying the authorization URL.
#[tokio::test]
async fn telegram_dm_gated_install_posts_oauth_authorization_link_not_silence() {
    // FIFO: install + activate; activate parks on the google OAuth gate, so
    // the post-resume entry and the follow-up entry may stay unconsumed
    // (nobody completes OAuth inside this test).
    let stack = build_journey_stack_with_google_oauth([
        RebornScriptedReply::tool_call(
            "builtin.extension_install",
            json!({"extension_id": "gmail"}),
        ),
        RebornScriptedReply::tool_call(
            "builtin.extension_activate",
            json!({"extension_id": "gmail"}),
        ),
        RebornScriptedReply::text("gmail is connected"),
        RebornScriptedReply::text("still here"),
    ])
    .await;

    let secret = admin_save(&stack).await;
    pair_via_webhook(&stack, &secret, 1).await;

    let status = stack.webhook_dm(&secret, 2, "can you set up gmail?").await;
    assert_eq!(status, StatusCode::OK, "paired DM webhook acks 200");

    stack
        .wait_for_dm_send(|text| text.contains("Ironclaw is thinking"))
        .await
        .expect("the DM turn must post the working message");

    // The actionable prompt: headline/body plus the Google authorization URL
    // rendered by the adapter — the tap-to-authorize path.
    let prompt = stack
        .wait_for_dm_send(|text| text.contains("accounts.google.com"))
        .await
        .expect("the gated install must DM the authorization link, not silence");
    assert_eq!(prompt["chat_id"], TG_CHAT_ID);
    let text = prompt["text"].as_str().expect("prompt text");
    assert!(
        text.contains("Open this link to authorize"),
        "prompt explains the link is the way to continue: {text}"
    );

    stack.runtime.shutdown().await.expect("runtime shuts down");
}

/// Multi-user identity isolation through the production stack.
///
/// Covers (docs/qa/telegram-coverage-map.md): qa-telegram:D1:01 (threads
/// owned by the bound user), qa-telegram:D1:02 (replies return only to the
/// originating bound user), qa-telegram:D4 (unpairing A leaves B intact),
/// qa-telegram:P6 integration leg (an identity bound to another user is
/// refused a re-bind by code), qa-telegram:R2 (self-service disconnect
/// unpairs only the requester), qa-multitenant-setup:J5:03 (one paired user
/// plus an unbound provider identity coexist without bleed), and
/// qa-telegram:R8 (a DM after removal gets the static hint, no turn).
#[tokio::test]
async fn telegram_two_users_stay_isolated_across_pairing_reply_and_unpair() {
    const USER_B: &str = "tg-journey-user-b";
    const TG_USER_B: i64 = 9002;
    const TG_CHAT_B: i64 = 556;

    // FIFO: A's DM reply, B's DM reply, B's post-unpair reply. A's
    // post-unpair DM must consume NOTHING (hint only, no turn).
    let stack = build_journey_stack([
        RebornScriptedReply::text("isolated reply for user A"),
        RebornScriptedReply::text("isolated reply for user B"),
        RebornScriptedReply::text("user B still works"),
    ])
    .await;
    let caller_a = stack.caller.clone();
    let caller_b = WebUiAuthenticatedCaller::new(
        TenantId::new(TENANT).expect("tenant"),
        UserId::new(USER_B).expect("user"),
        Some(AgentId::new(AGENT).expect("agent")),
        None,
    );

    let secret = admin_save(&stack).await;
    pair_user_via_webhook(&stack, &secret, &caller_a, TG_USER_ID, TG_CHAT_ID, 1).await;
    pair_user_via_webhook(&stack, &secret, &caller_b, TG_USER_B, TG_CHAT_B, 2).await;

    // Each DM routes to ITS user's thread and replies to ITS chat only.
    let status = stack.webhook_dm(&secret, 3, "hello from A").await;
    assert_eq!(status, StatusCode::OK);
    let reply_a = stack
        .wait_for_dm_send(|text| text.contains("isolated reply for user A"))
        .await
        .expect("A's DM gets A's reply");
    assert_eq!(reply_a["chat_id"], TG_CHAT_ID);

    let status = stack
        .webhook_update(
            &secret,
            dm_update_from(4, TG_USER_B, TG_CHAT_B, "hello from B"),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let reply_b = stack
        .wait_for_send_in(TG_CHAT_B, |text| text.contains("isolated reply for user B"))
        .await
        .expect("B's DM gets B's reply");
    assert_eq!(
        reply_b["chat_id"], TG_CHAT_B,
        "B's reply must land in B's chat only"
    );
    let cross_bleed = stack
        .network
        .request_bodies_for("/sendMessage")
        .iter()
        .any(|body| {
            (body["chat_id"] == TG_CHAT_ID
                && body["text"].as_str().is_some_and(|t| t.contains("user B")))
                || (body["chat_id"] == TG_CHAT_B
                    && body["text"].as_str().is_some_and(|t| t.contains("user A")))
        });
    assert!(!cross_bleed, "replies must never cross user boundaries");

    // P6: a code minted by B cannot re-bind A's telegram identity.
    let code_b = issue_pairing_code(&stack, &caller_b).await;
    let status = stack
        .webhook_update(
            &secret,
            dm_update_from(5, TG_USER_ID, TG_CHAT_ID, &format!("/start {code_b}")),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    stack
        .wait_for_dm_send(|text| text.contains("already paired"))
        .await
        .expect("the already-bound identity gets an explicit refusal");
    assert!(
        pairing_connected(&stack, &caller_a).await,
        "A's binding survives the re-bind attempt"
    );

    // D4/R2: A disconnects; B is untouched.
    let (status, _body) = call_route(
        stack.mounts.protected_routes().router,
        Method::DELETE,
        "/api/webchat/v2/channels/telegram/pairing",
        Some(caller_a.clone()),
        &[],
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "self-service unpair succeeds"
    );
    assert!(!pairing_connected(&stack, &caller_a).await, "A unpaired");
    assert!(pairing_connected(&stack, &caller_b).await, "B still paired");

    // R8: A's DM now fails closed with the static hint and starts NO turn —
    // proven at the send seam: the only new delivered 555-send is the hint,
    // and no scripted entry was consumed (a turn would have delivered
    // "user B still works", the next FIFO entry, into A's chat).
    let outcomes_before = stack.network.send_outcomes().len();
    let status = stack.webhook_dm(&secret, 6, "hello again from A").await;
    assert_eq!(status, StatusCode::OK);
    stack
        .wait_for_dm_send(|text| text.contains("Pair your account"))
        .await
        .expect("unpaired A gets the static pairing hint");
    let new_outcomes: Vec<(Value, u16)> = stack.network.send_outcomes().split_off(outcomes_before);
    assert!(
        new_outcomes.iter().all(|(body, _)| {
            body["chat_id"] == TG_CHAT_ID
                && body["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("Pair your account"))
        }),
        "a post-unpair DM must produce ONLY the static hint (no turn, no reply): {new_outcomes:?}"
    );

    // B keeps working end to end.
    let status = stack
        .webhook_update(
            &secret,
            dm_update_from(7, TG_USER_B, TG_CHAT_B, "still working?"),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let reply = stack
        .wait_for_send_in(TG_CHAT_B, |text| text.contains("user B still works"))
        .await
        .expect("B's channel is unaffected by A's unpair");
    assert_eq!(reply["chat_id"], TG_CHAT_B);

    stack.runtime.shutdown().await.expect("runtime shuts down");
}

/// Delivery idempotency and honest failure through the production stack.
///
/// Covers (docs/qa/telegram-coverage-map.md): qa-telegram:F1 (a retried
/// update produces exactly one turn and one reply), qa-telegram:F3:01 (a
/// blocked-recipient 403 on the reply send does not retry-storm),
/// qa-telegram:F3:02 (the next send succeeds once the failure clears), and
/// the integration leg of qa-telegram:F2 (send outages surface as honest
/// failures — the DeliveryStatus mapping itself is pinned by
/// ironclaw_telegram_v2_adapter's render tests).
#[tokio::test]
async fn telegram_duplicate_updates_and_send_failures_stay_honest() {
    let stack = build_journey_stack([
        RebornScriptedReply::text("first reply"),
        RebornScriptedReply::text("reply during outage"),
        RebornScriptedReply::text("reply after recovery"),
    ])
    .await;
    let secret = admin_save(&stack).await;
    pair_via_webhook(&stack, &secret, 1).await;

    // F1: the same update delivered twice (Telegram redelivery) produces
    // exactly one turn and one reply. Turn-count seam: the model script is a
    // strict FIFO, so a second (deduplication-escaping) turn MUST consume and
    // deliver the NEXT scripted entry — asserting zero later-entry sends is
    // the authoritative "no second turn" proof, and the delivered count
    // (2xx-answered, not merely captured) pins exactly one reply.
    let update = dm_update(2, "count me once");
    assert_eq!(
        stack.webhook_update(&secret, update.clone()).await,
        StatusCode::OK
    );
    assert_eq!(stack.webhook_update(&secret, update).await, StatusCode::OK);
    stack
        .wait_for_dm_send(|text| text.contains("first reply"))
        .await
        .expect("the update produces its reply");
    // Settle any straggling duplicate dispatch before counting.
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        stack.network.delivered_sends_containing("first reply"),
        1,
        "a redelivered update must not produce a second delivered reply"
    );
    for later_entry in ["reply during outage", "reply after recovery"] {
        assert_eq!(
            stack.network.delivered_sends_containing(later_entry),
            0,
            "a second turn would have consumed the next scripted entry ({later_entry}); \
             its absence proves exactly one turn ran"
        );
    }

    // F3:01 — the recipient blocks the bot: the reply send gets a 403.
    // Honest failure, no retry storm (the adapter maps 403 to
    // FailedUnauthorized — a terminal, non-retryable delivery status).
    stack
        .network
        .fail_send_containing("reply during outage", 403);
    assert_eq!(
        stack
            .webhook_dm(&secret, 3, "talk to me during the outage")
            .await,
        StatusCode::OK
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    let outage_outcomes: Vec<u16> = stack
        .network
        .send_outcomes()
        .iter()
        .filter(|(body, _)| {
            body["text"]
                .as_str()
                .is_some_and(|text| text.contains("reply during outage"))
        })
        .map(|(_, status)| *status)
        .collect();
    assert_eq!(
        outage_outcomes,
        vec![403],
        "the outage turn's reply must be attempted exactly once, answered 403, \
         and never retried"
    );
    assert_eq!(
        stack
            .network
            .delivered_sends_containing("reply after recovery"),
        0,
        "the recovery entry must not have been consumed before the recovery turn"
    );

    // F3:02 — the block clears; the next turn's reply DELIVERS (2xx-answered,
    // not merely captured — the 403'd request above is also "captured").
    assert_eq!(
        stack.webhook_dm(&secret, 4, "are we back?").await,
        StatusCode::OK
    );
    let mut recovered_deliveries = 0;
    for _ in 0..200 {
        recovered_deliveries = stack
            .network
            .delivered_sends_containing("reply after recovery");
        if recovered_deliveries > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(
        recovered_deliveries, 1,
        "exactly one delivered recovery reply after the failure clears"
    );

    stack.runtime.shutdown().await.expect("runtime shuts down");
}
