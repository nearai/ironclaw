//! Reborn host composition for OpenAI-compatible API routes.
//!
//! The route crate owns DTOs and HTTP handlers, but the Reborn host owns the
//! authority-bearing wiring: authenticated callers, ProductWorkflow,
//! conversation binding, durable idempotency/ref stores, and projection reads.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_filesystem::{RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    AgentId, InvocationId, MountAlias, MountGrant, MountPermissions, MountView, ProjectId,
    ResourceScope, TenantId, UserId, VirtualPath,
};
use ironclaw_product_adapters::{
    AdapterInstallationId, ProductAdapterId, ProductInboundAck, ProductOutboundEnvelope,
    ProductOutboundPayload, ProductProjectionItem, ProductProjectionState, ProductWorkflow,
    ProjectionCursor, ProjectionReadRequest, ProjectionStream, ProjectionSubscriptionRequest,
};
use ironclaw_product_workflow::{
    DefaultInboundTurnService, DefaultProductWorkflow, ProductActorUserResolutionRequest,
    ProductActorUserResolver, ProductConversationBindingService, ProductInstallationKey,
    ProductInstallationScope, ProductWorkflowError, StaticProductInstallationResolver,
};
use ironclaw_product_workflow_storage::RebornFilesystemIdempotencyLedger;
use ironclaw_reborn_openai_compat::{
    OPENAI_COMPAT_ACTOR_KIND, OPENAI_COMPAT_ADAPTER_ID, OPENAI_COMPAT_INSTALLATION_ID,
    OpenAiChatCompletionProjection, OpenAiChatCompletionProjectionReader,
    OpenAiChatCompletionProjectionRequest, OpenAiChatCompletionsWorkflow,
    OpenAiChatProjectionStreamRequest, OpenAiCompatErrorKind, OpenAiCompatHttpError,
    OpenAiCompatProjectionStreamer, OpenAiCompatRefStore, OpenAiCompatResourceBinding,
    OpenAiCompatRouterState, OpenAiResponseErrorObject, OpenAiResponseObject,
    OpenAiResponseOutputItem, OpenAiResponseOutputItemStatus, OpenAiResponseProjection,
    OpenAiResponseProjectionStreamRequest, OpenAiResponseReadRequest, OpenAiResponseStatus,
    OpenAiResponseWaitRequest, OpenAiResponsesMessageRole, OpenAiResponsesProjectionReader,
    OpenAiResponsesWorkflow, openai_compat_router_with_state, openai_compat_routes,
};
use ironclaw_reborn_openai_compat_storage::FilesystemOpenAiCompatRefStore;
use ironclaw_threads::{
    FinalizedAssistantMessageByRunRequest, SessionThreadError, SessionThreadService, ThreadScope,
};

use crate::RebornBuildError;
use crate::RebornRuntime;
use crate::webui_serve::ProtectedRouteMount;

#[cfg(test)]
mod tests;

const OPENAI_COMPAT_LEDGER_USER_ID: &str = "openai-compat";
const OPENAI_COMPAT_LEDGER_ENGINE_ROOT: &str = "/engine";
const OPENAI_COMPAT_PROJECTION_POLL_INTERVAL: Duration = Duration::from_millis(100);

pub async fn build_openai_compat_route_mount(
    runtime: &RebornRuntime,
    tenant_id: TenantId,
    default_agent_id: AgentId,
    default_project_id: Option<ProjectId>,
) -> Result<ProtectedRouteMount, RebornBuildError> {
    let local_runtime = runtime.services().local_runtime.as_ref().ok_or_else(|| {
        RebornBuildError::InvalidConfig {
            reason: "OpenAI-compatible routes require local runtime services".to_string(),
        }
    })?;
    let conversations = Arc::new(
        local_runtime
            .durable_trigger_conversation_services()
            .await
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("failed to open OpenAI-compatible conversation bindings: {error}"),
            })?,
    );
    let conversation_port: Arc<dyn ironclaw_conversations::ConversationBindingService> =
        conversations.clone();
    let actor_pairings: Arc<dyn ironclaw_conversations::ConversationActorPairingService> =
        conversations.clone();

    let adapter_id = ProductAdapterId::new(OPENAI_COMPAT_ADAPTER_ID)
        .map_err(invalid_openai_compat_config("adapter_id"))?;
    let installation_id = AdapterInstallationId::new(OPENAI_COMPAT_INSTALLATION_ID)
        .map_err(invalid_openai_compat_config("installation_id"))?;
    let installation_scope = ProductInstallationScope::with_default_scope(
        tenant_id.clone(),
        default_agent_id.clone(),
        default_project_id.clone(),
    )
    .with_actor_user_resolver(Arc::new(OpenAiCompatActorUserResolver), actor_pairings);
    let installation_resolver = StaticProductInstallationResolver::new([(
        ProductInstallationKey::new(adapter_id, installation_id),
        installation_scope,
    )]);
    let binding = ProductConversationBindingService::new(conversation_port, installation_resolver);
    let inbound = Arc::new(DefaultInboundTurnService::new(
        binding.clone(),
        runtime.webui_thread_service(),
        runtime.webui_turn_coordinator(),
    ));
    // `.with_delivered_gate_routes` is intentionally omitted here. The
    // OpenAI-compat surface never produces `ApprovalResolution`,
    // `ScopedApprovalResolution`, or `AuthResolution` payloads (verified: no
    // such payload constructions exist in `crates/ironclaw_reborn_openai_compat/`),
    // so the delivered-route conversation-fingerprint fallback is unreachable on
    // this surface. The workflow falls back to the default in-memory no-op store,
    // which is correct for this surface.
    let product_workflow: Arc<dyn ProductWorkflow> = Arc::new(
        DefaultProductWorkflow::new(
            inbound,
            Arc::new(RebornFilesystemIdempotencyLedger::new(
                openai_compat_ledger_filesystem(
                    local_runtime.extension_filesystem.clone(),
                    &tenant_id,
                )?,
                openai_compat_ledger_scope(
                    tenant_id.clone(),
                    default_agent_id.clone(),
                    default_project_id.clone(),
                )?,
            )),
            Arc::new(binding.clone()),
        )
        .with_approval_interaction_service(runtime.webui_approval_interaction_service())
        .with_auth_interaction_service(runtime.webui_auth_interaction_service()),
    );

    let ref_filesystem: Arc<dyn RootFilesystem> = local_runtime.extension_filesystem.clone();
    let ref_store: Arc<dyn OpenAiCompatRefStore> =
        Arc::new(FilesystemOpenAiCompatRefStore::with_root(
            ref_filesystem,
            openai_compat_ref_root(&tenant_id)?,
        ));
    let chat_projection_reader = Arc::new(OpenAiChatCompletionThreadProjectionReader::new(
        runtime.webui_thread_service(),
    ));
    let projection_stream = runtime.webui_event_stream();
    let responses_projection_reader = Arc::new(OpenAiResponsesThreadProjectionReader::new(
        runtime.webui_thread_service(),
        projection_stream.clone(),
    ));
    let projection_streamer = Arc::new(OpenAiCompatRuntimeProjectionStreamer::new(
        projection_stream,
    ));
    let chat_workflow = Arc::new(
        OpenAiChatCompletionsWorkflow::new(
            product_workflow.clone(),
            ref_store.clone(),
            chat_projection_reader,
        )
        .with_projection_streamer(projection_streamer.clone()),
    );
    let responses_workflow = Arc::new(
        OpenAiResponsesWorkflow::new(product_workflow, ref_store, responses_projection_reader)
            .with_projection_streamer(projection_streamer),
    );
    Ok(ProtectedRouteMount::new(
        openai_compat_router_with_state(
            OpenAiCompatRouterState::with_chat_completions(chat_workflow)
                .with_responses_workflow(responses_workflow),
        ),
        openai_compat_routes(),
    ))
}

struct OpenAiCompatRuntimeProjectionStreamer {
    projection_stream: Arc<dyn ProjectionStream>,
}

impl OpenAiCompatRuntimeProjectionStreamer {
    fn new(projection_stream: Arc<dyn ProjectionStream>) -> Self {
        Self { projection_stream }
    }
}

#[async_trait]
impl OpenAiCompatProjectionStreamer for OpenAiCompatRuntimeProjectionStreamer {
    async fn drain_chat(
        &self,
        request: OpenAiChatProjectionStreamRequest,
    ) -> Result<Vec<ProductOutboundEnvelope>, OpenAiCompatHttpError> {
        let mut subscription = request.projection_subscription;
        subscription.after_cursor = request.after_cursor;
        self.projection_stream
            .drain(subscription)
            .await
            .map_err(Into::into)
    }

    async fn drain_response(
        &self,
        request: OpenAiResponseProjectionStreamRequest,
    ) -> Result<Vec<ProductOutboundEnvelope>, OpenAiCompatHttpError> {
        let mut subscription = request.projection_subscription;
        subscription.after_cursor = request.after_cursor;
        self.projection_stream
            .drain(subscription)
            .await
            .map_err(Into::into)
    }
}

#[derive(Debug)]
struct OpenAiCompatActorUserResolver;

#[async_trait]
impl ProductActorUserResolver for OpenAiCompatActorUserResolver {
    async fn resolve_product_actor_user(
        &self,
        request: ProductActorUserResolutionRequest,
    ) -> Result<Option<UserId>, ProductWorkflowError> {
        if request.adapter_id.as_str() != OPENAI_COMPAT_ADAPTER_ID
            || request.installation_id.as_str() != OPENAI_COMPAT_INSTALLATION_ID
            || request.external_actor_ref.kind() != OPENAI_COMPAT_ACTOR_KIND
        {
            return Ok(None);
        }
        UserId::new(request.external_actor_ref.id())
            .map(Some)
            .map_err(|error| ProductWorkflowError::BindingResolutionFailed {
                reason: format!("invalid OpenAI-compatible actor user id: {error}"),
            })
    }
}

struct OpenAiChatCompletionThreadProjectionReader {
    thread_service: Arc<dyn SessionThreadService>,
    poll_interval: Duration,
}

impl OpenAiChatCompletionThreadProjectionReader {
    fn new(thread_service: Arc<dyn SessionThreadService>) -> Self {
        Self {
            thread_service,
            poll_interval: OPENAI_COMPAT_PROJECTION_POLL_INTERVAL,
        }
    }
}

#[async_trait]
impl OpenAiChatCompletionProjectionReader for OpenAiChatCompletionThreadProjectionReader {
    async fn read_chat_completion_projection(
        &self,
        request: OpenAiChatCompletionProjectionRequest,
    ) -> Result<OpenAiChatCompletionProjection, OpenAiCompatHttpError> {
        let submitted_run_id = match &request.accepted_ack {
            ProductInboundAck::Accepted {
                submitted_run_id, ..
            } => submitted_run_id.to_string(),
            _ => return Err(OpenAiCompatHttpError::internal()),
        };
        let thread_scope = thread_scope_from_projection_read(&request.projection_read)?;
        loop {
            match self
                .thread_service
                .finalized_assistant_message_by_run(FinalizedAssistantMessageByRunRequest {
                    scope: thread_scope.clone(),
                    thread_id: request.projection_read.scope.thread_id.clone(),
                    turn_run_id: submitted_run_id.clone(),
                })
                .await
            {
                Ok(Some(message)) => {
                    return Ok(OpenAiChatCompletionProjection::text(
                        message.content.unwrap_or_default(),
                    ));
                }
                Ok(None) => tokio::time::sleep(self.poll_interval).await,
                Err(
                    SessionThreadError::UnknownThread { .. }
                    | SessionThreadError::ThreadScopeMismatch { .. },
                ) => {
                    return Err(OpenAiCompatHttpError::not_found(Some(
                        "messages".to_string(),
                    )));
                }
                Err(error) => {
                    tracing::warn!(
                        target = "ironclaw::reborn::openai_compat",
                        error = %error,
                        "failed to read finalized assistant message for OpenAI-compatible chat completion"
                    );
                    return Err(OpenAiCompatHttpError::from_kind(
                        503,
                        true,
                        OpenAiCompatErrorKind::ServiceUnavailable,
                        None,
                    ));
                }
            }
        }
    }
}

struct OpenAiResponsesThreadProjectionReader {
    thread_service: Arc<dyn SessionThreadService>,
    projection_stream: Arc<dyn ProjectionStream>,
    poll_interval: Duration,
}

impl OpenAiResponsesThreadProjectionReader {
    fn new(
        thread_service: Arc<dyn SessionThreadService>,
        projection_stream: Arc<dyn ProjectionStream>,
    ) -> Self {
        Self {
            thread_service,
            projection_stream,
            poll_interval: OPENAI_COMPAT_PROJECTION_POLL_INTERVAL,
        }
    }

    async fn read_finalized_response_message(
        &self,
        request: &ProjectionReadRequest,
        turn_run_id: String,
    ) -> Result<Option<String>, OpenAiCompatHttpError> {
        let thread_scope = thread_scope_from_projection_read(request)?;
        match self
            .thread_service
            .finalized_assistant_message_by_run(FinalizedAssistantMessageByRunRequest {
                scope: thread_scope,
                thread_id: request.scope.thread_id.clone(),
                turn_run_id,
            })
            .await
        {
            Ok(message) => Ok(message.map(|message| message.content.unwrap_or_default())),
            Err(
                SessionThreadError::UnknownThread { .. }
                | SessionThreadError::ThreadScopeMismatch { .. },
            ) => Err(OpenAiCompatHttpError::not_found(Some(
                "response_id".to_string(),
            ))),
            Err(error) => {
                tracing::warn!(
                    target = "ironclaw::reborn::openai_compat",
                    error = %error,
                    "failed to read finalized assistant message for OpenAI-compatible response"
                );
                Err(OpenAiCompatHttpError::from_kind(
                    503,
                    true,
                    OpenAiCompatErrorKind::ServiceUnavailable,
                    None,
                ))
            }
        }
    }

    async fn read_projected_response_status(
        &self,
        request: &ProjectionReadRequest,
        submitted_run_id: &str,
        after_cursor: Option<ProjectionCursor>,
    ) -> Result<ProjectedResponseStatusRead, OpenAiCompatHttpError> {
        let events = self
            .projection_stream
            .drain(ProjectionSubscriptionRequest {
                actor: request.actor.clone(),
                scope: request.scope.clone(),
                after_cursor,
            })
            .await?;
        let next_cursor = events.last().map(|event| event.projection_cursor().clone());
        Ok(ProjectedResponseStatusRead {
            status: response_status_from_projection_events(&events, submitted_run_id),
            next_cursor,
        })
    }
}

#[async_trait]
impl OpenAiResponsesProjectionReader for OpenAiResponsesThreadProjectionReader {
    async fn wait_for_response_completion(
        &self,
        request: OpenAiResponseWaitRequest,
    ) -> Result<OpenAiResponseProjection, OpenAiCompatHttpError> {
        let submitted_run_id = match &request.accepted_ack {
            ProductInboundAck::Accepted {
                submitted_run_id, ..
            } => submitted_run_id.to_string(),
            _ => return Err(OpenAiCompatHttpError::internal()),
        };
        let mut projection_after_cursor = request.projection_read.after_cursor.clone();
        loop {
            if let Some(content) = self
                .read_finalized_response_message(&request.projection_read, submitted_run_id.clone())
                .await?
            {
                return Ok(OpenAiResponseProjection::new(response_object(
                    request.public_id,
                    request.mapping.created_at,
                    request.requested_model,
                    OpenAiResponseStatus::Completed,
                    Some(content),
                )));
            }
            let projected = self
                .read_projected_response_status(
                    &request.projection_read,
                    &submitted_run_id,
                    projection_after_cursor.clone(),
                )
                .await?;
            if let Some(next_cursor) = projected.next_cursor {
                projection_after_cursor = Some(next_cursor);
            }
            if let Some(status) = projected.status
                && matches!(
                    status,
                    OpenAiResponseStatus::Failed | OpenAiResponseStatus::Cancelled
                )
            {
                return Ok(OpenAiResponseProjection::new(response_object(
                    request.public_id,
                    request.mapping.created_at,
                    request.requested_model,
                    status,
                    None,
                )));
            }
            tokio::time::sleep(self.poll_interval).await;
        }
    }

    async fn read_response(
        &self,
        request: OpenAiResponseReadRequest,
    ) -> Result<OpenAiResponseObject, OpenAiCompatHttpError> {
        let submitted_run_id = response_turn_run_ref_from_mapping(&request)?;
        let content = self
            .read_finalized_response_message(&request.projection_read, submitted_run_id.clone())
            .await?;
        let projected_status = if content.is_some() {
            None
        } else {
            self.read_projected_response_status(
                &request.projection_read,
                &submitted_run_id,
                request.projection_read.after_cursor.clone(),
            )
            .await?
            .status
        };
        let status = match (content.is_some(), projected_status) {
            (true, _) => OpenAiResponseStatus::Completed,
            (false, Some(OpenAiResponseStatus::Completed | OpenAiResponseStatus::InProgress)) => {
                OpenAiResponseStatus::InProgress
            }
            (false, Some(status)) => status,
            (false, None) => OpenAiResponseStatus::InProgress,
        };
        Ok(response_object(
            request.public_id,
            request.mapping.created_at,
            request
                .requested_model
                .unwrap_or_else(|| "reborn".to_string()),
            status,
            content,
        ))
    }
}

struct ProjectedResponseStatusRead {
    status: Option<OpenAiResponseStatus>,
    next_cursor: Option<ProjectionCursor>,
}

fn response_turn_run_ref_from_mapping(
    request: &OpenAiResponseReadRequest,
) -> Result<String, OpenAiCompatHttpError> {
    let OpenAiCompatResourceBinding::Bound { internal_refs } = &request.mapping.binding else {
        return Err(OpenAiCompatHttpError::conflict(Some(
            "response_id".to_string(),
        )));
    };
    let Some(turn_run_ref) = internal_refs.turn_run_ref.as_ref() else {
        return Err(OpenAiCompatHttpError::not_found(Some(
            "response_id".to_string(),
        )));
    };
    Ok(turn_run_ref.as_str().to_string())
}

fn response_status_from_projection_events(
    events: &[ProductOutboundEnvelope],
    submitted_run_id: &str,
) -> Option<OpenAiResponseStatus> {
    events.iter().rev().find_map(|event| match event.payload() {
        ProductOutboundPayload::ProjectionSnapshot { state }
        | ProductOutboundPayload::ProjectionUpdate { state } => {
            response_status_from_projection_state(state, submitted_run_id)
        }
        ProductOutboundPayload::FinalReply(_)
        | ProductOutboundPayload::Progress(_)
        | ProductOutboundPayload::CapabilityActivity(_)
        | ProductOutboundPayload::CapabilityDisplayPreview(_)
        | ProductOutboundPayload::GatePrompt(_)
        | ProductOutboundPayload::AuthPrompt(_)
        | ProductOutboundPayload::KeepAlive => None,
    })
}

fn response_status_from_projection_state(
    state: &ProductProjectionState,
    submitted_run_id: &str,
) -> Option<OpenAiResponseStatus> {
    state.items.iter().rev().find_map(|item| match item {
        ProductProjectionItem::RunStatus { run_id, status, .. }
            if run_id.to_string() == submitted_run_id =>
        {
            response_status_from_projection_run_status(status)
        }
        ProductProjectionItem::Text { .. }
        | ProductProjectionItem::Thinking { .. }
        | ProductProjectionItem::CapabilityActivity(_)
        | ProductProjectionItem::WorkSummary { .. }
        | ProductProjectionItem::RunStatus { .. }
        | ProductProjectionItem::Gate { .. }
        | ProductProjectionItem::SkillActivation { .. } => None,
    })
}

fn response_status_from_projection_run_status(status: &str) -> Option<OpenAiResponseStatus> {
    match status {
        "running" => Some(OpenAiResponseStatus::InProgress),
        "completed" => Some(OpenAiResponseStatus::Completed),
        "failed" | "killed" => Some(OpenAiResponseStatus::Failed),
        "cancelled" => Some(OpenAiResponseStatus::Cancelled),
        _ => None,
    }
}

fn response_object(
    id: ironclaw_reborn_openai_compat::OpenAiResponseId,
    created_at: u64,
    model: String,
    status: OpenAiResponseStatus,
    content: Option<String>,
) -> OpenAiResponseObject {
    let output = content
        .map(|text| {
            vec![OpenAiResponseOutputItem::Message {
                id: format!("msg_{}", id.as_str()),
                status: Some(OpenAiResponseOutputItemStatus::Completed),
                role: OpenAiResponsesMessageRole::Assistant,
                content: serde_json::json!([{"type": "output_text", "text": text}]),
            }]
        })
        .unwrap_or_default();
    let error = if matches!(status, OpenAiResponseStatus::Failed) {
        Some(OpenAiResponseErrorObject::from_kind(
            OpenAiCompatErrorKind::Internal,
        ))
    } else {
        None
    };
    OpenAiResponseObject {
        id,
        object: "response".to_string(),
        created_at,
        status,
        model,
        output,
        error,
        incomplete_details: None,
        usage: None,
    }
}

fn thread_scope_from_projection_read(
    projection_read: &ProjectionReadRequest,
) -> Result<ThreadScope, OpenAiCompatHttpError> {
    let Some(agent_id) = projection_read.scope.agent_id.clone() else {
        return Err(OpenAiCompatHttpError::internal());
    };
    Ok(ThreadScope {
        tenant_id: projection_read.scope.tenant_id.clone(),
        agent_id,
        project_id: projection_read.scope.project_id.clone(),
        owner_user_id: projection_read
            .scope
            .explicit_owner_user_id()
            .cloned()
            .or_else(|| Some(projection_read.actor.user_id.clone())),
        mission_id: None,
    })
}

fn openai_compat_ledger_filesystem(
    root: Arc<crate::factory::LocalDevRootFilesystem>,
    tenant_id: &TenantId,
) -> Result<Arc<ScopedFilesystem<crate::factory::LocalDevRootFilesystem>>, RebornBuildError> {
    Ok(Arc::new(ScopedFilesystem::with_fixed_view(
        root,
        MountView::new(vec![MountGrant::new(
            MountAlias::new(OPENAI_COMPAT_LEDGER_ENGINE_ROOT)?,
            VirtualPath::new(format!(
                "/tenants/{}/shared/openai_compat/engine",
                tenant_id.as_str()
            ))?,
            MountPermissions::read_write_list_delete(),
        )])?,
    )))
}

fn openai_compat_ledger_scope(
    tenant_id: TenantId,
    default_agent_id: AgentId,
    default_project_id: Option<ProjectId>,
) -> Result<ResourceScope, RebornBuildError> {
    Ok(ResourceScope {
        tenant_id,
        user_id: UserId::new(OPENAI_COMPAT_LEDGER_USER_ID)?,
        agent_id: Some(default_agent_id),
        project_id: default_project_id,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    })
}

fn openai_compat_ref_root(tenant_id: &TenantId) -> Result<VirtualPath, RebornBuildError> {
    Ok(VirtualPath::new(format!(
        "/tenants/{}/shared/openai_compat/refs",
        tenant_id.as_str()
    ))?)
}

fn invalid_openai_compat_config(
    field: &'static str,
) -> impl FnOnce(ironclaw_product_adapters::ProductAdapterError) -> RebornBuildError {
    move |error| RebornBuildError::InvalidConfig {
        reason: format!("invalid OpenAI-compatible {field}: {error}"),
    }
}
