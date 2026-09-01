//! Coverage for the per-run prompt context budget the production host-build
//! seam resolves from the run's model. Split out of `loop_driver_host`'s
//! `mod tests` (sibling pattern, like `compaction_tests`) to keep that file
//! under the architecture size threshold.
//!
//! The behavior under test: `build_text_only_host_with_capabilities` asks the
//! run's gateway for the model's advertised context window and, when the
//! gateway reports one, derives a `PromptContextTokenBudget` from it and
//! carries it on `LoopRunContext`. A gateway that reports nothing must leave
//! the run exactly as it behaved before this seam existed.

use super::*;

use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, ThreadId};
use ironclaw_host_api::turn::{TurnId, TurnLeaseToken, TurnRunId, TurnScope};
use ironclaw_loop_contracts::{
    InMemoryLoopHostMilestoneSink, InMemoryRunProfileResolver, LoopRunInfoPort,
    RunProfileResolutionRequest, RunProfileResolver,
};
use ironclaw_threads::{AcceptInboundMessageRequest, EnsureThreadRequest, MessageContent};
use ironclaw_turns::test_support::{in_memory_agent_turn_runtime, in_memory_loop_checkpoint_store};

/// A gateway that advertises a configurable context window and is never asked
/// to stream a model call.
struct WindowAdvertisingGateway {
    window: Option<u64>,
}

#[async_trait]
impl HostManagedModelGateway for WindowAdvertisingGateway {
    async fn stream_model(
        &self,
        _request: ironclaw_loop_host::HostManagedModelRequest,
    ) -> Result<
        ironclaw_loop_host::HostManagedModelResponse,
        ironclaw_loop_host::HostManagedModelError,
    > {
        panic!("the context-budget tests never dispatch a model call"); // safety: test-only sentinel for an unreachable model call.
    }

    async fn advertised_context_window_tokens(
        &self,
        _model_profile_id: &ironclaw_loop_contracts::ModelProfileId,
        _resolved_model_route: Option<&ironclaw_loop_host::HostManagedModelRouteSnapshot>,
    ) -> Option<u64> {
        self.window
    }
}

/// Builds a host through the real production seam and hands back the
/// `LoopRunContext` it resolved, so the assertion reads what a real run would.
async fn resolved_budget_for(window: Option<u64>) -> Option<PromptContextTokenBudget> {
    let thread_service = Arc::new(ironclaw_threads::InMemorySessionThreadService::default());
    let suffix = window.map(|w| w.to_string()).unwrap_or("none".to_string());
    let tenant_id = TenantId::new(format!("tenant-ctx-budget-{suffix}")).unwrap();
    let agent_id = AgentId::new(format!("agent-ctx-budget-{suffix}")).unwrap();
    let project_id = ProjectId::new(format!("project-ctx-budget-{suffix}")).unwrap();
    let thread_id = ThreadId::new(format!("thread-ctx-budget-{suffix}")).unwrap();
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
            created_by_actor_id: "user-ctx-budget".to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    thread_service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: thread_scope.clone(),
            thread_id: thread_id.clone(),
            actor_id: "user-ctx-budget".to_string(),
            source_binding_id: Some("source-web".to_string()),
            reply_target_binding_id: Some("reply-web".to_string()),
            external_event_id: Some(format!("event-ctx-budget-{suffix}")),
            content: MessageContent::text("hello context budget"),
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
        resolved,
    );
    let claimed_run = claimed_run_for(&run_context, &turn_scope);

    let factory = RebornLoopDriverHostFactory::new(
        thread_service,
        thread_scope,
        Arc::new(WindowAdvertisingGateway { window }),
        Arc::new(in_memory_agent_turn_runtime()) as Arc<dyn AgentTurnSpawnTreeRuntimePort>,
        Arc::new(in_memory_loop_checkpoint_store()) as Arc<dyn LoopCheckpointStore>,
        Arc::new(InMemoryLoopHostMilestoneSink::default()) as Arc<dyn LoopHostMilestoneSink>,
        TextOnlyLoopHostConfig {
            max_messages: 8,
            prompt_context_budget: Default::default(),
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

    host.run_context().resolved_context_budget
}

fn claimed_run_for(
    run_context: &LoopRunContext,
    scope: &TurnScope,
) -> ironclaw_turns::runner::ClaimedTurnRun {
    use ironclaw_turns::{AcceptedMessageRef, TurnRunnerId, TurnStatus};

    ironclaw_turns::runner::ClaimedTurnRun {
        subagent_activation_provenance: None,
        state: ironclaw_turns::TurnRunState {
            scope: scope.clone(),
            actor: None,
            turn_id: run_context.turn_id,
            run_id: run_context.run_id,
            status: TurnStatus::Running,
            accepted_message_ref: AcceptedMessageRef::new("msg:accepted").expect("valid"), // safety: fixed fixture satisfies the bounded-ref grammar.
            output_contract: ironclaw_host_api::output::OutputContract::AssistantMessage,
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

#[tokio::test]
async fn resolved_budget_reaches_the_run_context_when_the_gateway_advertises_a_window() {
    let budget = resolved_budget_for(Some(40_000)).await;

    assert_eq!(
        budget,
        Some(PromptContextTokenBudget::from_advertised_window(Some(
            40_000
        ))),
        "a run whose model advertises a window must carry the derived budget"
    );
}

#[tokio::test]
async fn run_context_carries_no_budget_when_the_gateway_advertises_nothing() {
    let budget = resolved_budget_for(None).await;

    assert_eq!(
        budget, None,
        "a gateway that advertises nothing must leave the run on the compiled-in default"
    );
}
