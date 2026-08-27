//! Whole-path integration coverage for the unbound (prepared-context) lane:
//! `accept_prepared_context` → unbound `submit_turn` → planned loop →
//! terminal state, driven through the group's ONE shared planned runtime and
//! asserted at the thread-store and run-state seams.
//!
//! What this file pins (unbound-turn design, docs/internal/design/):
//! - The accept door mints a deterministic thread, seeds the prepared
//!   messages as ordinary rows, and replays idempotently by key. The
//!   engine-level tests drive the still-supported ownerless scope; the
//!   PRODUCT lane threads its authenticated caller as the thread owner
//!   (pinned by `unbound_service_threads_caller_as_thread_owner`).
//! - Admission persists the output contract independently from loop-family
//!   selection; JSON-schema output still uses the ordinary unbounded loop.
//! - A structured run reaches an ordinary assistant candidate, then performs
//!   exactly one host-owned, tools-disabled native-schema finalization call.
//! - A default (assistant-message) unbound run completes on a plain final.
//! - A prepared thread never appears in the owner-scoped listing a
//!   conversation UI reads — hidden by the prepared-context stamp, whether
//!   the thread is ownerless or caller-owned.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use std::sync::Arc;
use std::time::Duration;

use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_host_api::turn::{TurnScope, TurnThreadOwner};
use ironclaw_host_api::{output::OutputContract, prepared_context::PreparedTurnDeclarations};
use ironclaw_llm::agent_message::{AgentMessage, AgentMessageRole, ContentPart};
use ironclaw_llm::{CompletionResponseFormat, Role};
use ironclaw_threads::{
    ListThreadsForScopeRequest, PreparedContextRequest, ReadStructuredFinalizationRequest,
    SessionThreadService, ThreadHistoryRequest, ThreadScope,
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
        thread_id: ironclaw_host_api::ids::ThreadId::new(format!("unbound-{idempotency_key}"))?,
        title: None,
        metadata_json: None,
    })
}

fn structured_declarations() -> PreparedTurnDeclarations {
    PreparedTurnDeclarations {
        tools: Vec::new(),
        output: OutputContract::JsonSchema {
            name: "sentiment_v1".to_string(),
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
        require_no_approval: false,
    }
}

fn submit_request(
    scope: TurnScope,
    accepted_message_ref: ironclaw_host_api::turn::AcceptedMessageRef,
    idempotency_key: &str,
) -> HarnessResult<SubmitTurnRequest> {
    Ok(SubmitTurnRequest {
        subagent_activation_provenance: None,
        scope,
        actor: TurnActor::new(UserId::new(CALLER)?),
        accepted_message_ref,
        requested_run_profile: None,
        output_contract: None,
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
    scripted_llm: Arc<support::trace_llm::TraceLlm>,
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
    let scripted_llm = group
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
        scripted_llm,
    })
}

/// Structured happy path: the ordinary unbounded loop produces a candidate,
/// then the host performs one native-schema finalizer inference and durably
/// records the raw JSON plus its parsed view.
#[tokio::test(flavor = "multi_thread")]
async fn structured_unbound_run_completes_by_recording_a_validated_result() -> HarnessResult<()> {
    let group = RebornIntegrationGroup::builtin_tools().await?;
    let harness = group.thread("conv-unbound-anchor").build().await?;

    let run = run_unbound(
        &group,
        &harness,
        "unbound-structured-happy",
        structured_declarations(),
        vec![
            RebornScriptedReply::text("The release sentiment is strongly positive."),
            RebornScriptedReply::text(serde_json::to_string(
                &json!({"sentiment": "positive", "confidence": 0.9}),
            )?),
        ],
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
        "the output contract must not select a distinct loop family"
    );
    assert!(run.state.output_contract.is_json_schema());

    assert_eq!(run.scripted_llm.captured_requests().len(), 2);
    assert_eq!(
        run.scripted_llm.streaming_calls(),
        1,
        "the structured finalizer must use the streaming transport"
    );
    let requests = run.scripted_llm.captured_requests();
    assert!(
        requests[0].iter().any(|message| {
            message.role == Role::HostReminder
                && message
                    .content
                    .contains("work phase for a structured response")
                && message.content.contains("\"sentiment\"")
        }),
        "the initial work-phase request must retain the durable schema guidance as a host reminder"
    );
    for expected in [
        "You are a background extraction task.",
        "Classify the sentiment of: the release went great",
        "The release sentiment is strongly positive.",
    ] {
        assert!(
            requests[1]
                .iter()
                .any(|message| message.content.contains(expected)),
            "the one finalizer request must retain canonical context: {expected}"
        );
    }
    let formats = run.scripted_llm.captured_response_formats();
    assert!(formats[0].is_none(), "the work phase is ordinary inference");
    let final_format = match formats[1].as_ref() {
        Some(CompletionResponseFormat::JsonSchema(format)) => format,
        Some(CompletionResponseFormat::JsonObject) => {
            return Err("the finalizer must request a native JSON schema".into());
        }
        None => return Err("the finalizer must request a native JSON schema".into()),
    };
    assert_eq!(final_format.name, "sentiment_v1");
    assert!(final_format.is_strict());
    assert_eq!(
        final_format.schema,
        json!({
            "type": "object",
            "properties": {
                "sentiment": {"type": "string", "enum": ["positive", "negative"]},
                "confidence": {"type": "number"}
            },
            "required": ["sentiment"],
            "additionalProperties": false
        }),
        "the finalizer must forward the exact durable schema payload"
    );
    let offered_tools = run.scripted_llm.captured_tool_definitions();
    assert!(
        offered_tools[1].is_empty(),
        "the structured finalizer must expose no tools"
    );

    let thread_service = harness.thread_service_for_test()?;
    let record = thread_service
        .read_structured_finalization(ReadStructuredFinalizationRequest {
            scope: ownerless_thread_scope()?,
            thread_id: run.thread_id.clone(),
            turn_run_id: run.state.run_id,
        })
        .await?
        .ok_or("the structured finalization record must be durable")?;
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&record.raw_json)?,
        json!({"sentiment": "positive", "confidence": 0.9})
    );

    // Taxonomy: the ownerless thread is structurally invisible to the
    // owner-scoped listing every conversation UI reads. An anchor thread in
    // the caller's scope keeps the assertion non-vacuous — the listing must
    // show the conversation AND hide the unbound thread.
    let caller_scope = ThreadScope {
        owner_user_id: Some(UserId::new(CALLER)?),
        ..ownerless_thread_scope()?
    };
    let anchor = thread_service
        .ensure_thread(ironclaw_threads::EnsureThreadRequest {
            scope: caller_scope.clone(),
            thread_id: Some(ironclaw_host_api::ids::ThreadId::new("t-listing-anchor")?),
            created_by_actor_id: CALLER.to_string(),
            title: Some("visible conversation".to_string()),
            metadata_json: None,
        })
        .await?;
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
            .any(|thread| thread.thread_id == anchor.thread_id),
        "the owner-scoped listing must surface the caller's conversation"
    );
    assert!(
        listed
            .threads
            .iter()
            .all(|thread| thread.thread_id != run.thread_id),
        "an unbound thread must never surface in an owner-scoped listing"
    );
    Ok(())
}

/// Provider-native structured output is authoritative after the one terminal
/// inference. The host neither reparses it nor adds a schema-repair loop.
#[tokio::test(flavor = "multi_thread")]
async fn structured_unbound_run_does_not_revalidate_or_retry_provider_output() -> HarnessResult<()>
{
    let group = RebornIntegrationGroup::builtin_tools().await?;
    let harness = group.thread("conv-unbound-repair-anchor").build().await?;

    let run = run_unbound(
        &group,
        &harness,
        "unbound-structured-repair",
        structured_declarations(),
        vec![
            RebornScriptedReply::text("The sentiment is negative."),
            RebornScriptedReply::text("not json"),
        ],
    )
    .await?;

    assert_eq!(
        run.state.status,
        TurnStatus::Completed,
        "the native structured-response contract is authoritative"
    );
    assert_eq!(
        run.scripted_llm.captured_requests().len(),
        2,
        "the host must not run a schema-repair inference"
    );
    let thread_service = harness.thread_service_for_test()?;
    let record = thread_service
        .read_structured_finalization(ReadStructuredFinalizationRequest {
            scope: ownerless_thread_scope()?,
            thread_id: run.thread_id.clone(),
            turn_run_id: run.state.run_id,
        })
        .await?;
    assert_eq!(
        record.map(|record| record.raw_json),
        Some("not json".to_string())
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

/// Seeded tool history through the accept door: an assistant tool-call turn
/// plus its tool result persist replay-faithfully — the round lands as a
/// `ToolResultReference` row whose full outcome bytes are durable in the
/// record store — and the run completes over that seeded context through the
/// production context-load path.
#[tokio::test(flavor = "multi_thread")]
async fn unbound_run_over_seeded_tool_history_completes_and_persists_the_round() -> HarnessResult<()>
{
    use ironclaw_llm::agent_message::{ToolCallContent, ToolResultContent, ToolResultOutcome};

    let group = RebornIntegrationGroup::builtin_tools().await?;
    let harness = group.thread("conv-unbound-seeded-anchor").build().await?;
    let thread_service = harness.thread_service_for_test()?;

    let mut request = prepared_request(
        "unbound-seeded-accept",
        "Continue from the lookup above.",
        PreparedTurnDeclarations::default(),
    )?;
    request.messages = vec![
        AgentMessage {
            role: AgentMessageRole::User,
            content: vec![ContentPart::text("Look up the release status")],
        },
        AgentMessage {
            role: AgentMessageRole::Assistant,
            content: vec![ContentPart::ToolCall(ToolCallContent {
                call_id: "call_seeded_1".to_string(),
                capability: ironclaw_host_api::ids::CapabilityId::new("external_tool.lookup")?,
                arguments: json!({"query": "release status"}),
            })],
        },
        AgentMessage {
            role: AgentMessageRole::Tool,
            content: vec![ContentPart::ToolResult(ToolResultContent {
                call_id: "call_seeded_1".to_string(),
                outcome: ToolResultOutcome::Text {
                    text: "the release is green".to_string(),
                },
                is_error: false,
            })],
        },
        AgentMessage {
            role: AgentMessageRole::User,
            content: vec![ContentPart::text("Continue from the lookup above.")],
        },
    ];
    let accepted = thread_service.accept_prepared_context(request).await?;

    let scope = unbound_turn_scope(&accepted.thread_id)?;
    group
        .register_scope_script_for_test(
            scope.clone(),
            "unbound-seeded",
            vec![RebornScriptedReply::text("The release is green; done.")],
        )
        .await?;
    let coordinator = harness.turn_coordinator_for_test();
    let SubmitTurnResponse::Accepted { run_id, .. } = coordinator
        .submit_turn(submit_request(
            scope.clone(),
            accepted.accepted_message_ref.clone(),
            "unbound-seeded-submit",
        )?)
        .await?;
    let state = wait_for_terminal(&coordinator, &scope, run_id).await?;
    assert_eq!(
        state.status,
        TurnStatus::Completed,
        "failure={:?}",
        state.failure
    );

    // Seam assertion: the seeded tool round is a ToolResultReference row and
    // its FULL outcome bytes page back out of the durable record store.
    let history = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: ownerless_thread_scope()?,
            thread_id: accepted.thread_id.clone(),
        })
        .await?;
    let seeded_result_ref = history
        .messages
        .iter()
        .find(|message| message.kind == ironclaw_threads::MessageKind::ToolResultReference)
        .and_then(|message| message.tool_result_ref.clone())
        .ok_or("the seeded tool round must land as a ToolResultReference row")?;
    let chunk = thread_service
        .read_tool_result_record(ironclaw_threads::ReadToolResultRecordRequest {
            scope: ownerless_thread_scope()?,
            thread_id: accepted.thread_id.clone(),
            result_ref: seeded_result_ref,
            offset: 0,
            max_bytes: ironclaw_threads::effective_tool_result_read_max_bytes(),
        })
        .await?
        .ok_or("the seeded tool result record must be durable")?;
    assert!(
        String::from_utf8_lossy(&chunk.content).contains("the release is green"),
        "the record store must hold the seeded outcome bytes in full"
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

/// Step-1a owner threading (owner-nonnull proposal): the PRODUCT unbound lane
/// (`UnboundTurnService`) names its authenticated caller as the thread OWNER.
/// The minted thread lives under the caller's owner scope — not the tenant
/// `__system__` slot — read-back resolves with the caller identity, a foreign
/// caller's run-state read is rejected, and the thread STILL never surfaces
/// in the caller's conversation listing (hidden by the prepared-context
/// stamp, not by ownerlessness).
#[tokio::test(flavor = "multi_thread")]
async fn unbound_service_threads_caller_as_thread_owner() -> HarnessResult<()> {
    use ironclaw_assistant::{UnboundTurnService, UnboundTurnSubmission};
    use ironclaw_product_contracts::surface::ProductSurfaceCaller;

    let group = RebornIntegrationGroup::builtin_tools().await?;
    let harness = group.thread("conv-unbound-owner-anchor").build().await?;
    let thread_service = harness.thread_service_for_test()?;
    let coordinator = harness.turn_coordinator_for_test();

    let caller = UserId::new(CALLER)?;
    let service = UnboundTurnService::new(
        thread_service.clone(),
        coordinator.clone(),
        AgentId::new(AGENT)?,
        Some(ProjectId::new(PROJECT)?),
    );

    // The service now mints ExplicitUser-owned turn scopes; the scripted
    // reply must be registered under that exact scope.
    let public_id = "unbound-owner-thread";
    let thread_id = ironclaw_host_api::ids::ThreadId::new(public_id)?;
    let script_scope = TurnScope::new_with_owner(
        TenantId::new(TENANT)?,
        Some(AgentId::new(AGENT)?),
        Some(ProjectId::new(PROJECT)?),
        thread_id.clone(),
        Some(caller.clone()),
    );
    group
        .register_scope_script_for_test(
            script_scope.clone(),
            "unbound-owner",
            vec![RebornScriptedReply::text("Sentiment: positive.")],
        )
        .await?;

    let ack = service
        .accept_and_submit(UnboundTurnSubmission {
            caller: ProductSurfaceCaller::new(
                TenantId::new(TENANT)?,
                caller.clone(),
                Some(AgentId::new(AGENT)?),
                Some(ProjectId::new(PROJECT)?),
            ),
            public_id: public_id.to_string(),
            system_prompt: "You are a background extraction task.".to_string(),
            messages: vec![AgentMessage {
                role: AgentMessageRole::User,
                content: vec![ContentPart::text(
                    "Classify the sentiment of: the release went great",
                )],
            }],
            tools: Vec::new(),
            output: OutputContract::AssistantMessage,
            // Interactive product callers keep the profile's own ceilings.
            limits: Default::default(),
            requested_model: None,
            idempotency_key: "unbound-owner-accept".to_string(),
            require_no_approval: false,
        })
        .await?;
    let ironclaw_product_contracts::inbound::ProductInboundAck::Accepted {
        submitted_run_id, ..
    } = ack
    else {
        return Err("accept_and_submit must return Accepted".into());
    };

    // Read-back resolves under the caller identity and completes.
    let outcome = service
        .wait_for_completion(
            public_id,
            &ProductSurfaceCaller::new(
                TenantId::new(TENANT)?,
                caller.clone(),
                Some(AgentId::new(AGENT)?),
                Some(ProjectId::new(PROJECT)?),
            ),
            submitted_run_id,
            Duration::from_millis(50),
        )
        .await?;
    assert_eq!(outcome.text, "Sentiment: positive.");

    // The thread's rows live under the caller's OWNER scope, not the tenant
    // system slot: owner-scoped history sees the seeded rows; the ownerless
    // scope sees nothing.
    let owner_scope = ThreadScope {
        owner_user_id: Some(caller.clone()),
        ..ownerless_thread_scope()?
    };
    let owned_history = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: owner_scope.clone(),
            thread_id: thread_id.clone(),
        })
        .await?;
    assert!(
        !owned_history.messages.is_empty(),
        "the unbound thread must be stored under the caller's owner scope"
    );
    let ownerless_history = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: ownerless_thread_scope()?,
            thread_id: thread_id.clone(),
        })
        .await;
    match ownerless_history {
        Ok(history) => assert!(
            history.messages.is_empty(),
            "no unbound rows may land in the tenant __system__ slot"
        ),
        Err(ironclaw_threads::SessionThreadError::UnknownThread { .. }) => {
            // The thread does not exist under the ownerless (tenant
            // __system__) scope — the non-enumerating "unknown thread" shape
            // `SessionThreadService::read_thread`/`list_thread_history`
            // return for both "does not exist" and "exists under a
            // different scope" per their documented contract.
        }
        Err(other) => {
            return Err(format!(
                "expected UnknownThread reading the unbound thread under the ownerless scope, got: {other:?}"
            )
            .into());
        }
    }

    // Cross-user isolation is now ENFORCED, not just structural: a foreign
    // caller's run-state read under its own owner scope is rejected.
    let foreign_scope = TurnScope::new_with_owner(
        TenantId::new(TENANT)?,
        Some(AgentId::new(AGENT)?),
        Some(ProjectId::new(PROJECT)?),
        thread_id.clone(),
        Some(UserId::new("other-user")?),
    );
    assert!(
        coordinator
            .get_run_state(GetRunStateRequest {
                scope: foreign_scope,
                run_id: submitted_run_id,
            })
            .await
            .is_err(),
        "a foreign owner scope must not read an unbound run's state"
    );

    // Owner is set, but the thread still never surfaces in the caller's
    // conversation listing — hiding comes from the prepared-context stamp.
    let listed = thread_service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: owner_scope,
            limit: None,
            cursor: None,
        })
        .await?;
    assert!(
        listed
            .threads
            .iter()
            .all(|thread| thread.thread_id != thread_id),
        "an owned unbound thread must still be hidden from the listing"
    );
    Ok(())
}
