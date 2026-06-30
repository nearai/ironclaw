//! Coverage for `RebornLoopDriverHostFactory::build_compaction_ports`'
//! scope-resolved gateway routing. Split out of `loop_driver_host`'s
//! `mod tests` (sibling pattern, like `port_adapter_tests`) to keep that
//! file under the architecture size threshold.

use super::*;

use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId};
use ironclaw_turns::{
    InMemoryCheckpointStateStore, InMemoryLoopCheckpointStore, InMemoryRunProfileResolver,
    RunProfileResolver, TurnId, TurnRunId, TurnScope,
    run_profile::{InMemoryLoopHostMilestoneSink, RunProfileResolutionRequest},
};

/// Coverage gap closed: `build_compaction_ports` (see above) resolves a
/// scope-specific gateway via `self.model_gateway.resolve_for_scope(&run_context.scope)`,
/// falling back to the host's own gateway only when the resolver returns
/// `None`. The Reborn group integration scenario (`ScopeRegistryGateway`
/// in `tests/support/reborn/scope_gateway.rs`) only drives the MAIN model
/// path through `resolve_for_scope` — it never submits a compaction —
/// so a regression that drops or misroutes the scoped gateway for
/// compaction would leave that scenario green while compaction silently
/// dispatched through the fallback (or the wrong gateway).
///
/// This test pins the compaction arm directly: the host's own gateway
/// (`LoudFallbackGateway`) panics if it is ever asked to stream a model
/// call, while `resolve_for_scope` hands back a distinct
/// `RecordingScopedGateway` that records the dispatch. The test only
/// passes if compaction reaches the scoped gateway.
#[tokio::test]
async fn build_compaction_ports_dispatches_through_scope_resolved_gateway() {
    use ironclaw_loop_support::{
        HostManagedModelError, HostManagedModelRequest, HostManagedModelResponse,
    };
    use ironclaw_threads::{AcceptInboundMessageRequest, EnsureThreadRequest, MessageContent};
    use ironclaw_turns::run_profile::{LoopCompactionMode, SystemInferenceTaskId};

    /// Records every request it receives. This is the gateway
    /// `resolve_for_scope` hands back — the destination compaction must
    /// reach when scope routing is honored.
    #[derive(Default)]
    struct RecordingScopedGateway {
        calls: Mutex<usize>,
    }

    #[async_trait]
    impl HostManagedModelGateway for RecordingScopedGateway {
        async fn stream_model(
            &self,
            _request: HostManagedModelRequest,
        ) -> Result<HostManagedModelResponse, HostManagedModelError> {
            *self.calls.lock().unwrap() += 1;
            Ok(HostManagedModelResponse::assistant_reply(
                "scoped compaction summary",
            ))
        }
    }

    /// The host's own gateway. Its `stream_model` panics so any
    /// dispatch that bypasses `resolve_for_scope` (i.e. the old
    /// `Arc::clone(&self.model_gateway)`-only behavior) fails the test
    /// loudly instead of silently passing. `resolve_for_scope` hands
    /// back the distinct `RecordingScopedGateway` above.
    struct LoudFallbackGateway {
        scoped: Arc<RecordingScopedGateway>,
    }

    #[async_trait]
    impl HostManagedModelGateway for LoudFallbackGateway {
        async fn stream_model(
            &self,
            _request: HostManagedModelRequest,
        ) -> Result<HostManagedModelResponse, HostManagedModelError> {
            panic!(
                "compaction dispatched through the fallback gateway instead of the \
                 scope-resolved gateway — build_compaction_ports must call \
                 self.model_gateway.resolve_for_scope(&run_context.scope)"
            );
        }

        fn resolve_for_scope(
            &self,
            _scope: &TurnScope,
        ) -> Option<Arc<dyn HostManagedModelGateway>> {
            Some(Arc::clone(&self.scoped) as Arc<dyn HostManagedModelGateway>)
        }
    }

    let thread_service = Arc::new(ironclaw_threads::InMemorySessionThreadService::default());
    let tenant_id = TenantId::new("tenant-compaction-scope-test").unwrap();
    let agent_id = AgentId::new("agent-compaction-scope-test").unwrap();
    let project_id = ProjectId::new("project-compaction-scope-test").unwrap();
    let thread_id = ThreadId::new("thread-compaction-scope-test").unwrap();
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
            created_by_actor_id: "user-compaction-scope-test".to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    thread_service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: thread_scope.clone(),
            thread_id: thread_id.clone(),
            actor_id: "user-compaction-scope-test".to_string(),
            source_binding_id: Some("source-web".to_string()),
            reply_target_binding_id: Some("reply-web".to_string()),
            external_event_id: Some("event-compaction-scope-test".to_string()),
            content: MessageContent::text("hello compaction scope routing"),
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
    let run_context = LoopRunContext::new(turn_scope, TurnId::new(), TurnRunId::new(), resolved);

    let scoped_gateway = Arc::new(RecordingScopedGateway::default());
    let model_gateway = Arc::new(LoudFallbackGateway {
        scoped: Arc::clone(&scoped_gateway),
    });

    let factory = RebornLoopDriverHostFactory::new(
        thread_service,
        thread_scope,
        model_gateway,
        Arc::new(InMemoryCheckpointStateStore::default()) as Arc<dyn CheckpointStateStore>,
        Arc::new(ironclaw_turns::InMemoryTurnStateStore::default()) as Arc<dyn TurnStateStore>,
        Arc::new(InMemoryLoopCheckpointStore::default()) as Arc<dyn LoopCheckpointStore>,
        Arc::new(InMemoryLoopHostMilestoneSink::default()) as Arc<dyn LoopHostMilestoneSink>,
        TextOnlyLoopHostConfig {
            max_messages: 8,
            require_model_route_snapshot: false,
        },
        InstructionSafetyContext::local_development_noop(),
    );

    let compaction = factory.build_compaction_ports(&run_context);

    compaction
        .compact_loop_context(LoopCompactionRequest {
            task_id: SystemInferenceTaskId::new(),
            thread_id,
            last_compacted_through_seq: None,
            drop_through_seq: 1,
            preserve_tail_tokens: 8_000,
            mode: LoopCompactionMode::Fresh,
            deadline_ms: 1_000,
        })
        .await
        .expect("compaction should succeed through the scope-resolved gateway");

    assert_eq!(
        *scoped_gateway.calls.lock().unwrap(),
        1,
        "compaction must dispatch exactly once through the scope-resolved gateway"
    );
}
