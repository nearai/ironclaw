//! Coverage for the run-lease fence the production host-build seam installs on
//! the transcript port. Split out of `loop_driver_host`'s `mod tests` (sibling
//! pattern, like `compaction_tests`) to keep that file under the architecture
//! size threshold.

use super::*;

use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, ThreadId};
use ironclaw_host_api::turn::{TurnId, TurnLeaseToken, TurnRunId, TurnScope};
use ironclaw_loop_contracts::{
    AssistantReply, FinalizeAssistantMessage, InMemoryLoopHostMilestoneSink,
    InMemoryRunProfileResolver, LoopTranscriptPort, RunProfileResolutionRequest,
    RunProfileResolver,
};
use ironclaw_threads::{AcceptInboundMessageRequest, EnsureThreadRequest, MessageContent};
use ironclaw_turns::test_support::{in_memory_agent_turn_runtime, in_memory_loop_checkpoint_store};

/// A model gateway that is never reached — this test never runs a model call.
struct UnusedGateway;

#[async_trait]
impl HostManagedModelGateway for UnusedGateway {
    async fn stream_model(
        &self,
        _request: ironclaw_loop_host::HostManagedModelRequest,
    ) -> Result<
        ironclaw_loop_host::HostManagedModelResponse,
        ironclaw_loop_host::HostManagedModelError,
    > {
        panic!("this test never dispatches a model call"); // safety: test-only sentinel for an unreachable model call.
    }
}

/// The fence that stops a lease-reclaimed ("zombie") worker from appending a
/// second assistant message beside its replacement's lives on
/// `ThreadBackedLoopTranscriptPort`, but it only protects production if the
/// production build seam actually installs it — and it is an `Option`, so a
/// future construction path that forgets to call `with_run_lease_fence` would
/// silently fail *open* with every other test still green.
///
/// This pins the wiring at that seam rather than the fence's own logic: a host
/// built through `build_text_only_host_with_capabilities` must refuse a
/// transcript finalize whose claimed lease the journal does not record. The
/// journal here (a fresh in-memory runtime) holds no record for this run at
/// all, which is the strongest form of "not the live lease" — so an unfenced
/// adapter would happily persist the message and this test would fail.
#[tokio::test]
async fn production_host_build_fences_transcript_writes_on_the_claimed_lease() {
    let thread_service = Arc::new(ironclaw_threads::InMemorySessionThreadService::default());
    let tenant_id = TenantId::new("tenant-lease-fence-test").unwrap();
    let agent_id = AgentId::new("agent-lease-fence-test").unwrap();
    let project_id = ProjectId::new("project-lease-fence-test").unwrap();
    let thread_id = ThreadId::new("thread-lease-fence-test").unwrap();
    let thread_scope = ThreadScope {
        tenant_id: tenant_id.clone(),
        agent_id: agent_id.clone(),
        project_id: Some(project_id.clone()),
        owner_user_id: None,
        mission_id: None,
    };
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope.clone(),
            thread_id: Some(thread_id.clone()),
            created_by_actor_id: "user-lease-fence-test".to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    thread_service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: thread_scope.clone(),
            thread_id: thread_id.clone(),
            actor_id: "user-lease-fence-test".to_string(),
            source_binding_id: Some("source-web".to_string()),
            reply_target_binding_id: Some("reply-web".to_string()),
            external_event_id: Some("event-lease-fence-test".to_string()),
            content: MessageContent::text("hello lease fence"),
        })
        .await
        .unwrap();

    let turn_scope = TurnScope::new(
        tenant_id,
        Some(agent_id),
        Some(project_id),
        thread_id.clone(),
    );
    let resolved = InMemoryRunProfileResolver::default()
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .unwrap();
    let run_context = LoopRunContext::new(
        turn_scope.clone(),
        TurnId::new(),
        TurnRunId::new(),
        resolved.clone(),
    );
    let claimed_run = claimed_run_matching(&run_context, &turn_scope);

    let factory = RebornLoopDriverHostFactory::new(
        thread_service,
        thread_scope,
        Arc::new(UnusedGateway),
        Arc::new(in_memory_agent_turn_runtime()) as Arc<dyn AgentTurnSpawnTreeRuntimePort>,
        Arc::new(in_memory_loop_checkpoint_store()) as Arc<dyn LoopCheckpointStore>,
        Arc::new(InMemoryLoopHostMilestoneSink::default()) as Arc<dyn LoopHostMilestoneSink>,
        TextOnlyLoopHostConfig {
            max_messages: 8,
            require_model_route_snapshot: false,
        },
        InstructionSafetyContext::non_production_noop(),
    );

    let host = factory
        .build_text_only_host_with_capabilities(
            RebornLoopDriverHostRequest {
                claimed_run,
                loop_run_context: run_context,
            },
            Arc::new(EmptyLoopCapabilityPort),
        )
        .await
        .expect("host builds");

    let error = host
        .finalize_assistant_message(FinalizeAssistantMessage {
            reply: AssistantReply {
                content: "zombie worker output".to_string(),
            },
        })
        .await
        .expect_err(
            "a claimed lease the journal does not record must be refused — the production \
             build seam must install the run-lease fence on the transcript port",
        );
    assert_eq!(
        error.kind,
        AgentLoopHostErrorKind::TranscriptWriteFailed,
        "the refusal must be an explicit transcript-write failure the loop can carry into \
         its (lease-fenced) exit claim, not some other error class"
    );
}

/// A `Running` claim whose ids and profile match `run_context`, so
/// `validate_claimed_run_context` accepts it and the test exercises the fence
/// rather than the earlier validation.
fn claimed_run_matching(
    run_context: &LoopRunContext,
    scope: &TurnScope,
) -> ironclaw_turns::runner::ClaimedTurnRun {
    use ironclaw_turns::{AcceptedMessageRef, TurnRunnerId, TurnStatus};

    ironclaw_turns::runner::ClaimedTurnRun {
        state: ironclaw_turns::TurnRunState {
            scope: scope.clone(),
            actor: None,
            turn_id: run_context.turn_id,
            run_id: run_context.run_id,
            status: TurnStatus::Running,
            accepted_message_ref: AcceptedMessageRef::new("msg:accepted").expect("valid"), // safety: fixed fixture satisfies the bounded-ref grammar.
            // The journal persists the interactive-default alias under the
            // default profile id; `validate_claimed_run_context` compares
            // against that persisted form.
            resolved_run_profile_id: persisted_profile_id(
                &run_context.resolved_run_profile.profile_id,
            ),
            resolved_run_profile_version: run_context.resolved_run_profile.profile_version,
            allow_steering: true,
            resolved_model_route: None,
            model_usage: None,
            execution_outcome: None,
            received_at: chrono::Utc::now(),
            checkpoint_id: None,
            gate_ref: None,
            blocked_activity_id: None,
            credential_requirements: Vec::new(),
            failure: None,
            event_cursor: ironclaw_turns::EventCursor(0),
            product_context: None,
            resume_disposition: None,
        },
        resolved_run_profile: run_context.resolved_run_profile.clone(),
        subagent_depth: 0,
        spawn_tree_descendant_cap: None,
        runner_id: TurnRunnerId::new(),
        lease_token: TurnLeaseToken::new(),
    }
}
