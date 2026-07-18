//! Shared harness for the Telegram journey scenarios: the scripted
//! Telegram Bot API network fake, the production-composition
//! `JourneyStack`, and the webhook/pairing/admin helpers every
//! scenario drives.

use super::reborn_support;
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

pub(crate) const TENANT: &str = "tg-journey-tenant";
pub(crate) const AGENT: &str = "tg-journey-agent";
pub(crate) const USER: &str = "tg-journey-user";
pub(crate) const PUBLIC_BASE: &str = "https://tg-journey.example";
pub(crate) const BOT_ID: i64 = 777000111;
pub(crate) const BOT_USERNAME: &str = "ironclaw_journey_bot";
pub(crate) const BOT_TOKEN: &str = "777000111:journey-bot-token";
pub(crate) const TG_USER_ID: i64 = 9001;
pub(crate) const TG_CHAT_ID: i64 = 555;
pub(crate) const SECRET_HEADER: &str = "X-Telegram-Bot-Api-Secret-Token";
pub(crate) const INTERACTIVE_MODEL_PROFILE: &str = "interactive_model";

/// Scripted Telegram Bot API at the NETWORK boundary: the real
/// `HostEgressTelegramBotApi` / `TelegramProtocolHttpEgress` chains (policy
/// checks, credential-handle resolution, `PathPlaceholder` substitution) run
/// above it; this fake only answers the wire.
#[derive(Debug, Default)]
pub(crate) struct ScriptedTelegramNetwork {
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
    pub(crate) fn requests(&self) -> Vec<NetworkHttpRequest> {
        self.requests.lock().expect("network requests lock").clone()
    }

    pub(crate) fn fail_send_containing(&self, needle: &str, status: u16) {
        *self.fail_matching_send.lock().expect("fail toggle lock") =
            Some((needle.to_string(), status));
    }

    pub(crate) fn request_bodies_for(&self, url_substr: &str) -> Vec<Value> {
        self.requests()
            .iter()
            .filter(|request| request.url.contains(url_substr))
            .map(|request| {
                serde_json::from_slice(&request.body)
                    .expect("captured Bot API request body is valid JSON")
            })
            .collect()
    }

    /// `(body, answered_status)` for every `/sendMessage` attempt, in order.
    pub(crate) fn send_outcomes(&self) -> Vec<(Value, u16)> {
        self.send_outcomes
            .lock()
            .expect("send outcomes lock")
            .clone()
    }

    /// Count of DELIVERED (2xx-answered) sendMessage bodies whose text
    /// contains `needle`.
    pub(crate) fn delivered_sends_containing(&self, needle: &str) -> usize {
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

pub(crate) fn json_response(status: u16, body: Value) -> NetworkHttpResponse {
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
        let request_body: Value =
            serde_json::from_slice(&request.body).expect("captured sendMessage body is valid JSON");
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
pub(crate) struct UniformScopeGateway {
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
pub(crate) async fn scripted_chain_gateway(
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

pub(crate) fn journey_caller() -> WebUiAuthenticatedCaller {
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
pub(crate) async fn call_route(
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
        if bytes.is_empty() {
            // 204s and other empty-body responses are legitimate.
            Value::Null
        } else {
            // This helper serves JSON API routes AND opaque webhook acks; a
            // non-JSON body is preserved as a string so any JSON-field
            // assertion against it fails loudly SHOWING the body, rather
            // than silently collapsing to Null.
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
        }
    };
    (status, value)
}

/// A private-chat Telegram `message` update.
pub(crate) fn dm_update(update_id: i64, text: &str) -> Value {
    dm_update_from(update_id, TG_USER_ID, TG_CHAT_ID, text)
}

pub(crate) fn dm_update_from(update_id: i64, tg_user: i64, chat_id: i64, text: &str) -> Value {
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

pub(crate) async fn wait_for_run_status(
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
pub(crate) struct JourneyStack {
    pub(crate) _root: tempfile::TempDir,
    pub(crate) network: Arc<ScriptedTelegramNetwork>,
    pub(crate) runtime: ironclaw_reborn_composition::RebornRuntime,
    pub(crate) mounts: ironclaw_reborn_composition::TelegramHostMounts,
    pub(crate) webui: ironclaw_reborn_composition::RebornWebuiBundle,
    pub(crate) caller: WebUiAuthenticatedCaller,
}

impl JourneyStack {
    /// Deliver a verified private-chat webhook update and drain the
    /// immediate-ack dispatch tasks so its turn/consume settles.
    pub(crate) async fn webhook_dm(&self, secret: &str, update_id: i64, text: &str) -> StatusCode {
        self.webhook_update(secret, dm_update(update_id, text))
            .await
    }

    pub(crate) async fn webhook_update(&self, secret: &str, update: Value) -> StatusCode {
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
    pub(crate) async fn wait_for_dm_send(
        &self,
        predicate: impl Fn(&str) -> bool,
    ) -> Result<Value, String> {
        self.wait_for_send_in(TG_CHAT_ID, predicate).await
    }

    pub(crate) async fn wait_for_send_in(
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
pub(crate) async fn build_journey_stack(
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
pub(crate) async fn build_journey_stack_with_google_oauth(
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

pub(crate) async fn build_journey_stack_customized(
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
pub(crate) async fn admin_save(stack: &JourneyStack) -> String {
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
pub(crate) async fn pair_via_webhook(stack: &JourneyStack, secret: &str, update_id: i64) {
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
pub(crate) async fn pair_user_via_webhook(
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

pub(crate) async fn issue_pairing_code(
    stack: &JourneyStack,
    caller: &WebUiAuthenticatedCaller,
) -> String {
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

pub(crate) async fn pairing_connected(
    stack: &JourneyStack,
    caller: &WebUiAuthenticatedCaller,
) -> bool {
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
