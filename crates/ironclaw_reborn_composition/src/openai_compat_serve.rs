//! Reborn host composition for OpenAI-compatible API routes.
//!
//! The route crate owns DTOs and HTTP handlers, but the Reborn host owns the
//! authority-bearing wiring: authenticated callers, ProductWorkflow,
//! conversation binding, durable idempotency/ref stores, and projection reads.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_filesystem::{RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    AgentId, InvocationId, MountAlias, MountGrant, MountPermissions, MountView, ProjectId,
    ResourceScope, TenantId, ThreadId, UserId, VirtualPath,
};
use ironclaw_product_adapters::{
    AdapterInstallationId, ProductAdapterId, ProductInboundAck, ProductOutboundEnvelope,
    ProductWorkflow, ProjectionReadRequest, ProjectionStream,
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
    OpenAiCompatRouterState, OpenAiResponseId, OpenAiResponseObject, OpenAiResponseOutputItem,
    OpenAiResponseOutputItemStatus, OpenAiResponseProjection,
    OpenAiResponseProjectionStreamRequest, OpenAiResponseReadRequest, OpenAiResponseStatus,
    OpenAiResponseWaitRequest, OpenAiResponsesMessageRole, OpenAiResponsesProjectionReader,
    OpenAiResponsesWorkflow, openai_compat_router_with_state, openai_compat_routes,
};
use ironclaw_reborn_openai_compat_storage::FilesystemOpenAiCompatRefStore;
use ironclaw_threads::{
    FinalizedAssistantMessageByRunRequest, LoadContextMessagesRequest, MessageKind, MessageStatus,
    ProviderToolCallReferenceEnvelope, SessionThreadError, SessionThreadService,
    ThreadHistoryRequest, ThreadMessageId, ThreadMessageRecord, ThreadScope,
    ToolResultReferenceEnvelope,
};

use crate::RebornBuildError;
use crate::RebornRuntime;
use crate::webui_serve::ProtectedRouteMount;

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
    let responses_projection_reader = Arc::new(OpenAiResponsesThreadProjectionReader::new(
        runtime.webui_thread_service(),
    ));
    let projection_streamer = Arc::new(OpenAiCompatRuntimeProjectionStreamer::new(
        runtime.webui_event_stream(),
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
    poll_interval: Duration,
}

impl OpenAiResponsesThreadProjectionReader {
    fn new(thread_service: Arc<dyn SessionThreadService>) -> Self {
        Self {
            thread_service,
            poll_interval: OPENAI_COMPAT_PROJECTION_POLL_INTERVAL,
        }
    }

    /// Project a single response run into OpenAI Responses `output` items: every
    /// tool call paired with its raw output, then the run's finalized assistant
    /// message. Tool items always precede the assistant message regardless of
    /// transcript sequence — the assistant draft's sequence is reserved when the
    /// turn starts, which can predate the tool results it later produces.
    ///
    /// The data is joined from two thread-service reads because neither one
    /// alone carries everything: the history projection keeps `turn_run_id`
    /// (run attribution) plus the tool-result envelope content (the raw
    /// `model_observation`) but strips `tool_result_provider_call`, while the
    /// context projection preserves the provider call (function name, arguments,
    /// call id) but drops `turn_run_id`. Joining by `message_id` recovers both.
    async fn read_run_output(
        &self,
        request: &ProjectionReadRequest,
        turn_run_id: String,
        public_id: &OpenAiResponseId,
    ) -> Result<RunResponseProjection, OpenAiCompatHttpError> {
        let thread_scope = thread_scope_from_projection_read(request)?;
        let thread_id = request.scope.thread_id.clone();

        let history = self
            .thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: thread_scope.clone(),
                thread_id: thread_id.clone(),
            })
            .await
            .map_err(map_thread_read_error)?;

        let run_messages: Vec<&ThreadMessageRecord> = history
            .messages
            .iter()
            .filter(|message| message.turn_run_id.as_deref() == Some(turn_run_id.as_str()))
            .collect();

        // Tool calls/outputs, in transcript order.
        let mut tool_results: Vec<&ThreadMessageRecord> = run_messages
            .iter()
            .copied()
            .filter(|message| message.kind == MessageKind::ToolResultReference)
            .collect();
        tool_results.sort_by_key(|message| message.sequence);
        // The run's single finalized assistant reply (drafts dedup by run).
        let assistant = run_messages
            .iter()
            .copied()
            .filter(|message| {
                message.kind == MessageKind::Assistant && message.status == MessageStatus::Finalized
            })
            .max_by_key(|message| message.sequence);

        let provider_calls = self
            .load_provider_calls(
                &thread_scope,
                &thread_id,
                tool_results
                    .iter()
                    .map(|message| message.message_id)
                    .collect(),
            )
            .await?;

        let mut items = Vec::with_capacity(tool_results.len() * 2 + 1);
        for message in tool_results {
            push_tool_output_items(&mut items, message, &provider_calls);
        }
        if let Some(message) = assistant {
            items.push(OpenAiResponseOutputItem::Message {
                id: format!("msg_{}", public_id.as_str()),
                status: Some(OpenAiResponseOutputItemStatus::Completed),
                role: OpenAiResponsesMessageRole::Assistant,
                content: serde_json::json!([{
                    "type": "output_text",
                    "text": message.content.clone().unwrap_or_default(),
                }]),
            });
        }

        Ok(RunResponseProjection {
            items,
            assistant_finalized: assistant.is_some(),
        })
    }

    /// Re-read the run's tool-result messages through the context projection to
    /// recover `tool_result_provider_call`, which the history projection strips.
    async fn load_provider_calls(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_ids: Vec<ThreadMessageId>,
    ) -> Result<HashMap<ThreadMessageId, ProviderToolCallReferenceEnvelope>, OpenAiCompatHttpError>
    {
        if message_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let context = self
            .thread_service
            .load_context_messages(LoadContextMessagesRequest {
                scope: scope.clone(),
                thread_id: thread_id.clone(),
                message_ids,
            })
            .await
            .map_err(map_thread_read_error)?;
        Ok(context
            .messages
            .into_iter()
            .filter_map(|message| Some((message.message_id?, message.tool_result_provider_call?)))
            .collect())
    }

    /// Cheap completion gate for the wait poll loop: the run has produced its
    /// final reply once a finalized assistant message exists for it. Kept
    /// separate from [`read_run_output`] so polling does not repeatedly read the
    /// full transcript while waiting.
    async fn run_completed(
        &self,
        request: &ProjectionReadRequest,
        turn_run_id: String,
    ) -> Result<bool, OpenAiCompatHttpError> {
        let thread_scope = thread_scope_from_projection_read(request)?;
        let message = self
            .thread_service
            .finalized_assistant_message_by_run(FinalizedAssistantMessageByRunRequest {
                scope: thread_scope,
                thread_id: request.scope.thread_id.clone(),
                turn_run_id,
            })
            .await
            .map_err(map_thread_read_error)?;
        Ok(message.is_some())
    }
}

/// A response run's projected `output` items plus whether the run's final
/// assistant message has been finalized (i.e. the run is complete).
struct RunResponseProjection {
    items: Vec<OpenAiResponseOutputItem>,
    assistant_finalized: bool,
}

/// Append a tool call and its raw output to `items`. When the provider-call side
/// channel is available, both a `function_call` (name/arguments) and the paired
/// `function_call_output` are emitted; otherwise only the output is emitted,
/// keyed by the opaque tool-result ref so the call id still correlates.
fn push_tool_output_items(
    items: &mut Vec<OpenAiResponseOutputItem>,
    message: &ThreadMessageRecord,
    provider_calls: &HashMap<ThreadMessageId, ProviderToolCallReferenceEnvelope>,
) {
    let output = tool_result_output(message);
    match provider_calls.get(&message.message_id) {
        Some(provider_call) => {
            let call_id = provider_call.provider_call_id.clone();
            let arguments = serde_json::to_string(&provider_call.arguments)
                .unwrap_or_else(|_| "{}".to_string());
            items.push(OpenAiResponseOutputItem::FunctionCall {
                id: format!("fc_{call_id}"),
                status: Some(OpenAiResponseOutputItemStatus::Completed),
                call_id: call_id.clone(),
                name: provider_call.provider_tool_name.clone(),
                arguments,
            });
            items.push(OpenAiResponseOutputItem::FunctionCallOutput {
                id: format!("fco_{call_id}"),
                status: Some(OpenAiResponseOutputItemStatus::Completed),
                call_id,
                output,
            });
        }
        None => {
            let call_id = message
                .tool_result_ref
                .clone()
                .unwrap_or_else(|| format!("call_{}", message.sequence));
            items.push(OpenAiResponseOutputItem::FunctionCallOutput {
                id: format!("fco_{call_id}"),
                status: Some(OpenAiResponseOutputItemStatus::Completed),
                call_id,
                output,
            });
        }
    }
}

/// The raw tool output for a tool-result message: the model-visible
/// `model_observation` JSON when present, falling back to the safe summary.
fn tool_result_output(message: &ThreadMessageRecord) -> serde_json::Value {
    let Some(content) = message.content.as_deref() else {
        return serde_json::Value::Null;
    };
    match ToolResultReferenceEnvelope::from_json_str(content) {
        Ok(envelope) => envelope.model_observation.unwrap_or_else(|| {
            serde_json::Value::String(envelope.safe_summary.as_str().to_string())
        }),
        Err(_) => serde_json::Value::String(content.to_string()),
    }
}

fn map_thread_read_error(error: SessionThreadError) -> OpenAiCompatHttpError {
    match error {
        SessionThreadError::UnknownThread { .. }
        | SessionThreadError::ThreadScopeMismatch { .. } => {
            OpenAiCompatHttpError::not_found(Some("response_id".to_string()))
        }
        error => {
            tracing::warn!(
                target = "ironclaw::reborn::openai_compat",
                error = %error,
                "failed to read thread projection for OpenAI-compatible response"
            );
            OpenAiCompatHttpError::from_kind(
                503,
                true,
                OpenAiCompatErrorKind::ServiceUnavailable,
                None,
            )
        }
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
        while !self
            .run_completed(&request.projection_read, submitted_run_id.clone())
            .await?
        {
            tokio::time::sleep(self.poll_interval).await;
        }
        let projection = self
            .read_run_output(
                &request.projection_read,
                submitted_run_id,
                &request.public_id,
            )
            .await?;
        Ok(OpenAiResponseProjection::new(response_object(
            request.public_id,
            request.mapping.created_at,
            request.requested_model,
            OpenAiResponseStatus::Completed,
            projection.items,
        )))
    }

    async fn read_response(
        &self,
        request: OpenAiResponseReadRequest,
    ) -> Result<OpenAiResponseObject, OpenAiCompatHttpError> {
        let submitted_run_id = response_turn_run_ref_from_mapping(&request)?;
        let projection = self
            .read_run_output(
                &request.projection_read,
                submitted_run_id,
                &request.public_id,
            )
            .await?;
        let status = if projection.assistant_finalized {
            OpenAiResponseStatus::Completed
        } else {
            OpenAiResponseStatus::InProgress
        };
        Ok(response_object(
            request.public_id,
            request.mapping.created_at,
            request
                .requested_model
                .unwrap_or_else(|| "reborn".to_string()),
            status,
            projection.items,
        ))
    }
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

fn response_object(
    id: OpenAiResponseId,
    created_at: u64,
    model: String,
    status: OpenAiResponseStatus,
    output: Vec<OpenAiResponseOutputItem>,
) -> OpenAiResponseObject {
    OpenAiResponseObject {
        id,
        object: "response".to_string(),
        created_at,
        status,
        model,
        output,
        error: None,
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

#[cfg(test)]
mod tests {
    use super::*;

    use ironclaw_host_api::CapabilityId;
    use ironclaw_threads::{
        AppendAssistantDraftRequest, AppendToolResultReferenceRequest, EnsureThreadRequest,
        InMemorySessionThreadService, MessageContent, ToolResultSafeSummary,
    };
    use ironclaw_turns::{TurnActor, TurnScope};

    const RUN_ID: &str = "turn-run-1";

    fn test_scope() -> ThreadScope {
        ThreadScope {
            tenant_id: TenantId::new("tenant-1").expect("tenant"),
            agent_id: AgentId::new("agent-1").expect("agent"),
            project_id: Some(ProjectId::new("project-1").expect("project")),
            owner_user_id: Some(UserId::new("user-1").expect("user")),
            mission_id: None,
        }
    }

    /// A schema-valid `model_observation` so the thread service preserves it as
    /// the raw, model-visible tool output rather than dropping it.
    fn model_observation() -> serde_json::Value {
        serde_json::json!({
            "schema_version": 1,
            "status": "error",
            "summary": "search failed",
            "detail": {
                "kind": "invalid_input",
                "issues": [{ "path": "query", "code": "invalid_value" }]
            },
            "trust": "untrusted_tool_output"
        })
    }

    fn provider_call() -> ProviderToolCallReferenceEnvelope {
        ProviderToolCallReferenceEnvelope {
            provider_id: "openai".to_string(),
            provider_model_id: "gpt-test".to_string(),
            provider_turn_id: "turn-1".to_string(),
            provider_call_id: "call_abc".to_string(),
            provider_tool_name: "web_search".to_string(),
            capability_id: CapabilityId::new("web.search").expect("capability id"),
            arguments: serde_json::json!({ "query": "rust" }),
            response_reasoning: None,
            reasoning: None,
            signature: None,
        }
    }

    fn projection_read(scope: &ThreadScope, thread_id: &ThreadId) -> ProjectionReadRequest {
        ProjectionReadRequest {
            actor: TurnActor::new(scope.owner_user_id.clone().expect("owner")),
            scope: TurnScope::new_with_owner(
                scope.tenant_id.clone(),
                Some(scope.agent_id.clone()),
                scope.project_id.clone(),
                thread_id.clone(),
                scope.owner_user_id.clone(),
            ),
            after_cursor: None,
            limit: None,
        }
    }

    async fn ensure_thread(
        service: &InMemorySessionThreadService,
        scope: &ThreadScope,
        thread_id: &ThreadId,
    ) {
        service
            .ensure_thread(EnsureThreadRequest {
                scope: scope.clone(),
                thread_id: Some(thread_id.clone()),
                created_by_actor_id: "actor".to_string(),
                title: None,
                metadata_json: None,
            })
            .await
            .expect("ensure thread");
    }

    async fn append_tool_result(
        service: &InMemorySessionThreadService,
        scope: &ThreadScope,
        thread_id: &ThreadId,
    ) {
        service
            .append_tool_result_reference(AppendToolResultReferenceRequest {
                scope: scope.clone(),
                thread_id: thread_id.clone(),
                turn_run_id: RUN_ID.to_string(),
                result_ref: "result:tool-1".to_string(),
                safe_summary: ToolResultSafeSummary::new("search failed").expect("summary"),
                provider_call: Some(provider_call()),
                model_observation: Some(model_observation()),
            })
            .await
            .expect("append tool result");
    }

    #[tokio::test]
    async fn read_run_output_emits_paired_function_call_and_raw_output() {
        let service = Arc::new(InMemorySessionThreadService::default());
        let scope = test_scope();
        let thread_id = ThreadId::new("thread-1").expect("thread");
        ensure_thread(&service, &scope, &thread_id).await;
        append_tool_result(&service, &scope, &thread_id).await;
        let draft = service
            .append_assistant_draft(AppendAssistantDraftRequest {
                scope: scope.clone(),
                thread_id: thread_id.clone(),
                turn_run_id: RUN_ID.to_string(),
                content: MessageContent::text("here is the answer"),
            })
            .await
            .expect("append draft");
        service
            .finalize_assistant_message(
                &scope,
                &thread_id,
                draft.message_id,
                MessageContent::text("here is the answer"),
            )
            .await
            .expect("finalize assistant message");

        let reader = OpenAiResponsesThreadProjectionReader::new(service);
        let public_id = OpenAiResponseId::generate();
        let projection = reader
            .read_run_output(
                &projection_read(&scope, &thread_id),
                RUN_ID.to_string(),
                &public_id,
            )
            .await
            .expect("read run output");

        assert!(projection.assistant_finalized);
        assert_eq!(projection.items.len(), 3);

        match &projection.items[0] {
            OpenAiResponseOutputItem::FunctionCall {
                call_id,
                name,
                arguments,
                ..
            } => {
                assert_eq!(call_id, "call_abc");
                assert_eq!(name, "web_search");
                let parsed: serde_json::Value =
                    serde_json::from_str(arguments).expect("arguments json");
                assert_eq!(parsed, serde_json::json!({ "query": "rust" }));
            }
            other => panic!("expected function_call, got {other:?}"),
        }
        match &projection.items[1] {
            OpenAiResponseOutputItem::FunctionCallOutput {
                call_id, output, ..
            } => {
                assert_eq!(call_id, "call_abc");
                // The raw model_observation flows through verbatim, not the summary.
                assert_eq!(output, &model_observation());
            }
            other => panic!("expected function_call_output, got {other:?}"),
        }
        match &projection.items[2] {
            OpenAiResponseOutputItem::Message {
                id, role, content, ..
            } => {
                assert!(matches!(role, OpenAiResponsesMessageRole::Assistant));
                // Message ids are sequence-keyed so multi-step runs stay unique.
                assert!(id.starts_with("msg_"));
                assert_eq!(
                    content,
                    &serde_json::json!([{ "type": "output_text", "text": "here is the answer" }])
                );
            }
            other => panic!("expected message, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_run_output_orders_tool_items_before_assistant_message() {
        // The assistant draft is created (sequence reserved) BEFORE the tool runs,
        // so a naive sequence sort would place the final message first. Tool items
        // must still come before the assistant message.
        let service = Arc::new(InMemorySessionThreadService::default());
        let scope = test_scope();
        let thread_id = ThreadId::new("thread-3").expect("thread");
        ensure_thread(&service, &scope, &thread_id).await;
        let draft = service
            .append_assistant_draft(AppendAssistantDraftRequest {
                scope: scope.clone(),
                thread_id: thread_id.clone(),
                turn_run_id: RUN_ID.to_string(),
                content: MessageContent::text("draft"),
            })
            .await
            .expect("append draft");
        append_tool_result(&service, &scope, &thread_id).await;
        service
            .finalize_assistant_message(
                &scope,
                &thread_id,
                draft.message_id,
                MessageContent::text("here is the answer"),
            )
            .await
            .expect("finalize assistant message");

        let reader = OpenAiResponsesThreadProjectionReader::new(service);
        let public_id = OpenAiResponseId::generate();
        let projection = reader
            .read_run_output(
                &projection_read(&scope, &thread_id),
                RUN_ID.to_string(),
                &public_id,
            )
            .await
            .expect("read run output");

        assert_eq!(projection.items.len(), 3);
        assert!(matches!(
            projection.items[0],
            OpenAiResponseOutputItem::FunctionCall { .. }
        ));
        assert!(matches!(
            projection.items[1],
            OpenAiResponseOutputItem::FunctionCallOutput { .. }
        ));
        assert!(matches!(
            projection.items[2],
            OpenAiResponseOutputItem::Message { .. }
        ));
    }

    #[tokio::test]
    async fn read_run_output_in_progress_surfaces_tool_output_without_final_message() {
        let service = Arc::new(InMemorySessionThreadService::default());
        let scope = test_scope();
        let thread_id = ThreadId::new("thread-2").expect("thread");
        ensure_thread(&service, &scope, &thread_id).await;
        append_tool_result(&service, &scope, &thread_id).await;

        let reader = OpenAiResponsesThreadProjectionReader::new(service);
        let public_id = OpenAiResponseId::generate();
        let projection = reader
            .read_run_output(
                &projection_read(&scope, &thread_id),
                RUN_ID.to_string(),
                &public_id,
            )
            .await
            .expect("read run output");

        assert!(!projection.assistant_finalized);
        assert_eq!(projection.items.len(), 2);
        assert!(matches!(
            projection.items[0],
            OpenAiResponseOutputItem::FunctionCall { .. }
        ));
        assert!(matches!(
            projection.items[1],
            OpenAiResponseOutputItem::FunctionCallOutput { .. }
        ));
    }
}
