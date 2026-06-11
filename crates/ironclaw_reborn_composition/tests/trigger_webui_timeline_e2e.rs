//! End-to-end test: trigger fired through the REAL poller → WebUI timeline
//! access through the composed service stack.
//!
//! ## What this proves
//!
//! 1. **Happy path** (`timeline_opens_for_creator`):
//!    A trigger fired by the real poller creates a thread in the in-memory
//!    thread service. The trigger creator's WebUI bearer then retrieves 200
//!    from `GET /api/webchat/v2/threads/{thread_id}/timeline` with a timeline
//!    containing the thread record and at least one message (the trigger
//!    prompt). This exercises the full composition path:
//!
//!    real poller → `record_trigger_prompt` → `InMemorySessionThreadService`
//!    → `RebornServices::get_timeline` → composed WebUI v2 HTTP layer.
//!
//! 2. **Automation-service-stack path**
//!    (`timeline_opens_for_creator_via_automation_service_stack`):
//!    Same setup, second independent trigger. Confirms the automation facade
//!    wiring is correct across multiple trigger fires.
//!
//!    The thread_id is discovered via `session_thread_service()` (test-support
//!    accessor) because `GET /api/webchat/v2/threads` filters automation-trigger
//!    threads out of the list response (they carry `metadata_json` with the
//!    automation marker and are excluded by `is_automation_trigger_thread`).
//!
//!    **In-memory fallback limitation (read before assuming this tests the
//!    fallback):**
//!    `RebornServices::get_timeline` falls back to
//!    `check_automation_trigger_access` → `AutomationProductFacade::
//!    resolve_run_thread_scope` only when the primary session-scoped
//!    `list_thread_history` returns `UnknownThread` or `ThreadScopeMismatch`.
//!
//!    With the in-memory backend the thread is stored under the UUID assigned
//!    by the conversation binding layer; `TriggerRunRecord.thread_id` holds the
//!    hex `TriggerRouteThreadId`. These are two separate values, so calling
//!    `get_timeline` with the hex fails at the `list_thread_history` step even
//!    after `resolve_run_thread_scope` succeeds.
//!
//!    Tests 1 and 2 therefore use the real UUID (obtained via
//!    `session_thread_service().list_threads_for_scope`) for the timeline URL
//!    so the PRIMARY session-scoped lookup succeeds directly. The fallback is
//!    NOT triggered in those scenarios. Full end-to-end fallback coverage
//!    (hex → fallback → 200) requires a PostgreSQL backend.
//!
//! 3. **Negative / fallback-denial** (`timeline_denied_for_different_user`):
//!    A different authenticated user on the same UUID thread_id gets 404.
//!
//!    - Direct scope: `owner_user_id = OTHER_USER`; thread was stored with
//!      `owner_user_id = USER` → `UnknownThread` → fallback triggered.
//!    - Fallback authz: `list_scoped_triggers(creator = OTHER_USER)` → empty
//!      (OTHER_USER has no triggers) → `Ok(None)` → 404.
//!
//!    This test exercises the automation-trigger fallback's authorization-denial
//!    branch end-to-end through the full HTTP stack.

#![cfg(all(feature = "test-support", feature = "webui-v2-beta"))]

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chrono::Utc;
use http_body_util::BodyExt;
use ironclaw_host_api::{AgentId, TenantId, UserId};
use ironclaw_loop_support::{
    HostManagedModelError, HostManagedModelGateway, HostManagedModelRequest,
    HostManagedModelResponse,
};
use ironclaw_reborn_composition::{
    RebornCompositionProfile, RebornLocalRuntimeProfileOptions, RebornRuntime,
    RebornRuntimeIdentity, RebornRuntimeInput, RebornWebuiBundle, TriggerPollerSettings,
    WebuiAuthentication, WebuiAuthenticator, WebuiServeConfig, build_reborn_runtime,
    build_webui_services, local_runtime_build_input_with_options, webui_v2_app,
};
use ironclaw_threads::{ListThreadsForScopeRequest, SessionThreadService, ThreadScope};
use ironclaw_triggers::{
    TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID, TRIGGER_TRUSTED_ADAPTER_KIND,
    TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE, TriggerCompletionPolicy, TriggerId,
    TriggerPollerWorkerConfig, TriggerRecord, TriggerRepository, TriggerSchedule,
    TriggerSourceKind, TriggerState,
};
use tower::ServiceExt;

const TENANT: &str = "timeline-e2e-tenant";
const USER: &str = "timeline-e2e-owner";
const OTHER_USER: &str = "timeline-e2e-other-user";
const AGENT: &str = "timeline-e2e-agent";

/// Bearer token for the trigger creator (authorized to see the automation).
const OWNER_TOKEN: &str = "owner-bearer-token";

/// Bearer token for a different user (unauthorized to see the automation).
const OTHER_TOKEN: &str = "other-bearer-token";

const TRIGGER_PROMPT: &str = "timeline-e2e-trigger-prompt-do-not-rephrase";

// ─── stub model gateway ──────────────────────────────────────────────────────

#[derive(Debug, Default)]
struct StaticGateway;

#[async_trait]
impl HostManagedModelGateway for StaticGateway {
    async fn stream_model(
        &self,
        _request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        Ok(HostManagedModelResponse::assistant_reply(
            "trigger webui timeline e2e ok".to_string(),
        ))
    }
}

// ─── two-user authenticator ──────────────────────────────────────────────────

struct TwoUserAuthenticator {
    owner_user_id: UserId,
    other_user_id: UserId,
}

#[async_trait]
impl WebuiAuthenticator for TwoUserAuthenticator {
    async fn authenticate(&self, token: &str) -> Option<WebuiAuthentication> {
        match token {
            t if t == OWNER_TOKEN => Some(WebuiAuthentication::user(self.owner_user_id.clone())),
            t if t == OTHER_TOKEN => Some(WebuiAuthentication::user(self.other_user_id.clone())),
            _ => None,
        }
    }
}

// ─── runtime builder ─────────────────────────────────────────────────────────

async fn build_timeline_runtime(root: &tempfile::TempDir) -> RebornRuntime {
    let host_home_root = root.path().join("host-home");
    std::fs::create_dir_all(&host_home_root).expect("host home root");

    let input = local_runtime_build_input_with_options(
        RebornCompositionProfile::LocalDevYolo,
        USER,
        root.path().join("local-dev"),
        RebornLocalRuntimeProfileOptions {
            confirm_host_access: true,
        },
    )
    .expect("local-yolo runtime input")
    .with_local_dev_confirmed_host_home_root(host_home_root);

    let input = RebornRuntimeInput::from_services(input)
        .with_identity(RebornRuntimeIdentity {
            tenant_id: TENANT.to_string(),
            agent_id: AGENT.to_string(),
            source_binding_id: "timeline-e2e-source".to_string(),
            reply_target_binding_id: "timeline-e2e-reply".to_string(),
        })
        .with_trigger_poller_settings(
            TriggerPollerSettings::enabled_with_tenant_scoped_authorizer_for_test()
                .with_worker_config(TriggerPollerWorkerConfig {
                    poll_interval: Duration::from_millis(20),
                    ..Default::default()
                }),
        )
        .with_model_gateway_override(Arc::new(StaticGateway) as Arc<dyn HostManagedModelGateway>);

    build_reborn_runtime(input)
        .await
        .expect("timeline e2e runtime builds")
}

// ─── wait helpers ────────────────────────────────────────────────────────────

async fn wait_for_trigger_fire(
    repo: &Arc<dyn TriggerRepository>,
    tenant_id: &TenantId,
    trigger_id: TriggerId,
) -> TriggerRecord {
    let stop = Instant::now() + Duration::from_secs(15);
    let mut last: Option<TriggerRecord> = None;
    while Instant::now() < stop {
        let current = repo
            .get_trigger(tenant_id.clone(), trigger_id)
            .await
            .expect("get_trigger")
            .expect("record present");
        if current.last_run_at.is_some() {
            return current;
        }
        last = Some(current);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    last.expect("at least one read should have succeeded in wait_for_trigger_fire")
}

/// Wait until at least one thread appears in the creator's scope in the
/// session thread service (the store the trigger poller writes to via
/// `record_trigger_prompt`).
///
/// We bypass `GET /api/webchat/v2/threads` because that endpoint filters
/// automation-trigger threads out of its list response (they carry the
/// automation `metadata_json` marker). Instead, we read from the same
/// `SessionThreadService` the trigger poller writes to via the test-support
/// accessor `RebornRuntime::session_thread_service`.
///
/// Returns the `thread_id` (UUID) of the first thread found in the scope.
async fn wait_for_session_thread(
    thread_service: &Arc<dyn SessionThreadService>,
    tenant_id: &TenantId,
    user_id: &UserId,
    agent_id: &AgentId,
) -> String {
    let scope = ThreadScope {
        tenant_id: tenant_id.clone(),
        agent_id: agent_id.clone(),
        project_id: None,
        owner_user_id: Some(user_id.clone()),
        mission_id: None,
    };
    let stop = Instant::now() + Duration::from_secs(15);
    loop {
        let response = thread_service
            .list_threads_for_scope(ListThreadsForScopeRequest {
                scope: scope.clone(),
                limit: Some(5),
                cursor: None,
            })
            .await
            .expect("list_threads_for_scope");
        if let Some(first) = response.threads.first() {
            return first.thread_id.as_str().to_string();
        }
        if Instant::now() >= stop {
            panic!("no thread appeared in the session thread service within 15s");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

// ─── helper: build a trigger with project_id = None ──────────────────────────
//
// Using project_id = None means the WebUI config also omits a default
// project_id (`with_default_project_id` is NOT called). This ensures
// `list_scoped_triggers(project_id = None)` matches the trigger record in the
// automation fallback authz check (`timeline_denied_for_different_user`).

fn make_trigger_record(
    tenant_id: TenantId,
    user_id: UserId,
    agent_id: AgentId,
    name: &str,
) -> TriggerRecord {
    TriggerRecord {
        trigger_id: TriggerId::new(),
        tenant_id,
        creator_user_id: user_id,
        agent_id: Some(agent_id),
        // project_id = None so WebUI caller scope (also None) matches.
        project_id: None,
        name: name.to_string(),
        source: TriggerSourceKind::Schedule,
        schedule: TriggerSchedule::cron("* * * * *").expect("valid cron expression"),
        completion_policy: TriggerCompletionPolicy::CompleteAfterFirstFire,
        prompt: TRIGGER_PROMPT.to_string(),
        state: TriggerState::Scheduled,
        next_run_at: Utc::now() - chrono::Duration::seconds(120),
        last_run_at: None,
        last_fired_slot: None,
        last_status: None,
        active_fire_slot: None,
        active_run_ref: None,
        created_at: Utc::now(),
    }
}

// ─── helper: build the WebUI app for a given runtime ─────────────────────────
//
// `with_default_project_id` is deliberately NOT called so the WebUI caller
// scope has `project_id = None`, matching the trigger records (also None).
// This ensures `list_scoped_triggers(project_id = None)` finds the trigger
// in the fallback authz check.

fn build_timeline_app(runtime: &RebornRuntime) -> axum::Router {
    let bundle: RebornWebuiBundle =
        build_webui_services(runtime, None).expect("build_webui_services");

    let tenant_id = TenantId::new(TENANT).expect("tenant id");
    let owner_user_id = UserId::new(USER).expect("owner user id");
    let other_user_id = UserId::new(OTHER_USER).expect("other user id");

    let config = WebuiServeConfig::new(
        tenant_id,
        Arc::new(TwoUserAuthenticator {
            owner_user_id,
            other_user_id,
        }),
        vec![],
    )
    .with_default_agent_id(AgentId::new(AGENT).expect("agent id"));
    // NOTE: with_default_project_id intentionally omitted — trigger records
    // use project_id = None and the caller scope must match.

    webui_v2_app(bundle, config).expect("webui_v2_app")
}

// ─── HTTP helpers ─────────────────────────────────────────────────────────────

async fn http_get(app: &axum::Router, uri: &str, token: &str) -> (StatusCode, serde_json::Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(uri)
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body bytes")
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

async fn get_timeline(
    app: &axum::Router,
    thread_id: &str,
    token: &str,
) -> (StatusCode, serde_json::Value) {
    http_get(
        app,
        &format!("/api/webchat/v2/threads/{thread_id}/timeline"),
        token,
    )
    .await
}

/// Seed a trigger, wait for the poller to fire it, then return the actual UUID
/// `ThreadId` that `record_trigger_prompt` stored in the session thread service.
///
/// ## Thread-id resolution strategy
///
/// `TriggerRunRecord.thread_id` is the `TriggerRouteThreadId` (64-char hex
/// derived from trigger_id + fire_slot). The session thread service stores the
/// thread under the UUID assigned by the conversation binding layer — a
/// distinct value. The `GET /api/webchat/v2/threads` endpoint filters
/// automation-trigger threads out of its response (they carry the automation
/// `metadata_json` marker), so we cannot use it to discover the UUID.
///
/// Instead we query the session thread service directly via
/// `RebornRuntime::session_thread_service()` (test-support accessor) and
/// `list_threads_for_scope` with the creator's scope. This is the same store
/// the trigger poller writes to, so the UUID appears there as soon as
/// `record_trigger_prompt` completes.
async fn fire_trigger_and_get_session_thread_id(
    runtime: &RebornRuntime,
    tenant_id: &TenantId,
    user_id: &UserId,
    agent_id: &AgentId,
    trigger_name: &str,
) -> String {
    let repo = runtime.trigger_repository().expect("trigger repository");
    let pairing = runtime
        .trigger_conversation_pairing()
        .expect("conversation pairing");

    pairing
        .pair_external_actor(
            tenant_id.clone(),
            ironclaw_conversations::AdapterKind::new(TRIGGER_TRUSTED_ADAPTER_KIND)
                .expect("adapter kind"),
            ironclaw_conversations::AdapterInstallationId::new(
                TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID,
            )
            .expect("installation id"),
            ironclaw_conversations::ExternalActorRef::new(
                TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE,
                user_id.as_str(),
            )
            .expect("actor ref"),
            user_id.clone(),
        )
        .await
        .expect("pair external actor");

    let record = make_trigger_record(
        tenant_id.clone(),
        user_id.clone(),
        agent_id.clone(),
        trigger_name,
    );
    let trigger_id = record.trigger_id;
    repo.upsert_trigger(record).await.expect("upsert trigger");

    // Phase 1: wait for the trigger to fire (checks the trigger repo, no HTTP).
    let settled = wait_for_trigger_fire(&repo, tenant_id, trigger_id).await;
    assert!(
        settled.last_run_at.is_some(),
        "trigger was not fired by the poller within 15s — record: {settled:?}"
    );

    // Phase 2: after the trigger fires, record_trigger_prompt runs
    // asynchronously. Poll the session thread service directly (same Arc the
    // poller writes to) until the thread appears.
    let thread_service = runtime.session_thread_service();
    wait_for_session_thread(&thread_service, tenant_id, user_id, agent_id).await
}

// ─── tests ───────────────────────────────────────────────────────────────────

/// Happy path: the trigger creator's bearer retrieves the timeline for a
/// trigger-fired thread.
///
/// Exercises the full composition path:
///   real poller → `record_trigger_prompt` → `InMemorySessionThreadService`
///   → `RebornServices::get_timeline` → composed WebUI v2 HTTP layer.
///
/// The thread_id is the UUID stored by the session thread service (obtained
/// via `RebornRuntime::session_thread_service()`). The creator's scope matches
/// the stored scope exactly so the primary session-scoped lookup succeeds
/// without triggering the automation fallback.
#[tokio::test]
async fn timeline_opens_for_creator() {
    let root = tempfile::tempdir().expect("tempdir");
    let runtime = build_timeline_runtime(&root).await;

    let tenant_id = TenantId::new(TENANT).expect("tenant id");
    let user_id = UserId::new(USER).expect("user id");
    let agent_id = AgentId::new(AGENT).expect("agent id");

    let app = build_timeline_app(&runtime);

    let thread_id = fire_trigger_and_get_session_thread_id(
        &runtime,
        &tenant_id,
        &user_id,
        &agent_id,
        "timeline-e2e-happy",
    )
    .await;

    runtime.shutdown().await.expect("runtime shutdown");

    let (status, body) = get_timeline(&app, &thread_id, OWNER_TOKEN).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "creator should see timeline for their automation trigger thread \
         (thread_id={thread_id}) — body: {body}"
    );
    assert!(
        body.get("thread").is_some(),
        "timeline response must include the thread record — body: {body}"
    );
    let messages = body
        .get("messages")
        .and_then(|m| m.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    assert!(
        messages >= 1,
        "timeline must contain at least one message (the trigger prompt) — body: {body}"
    );
}

/// Exercises the composed automation product facade through the full HTTP→
/// product-workflow path using a second independent trigger.
///
/// Confirms the automation facade wiring is correct across multiple trigger
/// fires. Uses the session thread service accessor to discover the UUID.
///
/// See module-level comment for the in-memory backend limitation that prevents
/// isolating the fallback path from the direct session-scope path.
#[tokio::test]
async fn timeline_opens_for_creator_via_automation_service_stack() {
    let root = tempfile::tempdir().expect("tempdir");
    let runtime = build_timeline_runtime(&root).await;

    let tenant_id = TenantId::new(TENANT).expect("tenant id");
    let user_id = UserId::new(USER).expect("user id");
    let agent_id = AgentId::new(AGENT).expect("agent id");

    let app = build_timeline_app(&runtime);

    let thread_id = fire_trigger_and_get_session_thread_id(
        &runtime,
        &tenant_id,
        &user_id,
        &agent_id,
        "timeline-e2e-automation-stack",
    )
    .await;

    runtime.shutdown().await.expect("runtime shutdown");

    let (status, body) = get_timeline(&app, &thread_id, OWNER_TOKEN).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "automation owner should see timeline via the composed service stack \
         (thread_id={thread_id}) — body: {body}"
    );
    assert!(
        body.get("thread").is_some(),
        "timeline response must include the thread record — body: {body}"
    );
    let messages = body
        .get("messages")
        .and_then(|m| m.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    assert!(
        messages >= 1,
        "timeline must contain at least one message (the trigger prompt) — body: {body}"
    );
}

/// NEGATIVE: a different authenticated user cannot access the trigger-owned
/// thread.
///
/// With the thread stored under `owner_user_id = USER`:
/// - Direct scope lookup by OTHER_USER: `owner_user_id = OTHER_USER` ≠
///   stored `owner_user_id = USER` → `UnknownThread` → fallback triggered.
/// - Fallback authz: `list_scoped_triggers(creator = OTHER_USER)` → empty
///   (OTHER_USER has no triggers) → `Ok(None)` → 404.
///
/// This test exercises the automation-trigger fallback's authorization-denial
/// branch end-to-end through the full HTTP stack.
#[tokio::test]
async fn timeline_denied_for_different_user() {
    let root = tempfile::tempdir().expect("tempdir");
    let runtime = build_timeline_runtime(&root).await;

    let tenant_id = TenantId::new(TENANT).expect("tenant id");
    let user_id = UserId::new(USER).expect("user id");
    let agent_id = AgentId::new(AGENT).expect("agent id");

    let app = build_timeline_app(&runtime);

    let thread_id = fire_trigger_and_get_session_thread_id(
        &runtime,
        &tenant_id,
        &user_id,
        &agent_id,
        "timeline-e2e-denied",
    )
    .await;

    runtime.shutdown().await.expect("runtime shutdown");

    // Owner must still get 200 (sanity-check this is a real accessible thread).
    let (owner_status, _) = get_timeline(&app, &thread_id, OWNER_TOKEN).await;
    assert_eq!(
        owner_status,
        StatusCode::OK,
        "owner must still get 200 (test setup sanity-check)"
    );

    // Different user gets 404 via the fallback's authorization-denial branch:
    // - direct scope lookup (OTHER_USER as owner) misses (stored owner = USER)
    // - fallback: list_scoped_triggers(creator = OTHER_USER) = empty → 404
    let (other_status, other_body) = get_timeline(&app, &thread_id, OTHER_TOKEN).await;
    assert_eq!(
        other_status,
        StatusCode::NOT_FOUND,
        "different user must receive 404 — trigger is not in their automation list \
         (thread_id={thread_id}) — body: {other_body}"
    );
}
