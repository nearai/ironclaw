//! End-to-end test: trigger fired through the REAL poller → WebUI timeline
//! access through the composed service stack.
//!
//! ## What this proves
//!
//! 1. **Happy path** (`timeline_opens_for_creator`):
//!    A trigger fired by the real poller creates a thread in the in-memory
//!    thread service. The trigger run-history row's `thread_id` field holds
//!    the canonical UUID (not the 64-char hex route id placeholder). The
//!    trigger creator's WebUI bearer then retrieves 200 from
//!    `GET /api/webchat/v2/threads/{thread_id}/timeline` with a timeline
//!    containing the thread record and at least one message (the trigger
//!    prompt). This exercises the full composition path:
//!
//!    real poller → `record_trigger_prompt` → `InMemorySessionThreadService`
//!    → `TriggerRunRecord.thread_id` (canonical UUID)
//!    → `RebornServices::get_timeline` → composed WebUI v2 HTTP layer.
//!
//! 2. **Automation-service-stack path**
//!    (`timeline_opens_for_creator_via_automation_service_stack`):
//!    Same setup, second independent trigger. Confirms the automation facade
//!    wiring is correct across multiple trigger fires.
//!
//!    The thread_id is discovered via `list_trigger_run_history` on the
//!    trigger repository — the same field the WebUI Automations panel reads
//!    to build the `chat_path`. This replaces the old `session_thread_service()`
//!    workaround that bypassed the actual panel data path.
//!
//! 3. **thread_id is canonical UUID after acceptance**
//!    (`run_thread_id_is_canonical_uuid_after_fire`):
//!    After a trigger fires, the run-history row's `thread_id` must be a
//!    valid UUID (parseable by `uuid::Uuid::parse_str`) and must NOT be the
//!    64-char lowercase hex route id derived from `TriggerFireIdentity`.
//!    This directly regression-tests the fix for the "click run → 404" bug.
//!
//! 4. **Negative / fallback-denial** (`timeline_denied_for_different_user`):
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
use ironclaw_loop_host::{
    HostManagedModelError, HostManagedModelGateway, HostManagedModelRequest,
    HostManagedModelResponse,
};
use ironclaw_reborn_composition::{
    RebornCompositionProfile, RebornLocalRuntimeProfileOptions, RebornRuntime,
    RebornRuntimeIdentity, RebornRuntimeInput, RebornWebuiBundle, TriggerPollerSettings,
    build_reborn_runtime, build_webui_services, local_runtime_build_input_with_options,
};
use ironclaw_triggers::{
    TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID, TRIGGER_TRUSTED_ADAPTER_KIND,
    TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE, TriggerId, TriggerPollerWorkerConfig, TriggerRecord,
    TriggerRepository, TriggerSchedule, TriggerSourceKind, TriggerState,
};
use ironclaw_webui::{WebuiAuthentication, WebuiAuthenticator, WebuiServeConfig, webui_v2_app};
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
                .with_worker_config(
                    TriggerPollerWorkerConfig::default()
                        .set_poll_interval(Duration::from_millis(20)),
                ),
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

/// Wait until the trigger run-history row for `trigger_id` has a `thread_id`
/// that is a valid UUID (i.e. the canonical thread UUID has been persisted by
/// `mark_fire_accepted`).
///
/// This polls `list_trigger_run_history` — the same repository method the WebUI
/// Automations panel uses via `AutomationProductFacade::list_automations`. Once
/// the row's `thread_id` is a parseable UUID we know the panel's `chat_path`
/// would open a real thread.
///
/// Returns the canonical thread UUID as a `String`.
async fn wait_for_canonical_thread_id(
    repo: &Arc<dyn TriggerRepository>,
    tenant_id: &TenantId,
    trigger_id: TriggerId,
) -> String {
    let stop = Instant::now() + Duration::from_secs(15);
    loop {
        let runs = repo
            .list_trigger_run_history(tenant_id.clone(), trigger_id, 5)
            .await
            .expect("list_trigger_run_history");
        if let Some(run) = runs.first() {
            // `thread_id` is None until fire acceptance. Once Some, it must be a
            // canonical UUID; wait until that's the case.
            if let Some(thread_id) = run.thread_id.as_ref() {
                let thread_id_str = thread_id.as_str().to_string();
                if uuid::Uuid::parse_str(&thread_id_str).is_ok() {
                    return thread_id_str;
                }
            }
        }
        if Instant::now() >= stop {
            let runs = repo
                .list_trigger_run_history(tenant_id.clone(), trigger_id, 5)
                .await
                .unwrap_or_default();
            panic!(
                "canonical thread UUID did not appear in run history within 15s — \
                 last runs: {runs:?}"
            );
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
        prompt: TRIGGER_PROMPT.to_string(),
        delivery_target: None,
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

/// Seed a trigger, pair the external actor, wait for the poller to fire it,
/// then return the canonical `thread_id` from the run-history row.
///
/// ## Thread-id resolution strategy (post-fix)
///
/// `TriggerRunRecord.thread_id` is overwritten with the canonical UUID by
/// `mark_fire_accepted` when the fire is accepted. We poll
/// `list_trigger_run_history` until a UUID-parseable value appears — this is
/// the same field the WebUI Automations panel reads to build `chat_path`.
/// The old `session_thread_service()` workaround is no longer needed.
async fn fire_trigger_and_get_run_thread_id(
    runtime: &RebornRuntime,
    tenant_id: &TenantId,
    user_id: &UserId,
    agent_id: &AgentId,
    trigger_name: &str,
) -> (TriggerId, String) {
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

    // Phase 1: wait for the trigger to fire (last_run_at is set).
    let settled = wait_for_trigger_fire(&repo, tenant_id, trigger_id).await;
    assert!(
        settled.last_run_at.is_some(),
        "trigger was not fired by the poller within 15s — record: {settled:?}"
    );

    // Phase 2: wait for mark_fire_accepted to overwrite thread_id with the
    // canonical UUID (a UUID-parseable string replaces the 64-char hex route id).
    let thread_id = wait_for_canonical_thread_id(&repo, tenant_id, trigger_id).await;

    (trigger_id, thread_id)
}

// ─── tests ───────────────────────────────────────────────────────────────────

/// Happy path: the trigger creator's bearer retrieves the timeline for a
/// trigger-fired thread.
///
/// The thread_id is read from `TriggerRunRecord.thread_id` — the same field
/// the WebUI Automations panel uses to build `chat_path`. After the fix,
/// this holds the canonical UUID (not the 64-char hex route id placeholder),
/// so the timeline request succeeds without the `session_thread_service()`
/// workaround.
#[tokio::test]
async fn timeline_opens_for_creator() {
    let root = tempfile::tempdir().expect("tempdir");
    let runtime = build_timeline_runtime(&root).await;

    let tenant_id = TenantId::new(TENANT).expect("tenant id");
    let user_id = UserId::new(USER).expect("user id");
    let agent_id = AgentId::new(AGENT).expect("agent id");

    let app = build_timeline_app(&runtime);

    let (_trigger_id, thread_id) = fire_trigger_and_get_run_thread_id(
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
/// fires. Uses `list_trigger_run_history` to discover the canonical UUID —
/// the same data path the WebUI Automations panel uses.
#[tokio::test]
async fn timeline_opens_for_creator_via_automation_service_stack() {
    let root = tempfile::tempdir().expect("tempdir");
    let runtime = build_timeline_runtime(&root).await;

    let tenant_id = TenantId::new(TENANT).expect("tenant id");
    let user_id = UserId::new(USER).expect("user id");
    let agent_id = AgentId::new(AGENT).expect("agent id");

    let app = build_timeline_app(&runtime);

    let (_trigger_id, thread_id) = fire_trigger_and_get_run_thread_id(
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

/// Regression test for the "click run → 404" bug.
///
/// After a trigger fires and is accepted, the run-history row's `thread_id`
/// must be a valid UUID — not the 64-char lowercase hex route id placeholder
/// (`TriggerFireIdentity::route_thread_id`). The WebUI Automations panel
/// builds `chat_path: /chat/${run.thread_id}` from this field; if it holds
/// the hex placeholder, clicking the run 404s because no thread exists under
/// that id.
#[tokio::test]
async fn run_thread_id_is_canonical_uuid_after_fire() {
    let root = tempfile::tempdir().expect("tempdir");
    let runtime = build_timeline_runtime(&root).await;

    let tenant_id = TenantId::new(TENANT).expect("tenant id");
    let user_id = UserId::new(USER).expect("user id");
    let agent_id = AgentId::new(AGENT).expect("agent id");

    let (_trigger_id, thread_id) = fire_trigger_and_get_run_thread_id(
        &runtime,
        &tenant_id,
        &user_id,
        &agent_id,
        "timeline-e2e-uuid-check",
    )
    .await;

    runtime.shutdown().await.expect("runtime shutdown");

    // The thread_id returned by wait_for_canonical_thread_id is already
    // UUID-parseable (that's the poll condition — it waits until thread_id
    // is Some(canonical UUID)). Assert explicitly so a regression is
    // immediately visible in the test output.
    assert!(
        uuid::Uuid::parse_str(&thread_id).is_ok(),
        "run.thread_id must be a UUID after fire acceptance, got: {thread_id:?}"
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

    let (_trigger_id, thread_id) = fire_trigger_and_get_run_thread_id(
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
