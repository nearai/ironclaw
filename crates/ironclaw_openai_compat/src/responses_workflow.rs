//! ProductSurface-backed Responses route service.
//!
//! This slice routes Responses create/cancel through the ProductSurface facade,
//! resolves retrieve through a composition-supplied projection reader, and
//! translates projection-backed streaming creates into OpenAI-compatible SSE.
//! The ack and text helpers intentionally mirror the chat slice until the two
//! surfaces share a crate-private normalization module.

use std::sync::Arc;
use std::time::Duration;

use crate::ack_helpers::{internal_refs_from_ack, product_ack_from_ironclaw_submit};
use crate::content_parts::{
    content_array_item_text, non_text_part_marker, sanitize_product_text_fragment,
};
use crate::error::product_rejection_to_openai_error;
use crate::external_tools::parse_external_tools;
use crate::projection_helpers::{
    ensure_projection_read_matches_caller, ensure_projection_subscription_matches_caller,
};
use crate::{
    OpenAiCompatActorScope, OpenAiCompatAuthenticatedCaller, OpenAiCompatBindInternalRefs,
    OpenAiCompatExternalToolResume, OpenAiCompatExternalToolResumeRequest,
    OpenAiCompatExternalToolSpec, OpenAiCompatExternalToolStore, OpenAiCompatHttpError,
    OpenAiCompatIdempotencyKey, OpenAiCompatInternalRefs,
    OpenAiCompatMarkExternalToolResumeCompleted, OpenAiCompatProjectionRef,
    OpenAiCompatProjectionStreamer, OpenAiCompatPublicId, OpenAiCompatRecordAcceptedAck,
    OpenAiCompatRefLookup, OpenAiCompatRefOperation, OpenAiCompatRefReservation,
    OpenAiCompatRefReservationOutcome, OpenAiCompatRefStore, OpenAiCompatRequestFingerprint,
    OpenAiCompatResourceBinding, OpenAiCompatResourceMapping, OpenAiCompatRouteSurface,
    OpenAiCompatTurnRunRef, OpenAiResponseId, OpenAiResponseObject,
    OpenAiResponseProjectionStreamRequest, OpenAiResponsesCreateRequest, OpenAiResponsesInput,
    OpenAiResponsesInputItem, OpenAiResponsesMessageRole,
};
use async_trait::async_trait;
use axum::Json;
use axum::response::{IntoResponse, Response};
use ironclaw_host_api::ThreadId;
use ironclaw_product_adapters::{
    ProductInboundAck, ProductRejection, ProductTriggerReason, ProjectionReadRequest,
    ProjectionSubscriptionRequest, UserMessagePayload,
};
use ironclaw_product_workflow::{
    ProductSurface, WebUiCancelRunRequest, WebUiCreateThreadRequest, WebUiSendMessageRequest,
};

const DEFAULT_RESPONSES_WAIT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_BIND_INTERNAL_REFS_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_RESPONSES_BODY_BYTES: usize = 4 * 1024 * 1024;
const MAX_RESPONSES_CONTEXT_BYTES: usize = 10 * 1024;
const MAX_RESPONSES_INPUT_ITEMS: usize = 1_000;
#[derive(Clone)]
pub struct OpenAiResponsesWorkflow {
    product_surface: Arc<dyn ProductSurface>,
    ref_store: Arc<dyn OpenAiCompatRefStore>,
    projection_reader: Arc<dyn OpenAiResponsesProjectionReader>,
    /// Wired by host composition when OpenAI-compatible streaming is enabled.
    /// When `None`, `stream: true` requests fail closed.
    /// arch-exempt: optional Arc, streaming is a staged #4446 capability layered
    /// onto the non-streaming #4445 workflow.
    projection_streamer: Option<Arc<dyn OpenAiCompatProjectionStreamer>>,
    /// Wired by host composition when client-supplied ("external") tools are
    /// enabled. When `None`, `tools`/`tool_choice` and `function_call_output`
    /// resume inputs fail closed with a stable `400`.
    /// arch-exempt: optional_arc, external tools are a staged #4447 capability the
    /// binary may legitimately ship without; absence is a real fail-closed mode.
    external_tool_store: Option<Arc<dyn OpenAiCompatExternalToolStore>>,
    /// Wired alongside `external_tool_store`; resumes a parked external-tool run
    /// once its client outputs are submitted.
    /// arch-exempt: optional_arc, paired with external_tool_store (same #4447 gate).
    external_tool_resume: Option<Arc<dyn OpenAiCompatExternalToolResume>>,
    wait_timeout: Duration,
}

impl OpenAiResponsesWorkflow {
    pub fn new(
        product_surface: Arc<dyn ProductSurface>,
        ref_store: Arc<dyn OpenAiCompatRefStore>,
        projection_reader: Arc<dyn OpenAiResponsesProjectionReader>,
    ) -> Self {
        Self {
            product_surface,
            ref_store,
            projection_reader,
            projection_streamer: None,
            external_tool_store: None,
            external_tool_resume: None,
            wait_timeout: DEFAULT_RESPONSES_WAIT_TIMEOUT,
        }
    }

    pub fn with_wait_timeout(mut self, wait_timeout: Duration) -> Self {
        self.wait_timeout = wait_timeout;
        self
    }

    pub fn with_projection_streamer(
        mut self,
        projection_streamer: Arc<dyn OpenAiCompatProjectionStreamer>,
    ) -> Self {
        self.projection_streamer = Some(projection_streamer);
        self
    }

    /// Enable client-supplied ("external") tools: register specs at submit,
    /// submit client outputs, and resume parked runs. Both ports must be wired
    /// together; wiring only one leaves the surface fail-closed on `tools`.
    pub fn with_external_tools(
        mut self,
        store: Arc<dyn OpenAiCompatExternalToolStore>,
        resume: Arc<dyn OpenAiCompatExternalToolResume>,
    ) -> Self {
        self.external_tool_store = Some(store);
        self.external_tool_resume = Some(resume);
        self
    }

    fn external_tools_enabled(&self) -> bool {
        self.external_tool_store.is_some() && self.external_tool_resume.is_some()
    }

    pub async fn create_response(
        &self,
        caller: OpenAiCompatAuthenticatedCaller,
        raw_body: &[u8],
        idempotency_key: Option<OpenAiCompatIdempotencyKey>,
        surface: OpenAiCompatRouteSurface,
    ) -> Result<OpenAiResponseObject, OpenAiCompatHttpError> {
        let request = parse_response_create_request(raw_body)?;
        self.create_response_request(caller, request, raw_body, idempotency_key, surface)
            .await
    }

    pub(crate) async fn handle_response_create_request(
        &self,
        caller: OpenAiCompatAuthenticatedCaller,
        request: OpenAiResponsesCreateRequest,
        raw_body: &[u8],
        idempotency_key: Option<OpenAiCompatIdempotencyKey>,
        surface: OpenAiCompatRouteSurface,
    ) -> Result<Response, OpenAiCompatHttpError> {
        if request.stream.unwrap_or(false) {
            return self
                .stream_response_request(caller, request, raw_body, idempotency_key, surface)
                .await;
        }
        self.create_response_request(caller, request, raw_body, idempotency_key, surface)
            .await
            .map(|response| Json(response).into_response())
    }

    async fn create_response_request(
        &self,
        caller: OpenAiCompatAuthenticatedCaller,
        request: OpenAiResponsesCreateRequest,
        raw_body: &[u8],
        idempotency_key: Option<OpenAiCompatIdempotencyKey>,
        surface: OpenAiCompatRouteSurface,
    ) -> Result<OpenAiResponseObject, OpenAiCompatHttpError> {
        self.validate_responses_request(&request)?;

        let previous_mapping = if let Some(previous_response_id) = &request.previous_response_id {
            Some(
                self.lookup_response_mapping(
                    caller.scope(),
                    previous_response_id.clone(),
                    OpenAiCompatRefOperation::Retrieve,
                )
                .await?,
            )
        } else {
            None
        };

        // A continuation that carries `function_call_output` for a prior parked
        // run resumes that run rather than submitting a new turn (the parked run
        // still holds the thread's active lock, so a fresh submit would be
        // rejected busy). Requires external tools wired and the prior response.
        if self.external_tools_enabled()
            && request_has_function_call_output(&request)
            && let Some(previous_mapping) = previous_mapping.as_ref()
        {
            return self
                .resume_response_request(
                    &caller,
                    &request,
                    raw_body,
                    idempotency_key,
                    surface,
                    previous_mapping,
                )
                .await;
        }

        let external_tool_specs = self.parse_request_external_tools(&request)?;
        let user_message_payload = responses_user_message_payload(&request)?;
        let request_fingerprint = OpenAiCompatRequestFingerprint::from_body_bytes(raw_body);
        let reservation = self
            .ref_store
            .reserve(OpenAiCompatRefReservation::new(
                caller.scope().clone(),
                surface,
                request_fingerprint,
                idempotency_key,
            ))
            .await?;
        let (mapping, accepted_ack) = match reservation {
            OpenAiCompatRefReservationOutcome::Created(mapping) => {
                let public_id = response_public_id(&mapping)?;
                let (mapping, accepted_ack) = self
                    .submit_response_and_record_ack(
                        &caller,
                        &public_id,
                        previous_mapping.as_ref(),
                        user_message_payload,
                    )
                    .await?;
                let mapping = self
                    .ensure_response_mapping_bound(
                        caller.scope().clone(),
                        public_id,
                        mapping,
                        &accepted_ack,
                    )
                    .await?;
                (mapping, accepted_ack)
            }
            OpenAiCompatRefReservationOutcome::Replayed(mapping) => {
                let public_id = response_public_id(&mapping)?;
                if let Some(accepted_ack) = mapping.accepted_ack.clone() {
                    let mapping = self
                        .ensure_response_mapping_bound(
                            caller.scope().clone(),
                            public_id.clone(),
                            mapping,
                            &accepted_ack,
                        )
                        .await?;
                    self.register_external_tools(&mapping, &external_tool_specs)
                        .await?;
                    let projection_read = self
                        .response_projection_read_request(
                            &caller,
                            &mapping,
                            previous_mapping.as_ref(),
                        )
                        .await?;
                    let mapping = self
                        .ensure_response_projection_ref(
                            caller.scope().clone(),
                            public_id.clone(),
                            mapping,
                            &projection_read,
                        )
                        .await?;
                    return self
                        .projection_reader
                        .read_response(OpenAiResponseReadRequest {
                            public_id,
                            actor_scope: caller.scope().clone(),
                            requested_model: Some(request.model.clone()),
                            projection_read,
                            mapping,
                        })
                        .await;
                }
                let (mapping, accepted_ack) = self
                    .submit_response_and_record_ack(
                        &caller,
                        &public_id,
                        previous_mapping.as_ref(),
                        user_message_payload,
                    )
                    .await?;
                let mapping = self
                    .ensure_response_mapping_bound(
                        caller.scope().clone(),
                        public_id,
                        mapping,
                        &accepted_ack,
                    )
                    .await?;
                (mapping, accepted_ack)
            }
            OpenAiCompatRefReservationOutcome::Conflict(_) => {
                return Err(OpenAiCompatHttpError::conflict(Some(
                    "idempotency_key".to_string(),
                )));
            }
        };
        let public_id = response_public_id(&mapping)?;
        // Register the run's client tools right after submit so the model is
        // offered them on its first planning step. The submit only enqueues the
        // turn; this in-memory register lands before the loop resolves its
        // capability surface. No-op when the request declared no tools.
        self.register_external_tools(&mapping, &external_tool_specs)
            .await?;
        let projection_read = self
            .response_projection_read_request(&caller, &mapping, previous_mapping.as_ref())
            .await?;
        let mapping = self
            .ensure_response_projection_ref(
                caller.scope().clone(),
                public_id.clone(),
                mapping,
                &projection_read,
            )
            .await?;

        let wait_result = tokio::time::timeout(
            self.wait_timeout,
            self.projection_reader
                .wait_for_response_completion(OpenAiResponseWaitRequest {
                    public_id: public_id.clone(),
                    actor_scope: caller.scope().clone(),
                    accepted_ack: Some(accepted_ack),
                    requested_model: request.model.clone(),
                    projection_read: projection_read.clone(),
                    mapping,
                }),
        )
        .await
        .map_err(|_| {
            OpenAiCompatHttpError::from_kind(
                503,
                true,
                crate::OpenAiCompatErrorKind::ServiceUnavailable,
                None,
            )
        })??;

        if let Some(mut internal_refs) = wait_result.internal_refs {
            internal_refs.projection_ref = Some(projection_ref_from_thread_id(
                &projection_read.scope.thread_id,
            )?);
            self.bind_internal_refs(caller.scope().clone(), public_id, internal_refs)
                .await?;
        }

        Ok(wait_result.response)
    }

    pub async fn stream_response(
        &self,
        caller: OpenAiCompatAuthenticatedCaller,
        raw_body: &[u8],
        idempotency_key: Option<OpenAiCompatIdempotencyKey>,
        surface: OpenAiCompatRouteSurface,
    ) -> Result<Response, OpenAiCompatHttpError> {
        let request = parse_response_create_request(raw_body)?;
        self.stream_response_request(caller, request, raw_body, idempotency_key, surface)
            .await
    }

    async fn stream_response_request(
        &self,
        caller: OpenAiCompatAuthenticatedCaller,
        request: OpenAiResponsesCreateRequest,
        raw_body: &[u8],
        idempotency_key: Option<OpenAiCompatIdempotencyKey>,
        surface: OpenAiCompatRouteSurface,
    ) -> Result<Response, OpenAiCompatHttpError> {
        let projection_streamer = self
            .projection_streamer
            .clone()
            .ok_or_else(OpenAiCompatHttpError::not_wired)?;
        self.validate_responses_stream_request(&request)?;

        let previous_mapping = if let Some(previous_response_id) = &request.previous_response_id {
            Some(
                self.lookup_response_mapping(
                    caller.scope(),
                    previous_response_id.clone(),
                    OpenAiCompatRefOperation::Retrieve,
                )
                .await?,
            )
        } else {
            None
        };

        let external_tool_specs = self.parse_request_external_tools(&request)?;
        let user_message_payload = responses_user_message_payload(&request)?;
        let request_fingerprint = OpenAiCompatRequestFingerprint::from_body_bytes(raw_body);
        let reservation = self
            .ref_store
            .reserve(OpenAiCompatRefReservation::new(
                caller.scope().clone(),
                surface,
                request_fingerprint,
                idempotency_key,
            ))
            .await?;
        let (mapping, accepted_ack) = match reservation {
            OpenAiCompatRefReservationOutcome::Created(mapping) => {
                let public_id = response_public_id(&mapping)?;
                let (mapping, accepted_ack) = self
                    .submit_response_and_record_ack(
                        &caller,
                        &public_id,
                        previous_mapping.as_ref(),
                        user_message_payload,
                    )
                    .await?;
                let mapping = self
                    .ensure_response_mapping_bound(
                        caller.scope().clone(),
                        public_id,
                        mapping,
                        &accepted_ack,
                    )
                    .await?;
                (mapping, accepted_ack)
            }
            OpenAiCompatRefReservationOutcome::Replayed(mapping) => {
                let public_id = response_public_id(&mapping)?;
                if let Some(accepted_ack) = mapping.accepted_ack.clone() {
                    let mapping = self
                        .ensure_response_mapping_bound(
                            caller.scope().clone(),
                            public_id,
                            mapping,
                            &accepted_ack,
                        )
                        .await?;
                    (mapping, accepted_ack)
                } else {
                    let (mapping, accepted_ack) = self
                        .submit_response_and_record_ack(
                            &caller,
                            &public_id,
                            previous_mapping.as_ref(),
                            user_message_payload,
                        )
                        .await?;
                    let mapping = self
                        .ensure_response_mapping_bound(
                            caller.scope().clone(),
                            public_id,
                            mapping,
                            &accepted_ack,
                        )
                        .await?;
                    (mapping, accepted_ack)
                }
            }
            OpenAiCompatRefReservationOutcome::Conflict(_) => {
                return Err(OpenAiCompatHttpError::conflict(Some(
                    "idempotency_key".to_string(),
                )));
            }
        };
        let public_id = response_public_id(&mapping)?;
        self.register_external_tools(&mapping, &external_tool_specs)
            .await?;
        let projection_subscription = self
            .response_projection_subscription_request(&caller, &mapping, previous_mapping.as_ref())
            .await?;
        let mapping = self
            .ensure_response_projection_thread_ref(
                caller.scope().clone(),
                public_id.clone(),
                mapping,
                &projection_subscription.scope.thread_id,
            )
            .await?;

        Ok(crate::streaming::response_sse_response(
            projection_streamer,
            OpenAiResponseProjectionStreamRequest {
                public_id,
                actor_scope: caller.scope().clone(),
                accepted_ack,
                requested_model: request.model,
                projection_subscription,
                mapping,
                wait_timeout: self.wait_timeout,
                after_cursor: None,
            },
        ))
    }

    pub async fn retrieve_response(
        &self,
        caller: OpenAiCompatAuthenticatedCaller,
        response_id: OpenAiResponseId,
    ) -> Result<OpenAiResponseObject, OpenAiCompatHttpError> {
        let mapping = self
            .lookup_response_mapping(
                caller.scope(),
                response_id.clone(),
                OpenAiCompatRefOperation::Retrieve,
            )
            .await?;
        self.projection_reader
            .read_response(OpenAiResponseReadRequest {
                public_id: response_id,
                actor_scope: caller.scope().clone(),
                requested_model: None,
                projection_read: self
                    .response_projection_read_request(&caller, &mapping, None)
                    .await?,
                mapping,
            })
            .await
    }

    pub async fn cancel_response(
        &self,
        caller: OpenAiCompatAuthenticatedCaller,
        response_id: OpenAiResponseId,
    ) -> Result<OpenAiResponseObject, OpenAiCompatHttpError> {
        let mapping = self
            .lookup_response_mapping(
                caller.scope(),
                response_id.clone(),
                OpenAiCompatRefOperation::Cancel,
            )
            .await?;
        let projection_read = self
            .response_projection_read_request(&caller, &mapping, None)
            .await?;
        let run_ref = response_turn_run_ref(&mapping)?;
        let thread_id = projection_thread_id(&mapping)?
            .ok_or_else(|| OpenAiCompatHttpError::conflict(Some("response_id".to_string())))?;
        self.product_surface
            .cancel_run(
                caller.product_surface_caller(),
                WebUiCancelRunRequest {
                    client_action_id: Some(format!("{}:cancel", response_id.as_str())),
                    thread_id: Some(thread_id.as_str().to_string()),
                    run_id: Some(run_ref.as_str().to_string()),
                    reason: Some("cancelled by OpenAI-compatible Responses API".to_string()),
                },
            )
            .await?;

        self.projection_reader
            .read_response(OpenAiResponseReadRequest {
                public_id: response_id,
                actor_scope: caller.scope().clone(),
                requested_model: None,
                projection_read,
                mapping,
            })
            .await
    }

    async fn lookup_response_mapping(
        &self,
        scope: &OpenAiCompatActorScope,
        response_id: OpenAiResponseId,
        operation: OpenAiCompatRefOperation,
    ) -> Result<OpenAiCompatResourceMapping, OpenAiCompatHttpError> {
        self.ref_store
            .lookup_authorized(OpenAiCompatRefLookup::new(
                scope.clone(),
                OpenAiCompatPublicId::Response(response_id),
                operation,
            ))
            .await?
            .ok_or_else(|| OpenAiCompatHttpError::not_found(Some("response_id".to_string())))
    }

    async fn submit_response_and_record_ack(
        &self,
        caller: &OpenAiCompatAuthenticatedCaller,
        public_id: &OpenAiResponseId,
        previous_mapping: Option<&OpenAiCompatResourceMapping>,
        user_message_payload: UserMessagePayload,
    ) -> Result<(OpenAiCompatResourceMapping, ProductInboundAck), OpenAiCompatHttpError> {
        let thread_id = self
            .response_thread_id_from_previous_or_public(public_id, previous_mapping)
            .map_err(|_| OpenAiCompatHttpError::internal())?;
        self.ensure_response_thread(caller, &thread_id).await?;
        let ack = product_ack_from_ironclaw_submit(
            self.product_surface
                .submit_turn(
                    caller.product_surface_caller(),
                    response_surface_submit_request(public_id, &thread_id, user_message_payload),
                )
                .await?,
        );
        let accepted_ack = accepted_ack_from_ack(ack)?;
        // Persist accepted acks for both streaming and non-streaming creates so
        // idempotency replay can reuse the canonical product turn without
        // submitting another inbound request.
        let mapping = self
            .ref_store
            .record_accepted_ack(OpenAiCompatRecordAcceptedAck::new(
                caller.scope().clone(),
                OpenAiCompatPublicId::Response(public_id.clone()),
                accepted_ack.clone(),
            ))
            .await?
            .ok_or_else(|| OpenAiCompatHttpError::not_found(Some("response_id".to_string())))?;
        Ok((mapping, accepted_ack))
    }

    async fn ensure_response_mapping_bound(
        &self,
        owner: OpenAiCompatActorScope,
        public_id: OpenAiResponseId,
        mapping: OpenAiCompatResourceMapping,
        accepted_ack: &ProductInboundAck,
    ) -> Result<OpenAiCompatResourceMapping, OpenAiCompatHttpError> {
        if matches!(mapping.binding, OpenAiCompatResourceBinding::Bound { .. }) {
            return Ok(mapping);
        }
        let internal_refs = internal_refs_from_ack(accepted_ack)?;
        self.bind_internal_refs(owner, public_id, internal_refs)
            .await?
            .ok_or_else(bind_internal_refs_unavailable)
    }

    async fn bind_internal_refs(
        &self,
        owner: OpenAiCompatActorScope,
        public_id: OpenAiResponseId,
        internal_refs: OpenAiCompatInternalRefs,
    ) -> Result<Option<OpenAiCompatResourceMapping>, OpenAiCompatHttpError> {
        match tokio::time::timeout(
            DEFAULT_BIND_INTERNAL_REFS_TIMEOUT,
            self.ref_store
                .bind_internal_refs(OpenAiCompatBindInternalRefs::new(
                    owner,
                    OpenAiCompatPublicId::Response(public_id.clone()),
                    internal_refs,
                )),
        )
        .await
        {
            Ok(result) => result.map_err(Into::into),
            Err(_) => {
                tracing::warn!(
                    public_id = public_id.as_str(),
                    "bind_internal_refs timed out; continuing without binding"
                );
                Ok(None)
            }
        }
    }

    async fn ensure_response_projection_ref(
        &self,
        owner: OpenAiCompatActorScope,
        public_id: OpenAiResponseId,
        mapping: OpenAiCompatResourceMapping,
        projection_read: &ProjectionReadRequest,
    ) -> Result<OpenAiCompatResourceMapping, OpenAiCompatHttpError> {
        self.ensure_response_projection_thread_ref(
            owner,
            public_id,
            mapping,
            &projection_read.scope.thread_id,
        )
        .await
    }

    async fn ensure_response_projection_thread_ref(
        &self,
        owner: OpenAiCompatActorScope,
        public_id: OpenAiResponseId,
        mapping: OpenAiCompatResourceMapping,
        thread_id: &ThreadId,
    ) -> Result<OpenAiCompatResourceMapping, OpenAiCompatHttpError> {
        let Some(mut internal_refs) = mapping.binding.internal_refs().cloned() else {
            return Ok(mapping);
        };
        if internal_refs.projection_ref.is_some() {
            return Ok(mapping);
        }
        internal_refs.projection_ref = Some(projection_ref_from_thread_id(thread_id)?);
        self.bind_internal_refs(owner, public_id, internal_refs)
            .await?
            .ok_or_else(bind_internal_refs_unavailable)
    }

    async fn response_projection_read_request(
        &self,
        caller: &OpenAiCompatAuthenticatedCaller,
        mapping: &OpenAiCompatResourceMapping,
        previous_mapping: Option<&OpenAiCompatResourceMapping>,
    ) -> Result<ProjectionReadRequest, OpenAiCompatHttpError> {
        let projection_read = self.response_projection_read(caller, mapping, previous_mapping)?;
        ensure_projection_read_matches_caller(caller, &projection_read)?;
        Ok(projection_read)
    }

    async fn response_projection_subscription_request(
        &self,
        caller: &OpenAiCompatAuthenticatedCaller,
        mapping: &OpenAiCompatResourceMapping,
        previous_mapping: Option<&OpenAiCompatResourceMapping>,
    ) -> Result<ProjectionSubscriptionRequest, OpenAiCompatHttpError> {
        let projection_subscription =
            self.response_projection_subscription(caller, mapping, previous_mapping)?;
        ensure_projection_subscription_matches_caller(caller, &projection_subscription)?;
        Ok(projection_subscription)
    }

    fn response_projection_read(
        &self,
        caller: &OpenAiCompatAuthenticatedCaller,
        mapping: &OpenAiCompatResourceMapping,
        previous_mapping: Option<&OpenAiCompatResourceMapping>,
    ) -> Result<ProjectionReadRequest, OpenAiCompatHttpError> {
        let thread_id = self.response_thread_id(mapping, previous_mapping)?;
        let surface_caller = caller.product_surface_caller();
        Ok(ProjectionReadRequest {
            actor: surface_caller.actor(),
            scope: surface_caller.turn_scope(thread_id),
            after_cursor: None,
            limit: None,
        })
    }

    fn response_projection_subscription(
        &self,
        caller: &OpenAiCompatAuthenticatedCaller,
        mapping: &OpenAiCompatResourceMapping,
        previous_mapping: Option<&OpenAiCompatResourceMapping>,
    ) -> Result<ProjectionSubscriptionRequest, OpenAiCompatHttpError> {
        let thread_id = self.response_thread_id(mapping, previous_mapping)?;
        let surface_caller = caller.product_surface_caller();
        Ok(ProjectionSubscriptionRequest {
            actor: surface_caller.actor(),
            scope: surface_caller.turn_scope(thread_id),
            after_cursor: None,
        })
    }

    async fn ensure_response_thread(
        &self,
        caller: &OpenAiCompatAuthenticatedCaller,
        thread_id: &ThreadId,
    ) -> Result<(), OpenAiCompatHttpError> {
        self.product_surface
            .create_thread(
                caller.product_surface_caller(),
                WebUiCreateThreadRequest {
                    client_action_id: Some(thread_id.as_str().to_string()),
                    requested_thread_id: Some(thread_id.as_str().to_string()),
                    project_id: None,
                },
            )
            .await?;
        Ok(())
    }

    fn response_thread_id(
        &self,
        mapping: &OpenAiCompatResourceMapping,
        previous_mapping: Option<&OpenAiCompatResourceMapping>,
    ) -> Result<ThreadId, OpenAiCompatHttpError> {
        if let Some(thread_id) = projection_thread_id(mapping)? {
            return Ok(thread_id);
        }
        let public_id = response_public_id(mapping)?;
        self.response_thread_id_from_previous_or_public(&public_id, previous_mapping)
    }

    fn response_thread_id_from_previous_or_public(
        &self,
        public_id: &OpenAiResponseId,
        previous_mapping: Option<&OpenAiCompatResourceMapping>,
    ) -> Result<ThreadId, OpenAiCompatHttpError> {
        if let Some(mapping) = previous_mapping {
            if let Some(thread_id) = projection_thread_id(mapping)? {
                return Ok(thread_id);
            }
            return ThreadId::new(mapping.public_id.as_str().to_string())
                .map_err(|_| OpenAiCompatHttpError::internal());
        }
        ThreadId::new(public_id.as_str().to_string()).map_err(|_| OpenAiCompatHttpError::internal())
    }

    fn validate_responses_request(
        &self,
        request: &OpenAiResponsesCreateRequest,
    ) -> Result<(), OpenAiCompatHttpError> {
        if request.stream.unwrap_or(false) {
            return Err(OpenAiCompatHttpError::invalid_request(Some(
                "stream".to_string(),
            )));
        }
        if self.external_tools_enabled() {
            return validate_responses_supported_fields_with_external_tools(request);
        }
        validate_responses_supported_fields(request)
    }

    fn validate_responses_stream_request(
        &self,
        request: &OpenAiResponsesCreateRequest,
    ) -> Result<(), OpenAiCompatHttpError> {
        if !request.stream.unwrap_or(false) {
            return Err(OpenAiCompatHttpError::invalid_request(Some(
                "stream".to_string(),
            )));
        }
        if self.external_tools_enabled() {
            if request_has_function_call_output(request) {
                return Err(OpenAiCompatHttpError::invalid_request(Some(
                    "input".to_string(),
                )));
            }
            return validate_responses_supported_fields_with_external_tools(request);
        }
        validate_responses_supported_fields(request)
    }

    fn parse_request_external_tools(
        &self,
        request: &OpenAiResponsesCreateRequest,
    ) -> Result<Vec<OpenAiCompatExternalToolSpec>, OpenAiCompatHttpError> {
        let Some(tools) = request.tools.as_ref().filter(|tools| !tools.is_empty()) else {
            return Ok(Vec::new());
        };
        if !self.external_tools_enabled() {
            return Err(OpenAiCompatHttpError::invalid_request(Some(
                "tools".to_string(),
            )));
        }
        parse_external_tools(tools)
    }

    async fn register_external_tools(
        &self,
        mapping: &OpenAiCompatResourceMapping,
        specs: &[OpenAiCompatExternalToolSpec],
    ) -> Result<(), OpenAiCompatHttpError> {
        if specs.is_empty() {
            return Ok(());
        }
        let Some(store) = self.external_tool_store.as_ref() else {
            return Err(OpenAiCompatHttpError::invalid_request(Some(
                "tools".to_string(),
            )));
        };
        let run_ref = response_turn_run_ref(mapping)?;
        store.register_tools(run_ref, specs.to_vec()).await
    }

    /// Resume a parked external-tool run from a continuation request carrying
    /// `function_call_output`. Reserves a continuation response id bound to the
    /// same run/thread as the parked response, submits the client outputs, and
    /// resumes the run, then waits for it to complete (or park on a further call).
    async fn resume_response_request(
        &self,
        caller: &OpenAiCompatAuthenticatedCaller,
        request: &OpenAiResponsesCreateRequest,
        raw_body: &[u8],
        idempotency_key: Option<OpenAiCompatIdempotencyKey>,
        surface: OpenAiCompatRouteSurface,
        previous_mapping: &OpenAiCompatResourceMapping,
    ) -> Result<OpenAiResponseObject, OpenAiCompatHttpError> {
        let request_fingerprint = OpenAiCompatRequestFingerprint::from_body_bytes(raw_body);
        let reservation = self
            .ref_store
            .reserve(OpenAiCompatRefReservation::new(
                caller.scope().clone(),
                surface,
                request_fingerprint,
                idempotency_key,
            ))
            .await?;
        let mapping = match reservation {
            OpenAiCompatRefReservationOutcome::Created(mapping) => {
                self.bind_and_drive_resume(caller, request, previous_mapping, mapping, false)
                    .await?
            }
            OpenAiCompatRefReservationOutcome::Replayed(mapping) => {
                if mapping.external_tool_resume_completed {
                    mapping
                } else {
                    self.bind_and_drive_resume(caller, request, previous_mapping, mapping, true)
                        .await?
                }
            }
            OpenAiCompatRefReservationOutcome::Conflict(_) => {
                return Err(OpenAiCompatHttpError::conflict(Some(
                    "idempotency_key".to_string(),
                )));
            }
        };

        let public_id = response_public_id(&mapping)?;
        let projection_read = self
            .response_projection_read_request(caller, &mapping, Some(previous_mapping))
            .await?;
        let mapping = self
            .ensure_response_projection_ref(
                caller.scope().clone(),
                public_id.clone(),
                mapping,
                &projection_read,
            )
            .await?;
        let wait_result = tokio::time::timeout(
            self.wait_timeout,
            self.projection_reader
                .wait_for_response_completion(OpenAiResponseWaitRequest {
                    public_id,
                    actor_scope: caller.scope().clone(),
                    accepted_ack: None,
                    requested_model: request.model.clone(),
                    projection_read,
                    mapping,
                }),
        )
        .await
        .map_err(|_| {
            OpenAiCompatHttpError::from_kind(
                503,
                true,
                crate::OpenAiCompatErrorKind::ServiceUnavailable,
                None,
            )
        })??;
        Ok(wait_result.response)
    }

    /// Bind the continuation response to the parked run/thread, submit the
    /// client tool outputs, and resume the run.
    async fn bind_and_drive_resume(
        &self,
        caller: &OpenAiCompatAuthenticatedCaller,
        request: &OpenAiResponsesCreateRequest,
        previous_mapping: &OpenAiCompatResourceMapping,
        mapping: OpenAiCompatResourceMapping,
        allow_already_resumed_conflict: bool,
    ) -> Result<OpenAiCompatResourceMapping, OpenAiCompatHttpError> {
        let store = self
            .external_tool_store
            .as_ref()
            .ok_or_else(OpenAiCompatHttpError::not_wired)?;
        let resume = self
            .external_tool_resume
            .as_ref()
            .ok_or_else(OpenAiCompatHttpError::not_wired)?;
        let run_ref = response_turn_run_ref(previous_mapping)?;
        let thread_id = projection_thread_id(previous_mapping)?.ok_or_else(|| {
            OpenAiCompatHttpError::conflict(Some("previous_response_id".to_string()))
        })?;
        let public_id = response_public_id(&mapping)?;
        // Reuse the parked response's bound refs so the continuation reads the
        // same (now resumed) run's projection.
        let internal_refs = previous_mapping
            .binding
            .internal_refs()
            .cloned()
            .ok_or_else(|| {
                OpenAiCompatHttpError::conflict(Some("previous_response_id".to_string()))
            })?;
        let mapping = self
            .bind_internal_refs(caller.scope().clone(), public_id, internal_refs)
            .await?
            .ok_or_else(bind_internal_refs_unavailable)?;
        for (call_id, output) in function_call_outputs(request) {
            store
                .submit_tool_output(run_ref.clone(), call_id, output)
                .await?;
        }
        let resume_result = resume
            .resume_external_tool_run(OpenAiCompatExternalToolResumeRequest {
                actor_scope: caller.scope().clone(),
                run_ref: run_ref.clone(),
                thread_id: thread_id.as_str().to_string(),
            })
            .await;
        if let Err(error) = resume_result
            && !(allow_already_resumed_conflict && is_already_resumed_conflict(&error))
        {
            return Err(error);
        }
        self.ref_store
            .mark_external_tool_resume_completed(OpenAiCompatMarkExternalToolResumeCompleted::new(
                caller.scope().clone(),
                mapping.public_id.clone(),
            ))
            .await?
            .ok_or_else(bind_internal_refs_unavailable)
    }
}

fn is_already_resumed_conflict(error: &OpenAiCompatHttpError) -> bool {
    error.status_code() == 409
        && !error.retryable()
        && error.body().error.param() == Some("previous_response_id")
}

fn response_surface_submit_request(
    public_id: &OpenAiResponseId,
    thread_id: &ThreadId,
    user_message_payload: UserMessagePayload,
) -> WebUiSendMessageRequest {
    WebUiSendMessageRequest {
        client_action_id: Some(public_id.as_str().to_string()),
        thread_id: Some(thread_id.as_str().to_string()),
        content: Some(user_message_payload.text),
        attachments: Vec::new(),
        model: user_message_payload.requested_model,
    }
}

fn request_has_function_call_output(request: &OpenAiResponsesCreateRequest) -> bool {
    matches!(
        &request.input,
        OpenAiResponsesInput::Items(items)
            if items
                .iter()
                .any(|item| matches!(item, OpenAiResponsesInputItem::FunctionCallOutput { .. }))
    )
}

fn request_input_is_only_function_call_outputs(request: &OpenAiResponsesCreateRequest) -> bool {
    matches!(
        &request.input,
        OpenAiResponsesInput::Items(items)
            if !items.is_empty()
                && items
                    .iter()
                    .all(|item| matches!(item, OpenAiResponsesInputItem::FunctionCallOutput { .. }))
    )
}

fn function_call_outputs(
    request: &OpenAiResponsesCreateRequest,
) -> Vec<(String, serde_json::Value)> {
    match &request.input {
        OpenAiResponsesInput::Items(items) => items
            .iter()
            .filter_map(|item| match item {
                OpenAiResponsesInputItem::FunctionCallOutput { call_id, output } => {
                    Some((call_id.clone(), output.clone()))
                }
                _ => None,
            })
            .collect(),
        OpenAiResponsesInput::Text(_) => Vec::new(),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenAiResponseWaitRequest {
    pub public_id: OpenAiResponseId,
    pub actor_scope: OpenAiCompatActorScope,
    /// The accepted submit ack for a freshly-created response, or `None` for an
    /// external-tool resume (which reuses the parked run, with no new ack). The
    /// run id is read from `mapping`'s bound refs either way.
    pub accepted_ack: Option<ProductInboundAck>,
    pub requested_model: String,
    pub projection_read: ProjectionReadRequest,
    pub mapping: OpenAiCompatResourceMapping,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenAiResponseReadRequest {
    pub public_id: OpenAiResponseId,
    pub actor_scope: OpenAiCompatActorScope,
    pub requested_model: Option<String>,
    pub projection_read: ProjectionReadRequest,
    pub mapping: OpenAiCompatResourceMapping,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenAiResponseProjection {
    pub response: OpenAiResponseObject,
    pub internal_refs: Option<OpenAiCompatInternalRefs>,
}

impl OpenAiResponseProjection {
    pub fn new(response: OpenAiResponseObject) -> Self {
        Self {
            response,
            internal_refs: None,
        }
    }

    pub fn with_internal_refs(mut self, internal_refs: OpenAiCompatInternalRefs) -> Self {
        self.internal_refs = Some(internal_refs);
        self
    }
}

#[async_trait]
pub trait OpenAiResponsesProjectionReader: Send + Sync {
    async fn wait_for_response_completion(
        &self,
        request: OpenAiResponseWaitRequest,
    ) -> Result<OpenAiResponseProjection, OpenAiCompatHttpError>;

    async fn read_response(
        &self,
        request: OpenAiResponseReadRequest,
    ) -> Result<OpenAiResponseObject, OpenAiCompatHttpError>;
}

fn projection_ref_from_thread_id(
    thread_id: &ThreadId,
) -> Result<OpenAiCompatProjectionRef, OpenAiCompatHttpError> {
    OpenAiCompatProjectionRef::new(format!("thread:{}", thread_id.as_str())).map_err(Into::into)
}

fn projection_thread_id(
    mapping: &OpenAiCompatResourceMapping,
) -> Result<Option<ThreadId>, OpenAiCompatHttpError> {
    let Some(internal_refs) = mapping.binding.internal_refs() else {
        return Ok(None);
    };
    let Some(projection_ref) = &internal_refs.projection_ref else {
        return Ok(None);
    };
    let Some(thread_id) = projection_ref.as_str().strip_prefix("thread:") else {
        return Err(OpenAiCompatHttpError::internal());
    };
    ThreadId::new(thread_id)
        .map(Some)
        .map_err(|_| OpenAiCompatHttpError::internal())
}

fn validate_responses_supported_fields(
    request: &OpenAiResponsesCreateRequest,
) -> Result<(), OpenAiCompatHttpError> {
    if request
        .tools
        .as_ref()
        .is_some_and(|tools| !tools.is_empty())
    {
        return Err(OpenAiCompatHttpError::invalid_request(Some(
            "tools".to_string(),
        )));
    }
    if request.tool_choice.is_some() {
        return Err(OpenAiCompatHttpError::invalid_request(Some(
            "tool_choice".to_string(),
        )));
    }
    if request.previous_response_id.is_some() && request_has_function_call_output(request) {
        return Err(OpenAiCompatHttpError::invalid_request(Some(
            "input".to_string(),
        )));
    }
    if let Some(context) = &request.x_context
        && serialized_json_len(context) > MAX_RESPONSES_CONTEXT_BYTES
    {
        return Err(OpenAiCompatHttpError::invalid_request(Some(
            "x_context".to_string(),
        )));
    }
    Ok(())
}

fn validate_responses_supported_fields_with_external_tools(
    request: &OpenAiResponsesCreateRequest,
) -> Result<(), OpenAiCompatHttpError> {
    if request_has_function_call_output(request) {
        if request.previous_response_id.is_none() {
            return Err(OpenAiCompatHttpError::invalid_request(Some(
                "previous_response_id".to_string(),
            )));
        }
        if !request_input_is_only_function_call_outputs(request) {
            return Err(OpenAiCompatHttpError::invalid_request(Some(
                "input".to_string(),
            )));
        }
    }
    if let Some(context) = &request.x_context
        && serialized_json_len(context) > MAX_RESPONSES_CONTEXT_BYTES
    {
        return Err(OpenAiCompatHttpError::invalid_request(Some(
            "x_context".to_string(),
        )));
    }
    Ok(())
}

fn serialized_json_len(value: &serde_json::Value) -> usize {
    struct CountingWriter(usize);

    impl std::io::Write for CountingWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.saturating_add(buf.len());
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let mut writer = CountingWriter(0);
    serde_json::to_writer(&mut writer, value)
        .map(|_| writer.0)
        .unwrap_or(usize::MAX)
}

fn accepted_ack_from_ack(
    mut ack: ProductInboundAck,
) -> Result<ProductInboundAck, OpenAiCompatHttpError> {
    loop {
        match ack {
            ProductInboundAck::Accepted { .. } => return Ok(ack),
            ProductInboundAck::Duplicate { prior } => ack = *prior,
            ProductInboundAck::DeferredBusy { .. } => {
                return Err(OpenAiCompatHttpError::from_kind(
                    429,
                    true,
                    crate::OpenAiCompatErrorKind::RateLimited,
                    None,
                ));
            }
            ProductInboundAck::RejectedBusy { .. } => {
                // terminal/settled, not retryable — client must issue a new request
                return Err(OpenAiCompatHttpError::from_kind(
                    429,
                    false,
                    crate::OpenAiCompatErrorKind::RateLimited,
                    None,
                ));
            }
            ProductInboundAck::Rejected(rejection) => return Err(error_from_rejection(rejection)),
            ProductInboundAck::CommandResult { .. } | ProductInboundAck::NoOp => {
                return Err(OpenAiCompatHttpError::internal());
            }
        }
    }
}

fn error_from_rejection(rejection: ProductRejection) -> OpenAiCompatHttpError {
    product_rejection_to_openai_error(&rejection, Some("input"))
}

fn bind_internal_refs_unavailable() -> OpenAiCompatHttpError {
    OpenAiCompatHttpError::from_kind(
        503,
        true,
        crate::OpenAiCompatErrorKind::ServiceUnavailable,
        None,
    )
}

fn response_turn_run_ref(
    mapping: &OpenAiCompatResourceMapping,
) -> Result<OpenAiCompatTurnRunRef, OpenAiCompatHttpError> {
    let internal_refs = match &mapping.binding {
        OpenAiCompatResourceBinding::Pending => {
            return Err(OpenAiCompatHttpError::conflict(Some(
                "response_id".to_string(),
            )));
        }
        OpenAiCompatResourceBinding::Bound { internal_refs } => internal_refs,
    };
    let Some(turn_run_ref) = internal_refs.turn_run_ref.as_ref() else {
        return Err(OpenAiCompatHttpError::not_found(Some(
            "response_id".to_string(),
        )));
    };
    Ok(turn_run_ref.clone())
}

fn response_public_id(
    mapping: &OpenAiCompatResourceMapping,
) -> Result<OpenAiResponseId, OpenAiCompatHttpError> {
    let OpenAiCompatPublicId::Response(public_id) = &mapping.public_id else {
        return Err(OpenAiCompatHttpError::internal());
    };
    Ok(public_id.clone())
}

pub(crate) fn parse_response_create_request(
    raw_body: &[u8],
) -> Result<OpenAiResponsesCreateRequest, OpenAiCompatHttpError> {
    if raw_body.len() > MAX_RESPONSES_BODY_BYTES {
        return Err(OpenAiCompatHttpError::invalid_request(Some(
            "body".to_string(),
        )));
    }
    let request: OpenAiResponsesCreateRequest =
        serde_json::from_slice(raw_body).map_err(|error| {
            tracing::debug!(?error, "invalid OpenAI Responses create request body");
            OpenAiCompatHttpError::invalid_request(Some("body".to_string()))
        })?;
    crate::model_validation::validate_model_name(&request.model)?;
    validate_temperature(request.temperature)?;
    Ok(request)
}

fn validate_temperature(temperature: Option<f64>) -> Result<(), OpenAiCompatHttpError> {
    let Some(temperature) = temperature else {
        return Ok(());
    };
    if (0.0..=2.0).contains(&temperature) {
        return Ok(());
    }
    Err(OpenAiCompatHttpError::invalid_request(Some(
        "temperature".to_string(),
    )))
}

fn responses_user_message_payload(
    request: &OpenAiResponsesCreateRequest,
) -> Result<UserMessagePayload, OpenAiCompatHttpError> {
    let payload = UserMessagePayload::new(
        responses_input_to_product_text(request)?,
        vec![],
        ProductTriggerReason::DirectChat,
    )?
    .with_requested_model(ironclaw_common::model_selection::requested_model_hint(
        &request.model,
    ));
    // The builder attaches the model hint after `new`'s validation, so bound the
    // assembled payload before it is submitted.
    payload.validate()?;
    Ok(payload)
}

fn responses_input_to_product_text(
    request: &OpenAiResponsesCreateRequest,
) -> Result<String, OpenAiCompatHttpError> {
    let input = match &request.input {
        OpenAiResponsesInput::Text(text) => {
            if text.trim().is_empty() {
                return Err(OpenAiCompatHttpError::invalid_request(Some(
                    "input".to_string(),
                )));
            }
            vec![serde_json::json!({
                "type": "message",
                "role": "user",
                "content": sanitize_product_text_fragment(text),
            })]
        }
        OpenAiResponsesInput::Items(items) => {
            if items.is_empty() || items.len() > MAX_RESPONSES_INPUT_ITEMS {
                return Err(OpenAiCompatHttpError::invalid_request(Some(
                    "input".to_string(),
                )));
            }
            items.iter().map(response_input_item_to_value).collect()
        }
    };
    let mut payload = serde_json::json!({
        "format": "openai_compat.responses_input.v1",
        "instructions": request
            .instructions
            .as_ref()
            .filter(|value| !value.is_empty())
            .map(|value| sanitize_product_text_fragment(value)),
        "input": input,
    });
    if let Some(context) = &request.x_context {
        payload["context"] = serde_json::Value::String(responses_context_to_product_text(context));
    }
    if let Some(temperature) = request.temperature {
        payload["temperature"] = serde_json::json!(temperature);
    }
    serde_json::to_string(&payload).map_err(|_| OpenAiCompatHttpError::internal())
}

fn responses_context_to_product_text(context: &serde_json::Value) -> String {
    use std::fmt::Write as _;

    let Some(object) = context.as_object() else {
        let val_str = context_value_to_product_text(context);
        return format!("[Context: {val_str}]");
    };

    let mut result = String::new();
    for (index, (key, value)) in object.iter().enumerate() {
        if index > 0 {
            result.push('\n');
        }
        let key = sanitize_product_text_fragment(key);
        match value.as_object() {
            Some(inner) => {
                let mut fields = String::new();
                for (field_index, (field, value)) in inner.iter().enumerate() {
                    if field_index > 0 {
                        fields.push_str(", ");
                    }
                    let field = sanitize_product_text_fragment(field);
                    let value = context_value_to_product_text(value);
                    let _ = write!(&mut fields, "{field}: {value}");
                }
                let _ = write!(&mut result, "[Context: {key} - {fields}]");
            }
            None => {
                let value = context_value_to_product_text(value);
                let _ = write!(&mut result, "[Context: {key}: {value}]");
            }
        }
    }
    result
}

fn context_value_to_product_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => sanitize_product_text_fragment(text),
        other => sanitize_product_text_fragment(&other.to_string()),
    }
}

fn response_input_item_to_value(item: &OpenAiResponsesInputItem) -> serde_json::Value {
    match item {
        OpenAiResponsesInputItem::Message { role, content } => serde_json::json!({
            "type": "message",
            "role": response_role_name(*role),
            "content": content_value_to_text(content),
        }),
        OpenAiResponsesInputItem::FunctionCall {
            call_id,
            name,
            arguments,
        } => serde_json::json!({
            "type": "function_call",
            "call_id": sanitize_product_text_fragment(call_id),
            "name": sanitize_product_text_fragment(name),
            "arguments": sanitize_product_text_fragment(arguments),
        }),
        OpenAiResponsesInputItem::FunctionCallOutput { call_id, output } => serde_json::json!({
            "type": "function_call_output",
            "call_id": sanitize_product_text_fragment(call_id),
            "output": content_value_to_text(output),
        }),
    }
}

fn response_role_name(role: OpenAiResponsesMessageRole) -> &'static str {
    match role {
        OpenAiResponsesMessageRole::System => "system",
        OpenAiResponsesMessageRole::Developer => "developer",
        OpenAiResponsesMessageRole::User => "user",
        OpenAiResponsesMessageRole::Assistant => "assistant",
    }
}

fn content_value_to_text(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::String(text) => sanitize_product_text_fragment(text),
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(content_array_item_text)
            .collect::<Vec<_>>()
            .join(" "),
        // A bare object-form part (non-standard, but tolerated): run it through
        // the same per-part logic so a typed part gets its specific marker (or
        // text) instead of discarding the type for the generic marker.
        value @ serde_json::Value::Object(_) => {
            content_array_item_text(value).unwrap_or_else(|| non_text_part_marker(None).to_string())
        }
        value if !value.is_null() => non_text_part_marker(None).to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_product_adapters::ProductInboundAck;
    use ironclaw_turns::{AcceptedMessageRef, TurnRunId};

    use super::accepted_ack_from_ack;

    #[test]
    fn deferred_busy_ack_is_retryable_429_on_create() {
        let ack = ProductInboundAck::DeferredBusy {
            accepted_message_ref: AcceptedMessageRef::new("msg:deferred-busy").expect("ref"),
            active_run_id: TurnRunId::new(),
        };
        let err = accepted_ack_from_ack(ack).unwrap_err();
        assert_eq!(err.status_code(), 429);
        assert!(err.retryable(), "DeferredBusy must be retryable");
    }

    #[test]
    fn rejected_busy_ack_is_non_retryable_429_on_create() {
        let ack = ProductInboundAck::RejectedBusy {
            accepted_message_ref: AcceptedMessageRef::new("msg:rejected-busy").expect("ref"),
            active_run_id: None,
        };
        let err = accepted_ack_from_ack(ack).unwrap_err();
        assert_eq!(err.status_code(), 429);
        assert!(
            !err.retryable(),
            "RejectedBusy is terminal — must not be retryable"
        );
    }
}
