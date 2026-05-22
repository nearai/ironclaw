use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_event_projections::{
    EventProjectionService, ProjectionCursor as EventProjectionCursor, ProjectionReplay,
    ProjectionScope as EventProjectionScope, ProjectionSnapshot, ReplayEventProjectionService,
    RunProjectionStatus, RunStatusProjection,
};
use ironclaw_event_streams::{
    AllowAllProjectionAccessPolicy, EventStreamManager, InMemoryProjectionStreamAdmissionPolicy,
    InMemoryProjectionUpdateSource, NoExposureProjectionRedactionValidator,
    ProjectionStreamError as EventProjectionStreamError, ProjectionStreamItem,
    ProjectionSubscribeRequest, ProjectionTarget, ProjectionViewClass, SubscriberCapabilities,
};
use ironclaw_events::{DurableEventLog, EventStreamKey, ReadScope};
use ironclaw_outbound::InMemoryOutboundStateStore;
use ironclaw_product_adapters::{
    AdapterInstallationId, ExternalActorRef, ExternalConversationRef, ProductAdapterError,
    ProductAdapterId, ProductOutboundEnvelope, ProductOutboundPayload, ProductOutboundTarget,
    ProductProjectionItem, ProductProjectionState, ProductWorkflowRejectionKind,
    ProjectionCursor as ProductProjectionCursor, ProjectionStream, ProjectionSubscriptionRequest,
    RedactedString,
};
use ironclaw_turns::{ReplyTargetBindingRef, TurnActor, TurnRunId, TurnScope};

const WEBUI_PROJECTION_PAGE_LIMIT: usize = 256;
const WEBUI_PROJECTION_ADAPTER_ID: &str = "webui_v2";
const WEBUI_PROJECTION_INSTALLATION_ID: &str = "webui_v2.local";

#[derive(Clone)]
pub(crate) struct RebornProjectionServices {
    event_stream_manager: Arc<EventStreamManager>,
    webui_reply_target_binding_ref: ReplyTargetBindingRef,
}

impl RebornProjectionServices {
    pub(crate) fn webui_event_stream(&self) -> Arc<dyn ProjectionStream> {
        Arc::new(WebuiRunStatusProjectionStream {
            manager: Arc::clone(&self.event_stream_manager),
            reply_target_binding_ref: self.webui_reply_target_binding_ref.clone(),
        })
    }
}

pub(crate) fn build_reborn_projection_services(
    event_log: Arc<dyn DurableEventLog>,
    webui_reply_target_binding_ref: ReplyTargetBindingRef,
) -> RebornProjectionServices {
    let projection: Arc<dyn EventProjectionService> =
        Arc::new(ReplayEventProjectionService::from_runtime_log(event_log));
    let event_stream_manager = Arc::new(EventStreamManager::from_services(
        projection,
        Arc::new(AllowAllProjectionAccessPolicy),
        Arc::new(InMemoryProjectionStreamAdmissionPolicy::default()),
        Arc::new(InMemoryProjectionUpdateSource::new(128)),
        Arc::new(NoExposureProjectionRedactionValidator),
        Arc::new(InMemoryOutboundStateStore::default()),
    ));
    RebornProjectionServices {
        event_stream_manager,
        webui_reply_target_binding_ref,
    }
}

/// WebUI bridge over the shared EventStreamManager.
///
/// This intentionally exposes the current run-status slice of the product
/// thread projection. Timeline content stays behind the WebUI timeline facade
/// until the browser event schema grows a first-class timeline-entry mapper.
struct WebuiRunStatusProjectionStream {
    manager: Arc<EventStreamManager>,
    reply_target_binding_ref: ReplyTargetBindingRef,
}

#[async_trait]
impl ProjectionStream for WebuiRunStatusProjectionStream {
    async fn drain(
        &self,
        request: ProjectionSubscriptionRequest,
    ) -> Result<Vec<ProductOutboundEnvelope>, ProductAdapterError> {
        let projection_scope = runtime_projection_scope(&request.actor, &request.scope);
        let after_cursor = request
            .after_cursor
            .map(|cursor| parse_event_projection_cursor(cursor.as_str()))
            .transpose()?;
        let mut subscription = self
            .manager
            .subscribe(ProjectionSubscribeRequest {
                actor: request.actor.clone(),
                scope: projection_scope,
                view: ProjectionViewClass::ProductThread,
                target: ProjectionTarget::Thread {
                    thread_id: request.scope.thread_id.clone(),
                },
                after_cursor,
                limit: WEBUI_PROJECTION_PAGE_LIMIT,
                capabilities: SubscriberCapabilities::default(),
            })
            .await
            .map_err(map_event_stream_error)?;

        match subscription.next().await {
            Some(item) => item_to_outbound(
                item,
                &request.scope,
                &request.actor,
                &self.reply_target_binding_ref,
            )
            .map(|item| item.into_iter().collect()),
            None => Ok(Vec::new()),
        }
    }
}

fn runtime_projection_scope(actor: &TurnActor, scope: &TurnScope) -> EventProjectionScope {
    EventProjectionScope {
        stream: EventStreamKey::new(
            scope.tenant_id.clone(),
            actor.user_id.clone(),
            scope.agent_id.clone(),
        ),
        read_scope: ReadScope {
            project_id: scope.project_id.clone(),
            mission_id: None,
            thread_id: Some(scope.thread_id.clone()),
            process_id: None,
        },
    }
}

fn parse_event_projection_cursor(
    cursor: &str,
) -> Result<EventProjectionCursor, ProductAdapterError> {
    serde_json::from_str(cursor).map_err(|_| ProductAdapterError::InvalidIdentifier {
        kind: "projection_cursor",
        reason: "must be a WebUI projection cursor".to_string(),
    })
}

fn item_to_outbound(
    item: ProjectionStreamItem,
    scope: &TurnScope,
    actor: &TurnActor,
    reply_target_binding_ref: &ReplyTargetBindingRef,
) -> Result<Option<ProductOutboundEnvelope>, ProductAdapterError> {
    match item {
        ProjectionStreamItem::Snapshot(envelope) => envelope_to_outbound(
            envelope.cursor(),
            snapshot_state(scope, snapshot_from_envelope(envelope)?)?
                .map(|state| ProductOutboundPayload::ProjectionSnapshot { state }),
            scope,
            actor,
            reply_target_binding_ref,
        ),
        ProjectionStreamItem::Update(envelope) => envelope_to_outbound(
            envelope.cursor(),
            replay_state(scope, replay_from_envelope(envelope.as_ref())?)?
                .map(|state| ProductOutboundPayload::ProjectionUpdate { state }),
            scope,
            actor,
            reply_target_binding_ref,
        ),
        ProjectionStreamItem::RebaseRequired { snapshot, .. } => envelope_to_outbound(
            snapshot.cursor(),
            snapshot_state(scope, snapshot_from_envelope(*snapshot)?)?
                .map(|state| ProductOutboundPayload::ProjectionSnapshot { state }),
            scope,
            actor,
            reply_target_binding_ref,
        ),
        ProjectionStreamItem::Lagged { .. } => Err(ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::Unavailable,
            status_code: 503,
            retryable: true,
            reason: RedactedString::new("projection stream lagged; reconnect from origin"),
        }),
        ProjectionStreamItem::KeepAlive => Ok(None),
    }
}

fn snapshot_from_envelope(
    envelope: ironclaw_event_streams::ProductProjectionEnvelope,
) -> Result<ProjectionSnapshot, ProductAdapterError> {
    match envelope {
        ironclaw_event_streams::ProductProjectionEnvelope::ThreadSnapshot(snapshot) => Ok(snapshot),
        _ => Err(internal_projection_error(
            "unexpected projection snapshot envelope",
        )),
    }
}

fn replay_from_envelope(
    envelope: &ironclaw_event_streams::ProductProjectionEnvelope,
) -> Result<&ProjectionReplay, ProductAdapterError> {
    match envelope {
        ironclaw_event_streams::ProductProjectionEnvelope::ThreadUpdates(replay) => Ok(replay),
        _ => Err(internal_projection_error(
            "unexpected projection update envelope",
        )),
    }
}

fn snapshot_state(
    scope: &TurnScope,
    snapshot: ProjectionSnapshot,
) -> Result<Option<ProductProjectionState>, ProductAdapterError> {
    run_status_projection_state(scope, snapshot.runs)
}

fn replay_state(
    scope: &TurnScope,
    replay: &ProjectionReplay,
) -> Result<Option<ProductProjectionState>, ProductAdapterError> {
    run_status_projection_state(scope, replay.runs.clone())
}

fn run_status_projection_state(
    scope: &TurnScope,
    runs: Vec<RunStatusProjection>,
) -> Result<Option<ProductProjectionState>, ProductAdapterError> {
    let items = runs
        .into_iter()
        .map(|run| ProductProjectionItem::RunStatus {
            run_id: TurnRunId::from_uuid(run.invocation_id.as_uuid()),
            status: run_status_wire(run.status).to_string(),
        })
        .collect::<Vec<_>>();
    if items.is_empty() {
        return Ok(None);
    }
    ProductProjectionState::new(scope.thread_id.to_string(), items).map(Some)
}

fn envelope_to_outbound(
    cursor: EventProjectionCursor,
    payload: Option<ProductOutboundPayload>,
    scope: &TurnScope,
    actor: &TurnActor,
    reply_target_binding_ref: &ReplyTargetBindingRef,
) -> Result<Option<ProductOutboundEnvelope>, ProductAdapterError> {
    let Some(payload) = payload else {
        return Ok(None);
    };
    let projection_cursor = ProductProjectionCursor::new(
        serde_json::to_string(&cursor).map_err(|_| internal_projection_error("cursor encode"))?,
    )?;
    let adapter_id = ProductAdapterId::new(WEBUI_PROJECTION_ADAPTER_ID)?;
    let installation_id = AdapterInstallationId::new(WEBUI_PROJECTION_INSTALLATION_ID)?;
    let target = ProductOutboundTarget::new(
        reply_target_binding_ref.clone(),
        ExternalConversationRef::new(None, scope.thread_id.to_string(), None, None)?,
        Some(ExternalActorRef::new(
            "webui",
            actor.user_id.as_str(),
            None::<String>,
        )?),
    );
    Ok(Some(ProductOutboundEnvelope::new(
        adapter_id,
        installation_id,
        target,
        projection_cursor,
        payload,
    )))
}

fn run_status_wire(status: RunProjectionStatus) -> &'static str {
    match status {
        RunProjectionStatus::Running => "running",
        RunProjectionStatus::Completed => "completed",
        RunProjectionStatus::Cancelled => "cancelled",
        RunProjectionStatus::Failed => "failed",
        RunProjectionStatus::Killed => "killed",
    }
}

fn map_event_stream_error(error: EventProjectionStreamError) -> ProductAdapterError {
    match error {
        EventProjectionStreamError::InvalidRequest { reason } => {
            ProductAdapterError::InvalidIdentifier {
                kind: "projection_stream_request",
                reason: reason.to_string(),
            }
        }
        EventProjectionStreamError::AccessDenied => ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::Unauthorized,
            status_code: 403,
            retryable: false,
            reason: RedactedString::new("projection stream access denied"),
        },
        EventProjectionStreamError::AdmissionDenied => ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::Unavailable,
            status_code: 429,
            retryable: true,
            reason: RedactedString::new("projection stream admission denied"),
        },
        EventProjectionStreamError::Source => ProductAdapterError::WorkflowTransient {
            reason: RedactedString::new("projection stream source failed"),
        },
        EventProjectionStreamError::Redaction | EventProjectionStreamError::Outbound => {
            ProductAdapterError::Internal {
                detail: RedactedString::new("projection stream validation failed"),
            }
        }
    }
}

fn internal_projection_error(detail: &'static str) -> ProductAdapterError {
    ProductAdapterError::Internal {
        detail: RedactedString::new(detail),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use ironclaw_events::{InMemoryDurableEventLog, RuntimeEvent};
    use ironclaw_host_api::{
        AgentId, CapabilityId, InvocationId, ResourceScope, TenantId, ThreadId, UserId,
    };
    use ironclaw_product_adapters::{ProductOutboundEnvelope, ProductOutboundPayload};

    #[tokio::test]
    async fn webui_event_stream_drains_run_status_projection_from_event_stream_manager() {
        let tenant_id = TenantId::new("webui-events-tenant").unwrap();
        let user_id = UserId::new("webui-events-user").unwrap();
        let agent_id = AgentId::new("webui-events-agent").unwrap();
        let thread_id = ThreadId::new("webui-events-thread").unwrap();
        let invocation_id = InvocationId::new();
        let event_log = Arc::new(InMemoryDurableEventLog::new());
        event_log
            .append(RuntimeEvent::model_started(
                ResourceScope {
                    tenant_id: tenant_id.clone(),
                    user_id: user_id.clone(),
                    agent_id: Some(agent_id.clone()),
                    project_id: None,
                    mission_id: None,
                    thread_id: Some(thread_id.clone()),
                    invocation_id,
                },
                CapabilityId::new("loop.model").unwrap(),
            ))
            .await
            .unwrap();

        let event_log: Arc<dyn DurableEventLog> = event_log;
        let actor = TurnActor::new(user_id);
        let services = build_reborn_projection_services(
            event_log,
            ReplyTargetBindingRef::new("webui-events-reply").unwrap(),
        );
        let events = services
            .webui_event_stream()
            .drain(ProjectionSubscriptionRequest {
                actor,
                scope: TurnScope::new(tenant_id, Some(agent_id), None, thread_id),
                after_cursor: None,
            })
            .await
            .unwrap();

        assert_eq!(events.len(), 1);
        let ProductOutboundPayload::ProjectionSnapshot { state } = events[0].payload() else {
            panic!("expected projection snapshot");
        };
        assert_eq!(state.items.len(), 1);
        assert!(matches!(
            state.items[0],
            ProductProjectionItem::RunStatus { ref status, .. } if status == "running"
        ));
    }

    async fn webui_event_stream_resumes_after_serialized_projection_cursor() {
        let tenant_id = TenantId::new("webui-events-tenant").unwrap();
        let user_id = UserId::new("webui-events-user").unwrap();
        let agent_id = AgentId::new("webui-events-agent").unwrap();
        let thread_id = ThreadId::new("webui-events-thread").unwrap();
        let first_run = InvocationId::new();
        let second_run = InvocationId::new();
        let event_log = Arc::new(InMemoryDurableEventLog::new());
        event_log
            .append(RuntimeEvent::model_started(
                resource_scope(&tenant_id, &user_id, &agent_id, &thread_id, first_run),
                CapabilityId::new("loop.model").unwrap(),
            ))
            .await
            .unwrap();

        let event_log_dyn: Arc<dyn DurableEventLog> = event_log.clone();
        let actor = TurnActor::new(user_id.clone());
        let services = build_reborn_projection_services(
            event_log_dyn,
            ReplyTargetBindingRef::new("webui-events-reply").unwrap(),
        );
        let first = services
            .webui_event_stream()
            .drain(ProjectionSubscriptionRequest {
                actor: actor.clone(),
                scope: TurnScope::new(
                    tenant_id.clone(),
                    Some(agent_id.clone()),
                    None,
                    thread_id.clone(),
                ),
                after_cursor: None,
            })
            .await
            .unwrap();

        event_log
            .append(RuntimeEvent::model_started(
                resource_scope(&tenant_id, &user_id, &agent_id, &thread_id, second_run),
                CapabilityId::new("loop.model").unwrap(),
            ))
            .await
            .unwrap();
        let resumed = services
            .webui_event_stream()
            .drain(ProjectionSubscriptionRequest {
                actor,
                scope: TurnScope::new(tenant_id, Some(agent_id), None, thread_id),
                after_cursor: Some(first[0].projection_cursor().clone()),
            })
            .await
            .unwrap();

        assert!(contains_run_status(&resumed, second_run, "running"));
        assert!(!contains_run_status(&resumed, first_run, "running"));
    }

    #[tokio::test]
    async fn webui_event_stream_uses_request_actor_for_projection_scope() {
        let tenant_id = TenantId::new("webui-events-tenant").unwrap();
        let owner_user_id = UserId::new("webui-events-owner").unwrap();
        let other_user_id = UserId::new("webui-events-other").unwrap();
        let agent_id = AgentId::new("webui-events-agent").unwrap();
        let thread_id = ThreadId::new("webui-events-thread").unwrap();
        let event_log = Arc::new(InMemoryDurableEventLog::new());
        event_log
            .append(RuntimeEvent::model_started(
                resource_scope(
                    &tenant_id,
                    &owner_user_id,
                    &agent_id,
                    &thread_id,
                    InvocationId::new(),
                ),
                CapabilityId::new("loop.model").unwrap(),
            ))
            .await
            .unwrap();

        let event_log: Arc<dyn DurableEventLog> = event_log;
        let services = build_reborn_projection_services(
            event_log,
            ReplyTargetBindingRef::new("webui-events-reply").unwrap(),
        );
        let events = services
            .webui_event_stream()
            .drain(ProjectionSubscriptionRequest {
                actor: TurnActor::new(other_user_id),
                scope: TurnScope::new(tenant_id, Some(agent_id), None, thread_id),
                after_cursor: None,
            })
            .await
            .unwrap();

        assert!(
            events.is_empty(),
            "projection stream must not read another user's event stream through a hidden runtime actor"
        );
    }

    #[tokio::test]
    async fn webui_event_stream_rejects_malformed_projection_cursor() {
        let tenant_id = TenantId::new("webui-events-tenant").unwrap();
        let user_id = UserId::new("webui-events-user").unwrap();
        let agent_id = AgentId::new("webui-events-agent").unwrap();
        let thread_id = ThreadId::new("webui-events-thread").unwrap();
        let event_log: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
        let actor = TurnActor::new(user_id);
        let services = build_reborn_projection_services(
            event_log,
            ReplyTargetBindingRef::new("webui-events-reply").unwrap(),
        );

        let error = services
            .webui_event_stream()
            .drain(ProjectionSubscriptionRequest {
                actor,
                scope: TurnScope::new(tenant_id, Some(agent_id), None, thread_id),
                after_cursor: Some(ProductProjectionCursor::new("not-json").unwrap()),
            })
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            ProductAdapterError::InvalidIdentifier {
                kind: "projection_cursor",
                ..
            }
        ));
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
            | ProductOutboundPayload::ProjectionUpdate { state } => {
                state.items.iter().any(|item| {
                    matches!(
                        item,
                        ProductProjectionItem::RunStatus { run_id, status }
                            if *run_id == expected_run_id && status == expected_status
                    )
                })
            }
            _ => false,
        })
    }
}
