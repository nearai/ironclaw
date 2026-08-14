//! Whole-path integration coverage for the unbound (prepared-context) lane:
//! `accept_prepared_context` → refless `submit_turn` → planned loop →
//! terminal state, driven through the group's ONE shared planned runtime and
//! asserted at the thread-store and run-state seams.
//!
//! What this file pins (unbound-turn design, docs/internal/design/):
//! - The accept door mints a deterministic ownerless thread, seeds the
//!   prepared messages as ordinary rows, and replays idempotently by key.
//! - Admission derives the unbound profile from the journaled declarations
//!   (no requested profile on the wire; JSON-schema output → structured).
//! - A structured run completes by recording a validated result through
//!   `builtin.structured_result` — including the invalid-then-valid repair
//!   loop — with no assistant reply required.
//! - A default (assistant-message) unbound run completes on a plain final.
//! - The ownerless thread never appears in the owner-scoped listing a
//!   conversation UI reads.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use std::sync::Arc;
use std::time::Duration;

use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_host_api::prepared_context::{OutputContract, PreparedTurnDeclarations};
use ironclaw_host_api::turn::{TurnScope, TurnThreadOwner};
use ironclaw_llm::agent_message::{AgentMessage, AgentMessageRole, ContentPart};
use ironclaw_threads::{
    ListThreadsForScopeRequest, PreparedContextRequest, SessionThreadService, ThreadHistoryRequest,
    ThreadScope,
};
use ironclaw_turns::{
    GetRunStateRequest, IdempotencyKey, RunProfileId, SubmitTurnRequest, SubmitTurnResponse,
    TurnActor, TurnCoordinator, TurnRunState, TurnStatus,
};
use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;

type HarnessResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

const STRUCTURED_RESULT: &str = "builtin.structured_result";

/// Group scope axes (`test_product_scope` in the group support).
const TENANT: &str = "tenant-itest";
const AGENT: &str = "agent-itest";
const PROJECT: &str = "project-itest";
const CALLER: &str = "host-user";

fn ownerless_thread_scope() -> HarnessResult<ThreadScope> {
    Ok(ThreadScope {
        tenant_id: TenantId::new(TENANT)?,
        agent_id: AgentId::new(AGENT)?,
        project_id: Some(ProjectId::new(PROJECT)?),
        owner_user_id: None,
        mission_id: None,
    })
}

fn unbound_turn_scope(thread_id: &ironclaw_host_api::ids::ThreadId) -> HarnessResult<TurnScope> {
    let mut scope = TurnScope::new(
        TenantId::new(TENANT)?,
        Some(AgentId::new(AGENT)?),
        Some(ProjectId::new(PROJECT)?),
        thread_id.clone(),
    );
    scope.thread_owner = TurnThreadOwner::Ownerless;
    Ok(scope)
}

fn prepared_request(
    idempotency_key: &str,
    task: &str,
    declarations: PreparedTurnDeclarations,
) -> HarnessResult<PreparedContextRequest> {
    Ok(PreparedContextRequest {
        scope: ownerless_thread_scope()?,
        actor_id: CALLER.to_string(),
        system_prompt: "You are a background extraction task.".to_string(),
        messages: vec![AgentMessage {
            role: AgentMessageRole::User,
            content: vec![ContentPart::text(task)],
        }],
        declarations,
        idempotency_key: idempotency_key.to_string(),
        thread_id: None,
        title: None,
        metadata_json: None,
    })
}

fn structured_declarations() -> PreparedTurnDeclarations {
    PreparedTurnDeclarations {
        tools: Vec::new(),
        output: OutputContract::JsonSchema {
            schema: json!({
                "type": "object",
                "properties": {
                    "sentiment": {"type": "string", "enum": ["positive", "negative"]},
                    "confidence": {"type": "number"}
                },
                "required": ["sentiment"],
                "additionalProperties": false
            }),
        },
        limits: Default::default(),
    }
}

fn submit_request(
    scope: TurnScope,
    accepted_message_ref: ironclaw_host_api::turn::AcceptedMessageRef,
    idempotency_key: &str,
) -> HarnessResult<SubmitTurnRequest> {
    Ok(SubmitTurnRequest {
        scope,
        actor: TurnActor::new(UserId::new(CALLER)?),
        accepted_message_ref,
        requested_run_profile: None,
        requested_model: None,
        idempotency_key: IdempotencyKey::new(idempotency_key)?,
        received_at: chrono::Utc::now(),
        requested_run_id: None,
        parent_run_id: None,
        subagent_depth: 0,
        spawn_tree_root_run_id: None,
        product_context: None,
    })
}

async fn wait_for_terminal(
    coordinator: &Arc<dyn TurnCoordinator>,
    scope: &TurnScope,
    run_id: ironclaw_host_api::turn::TurnRunId,
) -> HarnessResult<TurnRunState> {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let state = coordinator
            .get_run_state(GetRunStateRequest {
                scope: scope.clone(),
                run_id,
            })
            .await?;
        if state.status.is_terminal() {
            return Ok(state);
        }
        if std::time::Instant::now() > deadline {
            return Err(format!(
                "unbound run never reached a terminal status; last={:?} failure={:?}",
                state.status, state.failure
            )
            .into());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Accept + script + submit one unbound run; returns everything the
/// assertions need. The harness parameter exists only to reach the group's
/// shared thread service and coordinator.
struct UnboundRun {
    thread_id: ironclaw_host_api::ids::ThreadId,
    state: TurnRunState,
}

async fn run_unbound(
    group: &RebornIntegrationGroup,
    harness: &RebornIntegrationHarness,
    label: &str,
    declarations: PreparedTurnDeclarations,
    replies: Vec<RebornScriptedReply>,
) -> HarnessResult<UnboundRun> {
    let thread_service = harness.thread_service_for_test()?;
    let accepted = thread_service
        .accept_prepared_context(prepared_request(
            &format!("{label}-accept"),
            "Classify the sentiment of: the release went great",
            declarations,
        )?)
        .await?;
    assert!(!accepted.idempotent_replay, "first accept must not replay");
    let scope = unbound_turn_scope(&accepted.thread_id)?;
    group
        .register_scope_script_for_test(scope.clone(), label, replies)
        .await?;
    let coordinator = harness.turn_coordinator_for_test();
    let response = coordinator
        .submit_turn(submit_request(
            scope.clone(),
            accepted.accepted_message_ref.clone(),
            &format!("{label}-submit"),
        )?)
        .await?;
    let SubmitTurnResponse::Accepted { run_id, .. } = response;
    let state = wait_for_terminal(&coordinator, &scope, run_id).await?;
    Ok(UnboundRun {
        thread_id: accepted.thread_id,
        state,
    })
}

/// Structured happy path: the model answers with one `builtin.structured_result`
/// call whose payload validates; the run completes with NO assistant reply,
/// the derived profile is `unbound_structured`, and the validated result is a
/// durable tool-result row on the ownerless thread.
#[tokio::test(flavor = "multi_thread")]
async fn structured_unbound_run_completes_by_recording_a_validated_result() -> HarnessResult<()> {
    let group = RebornIntegrationGroup::builtin_tools_with_durable_capability_io().await?;
    let harness = group.thread("conv-unbound-anchor").build().await?;

    let run = run_unbound(
        &group,
        &harness,
        "unbound-structured-happy",
        structured_declarations(),
        vec![RebornScriptedReply::tool_call(
            STRUCTURED_RESULT,
            json!({"sentiment": "positive", "confidence": 0.9}),
        )],
    )
    .await?;

    assert_eq!(
        run.state.status,
        TurnStatus::Completed,
        "failure={:?}",
        run.state.failure
    );
    assert_eq!(
        run.state.resolved_run_profile_id,
        RunProfileId::unbound_structured(),
        "admission must derive the structured profile from the journaled schema"
    );

    // Seam assertion: the validated result is a durable tool-result row on
    // the ownerless thread (the caller's read-back surface), and no
    // assistant reply was required to complete.
    let thread_service = harness.thread_service_for_test()?;
    let history = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: ownerless_thread_scope()?,
            thread_id: run.thread_id.clone(),
        })
        .await?;
    let result_rows = history
        .messages
        .iter()
        .filter(|message| {
            message
                .tool_result_ref
                .as_deref()
                .is_some_and(|value| !value.is_empty())
        })
        .count();
    assert!(
        result_rows >= 1,
        "the structured result must land as a durable tool-result row; rows={:?}",
        history
            .messages
            .iter()
            .map(|m| (m.kind, m.status))
            .collect::<Vec<_>>()
    );

    // Taxonomy: the ownerless thread is structurally invisible to the
    // owner-scoped listing every conversation UI reads.
    let caller_scope = ThreadScope {
        owner_user_id: Some(UserId::new(CALLER)?),
        ..ownerless_thread_scope()?
    };
    let listed = thread_service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: caller_scope,
            limit: None,
            cursor: None,
        })
        .await?;
    assert!(
        listed
            .threads
            .iter()
            .all(|thread| thread.thread_id != run.thread_id),
        "an unbound thread must never surface in an owner-scoped listing"
    );
    Ok(())
}

/// The repair loop: an invalid result payload is a model-correctable failure
/// (schema violations echoed back), and the corrected second call completes
/// the run — never a host error, never a silent success.
#[tokio::test(flavor = "multi_thread")]
async fn structured_unbound_run_repairs_an_invalid_result_payload() -> HarnessResult<()> {
    let group = RebornIntegrationGroup::builtin_tools_with_durable_capability_io().await?;
    let harness = group.thread("conv-unbound-repair-anchor").build().await?;

    let run = run_unbound(
        &group,
        &harness,
        "unbound-structured-repair",
        structured_declarations(),
        vec![
            RebornScriptedReply::tool_call(
                STRUCTURED_RESULT,
                json!({"sentiment": "confused", "extra": true}),
            ),
            RebornScriptedReply::tool_call(STRUCTURED_RESULT, json!({"sentiment": "negative"})),
        ],
    )
    .await?;

    assert_eq!(
        run.state.status,
        TurnStatus::Completed,
        "failure={:?}",
        run.state.failure
    );
    Ok(())
}

/// Default (assistant-message) contract: no schema declared, so admission
/// derives `unbound_default` and a plain text final completes the run.
#[tokio::test(flavor = "multi_thread")]
async fn default_unbound_run_completes_on_a_plain_text_final() -> HarnessResult<()> {
    let group = RebornIntegrationGroup::builtin_tools().await?;
    let harness = group.thread("conv-unbound-default-anchor").build().await?;

    let run = run_unbound(
        &group,
        &harness,
        "unbound-default-happy",
        PreparedTurnDeclarations::default(),
        vec![RebornScriptedReply::text("The sentiment is positive.")],
    )
    .await?;

    assert_eq!(
        run.state.status,
        TurnStatus::Completed,
        "failure={:?}",
        run.state.failure
    );
    assert_eq!(
        run.state.resolved_run_profile_id,
        RunProfileId::unbound_default(),
        "no declared schema must derive the default unbound profile"
    );
    let thread_service = harness.thread_service_for_test()?;
    let history = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: ownerless_thread_scope()?,
            thread_id: run.thread_id.clone(),
        })
        .await?;
    assert!(
        history
            .messages
            .iter()
            .any(|message| message.content.as_deref() == Some("The sentiment is positive.")),
        "the final reply must land on the ownerless thread"
    );
    Ok(())
}

/// Accept-door idempotency across the WHOLE lane: the same accept key returns
/// the same thread with `idempotent_replay`, and the same submit key returns
/// the same run instead of double-executing.
#[tokio::test(flavor = "multi_thread")]
async fn unbound_accept_and_submit_replay_idempotently() -> HarnessResult<()> {
    let group = RebornIntegrationGroup::builtin_tools().await?;
    let harness = group.thread("conv-unbound-replay-anchor").build().await?;
    let thread_service = harness.thread_service_for_test()?;

    let request = prepared_request(
        "unbound-replay-accept",
        "Summarize: the meeting moved to Tuesday",
        PreparedTurnDeclarations::default(),
    )?;
    let first = thread_service
        .accept_prepared_context(request.clone())
        .await?;
    let second = thread_service.accept_prepared_context(request).await?;
    assert!(!first.idempotent_replay);
    assert!(second.idempotent_replay, "second accept must replay");
    assert_eq!(first.thread_id, second.thread_id);
    assert_eq!(first.accepted_message_ref, second.accepted_message_ref);

    let scope = unbound_turn_scope(&first.thread_id)?;
    group
        .register_scope_script_for_test(
            scope.clone(),
            "unbound-replay",
            vec![RebornScriptedReply::text("Moved to Tuesday.")],
        )
        .await?;
    let coordinator = harness.turn_coordinator_for_test();
    let submit = submit_request(
        scope.clone(),
        first.accepted_message_ref.clone(),
        "unbound-replay-submit",
    )?;
    let SubmitTurnResponse::Accepted {
        run_id: first_run, ..
    } = coordinator.submit_turn(submit.clone()).await?;
    wait_for_terminal(&coordinator, &scope, first_run).await?;
    let SubmitTurnResponse::Accepted {
        run_id: second_run, ..
    } = coordinator.submit_turn(submit).await?;
    assert_eq!(
        first_run, second_run,
        "an idempotent resubmit must return the SAME run, not execute twice"
    );
    Ok(())
}
