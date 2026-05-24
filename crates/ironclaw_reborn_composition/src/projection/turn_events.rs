use std::sync::Arc;

use ironclaw_product_adapters::{
    AuthPromptView, GatePromptView, ProductAdapterError, ProductOutboundPayload,
    ProductProjectionItem, ProductProjectionState, ProductWorkflowRejectionKind, RedactedString,
};
use ironclaw_turns::{
    GetRunStateRequest, TurnCoordinator, TurnError, TurnEventKind, TurnEventProjectionCursor,
    TurnEventProjectionSource, TurnLifecycleEvent, TurnScope, TurnStatus,
};

const WEBUI_TURN_EVENT_PAGE_LIMIT: usize = 256;

pub(super) struct TurnEventPayload {
    pub(super) cursor: TurnEventProjectionCursor,
    pub(super) payload: ProductOutboundPayload,
}

pub(super) struct TurnEventDrain {
    pub(super) next_cursor: Option<TurnEventProjectionCursor>,
    pub(super) payloads: Vec<TurnEventPayload>,
}

#[derive(Clone, Default)]
pub(super) enum TurnEventBridge {
    #[default]
    Disabled,
    Enabled {
        source: Arc<dyn TurnEventProjectionSource>,
        coordinator: Arc<dyn TurnCoordinator>,
    },
}

impl TurnEventBridge {
    pub(super) fn enabled(
        source: Arc<dyn TurnEventProjectionSource>,
        coordinator: Arc<dyn TurnCoordinator>,
    ) -> Self {
        Self::Enabled {
            source,
            coordinator,
        }
    }

    pub(super) async fn drain(
        &self,
        scope: &TurnScope,
        after: Option<TurnEventProjectionCursor>,
    ) -> Result<TurnEventDrain, ProductAdapterError> {
        let Self::Enabled {
            source,
            coordinator,
        } = self
        else {
            return Ok(TurnEventDrain {
                next_cursor: after,
                payloads: Vec::new(),
            });
        };
        let mut after_event = match after.as_ref() {
            Some(cursor) if &cursor.scope == scope => Some(cursor.event),
            Some(_) => {
                return Err(ProductAdapterError::InvalidIdentifier {
                    kind: "projection_cursor",
                    reason: "turn cursor scope does not match subscription scope".to_string(),
                });
            }
            None => None,
        };
        let mut payloads = Vec::new();
        let mut next_cursor;
        loop {
            let page = source
                .read_turn_events_after(scope, after_event, WEBUI_TURN_EVENT_PAGE_LIMIT)
                .await
                .map_err(|_| ProductAdapterError::WorkflowRejected {
                    kind: ProductWorkflowRejectionKind::Unavailable,
                    status_code: 503,
                    retryable: true,
                    reason: RedactedString::new("turn event projection source unavailable"),
                })?;
            if page.rebase_required.is_some() {
                return Err(ProductAdapterError::WorkflowRejected {
                    kind: ProductWorkflowRejectionKind::Unavailable,
                    status_code: 503,
                    retryable: true,
                    reason: RedactedString::new("turn event projection rebase required; reconnect"),
                });
            }
            let next_event = page.next_cursor;
            next_cursor = Some(TurnEventProjectionCursor::for_scope(
                scope.clone(),
                next_event,
            ));
            for event in page.entries {
                if let Some(payload) =
                    turn_event_payload(coordinator.as_ref(), event.clone(), scope).await?
                {
                    payloads.push(TurnEventPayload {
                        cursor: TurnEventProjectionCursor::for_scope(scope.clone(), event.cursor),
                        payload,
                    });
                }
            }
            if !payloads.is_empty() || !page.truncated || after_event == Some(next_event) {
                break;
            }
            after_event = Some(next_event);
        }
        Ok(TurnEventDrain {
            next_cursor,
            payloads,
        })
    }
}

async fn turn_event_payload(
    coordinator: &dyn TurnCoordinator,
    event: TurnLifecycleEvent,
    scope: &TurnScope,
) -> Result<Option<ProductOutboundPayload>, ProductAdapterError> {
    if matches!(event.kind, TurnEventKind::Blocked)
        && let Some(prompt) = blocked_prompt_payload(coordinator, &event, scope).await?
    {
        return Ok(Some(prompt));
    }
    if projects_run_status(&event.kind) {
        return Ok(Some(ProductOutboundPayload::ProjectionUpdate {
            state: turn_event_projection_state(scope, &event)?,
        }));
    }
    Ok(None)
}

async fn blocked_prompt_payload(
    coordinator: &dyn TurnCoordinator,
    event: &TurnLifecycleEvent,
    scope: &TurnScope,
) -> Result<Option<ProductOutboundPayload>, ProductAdapterError> {
    let state = match coordinator
        .get_run_state(GetRunStateRequest {
            scope: scope.clone(),
            run_id: event.run_id,
        })
        .await
    {
        Ok(state) => state,
        Err(TurnError::ScopeNotFound) => return Ok(None),
        Err(_) => {
            return Err(ProductAdapterError::WorkflowTransient {
                reason: RedactedString::new("turn gate state lookup failed"),
            });
        }
    };
    if state.status != event.status || state.event_cursor != event.cursor {
        return Ok(None);
    }
    let Some(gate_ref) = state.gate_ref else {
        return Ok(None);
    };
    let gate_ref = gate_ref.as_str().to_string();
    match event.status {
        TurnStatus::BlockedAuth => Ok(Some(ProductOutboundPayload::AuthPrompt(AuthPromptView {
            turn_run_id: event.run_id,
            auth_request_ref: gate_ref,
            headline: "Authentication required".to_string(),
            body: event
                .sanitized_reason
                .clone()
                .unwrap_or_else(|| "Authenticate to continue this run.".to_string()),
        }))),
        TurnStatus::BlockedApproval | TurnStatus::BlockedResource => {
            Ok(Some(ProductOutboundPayload::GatePrompt(GatePromptView {
                turn_run_id: event.run_id,
                gate_ref,
                headline: match event.status {
                    TurnStatus::BlockedApproval => "Approval required",
                    TurnStatus::BlockedResource => "Resource unavailable",
                    _ => unreachable!(),
                }
                .to_string(),
                body: event
                    .sanitized_reason
                    .clone()
                    .unwrap_or_else(|| "Resolve this gate to continue the run.".to_string()),
            })))
        }
        _ => Ok(None),
    }
}

fn projects_run_status(kind: &TurnEventKind) -> bool {
    matches!(
        kind,
        &TurnEventKind::Submitted
            | &TurnEventKind::Resumed
            | &TurnEventKind::RunnerClaimed
            | &TurnEventKind::RecoveryRequired
            | &TurnEventKind::Blocked
            | &TurnEventKind::CancelRequested
            | &TurnEventKind::Cancelled
            | &TurnEventKind::Completed
            | &TurnEventKind::Failed
    )
}

fn turn_event_projection_state(
    scope: &TurnScope,
    event: &TurnLifecycleEvent,
) -> Result<ProductProjectionState, ProductAdapterError> {
    ProductProjectionState::new(
        scope.thread_id.to_string(),
        vec![ProductProjectionItem::RunStatus {
            run_id: event.run_id,
            status: turn_status_wire(event.status).to_string(),
        }],
    )
}

fn turn_status_wire(status: TurnStatus) -> &'static str {
    match status {
        TurnStatus::Queued => "queued",
        TurnStatus::Running => "running",
        TurnStatus::BlockedApproval => "blocked_approval",
        TurnStatus::BlockedAuth => "blocked_auth",
        TurnStatus::BlockedResource => "blocked_resource",
        TurnStatus::RecoveryRequired => "recovery_required",
        TurnStatus::CancelRequested => "cancel_requested",
        TurnStatus::Completed => "completed",
        TurnStatus::Cancelled => "cancelled",
        TurnStatus::Failed => "failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use async_trait::async_trait;
    use ironclaw_host_api::{AgentId, TenantId, ThreadId, UserId};
    use ironclaw_turns::{
        AcceptedMessageRef, CancelRunRequest, CancelRunResponse, GateRef, ReplyTargetBindingRef,
        ResumeTurnRequest, ResumeTurnResponse, RunProfileId, RunProfileVersion, SourceBindingRef,
        SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnEventPage, TurnId, TurnRunId,
        TurnRunState, events::EventCursor as TurnEventCursor,
    };

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

        async fn cancel_run(
            &self,
            _request: CancelRunRequest,
        ) -> Result<CancelRunResponse, TurnError> {
            unreachable!("projection tests only read run state")
        }

        async fn get_run_state(
            &self,
            request: GetRunStateRequest,
        ) -> Result<TurnRunState, TurnError> {
            if request.scope == self.state.scope && request.run_id == self.state.run_id {
                Ok(self.state.clone())
            } else {
                Err(TurnError::ScopeNotFound)
            }
        }
    }

    #[tokio::test]
    async fn bridge_projects_blocked_auth_turn_event_as_prompt() {
        let user_id = UserId::new("webui-events-user").unwrap();
        let scope = TurnScope::new(
            TenantId::new("webui-events-tenant").unwrap(),
            Some(AgentId::new("webui-events-agent").unwrap()),
            None,
            ThreadId::new("webui-events-thread").unwrap(),
        );
        let run_id = TurnRunId::new();
        let bridge = TurnEventBridge::enabled(
            Arc::new(FakeTurnEventSource {
                events: vec![TurnLifecycleEvent {
                    cursor: TurnEventCursor(1),
                    scope: scope.clone(),
                    run_id,
                    status: TurnStatus::BlockedAuth,
                    kind: TurnEventKind::Blocked,
                    sanitized_reason: Some("GitHub authentication required".to_string()),
                }],
            }),
            Arc::new(FakeTurnCoordinator {
                state: TurnRunState {
                    scope: scope.clone(),
                    actor: Some(TurnActor::new(user_id)),
                    turn_id: TurnId::new(),
                    run_id,
                    status: TurnStatus::BlockedAuth,
                    accepted_message_ref: AcceptedMessageRef::new("message:auth-required").unwrap(),
                    source_binding_ref: SourceBindingRef::new("source:auth-required").unwrap(),
                    reply_target_binding_ref: ReplyTargetBindingRef::new("reply:auth-required")
                        .unwrap(),
                    resolved_run_profile_id: RunProfileId::default_profile(),
                    resolved_run_profile_version: RunProfileVersion::new(1),
                    resolved_model_route: None,
                    received_at: chrono::Utc::now(),
                    checkpoint_id: None,
                    gate_ref: Some(GateRef::new("gate:auth-required").unwrap()),
                    failure: None,
                    event_cursor: TurnEventCursor(1),
                },
            }),
        );
        let drain = bridge.drain(&scope, None).await.unwrap();
        let payloads = drain.payloads;

        assert!(payloads.iter().any(|payload| match payload {
            TurnEventPayload {
                cursor,
                payload: ProductOutboundPayload::AuthPrompt(prompt),
            } =>
                cursor.event == TurnEventCursor(1)
                    && cursor.scope == scope
                    && prompt.turn_run_id == run_id
                    && prompt.auth_request_ref == "gate:auth-required"
                    && prompt.body == "GitHub authentication required",
            _ => false,
        }));
        assert_eq!(payloads.len(), 1);
    }

    #[tokio::test]
    async fn bridge_reads_past_filtered_heartbeat_pages() {
        let user_id = UserId::new("webui-events-user").unwrap();
        let scope = TurnScope::new(
            TenantId::new("webui-events-tenant").unwrap(),
            Some(AgentId::new("webui-events-agent").unwrap()),
            None,
            ThreadId::new("webui-events-thread").unwrap(),
        );
        let run_id = TurnRunId::new();
        let mut events = (1..=WEBUI_TURN_EVENT_PAGE_LIMIT as u64)
            .map(|cursor| TurnLifecycleEvent {
                cursor: TurnEventCursor(cursor),
                scope: scope.clone(),
                run_id,
                status: TurnStatus::Running,
                kind: TurnEventKind::RunnerHeartbeat,
                sanitized_reason: None,
            })
            .collect::<Vec<_>>();
        events.push(TurnLifecycleEvent {
            cursor: TurnEventCursor(WEBUI_TURN_EVENT_PAGE_LIMIT as u64 + 1),
            scope: scope.clone(),
            run_id,
            status: TurnStatus::BlockedAuth,
            kind: TurnEventKind::Blocked,
            sanitized_reason: Some("GitHub authentication required".to_string()),
        });
        let bridge = TurnEventBridge::enabled(
            Arc::new(FakeTurnEventSource { events }),
            Arc::new(FakeTurnCoordinator {
                state: TurnRunState {
                    scope: scope.clone(),
                    actor: Some(TurnActor::new(user_id)),
                    turn_id: TurnId::new(),
                    run_id,
                    status: TurnStatus::BlockedAuth,
                    accepted_message_ref: AcceptedMessageRef::new("message:auth-required").unwrap(),
                    source_binding_ref: SourceBindingRef::new("source:auth-required").unwrap(),
                    reply_target_binding_ref: ReplyTargetBindingRef::new("reply:auth-required")
                        .unwrap(),
                    resolved_run_profile_id: RunProfileId::default_profile(),
                    resolved_run_profile_version: RunProfileVersion::new(1),
                    resolved_model_route: None,
                    received_at: chrono::Utc::now(),
                    checkpoint_id: None,
                    gate_ref: Some(GateRef::new("gate:auth-required").unwrap()),
                    failure: None,
                    event_cursor: TurnEventCursor(WEBUI_TURN_EVENT_PAGE_LIMIT as u64 + 1),
                },
            }),
        );
        let drain = bridge.drain(&scope, None).await.unwrap();
        let payloads = drain.payloads;

        assert_eq!(payloads.len(), 1);
        assert_eq!(drain.next_cursor, Some(payloads[0].cursor.clone()));
        assert_eq!(
            payloads[0].cursor.event,
            TurnEventCursor(WEBUI_TURN_EVENT_PAGE_LIMIT as u64 + 1)
        );
        assert!(matches!(
            payloads[0].payload,
            ProductOutboundPayload::AuthPrompt(_)
        ));
    }

    #[tokio::test]
    async fn bridge_does_not_project_prompt_for_stale_blocked_event() {
        let user_id = UserId::new("webui-events-user").unwrap();
        let scope = TurnScope::new(
            TenantId::new("webui-events-tenant").unwrap(),
            Some(AgentId::new("webui-events-agent").unwrap()),
            None,
            ThreadId::new("webui-events-thread").unwrap(),
        );
        let run_id = TurnRunId::new();
        let bridge = TurnEventBridge::enabled(
            Arc::new(FakeTurnEventSource {
                events: vec![TurnLifecycleEvent {
                    cursor: TurnEventCursor(1),
                    scope: scope.clone(),
                    run_id,
                    status: TurnStatus::BlockedAuth,
                    kind: TurnEventKind::Blocked,
                    sanitized_reason: Some("stale auth gate".to_string()),
                }],
            }),
            Arc::new(FakeTurnCoordinator {
                state: TurnRunState {
                    scope: scope.clone(),
                    actor: Some(TurnActor::new(user_id)),
                    turn_id: TurnId::new(),
                    run_id,
                    status: TurnStatus::BlockedAuth,
                    accepted_message_ref: AcceptedMessageRef::new("message:auth-required").unwrap(),
                    source_binding_ref: SourceBindingRef::new("source:auth-required").unwrap(),
                    reply_target_binding_ref: ReplyTargetBindingRef::new("reply:auth-required")
                        .unwrap(),
                    resolved_run_profile_id: RunProfileId::default_profile(),
                    resolved_run_profile_version: RunProfileVersion::new(1),
                    resolved_model_route: None,
                    received_at: chrono::Utc::now(),
                    checkpoint_id: None,
                    gate_ref: Some(GateRef::new("gate:new-auth-required").unwrap()),
                    failure: None,
                    event_cursor: TurnEventCursor(2),
                },
            }),
        );
        let drain = bridge.drain(&scope, None).await.unwrap();
        let payloads = drain.payloads;

        assert_eq!(payloads.len(), 1);
        assert!(matches!(
            payloads[0].payload,
            ProductOutboundPayload::ProjectionUpdate { .. }
        ));
    }
}
