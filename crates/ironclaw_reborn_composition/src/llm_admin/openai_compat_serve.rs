//! Reborn host composition for OpenAI-compatible API routes.
//!
//! The route crate owns DTOs and HTTP handlers, but the Reborn host owns the
//! authority-bearing wiring: authenticated callers, ProductWorkflow,
//! conversation binding, durable idempotency/ref stores, and projection reads.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ironclaw_attachments::InboundAttachment;
use ironclaw_filesystem::{RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    AgentId, InvocationId, MountAlias, MountGrant, MountPermissions, MountView, ProjectId,
    ResourceScope, TenantId, ThreadId, UserId, VirtualPath,
};
use ironclaw_product_adapters::{
    AdapterInstallationId, ProductAdapterError, ProductAdapterId, ProductInboundAck,
    ProductInboundEnvelope, ProductOutboundEnvelope, ProductOutboundPayload, ProductProjectionItem,
    ProductProjectionState, ProductWorkflow, ProjectionCursor, ProjectionReadRequest,
    ProjectionStream, ProjectionSubscriptionRequest,
};
use ironclaw_product_workflow::RebornFilesystemIdempotencyLedger;
use ironclaw_product_workflow::{
    DefaultInboundTurnService, DefaultProductWorkflow, InboundAttachmentLander,
    ProductActorUserResolutionRequest, ProductActorUserResolver, ProductConversationBindingService,
    ProductInstallationKey, ProductInstallationScope, ProductWorkflowError,
    ResolvedProductActorUser, StaticProductInstallationResolver,
};
#[cfg(feature = "root-llm-provider")]
use ironclaw_product_workflow::{
    LlmConfigService, LlmConfigServiceError, LlmConfigSnapshot, WebUiAuthenticatedCaller,
};
use ironclaw_reborn_openai_compat::FilesystemOpenAiCompatRefStore;
use ironclaw_reborn_openai_compat::{
    OPENAI_COMPAT_ACTOR_KIND, OPENAI_COMPAT_ADAPTER_ID, OPENAI_COMPAT_INSTALLATION_ID,
    OpenAiChatCompletionProjection, OpenAiChatCompletionProjectionReader,
    OpenAiChatCompletionProjectionRequest, OpenAiChatCompletionsWorkflow,
    OpenAiChatProjectionStreamRequest, OpenAiCompatActorScope, OpenAiCompatErrorKind,
    OpenAiCompatHttpError, OpenAiCompatInboundAttachmentSubmit, OpenAiCompatProjectionStreamer,
    OpenAiCompatRefStore, OpenAiCompatResourceBinding, OpenAiCompatResourceMapping,
    OpenAiCompatRouterState, OpenAiResponseErrorObject, OpenAiResponseId, OpenAiResponseObject,
    OpenAiResponseOutputItem, OpenAiResponseOutputItemStatus, OpenAiResponseProjection,
    OpenAiResponseProjectionStreamRequest, OpenAiResponseReadRequest, OpenAiResponseStatus,
    OpenAiResponseUsage, OpenAiResponseWaitRequest, OpenAiResponsesMessageRole,
    OpenAiResponsesProjectionReader, OpenAiResponsesWorkflow, openai_compat_router_with_state,
    openai_compat_routes,
};
use ironclaw_reborn_openai_compat::{OpenAiCompatCost, OpenAiResponseInputTokensDetails};
use ironclaw_reborn_openai_compat::{
    OpenAiCompatExternalToolResume, OpenAiCompatExternalToolResumeRequest,
    OpenAiCompatExternalToolSpec, OpenAiCompatExternalToolStore, OpenAiCompatTurnRunRef,
};
#[cfg(feature = "root-llm-provider")]
use ironclaw_reborn_openai_compat::{OpenAiCompatModelCatalog, OpenAiCompatModelEntry};
use ironclaw_threads::{
    FinalizedAssistantMessageByRunRequest, LoadContextMessagesRequest, MessageKind, MessageStatus,
    ProviderToolCallReferenceEnvelope, SessionThreadError, SessionThreadService,
    ThreadHistoryRequest, ThreadMessageId, ThreadMessageRecord, ThreadScope,
    ToolResultReferenceEnvelope,
};
use ironclaw_turns::{
    ExternalToolCatalog, ExternalToolCatalogError, ExternalToolSpec, GateRef, GetRunStateRequest,
    IdempotencyKey, ResumeTurnPrecondition, ResumeTurnRequest, TurnCoordinator, TurnError,
    TurnErrorCategory, TurnRunId, TurnScope, TurnStatus, run_profile::LoopModelUsage,
};
use sha2::{Digest, Sha256};

use crate::RebornBuildError;
use crate::RebornRuntime;
use crate::webui::route_mounts::ProtectedRouteMount;

#[cfg(test)]
mod tests;

const OPENAI_COMPAT_LEDGER_USER_ID: &str = "openai-compat";
const OPENAI_COMPAT_LEDGER_ENGINE_ROOT: &str = "/engine";
const OPENAI_COMPAT_PROJECTION_POLL_INTERVAL: Duration = Duration::from_millis(100);
const OPENAI_COMPAT_PENDING_EXTERNAL_TOOL_FALLBACK_DELAY: Duration = Duration::from_secs(10);
const OPENAI_COMPAT_EXTERNAL_TOOL_RESUME_POLL_INTERVAL: Duration = Duration::from_millis(50);
const OPENAI_COMPAT_EXTERNAL_TOOL_RESUME_WAIT_TIMEOUT: Duration = Duration::from_secs(5);

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
    let mut inbound_turn_service = DefaultInboundTurnService::new(
        binding.clone(),
        runtime.webui_thread_service(),
        runtime.webui_turn_coordinator(),
    );
    // Lands inline image bytes (vision, #4644) through the same project-scoped
    // workspace authority the agent's file tools resolve through, so an image
    // attached to an OpenAI-compatible chat completion reaches the model.
    if let Some(workspace_filesystem) = runtime.webui_workspace_filesystem() {
        let lander: Arc<dyn InboundAttachmentLander> = Arc::new(
            crate::support::fs::ProjectScopedAttachmentLander::new(workspace_filesystem),
        );
        inbound_turn_service = inbound_turn_service.with_inbound_attachments(lander);
    }
    let inbound = Arc::new(inbound_turn_service);
    // `.with_delivered_gate_routes` is intentionally omitted here. The
    // OpenAI-compat surface never produces `ApprovalResolution`,
    // `ScopedApprovalResolution`, or `AuthResolution` payloads (verified: no
    // such payload constructions exist in `crates/ironclaw_reborn_openai_compat/`),
    // so the delivered-route conversation-fingerprint fallback is unreachable on
    // this surface. The workflow falls back to the default in-memory no-op store,
    // which is correct for this surface.
    // Keep the concrete type so the same instance can back both the bytes-free
    // `ProductWorkflow` door and the inline-attachment native door (vision,
    // #4644), the latter via `OpenAiCompatAttachmentSubmitAdapter`.
    let default_product_workflow = Arc::new(
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
    let attachment_submit: Arc<dyn OpenAiCompatInboundAttachmentSubmit> =
        Arc::new(OpenAiCompatAttachmentSubmitAdapter {
            workflow: default_product_workflow.clone(),
        });
    let product_workflow: Arc<dyn ProductWorkflow> = default_product_workflow;

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
    // The external-tool catalog is the run-scoped seam shared with the loop
    // host: the host records parked calls + completes them from submitted
    // outputs; the Responses surface registers specs, submits outputs, and reads
    // parked calls back here.
    let external_tool_catalog = local_runtime.external_tool_catalog.clone();
    let responses_projection_reader = Arc::new(
        OpenAiResponsesThreadProjectionReader::new(
            runtime.webui_thread_service(),
            projection_stream.clone(),
            external_tool_catalog.clone(),
        )
        .with_turn_coordinator(runtime.webui_turn_coordinator()),
    );
    let projection_streamer = Arc::new(OpenAiCompatRuntimeProjectionStreamer::new(
        projection_stream,
    ));
    let chat_workflow = Arc::new(
        OpenAiChatCompletionsWorkflow::new(
            product_workflow.clone(),
            ref_store.clone(),
            chat_projection_reader,
        )
        .with_projection_streamer(projection_streamer.clone())
        .with_attachment_submit(attachment_submit),
    );
    let external_tool_store: Arc<dyn OpenAiCompatExternalToolStore> =
        Arc::new(OpenAiCompatRuntimeExternalToolStore {
            catalog: external_tool_catalog,
        });
    let external_tool_resume: Arc<dyn OpenAiCompatExternalToolResume> =
        Arc::new(OpenAiCompatRuntimeExternalToolResume {
            coordinator: runtime.webui_turn_coordinator(),
        });
    let responses_workflow = Arc::new(
        OpenAiResponsesWorkflow::new(product_workflow, ref_store, responses_projection_reader)
            .with_projection_streamer(projection_streamer)
            .with_external_tools(external_tool_store, external_tool_resume),
    );
    let router_state = OpenAiCompatRouterState::with_chat_completions(chat_workflow)
        .with_responses_workflow(responses_workflow);
    // `GET /v1/models` lists the deployment's configured models from the same
    // LLM-config source the operator WebUI uses. Wired only when the root LLM
    // provider is compiled in; otherwise the route stays fail-closed (501).
    #[cfg(feature = "root-llm-provider")]
    let router_state = match crate::webui::facade::build_llm_config_service(runtime) {
        Some(llm_config) => {
            let catalog: Arc<dyn OpenAiCompatModelCatalog> =
                Arc::new(LlmConfigModelCatalog::new(llm_config));
            router_state.with_models_catalog(catalog)
        }
        None => router_state,
    };
    Ok(ProtectedRouteMount::new(
        openai_compat_router_with_state(router_state),
        openai_compat_routes(),
    ))
}

/// Maps an [`LlmConfigSnapshot`] to the OpenAI-compatible `/v1/models` listing.
///
/// The active selection (if any) is listed first, then every configured
/// provider's active or default model, de-duplicated by model id while
/// preserving order. Each entry's `owned_by` is the provider id.
#[cfg(feature = "root-llm-provider")]
fn model_entries_from_snapshot(snapshot: &LlmConfigSnapshot) -> Vec<OpenAiCompatModelEntry> {
    let mut candidates: Vec<(String, String)> = Vec::new();
    if let Some(active) = &snapshot.active
        && let Some(model) = &active.model
    {
        candidates.push((model.clone(), active.provider_id.clone()));
    }
    for provider in &snapshot.providers {
        let model = provider
            .active_model
            .clone()
            .unwrap_or_else(|| provider.default_model.clone());
        candidates.push((model, provider.id.clone()));
    }
    let mut seen = std::collections::HashSet::new();
    candidates
        .into_iter()
        .filter(|(id, _)| !id.is_empty() && seen.insert(id.clone()))
        .map(|(id, owner)| OpenAiCompatModelEntry::new(id).with_owner(owner))
        .collect()
}

/// [`OpenAiCompatModelCatalog`] backed by the runtime's operator LLM-config
/// service. Lists the deployment's configured models for `GET /v1/models`.
#[cfg(feature = "root-llm-provider")]
struct LlmConfigModelCatalog {
    llm_config: Arc<dyn LlmConfigService>,
}

#[cfg(feature = "root-llm-provider")]
impl LlmConfigModelCatalog {
    fn new(llm_config: Arc<dyn LlmConfigService>) -> Self {
        Self { llm_config }
    }
}

#[cfg(feature = "root-llm-provider")]
#[async_trait]
impl OpenAiCompatModelCatalog for LlmConfigModelCatalog {
    async fn list_models(
        &self,
        caller: &ironclaw_reborn_openai_compat::OpenAiCompatAuthenticatedCaller,
    ) -> Result<Vec<OpenAiCompatModelEntry>, OpenAiCompatHttpError> {
        // The route authenticated the caller; carry its tenant/user scope into
        // the LLM-config read so the snapshot is scoped to the same identity.
        let webui_caller = WebUiAuthenticatedCaller::new(
            caller.scope().tenant_id().clone(),
            caller.scope().user_id().clone(),
            caller.scope().agent_id().cloned(),
            caller.scope().project_id().cloned(),
        );
        let snapshot = self
            .llm_config
            .snapshot(webui_caller)
            .await
            .map_err(map_llm_config_error_to_openai)?;
        Ok(model_entries_from_snapshot(&snapshot))
    }
}

#[cfg(feature = "root-llm-provider")]
fn map_llm_config_error_to_openai(error: LlmConfigServiceError) -> OpenAiCompatHttpError {
    match error {
        LlmConfigServiceError::InvalidRequest { .. } => {
            OpenAiCompatHttpError::invalid_request(None)
        }
        LlmConfigServiceError::NotFound => OpenAiCompatHttpError::not_found(None),
        LlmConfigServiceError::Unavailable => OpenAiCompatHttpError::from_kind(
            503,
            true,
            OpenAiCompatErrorKind::ServiceUnavailable,
            None,
        ),
        LlmConfigServiceError::Internal => OpenAiCompatHttpError::internal(),
    }
}

/// Bridges the route crate's [`OpenAiCompatInboundAttachmentSubmit`] door to the
/// product-workflow's native attachment landing. Lives here (not in the route
/// crate) because the route crate must not depend on `ironclaw_product_workflow`
/// (enforced by `reborn_dependency_boundaries`).
struct OpenAiCompatAttachmentSubmitAdapter {
    workflow: Arc<DefaultProductWorkflow>,
}

#[async_trait]
impl OpenAiCompatInboundAttachmentSubmit for OpenAiCompatAttachmentSubmitAdapter {
    async fn submit_inbound_with_attachments(
        &self,
        envelope: ProductInboundEnvelope,
        attachments: Vec<InboundAttachment>,
    ) -> Result<ProductInboundAck, ProductAdapterError> {
        self.workflow
            .submit_inbound_with_attachments(envelope, attachments)
            .await
    }
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
    ) -> Result<Option<ResolvedProductActorUser>, ProductWorkflowError> {
        if request.adapter_id.as_str() != OPENAI_COMPAT_ADAPTER_ID
            || request.installation_id.as_str() != OPENAI_COMPAT_INSTALLATION_ID
            || request.external_actor_ref.kind() != OPENAI_COMPAT_ACTOR_KIND
        {
            return Ok(None);
        }
        UserId::new(request.external_actor_ref.id())
            .map(ResolvedProductActorUser::new)
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
    turn_coordinator: Option<Arc<dyn TurnCoordinator>>,
    /// Shared run-scoped catalog. Read to render a parked `BlockedExternalTool`
    /// run's pending calls as `function_call` output items.
    external_tool_catalog: Arc<dyn ExternalToolCatalog>,
    poll_interval: Duration,
}

impl OpenAiResponsesThreadProjectionReader {
    fn new(
        thread_service: Arc<dyn SessionThreadService>,
        projection_stream: Arc<dyn ProjectionStream>,
        external_tool_catalog: Arc<dyn ExternalToolCatalog>,
    ) -> Self {
        Self {
            thread_service,
            projection_stream,
            turn_coordinator: None,
            external_tool_catalog,
            poll_interval: OPENAI_COMPAT_PROJECTION_POLL_INTERVAL,
        }
    }

    fn with_turn_coordinator(mut self, turn_coordinator: Arc<dyn TurnCoordinator>) -> Self {
        self.turn_coordinator = Some(turn_coordinator);
        self
    }

    /// The parked external tool calls for a run, rendered as `function_call`
    /// output items. Empty when the run has no recorded pending calls.
    async fn pending_external_tool_items(
        &self,
        submitted_run_id: &str,
    ) -> Result<Vec<OpenAiResponseOutputItem>, OpenAiCompatHttpError> {
        let run_id = parse_turn_run_id(submitted_run_id)?;
        let pending = self
            .external_tool_catalog
            .pending_calls(run_id)
            .await
            .map_err(map_catalog_error)?;
        Ok(pending
            .into_iter()
            .map(|call| OpenAiResponseOutputItem::FunctionCall {
                id: format!("fc_{}", call.call_id()),
                status: Some(OpenAiResponseOutputItemStatus::Completed),
                call_id: call.call_id().to_string(),
                name: call.name().to_string(),
                arguments: serde_json::to_string(call.arguments())
                    .unwrap_or_else(|_| "{}".to_string()),
            })
            .collect())
    }

    async fn has_pending_external_tool_calls(
        &self,
        submitted_run_id: &str,
    ) -> Result<bool, OpenAiCompatHttpError> {
        let run_id = parse_turn_run_id(submitted_run_id)?;
        let pending = self
            .external_tool_catalog
            .pending_calls(run_id)
            .await
            .map_err(map_catalog_error)?;
        Ok(!pending.is_empty())
    }

    async fn can_surface_pending_external_tool_fallback(
        &self,
        request: &ProjectionReadRequest,
        actor_scope: &OpenAiCompatActorScope,
        submitted_run_id: &str,
    ) -> Result<bool, OpenAiCompatHttpError> {
        let Some(coordinator) = self.turn_coordinator.as_ref() else {
            return Ok(true);
        };
        let run_id = TurnRunId::parse(submitted_run_id)
            .map_err(|_| OpenAiCompatHttpError::not_found(Some("response_id".to_string())))?;
        match coordinator
            .get_run_state(GetRunStateRequest {
                scope: openai_compat_resume_turn_scope(
                    actor_scope,
                    request.scope.thread_id.clone(),
                ),
                run_id,
            })
            .await
        {
            Ok(state) => Ok(state.status == TurnStatus::BlockedExternalTool),
            Err(error) if error.category() == TurnErrorCategory::ScopeNotFound => Ok(false),
            Err(error) => Err(map_resume_turn_error(error)),
        }
    }

    /// Read the run's cumulative token usage from persisted run state and render
    /// it as an OpenAI-compatible `usage` object (with USD cost). Best-effort:
    /// returns `None` when no coordinator is wired, the run can't be read, or the
    /// run reported no usage (replay stubs, usage-less providers). The model is
    /// priced by the caller's requested model — which, once model selection
    /// routes it, is also the model that actually ran.
    async fn read_run_usage(
        &self,
        projection_read: &ProjectionReadRequest,
        actor_scope: &OpenAiCompatActorScope,
        submitted_run_id: &str,
        requested_model: &str,
    ) -> Option<OpenAiResponseUsage> {
        let coordinator = self.turn_coordinator.as_ref()?;
        // silent-ok: an unparseable run id means there is no usage to report.
        let run_id = TurnRunId::parse(submitted_run_id).ok()?;
        let state = coordinator
            .get_run_state(GetRunStateRequest {
                scope: openai_compat_resume_turn_scope(
                    actor_scope,
                    projection_read.scope.thread_id.clone(),
                ),
                run_id,
            })
            .await
            // silent-ok: best-effort usage read — a missing/unreadable run
            // state yields no usage object, not a request failure.
            .ok()?;
        let usage = state.model_usage?;
        Some(response_usage_from_model_usage(&usage, requested_model))
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

        // Tool calls/outputs, in transcript order. Finalized only: history can
        // surface redacted/deleted messages, and matching the finalized status
        // these rows are appended with avoids leaking redacted tool activity
        // (including a `tool_result_ref` fallback call id).
        let mut tool_results: Vec<&ThreadMessageRecord> = run_messages
            .iter()
            .copied()
            .filter(|message| {
                message.kind == MessageKind::ToolResultReference
                    && message.status == MessageStatus::Finalized
            })
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

    /// Drain the projection event stream for the run's latest status. Used while
    /// polling so a failed/cancelled run surfaces even before (or without) a
    /// finalized assistant message.
    async fn read_projected_response_status(
        &self,
        request: &ProjectionReadRequest,
        actor_scope: &OpenAiCompatActorScope,
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
        let blocked_external_tool = if self.turn_coordinator.is_some() {
            self.run_blocked_external_tool_from_state(request, actor_scope, submitted_run_id)
                .await?
        } else {
            run_blocked_external_tool_from_projection_events(&events, submitted_run_id)
        };
        Ok(ProjectedResponseStatusRead {
            status: response_status_from_projection_events(&events, submitted_run_id),
            blocked_external_tool,
            next_cursor,
        })
    }

    async fn run_blocked_external_tool_from_state(
        &self,
        request: &ProjectionReadRequest,
        actor_scope: &OpenAiCompatActorScope,
        submitted_run_id: &str,
    ) -> Result<bool, OpenAiCompatHttpError> {
        let Some(coordinator) = self.turn_coordinator.as_ref() else {
            return Ok(false);
        };
        let run_id = TurnRunId::parse(submitted_run_id)
            .map_err(|_| OpenAiCompatHttpError::not_found(Some("response_id".to_string())))?;
        match coordinator
            .get_run_state(GetRunStateRequest {
                scope: openai_compat_resume_turn_scope(
                    actor_scope,
                    request.scope.thread_id.clone(),
                ),
                run_id,
            })
            .await
        {
            Ok(state) => Ok(state.status == TurnStatus::BlockedExternalTool),
            Err(error) if error.category() == TurnErrorCategory::ScopeNotFound => Ok(false),
            Err(error) => Err(map_resume_turn_error(error)),
        }
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
                name: provider_call.provider_tool_name.as_str().to_string(),
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
///
/// Parsing is best-effort: the envelope shape is deserialized directly without
/// the strict `from_json_str` validation, so a version mismatch or a legacy
/// `model_observation`/`result_ref` shape still yields the intended
/// `model_observation`/`safe_summary` rather than leaking the entire raw
/// envelope. Only content that does not deserialize as an envelope at all
/// (`safe_summary` itself is still validated on deserialize) falls back to the
/// raw string.
fn tool_result_output(message: &ThreadMessageRecord) -> serde_json::Value {
    let Some(content) = message.content.as_deref() else {
        return serde_json::Value::Null;
    };
    match serde_json::from_str::<ToolResultReferenceEnvelope>(content) {
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
        // The run id is read from the response's bound refs (the mapping is
        // bound before the wait). A create carries the accepted ack too, but an
        // external-tool resume reuses the parked run with no new ack.
        let submitted_run_id = turn_run_ref_from_binding(&request.mapping)?;
        let mut projection_after_cursor = request.projection_read.after_cursor.clone();
        let pending_external_tool_fallback_at =
            tokio::time::Instant::now() + OPENAI_COMPAT_PENDING_EXTERNAL_TOOL_FALLBACK_DELAY;
        loop {
            // Cheap completion gate first; only read the full transcript
            // projection (tool calls/outputs + assistant message) once the run
            // has produced its finalized reply.
            if self
                .run_completed(&request.projection_read, submitted_run_id.clone())
                .await?
            {
                let usage = self
                    .read_run_usage(
                        &request.projection_read,
                        &request.actor_scope,
                        &submitted_run_id,
                        &request.requested_model,
                    )
                    .await;
                let projection = self
                    .read_run_output(
                        &request.projection_read,
                        submitted_run_id,
                        &request.public_id,
                    )
                    .await?;
                return Ok(OpenAiResponseProjection::new(response_object(
                    request.public_id,
                    request.mapping.created_at,
                    request.requested_model,
                    OpenAiResponseStatus::Completed,
                    projection.items,
                    usage,
                )));
            }
            let projected = self
                .read_projected_response_status(
                    &request.projection_read,
                    &request.actor_scope,
                    &submitted_run_id,
                    projection_after_cursor.clone(),
                )
                .await?;
            if let Some(next_cursor) = projected.next_cursor {
                projection_after_cursor = Some(next_cursor);
            }
            // A run parked on a client tool is "complete" from the API client's
            // view: surface the parked call(s) as `function_call` items (plus any
            // already-completed tool output for the run) and stop waiting. The
            // client resolves them with a `function_call_output` continuation.
            if projected.blocked_external_tool {
                let mut projection = self
                    .read_run_output(
                        &request.projection_read,
                        submitted_run_id.clone(),
                        &request.public_id,
                    )
                    .await?;
                projection
                    .items
                    .extend(self.pending_external_tool_items(&submitted_run_id).await?);
                let usage = self
                    .read_run_usage(
                        &request.projection_read,
                        &request.actor_scope,
                        &submitted_run_id,
                        &request.requested_model,
                    )
                    .await;
                return Ok(OpenAiResponseProjection::new(response_object(
                    request.public_id,
                    request.mapping.created_at,
                    request.requested_model,
                    OpenAiResponseStatus::Completed,
                    projection.items,
                    usage,
                )));
            }
            if self
                .has_pending_external_tool_calls(&submitted_run_id)
                .await?
                && tokio::time::Instant::now() >= pending_external_tool_fallback_at
                && self
                    .can_surface_pending_external_tool_fallback(
                        &request.projection_read,
                        &request.actor_scope,
                        &submitted_run_id,
                    )
                    .await?
            {
                let mut projection = self
                    .read_run_output(
                        &request.projection_read,
                        submitted_run_id.clone(),
                        &request.public_id,
                    )
                    .await?;
                projection
                    .items
                    .extend(self.pending_external_tool_items(&submitted_run_id).await?);
                let usage = self
                    .read_run_usage(
                        &request.projection_read,
                        &request.actor_scope,
                        &submitted_run_id,
                        &request.requested_model,
                    )
                    .await;
                return Ok(OpenAiResponseProjection::new(response_object(
                    request.public_id,
                    request.mapping.created_at,
                    request.requested_model,
                    OpenAiResponseStatus::Completed,
                    projection.items,
                    usage,
                )));
            }
            if let Some(status) = projected.status
                && matches!(
                    status,
                    OpenAiResponseStatus::Failed | OpenAiResponseStatus::Cancelled
                )
            {
                let usage = self
                    .read_run_usage(
                        &request.projection_read,
                        &request.actor_scope,
                        &submitted_run_id,
                        &request.requested_model,
                    )
                    .await;
                return Ok(OpenAiResponseProjection::new(response_object(
                    request.public_id,
                    request.mapping.created_at,
                    request.requested_model,
                    status,
                    Vec::new(),
                    usage,
                )));
            }
            tokio::time::sleep(self.poll_interval).await;
        }
    }

    async fn read_response(
        &self,
        request: OpenAiResponseReadRequest,
    ) -> Result<OpenAiResponseObject, OpenAiCompatHttpError> {
        let submitted_run_id = turn_run_ref_from_binding(&request.mapping)?;
        let mut projection = self
            .read_run_output(
                &request.projection_read,
                submitted_run_id.clone(),
                &request.public_id,
            )
            .await?;
        let projected = if projection.assistant_finalized {
            None
        } else {
            Some(
                self.read_projected_response_status(
                    &request.projection_read,
                    &request.actor_scope,
                    &submitted_run_id,
                    request.projection_read.after_cursor.clone(),
                )
                .await?,
            )
        };
        // A retrieve on a run still parked on a client tool surfaces the pending
        // `function_call` item(s) with a completed status, mirroring the create
        // path so a client polling `GET /responses/{id}` sees the same shape.
        if projected
            .as_ref()
            .is_some_and(|projected| projected.blocked_external_tool)
        {
            projection
                .items
                .extend(self.pending_external_tool_items(&submitted_run_id).await?);
            let model = request
                .requested_model
                .clone()
                .unwrap_or_else(|| "reborn".to_string());
            let usage = self
                .read_run_usage(
                    &request.projection_read,
                    &request.actor_scope,
                    &submitted_run_id,
                    &model,
                )
                .await;
            return Ok(response_object(
                request.public_id,
                request.mapping.created_at,
                model,
                OpenAiResponseStatus::Completed,
                projection.items,
                usage,
            ));
        }
        let status = match (
            projection.assistant_finalized,
            projected.and_then(|p| p.status),
        ) {
            (true, _) => OpenAiResponseStatus::Completed,
            (false, Some(OpenAiResponseStatus::Completed | OpenAiResponseStatus::InProgress)) => {
                OpenAiResponseStatus::InProgress
            }
            (false, Some(status)) => status,
            (false, None) => OpenAiResponseStatus::InProgress,
        };
        let model = request
            .requested_model
            .clone()
            .unwrap_or_else(|| "reborn".to_string());
        let usage = self
            .read_run_usage(
                &request.projection_read,
                &request.actor_scope,
                &submitted_run_id,
                &model,
            )
            .await;
        Ok(response_object(
            request.public_id,
            request.mapping.created_at,
            model,
            status,
            projection.items,
            usage,
        ))
    }
}

struct ProjectedResponseStatusRead {
    status: Option<OpenAiResponseStatus>,
    /// Whether the run's latest projected status is `blocked_external_tool`.
    /// Surfaced separately because that wire status has no `OpenAiResponseStatus`
    /// — the Responses surface renders it as a completed turn with pending
    /// `function_call` items.
    blocked_external_tool: bool,
    next_cursor: Option<ProjectionCursor>,
}

fn turn_run_ref_from_binding(
    mapping: &OpenAiCompatResourceMapping,
) -> Result<String, OpenAiCompatHttpError> {
    let OpenAiCompatResourceBinding::Bound { internal_refs } = &mapping.binding else {
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

/// Whether the run's most recent projected status is `blocked_external_tool` —
/// the wire status of a run parked on a client-supplied tool call. Has no
/// `OpenAiResponseStatus` mapping, so it is surfaced as a separate signal.
fn run_blocked_external_tool_from_projection_events(
    events: &[ProductOutboundEnvelope],
    submitted_run_id: &str,
) -> bool {
    events
        .iter()
        .rev()
        .find_map(|event| match event.payload() {
            ProductOutboundPayload::ProjectionSnapshot { state }
            | ProductOutboundPayload::ProjectionUpdate { state } => {
                run_status_wire_from_projection_state(state, submitted_run_id)
            }
            _ => None,
        })
        .is_some_and(|status| status == "blocked_external_tool")
}

fn run_status_wire_from_projection_state(
    state: &ProductProjectionState,
    submitted_run_id: &str,
) -> Option<String> {
    state.items.iter().rev().find_map(|item| match item {
        ProductProjectionItem::RunStatus { run_id, status, .. }
            if run_id.to_string() == submitted_run_id =>
        {
            Some(status.clone())
        }
        _ => None,
    })
}

fn parse_turn_run_id(raw: &str) -> Result<TurnRunId, OpenAiCompatHttpError> {
    TurnRunId::parse(raw).map_err(|_| OpenAiCompatHttpError::internal())
}

fn map_catalog_error(error: ExternalToolCatalogError) -> OpenAiCompatHttpError {
    match error {
        ExternalToolCatalogError::InvalidRegistration { .. } => {
            OpenAiCompatHttpError::invalid_request(Some("tools".to_string()))
        }
        ExternalToolCatalogError::CallNotPending => {
            OpenAiCompatHttpError::invalid_request(Some("input.call_id".to_string()))
        }
        ExternalToolCatalogError::OutputAlreadySubmitted => {
            OpenAiCompatHttpError::conflict(Some("input.call_id".to_string()))
        }
        ExternalToolCatalogError::Unavailable => OpenAiCompatHttpError::from_kind(
            503,
            true,
            OpenAiCompatErrorKind::ServiceUnavailable,
            None,
        ),
    }
}

fn map_catalog_submit_output_error(error: ExternalToolCatalogError) -> OpenAiCompatHttpError {
    match error {
        ExternalToolCatalogError::CallNotPending => {
            OpenAiCompatHttpError::invalid_request(Some("input.call_id".to_string()))
        }
        ExternalToolCatalogError::OutputAlreadySubmitted => {
            OpenAiCompatHttpError::conflict(Some("input.call_id".to_string()))
        }
        other => map_catalog_error(other),
    }
}

fn map_resume_turn_error(error: TurnError) -> OpenAiCompatHttpError {
    match error.category() {
        TurnErrorCategory::ScopeNotFound => {
            OpenAiCompatHttpError::not_found(Some("previous_response_id".to_string()))
        }
        _ => OpenAiCompatHttpError::from_kind(
            503,
            true,
            OpenAiCompatErrorKind::ServiceUnavailable,
            None,
        ),
    }
}

/// [`OpenAiCompatExternalToolStore`] backed by the runtime's shared external-tool
/// catalog. Registers the run's client tool specs and stores client-submitted
/// outputs so the loop host can complete a parked call from the catalog.
struct OpenAiCompatRuntimeExternalToolStore {
    catalog: Arc<dyn ExternalToolCatalog>,
}

#[async_trait]
impl OpenAiCompatExternalToolStore for OpenAiCompatRuntimeExternalToolStore {
    async fn register_tools(
        &self,
        run_ref: OpenAiCompatTurnRunRef,
        specs: Vec<OpenAiCompatExternalToolSpec>,
    ) -> Result<(), OpenAiCompatHttpError> {
        let run_id = parse_turn_run_id(run_ref.as_str())?;
        let engine_specs = specs
            .into_iter()
            .map(|spec| {
                ExternalToolSpec::new(spec.name, spec.description, spec.parameters_schema)
                    .map_err(|_| OpenAiCompatHttpError::invalid_request(Some("tools".to_string())))
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.catalog
            .register(run_id, engine_specs)
            .await
            .map_err(map_catalog_error)
    }

    async fn submit_tool_output(
        &self,
        run_ref: OpenAiCompatTurnRunRef,
        call_id: String,
        output: serde_json::Value,
    ) -> Result<(), OpenAiCompatHttpError> {
        let run_id = parse_turn_run_id(run_ref.as_str())?;
        self.catalog
            .submit_output_for_pending_call(run_id, call_id, output)
            .await
            .map_err(map_catalog_submit_output_error)
    }
}

/// [`OpenAiCompatExternalToolResume`] backed by the turn coordinator. Reads the
/// parked run's current gate and binding refs, then resumes it under the
/// `BlockedExternalToolGate` precondition so the loop re-dispatches the parked
/// call and consumes the client output.
struct OpenAiCompatRuntimeExternalToolResume {
    coordinator: Arc<dyn TurnCoordinator>,
}

#[async_trait]
impl OpenAiCompatExternalToolResume for OpenAiCompatRuntimeExternalToolResume {
    async fn resume_external_tool_run(
        &self,
        request: OpenAiCompatExternalToolResumeRequest,
    ) -> Result<(), OpenAiCompatHttpError> {
        let OpenAiCompatExternalToolResumeRequest {
            actor_scope,
            run_ref,
            thread_id,
        } = request;
        let run_id = parse_turn_run_id(run_ref.as_str())?;
        let thread_id = ThreadId::new(thread_id).map_err(|_| OpenAiCompatHttpError::internal())?;
        let scope = openai_compat_resume_turn_scope(&actor_scope, thread_id);
        let deadline =
            tokio::time::Instant::now() + OPENAI_COMPAT_EXTERNAL_TOOL_RESUME_WAIT_TIMEOUT;
        let state = loop {
            let state = match self
                .coordinator
                .get_run_state(GetRunStateRequest {
                    scope: scope.clone(),
                    run_id,
                })
                .await
            {
                Ok(state) => state,
                Err(error) if error.category() == TurnErrorCategory::ScopeNotFound => {
                    return Err(OpenAiCompatHttpError::not_found(Some(
                        "previous_response_id".to_string(),
                    )));
                }
                Err(error) => return Err(map_resume_turn_error(error)),
            };
            if state.status == TurnStatus::BlockedExternalTool
                || state.status.is_terminal()
                || tokio::time::Instant::now() >= deadline
            {
                break state;
            }
            tokio::time::sleep(OPENAI_COMPAT_EXTERNAL_TOOL_RESUME_POLL_INTERVAL).await;
        };
        // Only resume a run actually parked on an external-tool gate. Completed
        // runs are accepted as idempotent replay; every other state fails closed
        // so unrelated or still-running responses cannot silently accept output.
        if state.status != TurnStatus::BlockedExternalTool {
            if state.status == TurnStatus::Completed {
                return Ok(());
            }
            return Err(OpenAiCompatHttpError::conflict(Some(
                "previous_response_id".to_string(),
            )));
        }
        let Some(gate_ref) = state.gate_ref.clone() else {
            tracing::error!(
                turn_run_id = %run_id,
                "openai compat external-tool resume found blocked run without gate ref"
            );
            return Err(OpenAiCompatHttpError::internal());
        };
        let Some(actor) = state.actor.clone() else {
            tracing::error!(
                turn_run_id = %run_id,
                "openai compat external-tool resume found blocked run without actor"
            );
            return Err(OpenAiCompatHttpError::internal());
        };
        let idempotency_key = openai_compat_external_tool_resume_idempotency_key(&gate_ref)?;
        self.coordinator
            .resume_turn(ResumeTurnRequest {
                scope: state.scope.clone(),
                actor,
                run_id,
                gate_resolution_ref: gate_ref,
                source_binding_ref: state.source_binding_ref.clone(),
                reply_target_binding_ref: state.reply_target_binding_ref.clone(),
                idempotency_key,
                precondition: ResumeTurnPrecondition::BlockedExternalToolGate,
                resume_disposition: None,
            })
            .await
            .map(|_| ())
            .map_err(map_resume_turn_error)
    }
}

fn openai_compat_resume_turn_scope(
    actor_scope: &OpenAiCompatActorScope,
    thread_id: ThreadId,
) -> TurnScope {
    // Mirror the inbound submit's scope (owner = caller user) so the
    // coordinator's exact-scope run-state lookup resolves.
    TurnScope::new_with_owner(
        actor_scope.tenant_id().clone(),
        actor_scope.agent_id().cloned(),
        actor_scope.project_id().cloned(),
        thread_id,
        Some(actor_scope.user_id().clone()),
    )
}

fn openai_compat_external_tool_resume_idempotency_key(
    gate_ref: &GateRef,
) -> Result<IdempotencyKey, OpenAiCompatHttpError> {
    const PREFIX: &str = "openai-compat-ext-resume-v1";

    // Key by structured, length-framed fields rather than raw concatenation so
    // later field additions cannot collide with an existing gate string.
    let digest = digest_idempotency_parts(&[
        b"openai-compat",
        b"external-tool-resume",
        b"v1",
        gate_ref.as_str().as_bytes(),
    ]);
    IdempotencyKey::new(format!("{PREFIX}-{digest}")).map_err(|_| OpenAiCompatHttpError::internal())
}

fn digest_idempotency_parts(parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part);
    }
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}

fn response_object(
    id: OpenAiResponseId,
    created_at: u64,
    model: String,
    status: OpenAiResponseStatus,
    output: Vec<OpenAiResponseOutputItem>,
    usage: Option<OpenAiResponseUsage>,
) -> OpenAiResponseObject {
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
        usage,
    }
}

/// Build the OpenAI-compatible `usage` object from a run's cumulative token
/// totals, pricing it (when the LLM cost table is compiled in) for the given
/// effective model. Token counts are always reported; `cost` is present only
/// under `root-llm-provider`.
fn response_usage_from_model_usage(usage: &LoopModelUsage, model: &str) -> OpenAiResponseUsage {
    // OpenAI reports total input (including cache) as `input_tokens`, with the
    // cached subset broken out under `input_tokens_details`. `cache_read` is
    // already a subset of `LoopModelUsage::input_tokens`, so it must NOT be
    // added again; `cache_creation` is a separate write-side count and is
    // added on top.
    let total_input = usage
        .input_tokens
        .saturating_add(usage.cache_creation_input_tokens);
    OpenAiResponseUsage {
        input_tokens: total_input,
        output_tokens: usage.output_tokens,
        total_tokens: total_input.saturating_add(usage.output_tokens),
        input_tokens_details: (usage.cache_read_input_tokens > 0).then_some(
            OpenAiResponseInputTokensDetails {
                cached_tokens: usage.cache_read_input_tokens,
            },
        ),
        cost: response_cost_from_model_usage(usage, model),
    }
}

/// Price a run's token usage in USD for the given model. Fresh input and
/// cache-creation tokens bill at the input rate; cache-read tokens bill at the
/// provider's cache-read discount; output at the output rate. Unknown models
/// fall back to the cost table's default (≈GPT-4o), so a new paid model never
/// silently prices at zero.
#[cfg(feature = "root-llm-provider")]
fn response_cost_from_model_usage(usage: &LoopModelUsage, model: &str) -> Option<OpenAiCompatCost> {
    use ironclaw_common::llm_costs::{format_usd, price_usage};

    let cost = price_usage(
        model,
        usage.input_tokens,
        usage.output_tokens,
        usage.cache_read_input_tokens,
        usage.cache_creation_input_tokens,
    );
    Some(OpenAiCompatCost {
        input_cost_usd: format_usd(cost.input_cost),
        cached_input_cost_usd: format_usd(cost.cached_input_cost),
        output_cost_usd: format_usd(cost.output_cost),
        total_cost_usd: format_usd(cost.total_cost),
        currency: OpenAiCompatCost::USD.to_string(),
    })
}

#[cfg(not(feature = "root-llm-provider"))]
fn response_cost_from_model_usage(
    _usage: &LoopModelUsage,
    _model: &str,
) -> Option<OpenAiCompatCost> {
    None
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
