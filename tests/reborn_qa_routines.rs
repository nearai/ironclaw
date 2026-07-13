//! QA use-case coverage for routine (scheduled trigger) flows:
//!
//! - "Every 30 minutes, send me an email with a summary about the company
//!   ..." → Routine created.
//! - "Every 5 minutes, ping [endpoint URL] and send me a dm in Slack if it
//!   does not return a 200." → Routine created.
//! - "Every 5 minutes, check https://github.com/nearai/ironclaw for latest
//!   releases ..." → Routine created.
//! - "Every 30 minutes, check my inbox and add any new emails from a
//!   near.ai address to my Google Sheet called ABC." → Routine created.
//! - "Every hour, check Hacker News for new posts mentioning 'IronClaw' or
//!   'NEAR AI' and send a summary to Slack." → Routine created.
//!
//! Creation runs through the Reborn binary-E2E harness (chat turn →
//! `builtin.trigger_create` → `builtin.trigger_list`). Firing runs through
//! the composition-owned trigger poller: a routine created via
//! `builtin.trigger_create` is made due and must submit a real agent turn
//! carrying the routine prompt.

#[allow(dead_code)]
#[path = "support/reborn_parity_qa/mod.rs"]
mod parity_qa_support;
#[allow(dead_code)]
#[path = "integration/support/mod.rs"]
mod reborn_support;
mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_approvals::AutoApproveSettingInput;
use ironclaw_host_api::{
    AgentId, CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, EffectKind,
    ExecutionContext, ExtensionId, GrantConstraints, MountView, NetworkPolicy, Principal,
    ResourceEstimate, RuntimeKind, TenantId, TrustClass, UserId,
};
use ironclaw_host_runtime::{
    ECHO_CAPABILITY_ID, RuntimeCapabilityOutcome, RuntimeCapabilityRequest,
    TRIGGER_CREATE_CAPABILITY_ID, TRIGGER_LIST_CAPABILITY_ID,
};
use ironclaw_loop_support::{
    HostManagedModelError, HostManagedModelGateway, HostManagedModelMessageRole,
    HostManagedModelRequest, HostManagedModelResponse, HostManagedToolResultContent,
};
use ironclaw_reborn_composition::{
    RebornCompositionProfile, RebornLocalRuntimeProfileOptions, RebornRuntime,
    RebornRuntimeIdentity, RebornRuntimeInput, TriggerPollerSettings, build_reborn_runtime,
    local_runtime_build_input_with_options,
};
use ironclaw_triggers::{TriggerId, TriggerPollerWorkerConfig, TriggerRunStatus, TriggerState};
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
use ironclaw_turns::TurnStatus;
use parity_qa_support::binary_e2e::RebornBinaryE2EHarness;
use parity_qa_support::model_replay::{
    RebornModelReplayStep, RebornScriptedProviderToolCall, RebornTraceReplayModelGateway,
};
use serde_json::{Value, json};
use tokio::sync::Mutex as TokioMutex;

struct RoutineCreationCase {
    room: &'static str,
    event_id: &'static str,
    user_request: &'static str,
    trigger_name: &'static str,
    cron: &'static str,
    prompt: &'static str,
    created_reply: &'static str,
}

async fn run_routine_creation(case: RoutineCreationCase) {
    let trigger_create =
        CapabilityId::new(TRIGGER_CREATE_CAPABILITY_ID).expect("valid capability id");
    let trigger_list = CapabilityId::new(TRIGGER_LIST_CAPABILITY_ID).expect("valid capability id");
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                trigger_create.clone(),
                "call_qa_trigger_create",
                json!({
                    "name": case.trigger_name,
                    "prompt": case.prompt,
                    "schedule": {
                        "kind": "cron",
                        "expression": case.cron,
                        "timezone": "UTC"
                    },
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                trigger_list.clone(),
                "call_qa_trigger_list",
                json!({}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply(case.created_reply),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = RebornBinaryE2EHarness::with_host_runtime_trigger_management_capabilities(
        case.room,
        model_gateway,
    )
    .await
    .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text(case.event_id, case.user_request)
        .await
        .expect("submit routine request");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    harness
        .assert_final_reply(case.created_reply)
        .await
        .expect("routine created reply");

    let invocations = harness.capability_invocations();
    assert_eq!(invocations.len(), 2);
    assert_eq!(invocations[0].capability_id, trigger_create);
    assert_eq!(invocations[1].capability_id, trigger_list);

    let results = harness.capability_results();
    assert_eq!(results.len(), 2);
    assert_eq!(
        results[0].output["trigger"]["name"],
        json!(case.trigger_name)
    );
    assert_eq!(results[0].output["trigger"]["state"], json!("scheduled"));
    let trigger_id = results[0].output["trigger"]["trigger_id"]
        .as_str()
        .expect("created trigger id");
    assert_eq!(
        results[1].output["triggers"][0]["trigger_id"],
        json!(trigger_id),
        "trigger_list should expose the routine created in the same turn"
    );
    harness.assert_model_exhausted();

    harness.shutdown().await;
}

#[tokio::test]
async fn reborn_qa_routine_created_for_meeting_prep_email_every_30_minutes() {
    run_routine_creation(RoutineCreationCase {
        room: "room-qa-routine-meeting-prep",
        event_id: "event-qa-routine-meeting-prep",
        user_request: "Every 30 minutes, send me an email with a summary about the company from my Google Drive and the latest news about the company that I will meet.",
        trigger_name: "Meeting prep email",
        cron: "*/30 * * * *",
        prompt: "Summarize the company brief from Google Drive plus the latest company news and email it to the user",
        created_reply: "Routine created: meeting prep email every 30 minutes",
    })
    .await;
}

#[tokio::test]
async fn reborn_qa_routine_created_for_endpoint_health_ping_every_5_minutes() {
    run_routine_creation(RoutineCreationCase {
        room: "room-qa-routine-health-ping",
        event_id: "event-qa-routine-health-ping",
        user_request: "Every 5 minutes, ping https://cloud-api.near.ai/health and send me a dm in Slack if it does not return a 200.",
        trigger_name: "Deployment health watcher",
        cron: "*/5 * * * *",
        prompt: "Ping https://cloud-api.near.ai/health and send a Slack DM if the response is not HTTP 200",
        created_reply: "Routine created: endpoint health check every 5 minutes",
    })
    .await;
}

#[tokio::test]
async fn reborn_qa_routine_created_for_github_release_watch_every_5_minutes() {
    run_routine_creation(RoutineCreationCase {
        room: "room-qa-routine-release-watch",
        event_id: "event-qa-routine-release-watch",
        user_request: "Every 5 minutes, check https://github.com/nearai/ironclaw for latest releases and send me a Slack message summarizing any new ones.",
        trigger_name: "Competitor release tracker",
        cron: "*/5 * * * *",
        prompt: "Check https://github.com/nearai/ironclaw for new releases and send a Slack summary of any new release",
        created_reply: "Routine created: GitHub release watch every 5 minutes",
    })
    .await;
}

#[tokio::test]
async fn reborn_qa_routine_created_for_crm_inbox_sweep_every_30_minutes() {
    run_routine_creation(RoutineCreationCase {
        room: "room-qa-routine-crm-inbox",
        event_id: "event-qa-routine-crm-inbox",
        user_request: "Every 30 minutes, check my inbox and add any new emails from a near.ai address to my Google Sheet called ABC.",
        trigger_name: "CRM inbound tracker",
        cron: "*/30 * * * *",
        prompt: "Check the inbox for new emails from near.ai senders and append them as rows to the Google Sheet called ABC",
        created_reply: "Routine created: CRM inbound sweep every 30 minutes",
    })
    .await;
}

#[tokio::test]
async fn reborn_qa_routine_created_for_hacker_news_monitor_every_hour() {
    run_routine_creation(RoutineCreationCase {
        room: "room-qa-routine-hn-monitor",
        event_id: "event-qa-routine-hn-monitor",
        user_request: "Every hour, check Hacker News for new posts mentioning 'IronClaw' or 'NEAR AI' and send a summary to Slack.",
        trigger_name: "HN keyword monitor",
        cron: "0 * * * *",
        prompt: "Search Hacker News for new posts mentioning IronClaw or NEAR AI and send a Slack summary of matches",
        created_reply: "Routine created: Hacker News monitor every hour",
    })
    .await;
}

// --- Routine firing -------------------------------------------------------
//
// QA expected result beyond "Routine created": when the schedule comes due,
// the routine actually runs an agent turn with the routine prompt. This
// drives the composition-owned trigger poller end to end: routine created
// through `builtin.trigger_create`, made due, fired by the poller, and the
// scripted model gateway receives the routine prompt as a real turn.

const QA_TENANT: &str = "qa-routine-fire-tenant";
const QA_USER: &str = "qa-routine-fire-owner";
const QA_AGENT: &str = "qa-routine-fire-agent";
const QA_ROUTINE_PROMPT: &str = "qa-routine-fire-prompt: ping https://cloud-api.near.ai/health and send a Slack DM if it does not return a 200";

#[derive(Debug, Default)]
struct RecordingGateway {
    requests: Arc<TokioMutex<Vec<HostManagedModelRequest>>>,
}

impl RecordingGateway {
    async fn captured_message_contents(&self) -> Vec<String> {
        let snapshot = self.requests.lock().await.clone();
        snapshot
            .iter()
            .flat_map(|req| req.messages.iter().map(|m| m.content.clone()))
            .collect()
    }
}

#[async_trait]
impl HostManagedModelGateway for RecordingGateway {
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.requests.lock().await.push(request);
        Ok(HostManagedModelResponse::assistant_reply(
            "qa routine fire ok".to_string(),
        ))
    }
}

/// Builds a local-dev yolo `RebornRuntime` with the trigger poller enabled
/// and the supplied model gateway override — the shared setup for the
/// fire-path tests below.
async fn build_qa_fire_runtime(
    root: &tempfile::TempDir,
    gateway: Arc<dyn HostManagedModelGateway>,
) -> RebornRuntime {
    let host_home_root = root.path().join("host-home");
    std::fs::create_dir_all(&host_home_root).expect("host home root");
    let input = local_runtime_build_input_with_options(
        RebornCompositionProfile::LocalDevYolo,
        QA_USER,
        root.path().join("local-dev"),
        RebornLocalRuntimeProfileOptions {
            confirm_host_access: true,
        },
    )
    .expect("local-yolo runtime input")
    .with_local_dev_confirmed_host_home_root(host_home_root);
    let input = RebornRuntimeInput::from_services(input)
        .with_identity(RebornRuntimeIdentity {
            tenant_id: QA_TENANT.to_string(),
            agent_id: QA_AGENT.to_string(),
            source_binding_id: "qa-routine-fire-source".to_string(),
            reply_target_binding_id: "qa-routine-fire-reply".to_string(),
        })
        .with_trigger_poller_settings(
            TriggerPollerSettings::enabled_with_tenant_scoped_authorizer_for_test()
                .with_worker_config(TriggerPollerWorkerConfig {
                    poll_interval: Duration::from_millis(20),
                    ..Default::default()
                }),
        )
        .with_model_gateway_override(gateway);
    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    seed_qa_fire_auto_approve(&runtime).await;
    runtime
}

async fn seed_qa_fire_auto_approve(runtime: &RebornRuntime) {
    let auto_approve = runtime
        .services()
        .local_dev_auto_approve_settings_for_test()
        .expect("QA fire runtime exposes local-dev auto-approve settings");
    auto_approve
        .set(AutoApproveSettingInput {
            scope: trigger_management_execution_context().resource_scope,
            enabled: true,
            updated_by: Principal::User(UserId::new(QA_USER).expect("QA user id")),
        })
        .await
        .expect("seed QA fire global auto-approve");
}

#[tokio::test]
async fn reborn_qa_routine_created_by_tool_fires_and_runs_routine_prompt() {
    let root = tempfile::tempdir().expect("tempdir");
    let recording_gateway = Arc::new(RecordingGateway::default());
    let runtime = build_qa_fire_runtime(
        &root,
        Arc::clone(&recording_gateway) as Arc<dyn HostManagedModelGateway>,
    )
    .await;

    let created = invoke_trigger_create(
        &runtime,
        json!({
            "name": "Deployment health watcher",
            "prompt": QA_ROUTINE_PROMPT,
            "schedule": {
                "kind": "cron",
                "expression": "*/5 * * * *",
                "timezone": "UTC"
            },
        }),
    )
    .await;
    assert_eq!(
        created["trigger"]["name"],
        json!("Deployment health watcher")
    );
    assert_eq!(created["trigger"]["state"], json!("scheduled"));

    let repo = runtime
        .trigger_repository()
        .expect("local-dev runtime exposes trigger repository");
    let tenant_id = TenantId::new(QA_TENANT).expect("tenant id");
    let trigger_id = TriggerId::parse(
        created["trigger"]["trigger_id"]
            .as_str()
            .expect("created trigger id"),
    )
    .expect("valid trigger id");

    // Pull the schedule forward so the routine is due now instead of in
    // five minutes.
    let mut record = repo
        .get_trigger(tenant_id.clone(), trigger_id)
        .await
        .expect("get created trigger")
        .expect("created trigger persisted");
    record.next_run_at = Utc::now() - chrono::Duration::try_seconds(120).expect("valid duration");
    repo.upsert_trigger(record)
        .await
        .expect("make created routine due");

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut record_was_mutated = false;
    let mut prompt_seen = false;
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let current = repo
            .get_trigger(tenant_id.clone(), trigger_id)
            .await
            .expect("get trigger")
            .expect("record present");
        let mutated = current.last_fired_slot.is_some()
            || current.last_run_at.is_some()
            || current.last_status.is_some()
            || current.active_fire_slot.is_some();
        if mutated {
            record_was_mutated = true;
            let contents = recording_gateway.captured_message_contents().await;
            if contents
                .iter()
                .any(|content| content.contains(QA_ROUTINE_PROMPT))
            {
                prompt_seen = true;
            }
        }
        if record_was_mutated && prompt_seen {
            break;
        }
    }

    // Wait for the settle writes (last_fired_slot, last_run_at) to land;
    // the first-pass loop breaks as soon as the claim+prompt are seen.
    let settle_deadline = Instant::now() + Duration::from_secs(5);
    let mut settled = repo
        .get_trigger(tenant_id.clone(), trigger_id)
        .await
        .expect("get trigger")
        .expect("record present");
    while Instant::now() < settle_deadline
        && !(settled.last_fired_slot.is_some() && settled.last_run_at.is_some())
    {
        tokio::time::sleep(Duration::from_millis(50)).await;
        settled = repo
            .get_trigger(tenant_id.clone(), trigger_id)
            .await
            .expect("get trigger")
            .expect("record present");
    }

    runtime.shutdown().await.expect("runtime shutdown");

    let captured_contents = recording_gateway.captured_message_contents().await;
    assert!(
        record_was_mutated,
        "poller did not fire the QA routine within 15s — record: {settled:?}"
    );
    assert!(
        prompt_seen,
        "fired routine never submitted a turn carrying the routine prompt — captured: {captured_contents:?}"
    );
    assert_eq!(
        settled.last_status,
        Some(TriggerRunStatus::Ok),
        "fired QA routine should settle with Ok status — record: {settled:?}"
    );
    assert_eq!(
        settled.state,
        TriggerState::Scheduled,
        "recurring QA routine must stay scheduled after firing — record: {settled:?}"
    );
    let fired_slot = settled
        .last_fired_slot
        .expect("fired QA routine should record its fired slot");
    assert!(
        settled.next_run_at > fired_slot,
        "recurring QA routine should reschedule past the fired slot — fired: {fired_slot:?}, next: {:?}",
        settled.next_run_at
    );
}

/// QA expected result for the deployment health watcher: the fired routine
/// does not just receive its prompt — it executes the downstream action
/// (here `builtin.echo` standing in for "send a Slack DM", dispatched through
/// the real host runtime) and finalizes a reply. The scripted gateway only
/// serves the final reply after the loop returns the action's tool result,
/// so an Ok settle proves the full fired-turn chain:
/// poller fire → trusted turn → capability dispatch → tool result → reply.
#[tokio::test]
async fn reborn_qa_fired_routine_executes_action_and_finalizes_reply() {
    // No slashes: the model-visible output sanitizer rewrites path-like
    // tokens in tool results, which would break the exact-marker match.
    const QA_DM_ACTION_MARKER: &str =
        "qa-slack-dm: deployment health endpoint returned 503, alerting user";
    const QA_FIRED_REPLY: &str = "Slack DM sent: the endpoint did not return a 200";

    let root = tempfile::tempdir().expect("tempdir");
    let echo = CapabilityId::new(ECHO_CAPABILITY_ID).expect("valid capability id");
    let gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                echo.clone(),
                "call_qa_fired_dm_action",
                json!({"message": QA_DM_ACTION_MARKER}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply(QA_FIRED_REPLY),
            expected_tool_results: Vec::new(),
        },
    ]);
    let runtime = build_qa_fire_runtime(
        &root,
        Arc::new(gateway.clone()) as Arc<dyn HostManagedModelGateway>,
    )
    .await;

    let created = invoke_trigger_create(
        &runtime,
        json!({
            "name": "Deployment health watcher action",
            "prompt": QA_ROUTINE_PROMPT,
            "schedule": {
                "kind": "cron",
                "expression": "*/5 * * * *",
                "timezone": "UTC"
            },
        }),
    )
    .await;
    assert_eq!(created["trigger"]["state"], json!("scheduled"));

    let repo = runtime
        .trigger_repository()
        .expect("local-dev runtime exposes trigger repository");
    let tenant_id = TenantId::new(QA_TENANT).expect("tenant id");
    let trigger_id = TriggerId::parse(
        created["trigger"]["trigger_id"]
            .as_str()
            .expect("created trigger id"),
    )
    .expect("valid trigger id");

    let mut record = repo
        .get_trigger(tenant_id.clone(), trigger_id)
        .await
        .expect("get created trigger")
        .expect("created trigger persisted");
    record.next_run_at = Utc::now() - chrono::Duration::try_seconds(120).expect("valid duration");
    repo.upsert_trigger(record)
        .await
        .expect("make created routine due");

    // Wait for the fired turn to consume the full model script: both steps
    // taken means the action's tool result made it back to the model and the
    // final reply was served.
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline && gateway.remaining_responses() > 0 {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Wait for the settle writes before asserting the trigger outcome.
    let settle_deadline = Instant::now() + Duration::from_secs(5);
    let mut settled = repo
        .get_trigger(tenant_id.clone(), trigger_id)
        .await
        .expect("get trigger")
        .expect("record present");
    while Instant::now() < settle_deadline
        && !(settled.last_fired_slot.is_some() && settled.last_run_at.is_some())
    {
        tokio::time::sleep(Duration::from_millis(50)).await;
        settled = repo
            .get_trigger(tenant_id.clone(), trigger_id)
            .await
            .expect("get trigger")
            .expect("record present");
    }

    runtime.shutdown().await.expect("runtime shutdown");

    gateway.assert_exhausted();
    let requests = gateway.requests();
    assert_eq!(
        requests.len(),
        2,
        "fired routine turn should make exactly two model requests (prompt, then tool result)"
    );
    assert!(
        requests[0]
            .messages
            .iter()
            .any(|message| message.content.contains(QA_ROUTINE_PROMPT)),
        "first fired-turn request should carry the routine prompt"
    );
    let tool_result = requests[1]
        .messages
        .iter()
        .find(|message| message.role == HostManagedModelMessageRole::ToolResult)
        .expect("the fired routine's action must reach the model");
    // Issue #5838: a result under the inline first-look preview cap
    // (`LOCAL_DEV_RESULT_PREVIEW_MAX_BYTES`) legitimately appears inline in
    // `detail.preview` so the model does not need a follow-up `result_read`
    // call; the marker here is well under the cap. Mirrors
    // `assert_local_dev_result_reference` in
    // `crates/ironclaw_reborn_composition/src/runtime.rs`.
    assert!(
        tool_result.content.contains(QA_DM_ACTION_MARKER),
        "a result under the first-look preview cap should appear inline in model replay: {}",
        tool_result.content
    );
    let Some(HostManagedToolResultContent::Reference { envelope }) =
        tool_result.tool_result_content.as_ref()
    else {
        panic!(
            "fired routine replay should carry a result-reference envelope, got {:?}",
            tool_result.tool_result_content
        );
    };
    assert_eq!(envelope.version, 1);
    assert!(envelope.result_ref.starts_with("result:"));
    let observation = envelope
        .model_observation
        .as_ref()
        .expect("fired routine replay should include a model observation");
    assert_eq!(observation["schema_version"], json!(1));
    assert_eq!(observation["status"], json!("success"));
    assert_eq!(observation["detail"]["kind"], json!("result_reference"));
    assert_eq!(
        observation["detail"]["result_ref"],
        json!(envelope.result_ref)
    );
    assert_eq!(
        settled.last_status,
        Some(TriggerRunStatus::Ok),
        "fired QA routine with an action should settle with Ok status — record: {settled:?}"
    );
}

async fn invoke_trigger_create(runtime: &RebornRuntime, input: Value) -> Value {
    let host_runtime = runtime
        .services()
        .host_runtime
        .as_deref()
        .expect("runtime exposes host runtime");
    let outcome = host_runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            trigger_management_execution_context(),
            CapabilityId::new(TRIGGER_CREATE_CAPABILITY_ID).expect("capability id"),
            ResourceEstimate::default(),
            input,
            trigger_management_trust_decision(),
        ))
        .await
        .expect("trigger create invocation completes");
    let RuntimeCapabilityOutcome::Completed(completed) = outcome else {
        panic!("expected trigger create to complete, got {outcome:?}");
    };
    completed.output
}

fn trigger_management_execution_context() -> ExecutionContext {
    let tenant_id = TenantId::new(QA_TENANT).expect("tenant id");
    let user_id = UserId::new(QA_USER).expect("user id");
    let agent_id = AgentId::new(QA_AGENT).expect("agent id");
    let extension_id = ExtensionId::new("qa-routine-fire-caller").expect("extension id");
    let mut context = ExecutionContext::local_default(
        user_id,
        extension_id.clone(),
        RuntimeKind::FirstParty,
        TrustClass::UserTrusted,
        CapabilitySet {
            grants: vec![CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: CapabilityId::new(TRIGGER_CREATE_CAPABILITY_ID).expect("capability id"),
                grantee: Principal::Extension(extension_id),
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: vec![
                        EffectKind::DispatchCapability,
                        EffectKind::ExternalWrite,
                    ],
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: None,
                },
            }],
        },
        MountView::default(),
    )
    .expect("execution context");
    context.tenant_id = tenant_id.clone();
    context.agent_id = Some(agent_id.clone());
    context.project_id = None;
    context.resource_scope.tenant_id = tenant_id;
    context.resource_scope.agent_id = Some(agent_id);
    context.resource_scope.project_id = None;
    context
}

fn trigger_management_trust_decision() -> TrustDecision {
    TrustDecision {
        effective_trust: EffectiveTrustClass::user_trusted(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects: vec![EffectKind::DispatchCapability, EffectKind::ExternalWrite],
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::AdminConfig,
        evaluated_at: Utc::now(),
    }
}
