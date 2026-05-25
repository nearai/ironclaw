use super::turn_events::WEBUI_TURN_EVENT_PAGE_LIMIT;
use super::*;

use async_trait::async_trait;
use ironclaw_event_projections::{
    CapabilityActivityProjection, ProjectionSnapshot, ThreadTimeline,
};
use ironclaw_events::{InMemoryDurableEventLog, RuntimeEvent};
use ironclaw_host_api::{
    AgentId, CapabilityId, ExtensionId, InvocationId, ResourceScope, RuntimeKind, TenantId,
    ThreadId, UserId,
};
use ironclaw_product_adapters::{
    CapabilityActivityStatusView, ProductOutboundEnvelope, ProductOutboundPayload,
};
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunRequest, CancelRunResponse, EventCursor as TurnEventCursor,
    GateRef, GetRunStateRequest, ResumeTurnRequest, ResumeTurnResponse, RunProfileId,
    RunProfileVersion, SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse,
    TurnBlockedGateKind, TurnBlockedGateMetadata, TurnError, TurnEventKind, TurnEventPage,
    TurnLifecycleEvent, TurnRunState, TurnStatus,
};

mod cursor_validation;
mod display_preview;
mod runtime_stream;
mod turn_stream;

fn long_test_id(prefix: &str, character: char) -> String {
    format!("{prefix}-{}", character.to_string().repeat(96))
}

fn resource_scope(
    tenant_id: &TenantId,
    user_id: &UserId,
    agent_id: &AgentId,
    thread_id: &ThreadId,
    invocation_id: InvocationId,
) -> ResourceScope {
    ResourceScope {
        tenant_id: tenant_id.clone(),
        user_id: user_id.clone(),
        agent_id: Some(agent_id.clone()),
        project_id: None,
        mission_id: None,
        thread_id: Some(thread_id.clone()),
        invocation_id,
    }
}

fn contains_run_status(
    events: &[ProductOutboundEnvelope],
    invocation_id: InvocationId,
    expected_status: &str,
) -> bool {
    let expected_run_id = TurnRunId::from_uuid(invocation_id.as_uuid());
    events.iter().any(|event| match event.payload() {
        ProductOutboundPayload::ProjectionSnapshot { state }
        | ProductOutboundPayload::ProjectionUpdate { state } => state.items.iter().any(|item| {
            matches!(
                item,
                ProductProjectionItem::RunStatus { run_id, status }
                    if *run_id == expected_run_id && status == expected_status
            )
        }),
        _ => false,
    })
}

struct FakeTurnEventSource {
    events: Vec<TurnLifecycleEvent>,
}

#[async_trait]
impl TurnEventProjectionSource for FakeTurnEventSource {
    async fn read_turn_events_after(
        &self,
        scope: &TurnScope,
        after: Option<TurnEventCursor>,
        limit: usize,
    ) -> Result<TurnEventPage, TurnError> {
        let after = after.unwrap_or_default();
        let mut events = self
            .events
            .iter()
            .filter(|event| &event.scope == scope && event.cursor > after)
            .cloned()
            .collect::<Vec<_>>();
        events.sort_by_key(|event| event.cursor);
        let truncated = events.len() > limit;
        if truncated {
            events.truncate(limit);
        }
        let next_cursor = events.last().map(|event| event.cursor).unwrap_or(after);
        Ok(TurnEventPage {
            entries: events,
            next_cursor,
            truncated,
            rebase_required: None,
        })
    }
}

struct FakeTurnCoordinator {
    state: TurnRunState,
}

#[async_trait]
impl TurnCoordinator for FakeTurnCoordinator {
    async fn submit_turn(
        &self,
        _request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        unreachable!("projection tests only read run state")
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        unreachable!("projection tests only read run state")
    }

    async fn cancel_run(&self, _request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        unreachable!("projection tests only read run state")
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        if request.scope == self.state.scope && request.run_id == self.state.run_id {
            Ok(self.state.clone())
        } else {
            Err(TurnError::ScopeNotFound)
        }
    }
}

fn turn_run_state(
    scope: &TurnScope,
    user_id: &UserId,
    run_id: TurnRunId,
    cursor: TurnEventCursor,
) -> TurnRunState {
    TurnRunState {
        scope: scope.clone(),
        actor: Some(TurnActor::new(user_id.clone())),
        turn_id: ironclaw_turns::TurnId::new(),
        run_id,
        status: TurnStatus::BlockedAuth,
        accepted_message_ref: AcceptedMessageRef::new("message:auth-required").unwrap(),
        source_binding_ref: SourceBindingRef::new("source:auth-required").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply:auth-required").unwrap(),
        resolved_run_profile_id: RunProfileId::default_profile(),
        resolved_run_profile_version: RunProfileVersion::new(1),
        resolved_model_route: None,
        received_at: chrono::Utc::now(),
        checkpoint_id: None,
        gate_ref: Some(GateRef::new("gate:auth-required").unwrap()),
        failure: None,
        event_cursor: cursor,
    }
}
