use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_event_projections::{
    ProjectionCursor as EventProjectionCursor, ProjectionReplay,
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
use ironclaw_product_workflow::{
    RebornServices as ProductRebornServices, RebornServicesApi, RebornServicesError,
    RebornServicesErrorCode, RebornServicesErrorKind,
};
use ironclaw_turns::{ReplyTargetBindingRef, TurnActor, TurnRunId, TurnScope};

use crate::{RebornBuildError, RebornReadiness, RebornRuntime};

const WEBUI_PROJECTION_PAGE_LIMIT: usize = 256;
const WEBUI_PROJECTION_ADAPTER_ID: &str = "webui_v2";
const WEBUI_PROJECTION_INSTALLATION_ID: &str = "webui_v2.local";

/// WebUI-facing Reborn service bundle for host composition.
///
/// This bundle deliberately exposes only the product facade consumed by
/// WebChat v2 routes. HTTP routing, auth middleware, static assets, and
/// SSE transport stay in the WebUI crate (or, when the `webui-v2-beta`
/// feature is on, the [`crate::webui_serve`] module in this crate);
/// lower runtime handles stay behind the existing Reborn runtime /
/// composition services.
#[derive(Clone)]
pub struct RebornWebuiBundle {
    pub api: Arc<dyn RebornServicesApi>,
    pub readiness: RebornReadiness,
}

impl std::fmt::Debug for RebornWebuiBundle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RebornWebuiBundle")
            .field("api", &"Arc<dyn RebornServicesApi>")
            .field("readiness", &self.readiness)
            .finish()
    }
}

/// Compose the WebUI-facing product facade from an already-built Reborn runtime.
///
/// This function does not create a second turn coordinator, thread service,
/// host runtime or route server. It reuses the runtime's existing task-level
/// composition and attaches the runtime-owned projection stream unless the
/// caller supplies a custom stream.
pub fn build_webui_services(
    runtime: &RebornRuntime,
    event_stream: Option<Arc<dyn ProjectionStream>>,
) -> Result<RebornWebuiBundle, RebornBuildError> {
    let services = runtime.services();

    let mut api = ProductRebornServices::new(
        runtime.webui_thread_service(),
        runtime.webui_turn_coordinator(),
    );
    if let Some(skill_activation_source) = runtime.webui_skill_activation_source() {
        let activation_recorder = Arc::clone(&skill_activation_source);
        let activation_clearer = skill_activation_source;
        api = api.with_skill_activation_hooks(
            move |scope, accepted_message_ref, message| {
                activation_recorder
                    .record_user_message(scope.clone(), accepted_message_ref.clone(), message)
                    .map_err(|_| RebornServicesError {
                        code: RebornServicesErrorCode::Internal,
                        kind: RebornServicesErrorKind::Internal,
                        status_code: 500,
                        retryable: false,
                        field: None,
                        validation_code: None,
                    })
            },
            move |scope, accepted_message_ref| {
                activation_clearer
                    .clear_accepted_message(scope, accepted_message_ref)
                    .map_err(|_| RebornServicesError {
                        code: RebornServicesErrorCode::Internal,
                        kind: RebornServicesErrorKind::Internal,
                        status_code: 500,
                        retryable: false,
                        field: None,
                        validation_code: None,
                    })
            },
        );
    }
    api = api.with_event_stream(event_stream.unwrap_or_else(|| runtime.webui_event_stream()));

    Ok(RebornWebuiBundle {
        api: Arc::new(api),
        readiness: services.readiness,
    })
}

pub(crate) fn build_webui_event_stream(
    event_log: Arc<dyn DurableEventLog>,
    stream_actor: TurnActor,
    reply_target_binding_ref: ReplyTargetBindingRef,
) -> Arc<dyn ProjectionStream> {
    let projection: Arc<dyn ironclaw_event_projections::EventProjectionService> =
        Arc::new(ReplayEventProjectionService::from_runtime_log(event_log));
    let manager = Arc::new(EventStreamManager::from_services(
        projection,
        Arc::new(AllowAllProjectionAccessPolicy),
        Arc::new(InMemoryProjectionStreamAdmissionPolicy::default()),
        Arc::new(InMemoryProjectionUpdateSource::new(128)),
        Arc::new(NoExposureProjectionRedactionValidator),
        Arc::new(InMemoryOutboundStateStore::default()),
    ));
    Arc::new(WebuiEventProjectionStream {
        manager,
        stream_actor,
        reply_target_binding_ref,
    })
}

struct WebuiEventProjectionStream {
    manager: Arc<EventStreamManager>,
    stream_actor: TurnActor,
    reply_target_binding_ref: ReplyTargetBindingRef,
}

#[async_trait]
impl ProjectionStream for WebuiEventProjectionStream {
    async fn drain(
        &self,
        request: ProjectionSubscriptionRequest,
    ) -> Result<Vec<ProductOutboundEnvelope>, ProductAdapterError> {
        let projection_scope = projection_scope_for_turn_scope(&self.stream_actor, &request.scope);
        let after_cursor = request
            .after_cursor
            .map(|cursor| parse_event_projection_cursor(cursor.as_str()))
            .transpose()?;
        let mut subscription = self
            .manager
            .subscribe(ProjectionSubscribeRequest {
                actor: self.stream_actor.clone(),
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

fn projection_scope_for_turn_scope(actor: &TurnActor, scope: &TurnScope) -> EventProjectionScope {
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
    projection_state(scope, snapshot.runs)
}

fn replay_state(
    scope: &TurnScope,
    replay: &ProjectionReplay,
) -> Result<Option<ProductProjectionState>, ProductAdapterError> {
    projection_state(scope, replay.runs.clone())
}

fn projection_state(
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
    use ironclaw_product_adapters::ProductOutboundPayload;

    #[tokio::test]
    async fn webui_event_stream_drains_event_stream_manager_projection() {
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
        let stream = build_webui_event_stream(
            event_log,
            actor.clone(),
            ReplyTargetBindingRef::new("webui-events-reply").unwrap(),
        );
        let events = stream
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
}
