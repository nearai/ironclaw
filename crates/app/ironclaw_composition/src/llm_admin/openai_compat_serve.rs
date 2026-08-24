// arch-exempt: large_file, §4.4.1 mechanical inline of the redundant local-root filesystem alias -> CompositeRootFilesystem (no logic change), plan #6168
//! Reborn host composition for OpenAI-compatible API routes.
//!
//! What is left here after the WS6 OpenAI-compat eviction (2026-08-05) is the
//! half that cannot leave: the **port implementations** that reach runtime
//! services. Every adapter below names `ironclaw_threads`, `ironclaw_turns` or
//! `ironclaw_event_streams` — all three on `ironclaw_openai_compat`'s own armed
//! `BoundaryRule` forbidden list, which is exactly the shape that rule exists
//! to require: the owner crate declares the port, the host implements it.
//!
//! The **assembly** moved to `ironclaw_openai_compat::mount`. This module now
//! builds the runtime-backed adapters, fills in `OpenAiCompatRouteMountPorts`,
//! and hands them over; the route crate owns the builder order, the shared
//! projection streamer, the `/v1/models` catalog, and the fail-closed rule for
//! a deployment with no LLM config.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

use ironclaw_assistant::{
    RebornTimelineRequest, TIMELINE_VIEW, UnboundTurnError, UnboundTurnService,
    UnboundTurnSubmission,
};
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::turn::{IdempotencyKey, TurnGateRef, TurnRunId, TurnScope, TurnStatus};
use ironclaw_host_api::{
    ids::{AgentId, ProjectId, TenantId, ThreadId},
    path::VirtualPath,
};
use ironclaw_loop_contracts::LoopModelUsage;
use ironclaw_openai_compat::OpenAiCompatRefStore;
use ironclaw_openai_compat::{
    OpenAiChatCompletionProjection, OpenAiChatCompletionProjectionReader,
    OpenAiChatCompletionProjectionRequest, OpenAiCompatActorScope, OpenAiCompatErrorKind,
    OpenAiCompatHttpError, OpenAiCompatRefStorePort, OpenAiCompatResourceBinding,
    OpenAiCompatResourceMapping, OpenAiCompatRouteMountPorts, OpenAiResponseErrorObject,
    OpenAiResponseId, OpenAiResponseObject, OpenAiResponseOutputItem,
    OpenAiResponseOutputItemStatus, OpenAiResponseProjection, OpenAiResponseReadRequest,
    OpenAiResponseStatus, OpenAiResponseUsage, OpenAiResponseWaitRequest,
    OpenAiResponsesMessageRole, OpenAiResponsesProjectionReader, openai_compat_route_mount,
    product_surface_caller_from_openai_scope,
};
use ironclaw_openai_compat::{OpenAiCompatCost, OpenAiResponseInputTokensDetails};
use ironclaw_openai_compat::{
    OpenAiCompatExternalToolResume, OpenAiCompatExternalToolResumeRequest,
    OpenAiCompatExternalToolSpec, OpenAiCompatExternalToolStore, OpenAiCompatTurnRunRef,
};
use ironclaw_product_contracts::inbound::ProductInboundAck;
use ironclaw_product_contracts::outbound::{
    ProductOutboundEnvelope, ProductOutboundPayload, ProductProjectionItem, ProductProjectionState,
    ProjectionCursor,
};
use ironclaw_product_contracts::projection::ProjectionStream;
use ironclaw_product_contracts::projection::{
    ProjectionReadRequest, ProjectionSubscriptionRequest,
};
use ironclaw_product_contracts::surface::{BoundProductSurface, ProductSurface};
use ironclaw_threads::{
    FinalizedAssistantMessageByRunRequest, LoadContextMessagesRequest, MessageKind, MessageStatus,
    ProviderToolCallReferenceEnvelope, SessionThreadError, SessionThreadService,
    ThreadHistoryRequest, ThreadMessageId, ThreadMessageRecord, ThreadScope,
    ToolResultReferenceEnvelope,
};
use ironclaw_turns::{
    ExternalToolCatalog, ExternalToolCatalogError, ExternalToolSpec, GetRunStateRequest,
    ResumeTurnPrecondition, ResumeTurnRequest, TurnCoordinator, TurnError, TurnErrorCategory,
};
use sha2::{Digest, Sha256};

use crate::RebornBuildError;
use crate::RebornRuntime;
use ironclaw_host_ingress::ProtectedRouteMount;

#[cfg(test)]
mod tests;

const OPENAI_COMPAT_PROJECTION_POLL_INTERVAL: Duration = Duration::from_millis(100);
const OPENAI_COMPAT_PENDING_EXTERNAL_TOOL_FALLBACK_DELAY: Duration = Duration::from_secs(10);
const OPENAI_COMPAT_EXTERNAL_TOOL_RESUME_POLL_INTERVAL: Duration = Duration::from_millis(50);
const OPENAI_COMPAT_EXTERNAL_TOOL_RESUME_WAIT_TIMEOUT: Duration = Duration::from_secs(5);

pub async fn build_openai_compat_route_mount(
    runtime: &RebornRuntime,
    tenant_id: TenantId,
    _default_agent_id: AgentId,
    _default_project_id: Option<ProjectId>,
) -> Result<ProtectedRouteMount, RebornBuildError> {
    let ref_filesystem: Arc<dyn RootFilesystem> = runtime.extension_filesystem.clone();
    let ref_store: Arc<dyn OpenAiCompatRefStorePort> = Arc::new(OpenAiCompatRefStore::with_root(
        ref_filesystem,
        openai_compat_ref_root(&tenant_id)?,
    ));
    let projection_stream = runtime.product_event_stream();
    let product_surface = runtime.product_surface(Some(projection_stream.clone()))?;
    // The prepared-context door shared by structured-output / tool-history
    // chat requests: same thread service and coordinator the runtime's own
    // turn path uses, scoped to the deployment's default agent/project axes
    // with the authenticated caller threaded through as the thread owner.
    let prepared_turn_gateway = Arc::new(OpenAiCompatPreparedTurnGateway {
        service: Arc::new(UnboundTurnService::new(
            runtime.product_thread_service(),
            runtime.product_turn_coordinator(),
            _default_agent_id.clone(),
            _default_project_id.clone(),
        )),
    });
    let chat_projection_reader = Arc::new(OpenAiChatCompletionThreadProjectionReader::new(
        product_surface.clone(),
        Arc::clone(&prepared_turn_gateway),
    ));
    // The external-tool catalog is the run-scoped seam shared with the loop
    // host: the host records parked calls + completes them from submitted
    // outputs; the Responses surface registers specs, submits outputs, and reads
    // parked calls back here.
    let external_tool_catalog = runtime.external_tool_catalog.clone();
    let responses_projection_reader = Arc::new(
        OpenAiResponsesThreadProjectionReader::new(
            runtime.product_thread_service(),
            projection_stream.clone(),
            external_tool_catalog.clone(),
        )
        .with_turn_coordinator(runtime.product_turn_coordinator()),
    );
    // Composition supplies the ports; the route crate owns the builder order,
    // the projection streamer, and the `/v1/models` catalog (WS6 eviction).
    Ok(openai_compat_route_mount(OpenAiCompatRouteMountPorts {
        product_surface,
        ref_store,
        chat_projection_reader,
        responses_projection_reader,
        external_tool_store: Arc::new(OpenAiCompatRuntimeExternalToolStore {
            catalog: external_tool_catalog,
        }),
        external_tool_resume: Arc::new(OpenAiCompatRuntimeExternalToolResume {
            coordinator: runtime.product_turn_coordinator(),
        }),
        llm_config: crate::product_surface::build_llm_config_service(runtime),
        prepared_turn_port: prepared_turn_gateway,
    }))
}

struct OpenAiChatCompletionThreadProjectionReader {
    product_surface: Arc<dyn ProductSurface>,
    poll_interval: Duration,
    /// Prepared-lane resolver: unbound runs live on stamped-hidden threads the
    /// caller-scoped timeline projection never surfaces, so their outcome is
    /// read from run state + the unbound thread directly.
    prepared_lane: Arc<OpenAiCompatPreparedTurnGateway>,
}

impl OpenAiChatCompletionThreadProjectionReader {
    fn new(
        product_surface: Arc<dyn ProductSurface>,
        prepared_lane: Arc<OpenAiCompatPreparedTurnGateway>,
    ) -> Self {
        Self {
            product_surface,
            poll_interval: OPENAI_COMPAT_PROJECTION_POLL_INTERVAL,
            prepared_lane,
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
        if request.prepared {
            return self
                .prepared_lane
                .wait_for_unbound_completion(&request, self.poll_interval)
                .await;
        }
        loop {
            let surface = BoundProductSurface::new(
                Arc::clone(&self.product_surface),
                product_surface_caller_from_openai_scope(&request.actor_scope),
            );
            let timeline = TIMELINE_VIEW
                .query_on(
                    &surface,
                    RebornTimelineRequest::new(
                        request.projection_read.scope.thread_id.as_str().to_string(),
                    ),
                    None,
                )
                .await?;
            match timeline.messages.into_iter().find(|message| {
                message.turn_run_id.as_deref() == Some(submitted_run_id.as_str())
                    && message.kind == MessageKind::Assistant
                    && message.status == MessageStatus::Finalized
            }) {
                Some(message) => {
                    return Ok(OpenAiChatCompletionProjection::text(
                        message.content.unwrap_or_default(),
                    ));
                }
                None => tokio::time::sleep(self.poll_interval).await,
            }
        }
    }
}

/// Thin adapter over the assistant-owned prepared-turn service: composition
/// owns only the wire-DTO mapping and error translation; accept/submit and
/// terminal read-back behavior live in
/// `ironclaw_assistant::UnboundTurnService`.
struct OpenAiCompatPreparedTurnGateway {
    service: Arc<UnboundTurnService>,
}

impl OpenAiCompatPreparedTurnGateway {
    async fn wait_for_unbound_completion(
        &self,
        request: &OpenAiChatCompletionProjectionRequest,
        poll_interval: Duration,
    ) -> Result<OpenAiChatCompletionProjection, OpenAiCompatHttpError> {
        let ProductInboundAck::Accepted {
            submitted_run_id, ..
        } = &request.accepted_ack
        else {
            return Err(OpenAiCompatHttpError::internal());
        };
        let outcome = self
            .service
            .wait_for_completion(
                request.public_id.as_str(),
                &product_surface_caller_from_openai_scope(&request.actor_scope),
                *submitted_run_id,
                poll_interval,
            )
            .await
            .map_err(map_prepared_turn_error)?;
        let mut projection = OpenAiChatCompletionProjection::text(outcome.text);
        // Run evidence: report the model that actually ran and the
        // provider-reported usage (unbound-turns design §4.3).
        projection.effective_model = outcome.effective_model;
        projection.usage = outcome
            .model_usage
            .map(|usage| chat_usage_from_model_usage(&usage));
        Ok(projection)
    }
}

fn map_prepared_turn_error(error: UnboundTurnError) -> OpenAiCompatHttpError {
    match error {
        UnboundTurnError::InvalidRequest { reason } => {
            OpenAiCompatHttpError::invalid_request(Some(reason))
        }
        UnboundTurnError::Unavailable => OpenAiCompatHttpError::from_kind(
            503,
            true,
            OpenAiCompatErrorKind::ServiceUnavailable,
            None,
        ),
        UnboundTurnError::RunFailed { category } => {
            OpenAiCompatHttpError::from_kind(502, false, OpenAiCompatErrorKind::Internal, category)
        }
        UnboundTurnError::RunCancelled => OpenAiCompatHttpError::from_kind(
            409,
            false,
            OpenAiCompatErrorKind::Conflict,
            Some("the run was cancelled".to_string()),
        ),
        UnboundTurnError::Internal { reason } => {
            // Operator diagnostics; the wire stays a bare 500.
            tracing::debug!(%reason, "unbound turn internal error");
            OpenAiCompatHttpError::internal()
        }
    }
}

#[async_trait]
impl ironclaw_openai_compat::OpenAiCompatPreparedTurnPort for OpenAiCompatPreparedTurnGateway {
    async fn accept_and_submit(
        &self,
        request: ironclaw_openai_compat::OpenAiCompatPreparedTurnRequest,
    ) -> Result<ProductInboundAck, OpenAiCompatHttpError> {
        let idempotency_key = format!("openai-chat:{}", request.public_id);
        self.service
            .accept_and_submit(UnboundTurnSubmission {
                caller: product_surface_caller_from_openai_scope(&request.scope),
                public_id: request.public_id,
                system_prompt: request.system_prompt,
                messages: request.messages,
                tools: Vec::new(),
                // The OpenAI-compatible chat lane keeps its normal surface:
                // it is a caller-driven API, not an unattended run.
                require_no_approval: false,
                output: request.output,
                requested_model: request.requested_model,
                idempotency_key,
            })
            .await
            .map_err(map_prepared_turn_error)
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
                target: "ironclaw::reborn::openai_compat",
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
    gate_ref: &TurnGateRef,
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

/// Total prompt/input tokens including cache-creation writes. `cache_read` is
/// already a subset of `LoopModelUsage::input_tokens` and must NOT be added
/// again; `cache_creation` is a separate write-side count that OpenAI folds
/// into the reported input/prompt total. Shared chokepoint for both
/// OpenAI-compatible usage shapes (Responses `input_tokens` and Chat
/// Completions `prompt_tokens`) so this accounting has exactly one
/// definition.
fn total_prompt_tokens_with_cache_creation(usage: &LoopModelUsage) -> u32 {
    usage
        .input_tokens
        .saturating_add(usage.cache_creation_input_tokens)
}

/// Build the Chat Completions `usage` object from a run's cumulative token
/// totals (unbound-turns prepared-chat lane, §4.3). Mirrors
/// `response_usage_from_model_usage`'s cache-creation accounting through the
/// shared `total_prompt_tokens_with_cache_creation` chokepoint so the two
/// OpenAI-compatible wire shapes never drift.
fn chat_usage_from_model_usage(usage: &LoopModelUsage) -> ironclaw_openai_compat::OpenAiUsage {
    let prompt_tokens = total_prompt_tokens_with_cache_creation(usage);
    ironclaw_openai_compat::OpenAiUsage {
        prompt_tokens,
        completion_tokens: usage.output_tokens,
        total_tokens: prompt_tokens.saturating_add(usage.output_tokens),
        prompt_tokens_details: (usage.cache_read_input_tokens > 0).then_some(
            ironclaw_openai_compat::OpenAiPromptTokensDetails {
                cached_tokens: usage.cache_read_input_tokens,
            },
        ),
        cost: None,
    }
}

/// Build the OpenAI-compatible `usage` object from a run's cumulative token
/// totals, pricing it for the given effective model.
fn response_usage_from_model_usage(usage: &LoopModelUsage, model: &str) -> OpenAiResponseUsage {
    let total_input = total_prompt_tokens_with_cache_creation(usage);
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

fn openai_compat_ref_root(tenant_id: &TenantId) -> Result<VirtualPath, RebornBuildError> {
    Ok(VirtualPath::new(format!(
        "/tenants/{}/shared/openai_compat/refs",
        tenant_id.as_str()
    ))?)
}
