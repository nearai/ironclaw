//! Bridge existing v1 channels into the Reborn transport adapter contract.

use std::{path::Path, sync::Arc};

use async_trait::async_trait;
use futures::StreamExt;
use ironclaw_common::{ExtensionName, ExternalThreadId, JobResultStatus};
use ironclaw_host_api::{InvocationId, ResourceScope, UserId};
use ironclaw_transport::{
    AttachmentKind as TransportAttachmentKind, TransportAdapter, TransportAdapterId,
    TransportAttachment, TransportAuthPrompt, TransportDeliveryAck, TransportEgress,
    TransportError, TransportErrorKind, TransportHealth, TransportIngress, TransportIngressSink,
    TransportMessage, TransportMessageId, TransportMetadata, TransportReply, TransportRoute,
    TransportStatus, TransportStatusUpdate, TransportThreadId,
};
use serde_json::{Map, Value, json};
use tokio::{sync::Mutex, task::JoinHandle};

use crate::channels::{
    AttachmentKind, Channel, ChatApprovalPrompt, EngineThreadSummary, HistoryMessage,
    IncomingAttachment, IncomingMessage, OutgoingResponse, StatusUpdate, ThreadSummary,
    ToolDecision, routing_target_from_metadata,
};
use crate::error::ChannelError;

/// Internal-only metadata key used to round-trip the full legacy [`StatusUpdate`]
/// payload through the transport boundary. Never expose this to third-party
/// adapters — strip on inbound, refuse to honor on outbound from external
/// metadata sources.
pub(crate) const LEGACY_STATUS_METADATA_KEY: &str = "__ironclaw_status_update";

/// Adapter wrapper that lets an existing [`Channel`] speak the Reborn transport contract.
pub struct ChannelTransportAdapter {
    adapter_id: TransportAdapterId,
    channel: Arc<dyn Channel>,
    pump_handle: Mutex<Option<JoinHandle<()>>>,
}

impl ChannelTransportAdapter {
    pub fn new(channel: Arc<dyn Channel>) -> Result<Self, TransportError> {
        let adapter_id = TransportAdapterId::new(channel.name())?;
        Ok(Self {
            adapter_id,
            channel,
            pump_handle: Mutex::new(None),
        })
    }

    fn ensure_route_matches(&self, egress: &TransportEgress) -> Result<(), TransportError> {
        if let Some(route) = egress.route()
            && route.adapter_id != self.adapter_id
        {
            return Err(TransportError::new(
                TransportErrorKind::InvalidRequest,
                "egress route adapter does not match channel adapter",
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl TransportAdapter for ChannelTransportAdapter {
    fn adapter_id(&self) -> &TransportAdapterId {
        &self.adapter_id
    }

    async fn start(&self, sink: Arc<dyn TransportIngressSink>) -> Result<(), TransportError> {
        let mut stream = self.channel.start().await.map_err(channel_error)?;
        let adapter_id = self.adapter_id.clone();
        let handle = tokio::spawn(async move {
            while let Some(message) = stream.next().await {
                match incoming_message_to_transport(&message) {
                    Ok(ingress) => {
                        if let Err(error) = sink.submit_ingress(ingress).await {
                            tracing::warn!(
                                adapter = %adapter_id,
                                kind = %error.kind(),
                                reason = error.safe_reason(),
                                "transport sink rejected channel ingress"
                            );
                        }
                    }
                    Err(error) => {
                        tracing::warn!(
                            adapter = %adapter_id,
                            channel = %message.channel,
                            kind = %error.kind(),
                            reason = error.safe_reason(),
                            "dropping channel message that could not be normalized for transport"
                        );
                    }
                }
            }
        });
        let mut slot = self.pump_handle.lock().await;
        if let Some(prev) = slot.replace(handle) {
            prev.abort();
        }
        Ok(())
    }

    async fn deliver(
        &self,
        egress: TransportEgress,
    ) -> Result<TransportDeliveryAck, TransportError> {
        self.ensure_route_matches(&egress)?;
        match egress {
            TransportEgress::Reply(reply) => {
                let request = incoming_message_from_route(&reply.route, "");
                let response = outgoing_response_from_reply(reply);
                self.channel
                    .respond(&request, response)
                    .await
                    .map_err(channel_error)?;
            }
            TransportEgress::Status(status) => {
                let mut metadata = merged_metadata_value(&status.route.metadata, &status.metadata);
                strip_legacy_status_metadata(&mut metadata);
                self.channel
                    .send_status(channel_status_from_transport(&status), &metadata)
                    .await
                    .map_err(channel_error)?;
            }
            TransportEgress::ApprovalPrompt(prompt) => {
                let metadata = merged_metadata_value(&prompt.route.metadata, &prompt.metadata);
                self.channel
                    .send_status(
                        StatusUpdate::ApprovalNeeded {
                            request_id: prompt.request_id.to_string(),
                            tool_name: prompt.title,
                            description: prompt.summary,
                            parameters: metadata
                                .get("parameters")
                                .cloned()
                                .unwrap_or_else(|| json!({})),
                            allow_always: metadata
                                .get("allow_always")
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                        },
                        &metadata,
                    )
                    .await
                    .map_err(channel_error)?;
            }
            TransportEgress::AuthPrompt(prompt) => {
                let metadata = merged_metadata_value(&prompt.route.metadata, &prompt.metadata);
                self.channel
                    .send_status(
                        channel_auth_status_from_transport(prompt, &metadata)?,
                        &metadata,
                    )
                    .await
                    .map_err(channel_error)?;
            }
            TransportEgress::ProjectionUpdate(_) => {
                return Err(TransportError::new(
                    TransportErrorKind::Unsupported,
                    "projection updates are not supported by v1 channel bridge",
                ));
            }
            TransportEgress::Heartbeat(_) => {}
        }

        Ok(TransportDeliveryAck {
            adapter_id: self.adapter_id.clone(),
            delivered_at: chrono::Utc::now(),
        })
    }

    async fn health_check(&self) -> Result<TransportHealth, TransportError> {
        self.channel.health_check().await.map_err(channel_error)?;
        // If the ingress pump panicked or completed early, the adapter is
        // not actually receiving messages even though the underlying channel
        // reports healthy. Reflect that in the transport health view.
        let slot = self.pump_handle.lock().await;
        if let Some(handle) = slot.as_ref()
            && handle.is_finished()
        {
            return Err(TransportError::new(
                TransportErrorKind::Unavailable,
                "channel ingress pump task is no longer running",
            ));
        }
        Ok(TransportHealth::healthy())
    }

    async fn shutdown(&self) -> Result<(), TransportError> {
        let handle = self.pump_handle.lock().await.take();
        if let Some(handle) = handle {
            handle.abort();
        }
        self.channel.shutdown().await.map_err(channel_error)
    }
}

pub fn incoming_message_to_transport(
    message: &IncomingMessage,
) -> Result<TransportIngress, TransportError> {
    let user_id = UserId::new(&message.user_id).map_err(|error| {
        TransportError::new(TransportErrorKind::InvalidRequest, error.to_string())
    })?;
    let scope = ResourceScope::local_default(user_id, InvocationId::new()).map_err(|error| {
        TransportError::new(TransportErrorKind::InvalidRequest, error.to_string())
    })?;
    let adapter_id = TransportAdapterId::new(&message.channel)?;
    let thread_id = message
        .thread_id
        .as_ref()
        .map(|thread_id| TransportThreadId::new(thread_id.as_str()))
        .transpose()?;

    // Internal-only legacy status payload key must never round-trip through
    // the transport boundary in either direction. Strip on inbound so an
    // adapter cannot smuggle a synthetic legacy status into the agent.
    let mut sanitized_metadata = metadata_from_value(&message.metadata);
    sanitized_metadata.remove(LEGACY_STATUS_METADATA_KEY);

    Ok(TransportIngress {
        message_id: TransportMessageId::new(message.id.to_string())?,
        route: TransportRoute {
            adapter_id,
            scope,
            recipient: message.routing_target(),
            conversation_id: message.conversation_scope().map(ToString::to_string),
            thread_id,
            metadata: sanitized_metadata.clone(),
        },
        message: TransportMessage {
            text: message.content.clone(),
            attachments: message
                .attachments
                .iter()
                .map(transport_attachment_from_channel)
                .collect(),
        },
        sender_display_name: message.user_name.clone(),
        timezone: message.timezone.clone(),
        received_at: message.received_at,
        metadata: sanitized_metadata,
    })
}

pub(crate) fn transport_reply_from_channel_response(
    message: &IncomingMessage,
    response: OutgoingResponse,
) -> Result<TransportReply, TransportError> {
    let mut route = incoming_message_to_transport(message)?.route;
    if let Some(thread_id) = &response.thread_id {
        route.thread_id = Some(TransportThreadId::new(thread_id.as_str())?);
    }

    Ok(TransportReply {
        route,
        content: response.content,
        attachments: response
            .attachments
            .into_iter()
            .enumerate()
            .map(|(index, path)| transport_attachment_from_response_path(index, path))
            .collect(),
        metadata: metadata_from_value(&response.metadata),
    })
}

pub(crate) fn transport_status_from_channel_status(
    channel_name: &str,
    status: StatusUpdate,
    metadata: &Value,
) -> Result<TransportEgress, TransportError> {
    let mut transport_metadata = metadata_from_value(metadata);
    transport_metadata.insert(
        LEGACY_STATUS_METADATA_KEY.to_string(),
        legacy_status_payload(&status),
    );

    Ok(TransportEgress::Status(TransportStatusUpdate {
        route: transport_route_from_channel_status(channel_name, metadata)?,
        status: transport_status_from_channel_status_kind(&status),
        metadata: transport_metadata,
    }))
}

fn transport_route_from_channel_status(
    channel_name: &str,
    metadata: &Value,
) -> Result<TransportRoute, TransportError> {
    // Fail-closed: status egress must carry one of the canonical user-id
    // fields. Falling back to a literal "default" routes status updates to a
    // shared synthetic user, leaks one user's status to another, and breaks
    // multi-tenant isolation.
    let user_id = metadata_string(metadata, "transport_user_id")
        .or_else(|| metadata_string(metadata, "owner_id"))
        .or_else(|| metadata_string(metadata, "user_id"))
        .or_else(|| metadata_string(metadata, "sender_id"))
        .ok_or_else(|| {
            TransportError::new(
                TransportErrorKind::InvalidRequest,
                "status egress metadata missing user identity (transport_user_id/owner_id/user_id/sender_id)",
            )
        })?;
    let user_id = UserId::new(&user_id).map_err(|error| {
        TransportError::new(TransportErrorKind::InvalidRequest, error.to_string())
    })?;
    let scope = ResourceScope::local_default(user_id, InvocationId::new()).map_err(|error| {
        TransportError::new(TransportErrorKind::InvalidRequest, error.to_string())
    })?;

    let thread_id = metadata_string(metadata, "transport_thread_id")
        .or_else(|| metadata_string(metadata, "thread_id"))
        .map(TransportThreadId::new)
        .transpose()?;

    Ok(TransportRoute {
        adapter_id: TransportAdapterId::new(channel_name)?,
        scope,
        recipient: routing_target_from_metadata(metadata),
        conversation_id: metadata_string(metadata, "transport_conversation_id")
            .or_else(|| metadata_string(metadata, "conversation_id")),
        thread_id,
        metadata: metadata_from_value(metadata),
    })
}

fn incoming_message_from_route(
    route: &TransportRoute,
    content: impl Into<String>,
) -> IncomingMessage {
    let mut message = IncomingMessage::new(
        route.adapter_id.as_str(),
        route.scope.user_id.as_str(),
        content,
    )
    .with_sender_id(
        route
            .recipient
            .clone()
            .unwrap_or_else(|| route.scope.user_id.as_str().to_string()),
    )
    .with_metadata(metadata_value(&route.metadata));

    if let Some(thread_id) = &route.thread_id {
        message = message.with_external_thread(ExternalThreadId::from_trusted(
            thread_id.as_str().to_string(),
        ));
    }
    if let Some(conversation_id) = &route.conversation_id {
        message = message.with_conversation_scope(conversation_id.clone());
    }

    message
}

fn outgoing_response_from_reply(reply: TransportReply) -> OutgoingResponse {
    let mut response = OutgoingResponse::text(reply.content)
        .with_attachments(attachment_paths_from_transport(&reply.attachments));
    if let Some(thread_id) = &reply.route.thread_id {
        response = response.in_thread(thread_id.as_str().to_string());
    }
    response.metadata = metadata_value(&reply.metadata);
    response
}

fn channel_status_from_transport(update: &TransportStatusUpdate) -> StatusUpdate {
    if let Some(status) = update
        .metadata
        .get(LEGACY_STATUS_METADATA_KEY)
        .and_then(legacy_status_from_payload)
    {
        return status;
    }

    match &update.status {
        TransportStatus::Thinking { message } => {
            StatusUpdate::Thinking(message.clone().unwrap_or_else(|| "Thinking...".to_string()))
        }
        TransportStatus::StreamChunk { content } => StatusUpdate::StreamChunk(content.clone()),
        TransportStatus::ToolStarted { tool_name, call_id } => StatusUpdate::ToolStarted {
            name: tool_name.clone(),
            detail: None,
            call_id: call_id.clone(),
        },
        TransportStatus::ToolCompleted {
            tool_name,
            call_id,
            success,
            error_kind,
        } => StatusUpdate::ToolCompleted {
            name: tool_name.clone(),
            success: *success,
            error: error_kind.clone(),
            parameters: None,
            call_id: call_id.clone(),
            duration_ms: None,
        },
        TransportStatus::Generic { label, message } => {
            StatusUpdate::Status(message.clone().unwrap_or_else(|| label.clone()))
        }
    }
}

fn channel_auth_status_from_transport(
    prompt: TransportAuthPrompt,
    metadata: &Value,
) -> Result<StatusUpdate, TransportError> {
    let extension_name = match prompt.extension_id {
        Some(extension_id) => ExtensionName::new(extension_id.as_str()).map_err(|error| {
            TransportError::new(TransportErrorKind::InvalidRequest, error.to_string())
        })?,
        None => ExtensionName::new(prompt.credential_name.as_deref().unwrap_or("unknown"))
            .map_err(|error| {
                TransportError::new(TransportErrorKind::InvalidRequest, error.to_string())
            })?,
    };

    Ok(StatusUpdate::AuthRequired {
        extension_name,
        instructions: Some(prompt.instructions),
        auth_url: metadata
            .get("auth_url")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        setup_url: metadata
            .get("setup_url")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        request_id: metadata
            .get("request_id")
            .and_then(Value::as_str)
            .map(ToString::to_string),
    })
}

fn transport_status_from_channel_status_kind(status: &StatusUpdate) -> TransportStatus {
    match status {
        StatusUpdate::Thinking(message) => TransportStatus::Thinking {
            message: Some(message.clone()),
        },
        StatusUpdate::StreamChunk(content) => TransportStatus::StreamChunk {
            content: content.clone(),
        },
        StatusUpdate::ToolStarted { name, call_id, .. } => TransportStatus::ToolStarted {
            tool_name: name.clone(),
            call_id: call_id.clone(),
        },
        StatusUpdate::ToolCompleted {
            name,
            success,
            error,
            call_id,
            ..
        } => TransportStatus::ToolCompleted {
            tool_name: name.clone(),
            call_id: call_id.clone(),
            success: *success,
            error_kind: error.clone(),
        },
        StatusUpdate::Status(message) => TransportStatus::Generic {
            label: "status".to_string(),
            message: Some(message.clone()),
        },
        StatusUpdate::ApprovalNeeded { tool_name, .. } => TransportStatus::Generic {
            label: "approval_needed".to_string(),
            message: Some(format!("Approval needed: {tool_name}")),
        },
        StatusUpdate::AuthRequired { extension_name, .. } => TransportStatus::Generic {
            label: "auth_required".to_string(),
            message: Some(format!("Authentication required: {extension_name}")),
        },
        StatusUpdate::AuthCompleted { extension_name, .. } => TransportStatus::Generic {
            label: "auth_completed".to_string(),
            message: Some(format!("Authentication completed: {extension_name}")),
        },
        StatusUpdate::JobStarted { title, .. } => TransportStatus::Generic {
            label: "job_started".to_string(),
            message: Some(title.clone()),
        },
        StatusUpdate::JobStatus { status, .. } => TransportStatus::Generic {
            label: "job_status".to_string(),
            message: Some(status.clone()),
        },
        StatusUpdate::JobResult { status, .. } => TransportStatus::Generic {
            label: "job_result".to_string(),
            message: Some(status.to_string()),
        },
        StatusUpdate::ToolResult { preview, .. } => TransportStatus::Generic {
            label: "tool_result".to_string(),
            message: Some(preview.clone()),
        },
        StatusUpdate::ReasoningUpdate { narrative, .. } => TransportStatus::Generic {
            label: "reasoning_update".to_string(),
            message: Some(narrative.clone()),
        },
        StatusUpdate::ToolResultFull { output, .. } => TransportStatus::Generic {
            label: "tool_result_full".to_string(),
            message: Some(output.clone()),
        },
        StatusUpdate::ImageGenerated { .. } => TransportStatus::Generic {
            label: "image_generated".to_string(),
            message: None,
        },
        StatusUpdate::RoutineUpdate { name, .. } => TransportStatus::Generic {
            label: "routine_update".to_string(),
            message: Some(name.clone()),
        },
        StatusUpdate::ContextPressure { percentage, .. } => TransportStatus::Generic {
            label: "context_pressure".to_string(),
            message: Some(format!("{percentage}%")),
        },
        StatusUpdate::SandboxStatus { status, .. } => TransportStatus::Generic {
            label: "sandbox_status".to_string(),
            message: Some(status.clone()),
        },
        StatusUpdate::SecretsStatus { .. } => TransportStatus::Generic {
            label: "secrets_status".to_string(),
            message: None,
        },
        StatusUpdate::CostGuard { spent_usd, .. } => TransportStatus::Generic {
            label: "cost_guard".to_string(),
            message: Some(spent_usd.clone()),
        },
        StatusUpdate::Suggestions { .. } => TransportStatus::Generic {
            label: "suggestions".to_string(),
            message: None,
        },
        StatusUpdate::TurnCost { cost_usd, .. } => TransportStatus::Generic {
            label: "turn_cost".to_string(),
            message: Some(cost_usd.clone()),
        },
        StatusUpdate::TurnMetrics { model, .. } => TransportStatus::Generic {
            label: "turn_metrics".to_string(),
            message: Some(model.clone()),
        },
        StatusUpdate::SkillActivated { skill_names, .. } => TransportStatus::Generic {
            label: "skill_activated".to_string(),
            message: Some(skill_names.join(", ")),
        },
        StatusUpdate::ThreadList { .. } => TransportStatus::Generic {
            label: "thread_list".to_string(),
            message: None,
        },
        StatusUpdate::EngineThreadList { .. } => TransportStatus::Generic {
            label: "engine_thread_list".to_string(),
            message: None,
        },
        StatusUpdate::ConversationHistory { thread_id, .. } => TransportStatus::Generic {
            label: "conversation_history".to_string(),
            message: Some(thread_id.clone()),
        },
    }
}

fn legacy_status_payload(status: &StatusUpdate) -> Value {
    match status {
        StatusUpdate::Thinking(message) => json!({
            "kind": "thinking",
            "message": message,
        }),
        StatusUpdate::ToolStarted {
            name,
            detail,
            call_id,
        } => json!({
            "kind": "tool_started",
            "name": name,
            "detail": detail,
            "call_id": call_id,
        }),
        StatusUpdate::ToolCompleted {
            name,
            success,
            error,
            parameters,
            call_id,
            duration_ms,
        } => json!({
            "kind": "tool_completed",
            "name": name,
            "success": success,
            "error": error,
            "parameters": parameters,
            "call_id": call_id,
            "duration_ms": duration_ms,
        }),
        StatusUpdate::ToolResult {
            name,
            preview,
            call_id,
        } => json!({
            "kind": "tool_result",
            "name": name,
            "preview": preview,
            "call_id": call_id,
        }),
        StatusUpdate::StreamChunk(content) => json!({
            "kind": "stream_chunk",
            "content": content,
        }),
        StatusUpdate::Status(message) => json!({
            "kind": "status",
            "message": message,
        }),
        StatusUpdate::JobStarted {
            job_id,
            title,
            browse_url,
        } => json!({
            "kind": "job_started",
            "job_id": job_id,
            "title": title,
            "browse_url": browse_url,
        }),
        StatusUpdate::ApprovalNeeded {
            request_id,
            tool_name,
            description,
            parameters,
            allow_always,
        } => json!({
            "kind": "approval_needed",
            "request_id": request_id,
            "tool_name": tool_name,
            "description": description,
            "parameters": parameters,
            "allow_always": allow_always,
        }),
        StatusUpdate::AuthRequired {
            extension_name,
            instructions,
            auth_url,
            setup_url,
            request_id,
        } => json!({
            "kind": "auth_required",
            "extension_name": extension_name.as_str(),
            "instructions": instructions,
            "auth_url": auth_url,
            "setup_url": setup_url,
            "request_id": request_id,
        }),
        StatusUpdate::AuthCompleted {
            extension_name,
            success,
            message,
        } => json!({
            "kind": "auth_completed",
            "extension_name": extension_name.as_str(),
            "success": success,
            "message": message,
        }),
        StatusUpdate::ImageGenerated {
            event_id,
            data_url,
            path,
        } => json!({
            "kind": "image_generated",
            "event_id": event_id,
            "data_url": data_url,
            "path": path,
        }),
        StatusUpdate::JobStatus { job_id, status } => json!({
            "kind": "job_status",
            "job_id": job_id,
            "status": status,
        }),
        StatusUpdate::JobResult { job_id, status } => json!({
            "kind": "job_result",
            "job_id": job_id,
            "status": status.as_str(),
        }),
        StatusUpdate::RoutineUpdate {
            id,
            name,
            trigger_type,
            enabled,
            last_run,
            next_fire,
        } => json!({
            "kind": "routine_update",
            "id": id,
            "name": name,
            "trigger_type": trigger_type,
            "enabled": enabled,
            "last_run": last_run,
            "next_fire": next_fire,
        }),
        StatusUpdate::ContextPressure {
            used_tokens,
            max_tokens,
            percentage,
            warning,
        } => json!({
            "kind": "context_pressure",
            "used_tokens": used_tokens,
            "max_tokens": max_tokens,
            "percentage": percentage,
            "warning": warning,
        }),
        StatusUpdate::SandboxStatus {
            docker_available,
            running_containers,
            status,
        } => json!({
            "kind": "sandbox_status",
            "docker_available": docker_available,
            "running_containers": running_containers,
            "status": status,
        }),
        StatusUpdate::SecretsStatus {
            count,
            vault_unlocked,
        } => json!({
            "kind": "secrets_status",
            "count": count,
            "vault_unlocked": vault_unlocked,
        }),
        StatusUpdate::CostGuard {
            session_budget_usd,
            spent_usd,
            remaining_usd,
            limit_reached,
        } => json!({
            "kind": "cost_guard",
            "session_budget_usd": session_budget_usd,
            "spent_usd": spent_usd,
            "remaining_usd": remaining_usd,
            "limit_reached": limit_reached,
        }),
        StatusUpdate::Suggestions { suggestions } => json!({
            "kind": "suggestions",
            "suggestions": suggestions,
        }),
        StatusUpdate::ReasoningUpdate {
            narrative,
            decisions,
        } => json!({
            "kind": "reasoning_update",
            "narrative": narrative,
            "decisions": decisions
                .iter()
                .map(|decision| json!({
                    "tool_name": decision.tool_name,
                    "rationale": decision.rationale,
                }))
                .collect::<Vec<_>>(),
        }),
        StatusUpdate::TurnCost {
            input_tokens,
            output_tokens,
            cost_usd,
        } => json!({
            "kind": "turn_cost",
            "input_tokens": input_tokens,
            "output_tokens": output_tokens,
            "cost_usd": cost_usd,
        }),
        StatusUpdate::ToolResultFull {
            name,
            output,
            truncated,
            call_id,
        } => json!({
            "kind": "tool_result_full",
            "name": name,
            "output": output,
            "truncated": truncated,
            "call_id": call_id,
        }),
        StatusUpdate::TurnMetrics {
            input_tokens,
            output_tokens,
            cache_read_tokens,
            model,
            duration_ms,
            iteration,
        } => json!({
            "kind": "turn_metrics",
            "input_tokens": input_tokens,
            "output_tokens": output_tokens,
            "cache_read_tokens": cache_read_tokens,
            "model": model,
            "duration_ms": duration_ms,
            "iteration": iteration,
        }),
        StatusUpdate::SkillActivated {
            skill_names,
            feedback,
        } => json!({
            "kind": "skill_activated",
            "skill_names": skill_names,
            "feedback": feedback,
        }),
        StatusUpdate::ThreadList { threads } => json!({
            "kind": "thread_list",
            "threads": json_or_null(threads),
        }),
        StatusUpdate::EngineThreadList { threads } => json!({
            "kind": "engine_thread_list",
            "threads": threads
                .iter()
                .map(|thread| json!({
                    "id": thread.id,
                    "goal": thread.goal,
                    "thread_type": thread.thread_type,
                    "state": thread.state,
                    "step_count": thread.step_count,
                    "total_tokens": thread.total_tokens,
                    "created_at": thread.created_at,
                    "updated_at": thread.updated_at,
                }))
                .collect::<Vec<_>>(),
        }),
        StatusUpdate::ConversationHistory {
            thread_id,
            messages,
            pending_approval,
        } => json!({
            "kind": "conversation_history",
            "thread_id": thread_id,
            "messages": json_or_null(messages),
            "pending_approval": pending_approval.as_ref().map(|approval| json!({
                "request_id": approval.request_id,
                "tool_name": approval.tool_name,
                "description": approval.description,
                "parameters": approval.parameters,
                "allow_always": approval.allow_always,
            })),
        }),
    }
}

fn legacy_status_from_payload(payload: &Value) -> Option<StatusUpdate> {
    let kind = payload.get("kind")?.as_str()?;
    Some(match kind {
        "thinking" => StatusUpdate::Thinking(payload_string(payload, "message")?),
        "tool_started" => StatusUpdate::ToolStarted {
            name: payload_string(payload, "name")?,
            detail: payload_optional_string(payload, "detail"),
            call_id: payload_optional_string(payload, "call_id"),
        },
        "tool_completed" => StatusUpdate::ToolCompleted {
            name: payload_string(payload, "name")?,
            success: payload_bool(payload, "success")?,
            error: payload_optional_string(payload, "error"),
            parameters: payload_optional_string(payload, "parameters"),
            call_id: payload_optional_string(payload, "call_id"),
            duration_ms: payload_optional_u64(payload, "duration_ms"),
        },
        "tool_result" => StatusUpdate::ToolResult {
            name: payload_string(payload, "name")?,
            preview: payload_string(payload, "preview")?,
            call_id: payload_optional_string(payload, "call_id"),
        },
        "stream_chunk" => StatusUpdate::StreamChunk(payload_string(payload, "content")?),
        "status" => StatusUpdate::Status(payload_string(payload, "message")?),
        "job_started" => StatusUpdate::JobStarted {
            job_id: payload_string(payload, "job_id")?,
            title: payload_string(payload, "title")?,
            browse_url: payload_string(payload, "browse_url")?,
        },
        "approval_needed" => StatusUpdate::ApprovalNeeded {
            request_id: payload_string(payload, "request_id")?,
            tool_name: payload_string(payload, "tool_name")?,
            description: payload_string(payload, "description")?,
            parameters: payload
                .get("parameters")
                .cloned()
                .unwrap_or_else(|| json!({})),
            allow_always: payload_bool(payload, "allow_always")?,
        },
        "auth_required" => StatusUpdate::AuthRequired {
            extension_name: ExtensionName::new(payload_string(payload, "extension_name")?).ok()?,
            instructions: payload_optional_string(payload, "instructions"),
            auth_url: payload_optional_string(payload, "auth_url"),
            setup_url: payload_optional_string(payload, "setup_url"),
            request_id: payload_optional_string(payload, "request_id"),
        },
        "auth_completed" => StatusUpdate::AuthCompleted {
            extension_name: ExtensionName::new(payload_string(payload, "extension_name")?).ok()?,
            success: payload_bool(payload, "success")?,
            message: payload_string(payload, "message")?,
        },
        "image_generated" => StatusUpdate::ImageGenerated {
            event_id: payload_string(payload, "event_id")?,
            data_url: payload_string(payload, "data_url")?,
            path: payload_optional_string(payload, "path"),
        },
        "job_status" => StatusUpdate::JobStatus {
            job_id: payload_string(payload, "job_id")?,
            status: payload_string(payload, "status")?,
        },
        "job_result" => StatusUpdate::JobResult {
            job_id: payload_string(payload, "job_id")?,
            status: payload_string(payload, "status")?
                .parse::<JobResultStatus>()
                .ok()?,
        },
        "routine_update" => StatusUpdate::RoutineUpdate {
            id: payload_string(payload, "id")?,
            name: payload_string(payload, "name")?,
            trigger_type: payload_string(payload, "trigger_type")?,
            enabled: payload_bool(payload, "enabled")?,
            last_run: payload_optional_string(payload, "last_run"),
            next_fire: payload_optional_string(payload, "next_fire"),
        },
        "context_pressure" => StatusUpdate::ContextPressure {
            used_tokens: payload_u64(payload, "used_tokens")?,
            max_tokens: payload_u64(payload, "max_tokens")?,
            percentage: payload_u8(payload, "percentage")?,
            warning: payload_optional_string(payload, "warning"),
        },
        "sandbox_status" => StatusUpdate::SandboxStatus {
            docker_available: payload_bool(payload, "docker_available")?,
            running_containers: payload_u32(payload, "running_containers")?,
            status: payload_string(payload, "status")?,
        },
        "secrets_status" => StatusUpdate::SecretsStatus {
            count: payload_u32(payload, "count")?,
            vault_unlocked: payload_bool(payload, "vault_unlocked")?,
        },
        "cost_guard" => StatusUpdate::CostGuard {
            session_budget_usd: payload_optional_string(payload, "session_budget_usd"),
            spent_usd: payload_string(payload, "spent_usd")?,
            remaining_usd: payload_optional_string(payload, "remaining_usd"),
            limit_reached: payload_bool(payload, "limit_reached")?,
        },
        "suggestions" => StatusUpdate::Suggestions {
            suggestions: payload_string_vec(payload, "suggestions"),
        },
        "reasoning_update" => StatusUpdate::ReasoningUpdate {
            narrative: payload_string(payload, "narrative")?,
            decisions: payload_tool_decisions(payload.get("decisions")?),
        },
        "turn_cost" => StatusUpdate::TurnCost {
            input_tokens: payload_u64(payload, "input_tokens")?,
            output_tokens: payload_u64(payload, "output_tokens")?,
            cost_usd: payload_string(payload, "cost_usd")?,
        },
        "tool_result_full" => StatusUpdate::ToolResultFull {
            name: payload_string(payload, "name")?,
            output: payload_string(payload, "output")?,
            truncated: payload_bool(payload, "truncated")?,
            call_id: payload_optional_string(payload, "call_id"),
        },
        "turn_metrics" => StatusUpdate::TurnMetrics {
            input_tokens: payload_u64(payload, "input_tokens")?,
            output_tokens: payload_u64(payload, "output_tokens")?,
            cache_read_tokens: payload_u64(payload, "cache_read_tokens")?,
            model: payload_string(payload, "model")?,
            duration_ms: payload_u64(payload, "duration_ms")?,
            iteration: payload_usize(payload, "iteration")?,
        },
        "skill_activated" => StatusUpdate::SkillActivated {
            skill_names: payload_string_vec(payload, "skill_names"),
            feedback: payload_string_vec(payload, "feedback"),
        },
        "thread_list" => StatusUpdate::ThreadList {
            threads: serde_json::from_value::<Vec<ThreadSummary>>(
                payload.get("threads").cloned().unwrap_or(Value::Null),
            )
            .ok()?,
        },
        "engine_thread_list" => StatusUpdate::EngineThreadList {
            threads: payload_engine_threads(payload.get("threads")?),
        },
        "conversation_history" => StatusUpdate::ConversationHistory {
            thread_id: payload_string(payload, "thread_id")?,
            messages: serde_json::from_value::<Vec<HistoryMessage>>(
                payload.get("messages").cloned().unwrap_or(Value::Null),
            )
            .ok()?,
            pending_approval: payload
                .get("pending_approval")
                .and_then(payload_chat_approval),
        },
        _ => return None,
    })
}

fn transport_attachment_from_response_path(index: usize, path: String) -> TransportAttachment {
    let filename = Path::new(&path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(ToString::to_string);

    TransportAttachment {
        id: format!("response-attachment-{index}"),
        kind: TransportAttachmentKind::Document,
        mime_type: None,
        filename,
        size_bytes: None,
        data: Vec::new(),
        storage_ref: Some(path),
        source_url: None,
        metadata: TransportMetadata::new(),
    }
}

fn transport_attachment_from_channel(attachment: &IncomingAttachment) -> TransportAttachment {
    let mut metadata = TransportMetadata::new();
    if let Some(duration_secs) = attachment.duration_secs {
        metadata.insert("duration_secs".to_string(), json!(duration_secs));
    }
    if let Some(local_path) = &attachment.local_path {
        metadata.insert("local_path".to_string(), json!(local_path));
    }

    TransportAttachment {
        id: attachment.id.clone(),
        kind: match attachment.kind {
            AttachmentKind::Audio => TransportAttachmentKind::Audio,
            AttachmentKind::Image => TransportAttachmentKind::Image,
            AttachmentKind::Document => TransportAttachmentKind::Document,
        },
        mime_type: Some(attachment.mime_type.clone()),
        filename: attachment.filename.clone(),
        size_bytes: attachment.size_bytes,
        data: attachment.data.clone(),
        storage_ref: attachment
            .storage_key
            .clone()
            .or_else(|| attachment.local_path.clone()),
        source_url: attachment.source_url.clone(),
        metadata,
    }
}

fn attachment_paths_from_transport(attachments: &[TransportAttachment]) -> Vec<String> {
    attachments
        .iter()
        .filter_map(|attachment| {
            attachment
                .storage_ref
                .clone()
                .or_else(|| attachment.source_url.clone())
        })
        .collect()
}

fn metadata_from_value(value: &Value) -> TransportMetadata {
    match value {
        Value::Object(map) => map
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
        Value::Null => TransportMetadata::new(),
        other => TransportMetadata::from([("value".to_string(), other.clone())]),
    }
}

fn metadata_string(metadata: &Value, key: &str) -> Option<String> {
    metadata.get(key).and_then(|value| match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    })
}

fn metadata_value(metadata: &TransportMetadata) -> Value {
    Value::Object(
        metadata
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<Map<_, _>>(),
    )
}

fn merged_metadata_value(route: &TransportMetadata, update: &TransportMetadata) -> Value {
    let mut merged = route.clone();
    merged.extend(
        update
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
    );
    metadata_value(&merged)
}

fn strip_legacy_status_metadata(metadata: &mut Value) {
    if let Value::Object(map) = metadata {
        map.remove(LEGACY_STATUS_METADATA_KEY);
    }
}

fn json_or_null<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

fn payload_string(payload: &Value, key: &str) -> Option<String> {
    payload.get(key).and_then(|value| match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    })
}

fn payload_optional_string(payload: &Value, key: &str) -> Option<String> {
    payload.get(key).and_then(|value| match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    })
}

fn payload_bool(payload: &Value, key: &str) -> Option<bool> {
    payload.get(key).and_then(Value::as_bool)
}

fn payload_u64(payload: &Value, key: &str) -> Option<u64> {
    payload.get(key).and_then(Value::as_u64)
}

fn payload_optional_u64(payload: &Value, key: &str) -> Option<u64> {
    payload.get(key).and_then(Value::as_u64)
}

fn payload_u32(payload: &Value, key: &str) -> Option<u32> {
    payload_u64(payload, key).and_then(|value| u32::try_from(value).ok())
}

fn payload_u8(payload: &Value, key: &str) -> Option<u8> {
    payload_u64(payload, key).and_then(|value| u8::try_from(value).ok())
}

fn payload_usize(payload: &Value, key: &str) -> Option<usize> {
    payload_u64(payload, key).and_then(|value| usize::try_from(value).ok())
}

fn payload_string_vec(payload: &Value, key: &str) -> Vec<String> {
    payload
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn payload_tool_decisions(payload: &Value) -> Vec<ToolDecision> {
    payload
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    Some(ToolDecision {
                        tool_name: payload_string(item, "tool_name")?,
                        rationale: payload_string(item, "rationale")?,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn payload_engine_threads(payload: &Value) -> Vec<EngineThreadSummary> {
    payload
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    Some(EngineThreadSummary {
                        id: payload_string(item, "id")?,
                        goal: payload_string(item, "goal")?,
                        thread_type: payload_string(item, "thread_type")?,
                        state: payload_string(item, "state")?,
                        step_count: payload_usize(item, "step_count")?,
                        total_tokens: payload_u64(item, "total_tokens")?,
                        created_at: payload_string(item, "created_at")?,
                        updated_at: payload_string(item, "updated_at")?,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn payload_chat_approval(payload: &Value) -> Option<ChatApprovalPrompt> {
    if payload.is_null() {
        return None;
    }

    Some(ChatApprovalPrompt {
        request_id: payload_string(payload, "request_id")?,
        tool_name: payload_string(payload, "tool_name")?,
        description: payload_string(payload, "description")?,
        parameters: payload
            .get("parameters")
            .cloned()
            .unwrap_or_else(|| json!({})),
        allow_always: payload_bool(payload, "allow_always")?,
    })
}

fn channel_error(error: ChannelError) -> TransportError {
    let kind = match error {
        ChannelError::StartupFailed { .. } => TransportErrorKind::StartupFailed,
        ChannelError::Disconnected { .. }
        | ChannelError::HealthCheckFailed { .. }
        | ChannelError::RateLimited { .. } => TransportErrorKind::Unavailable,
        ChannelError::SendFailed { .. } | ChannelError::MissingRoutingTarget { .. } => {
            TransportErrorKind::DeliveryFailed
        }
        ChannelError::InvalidMessage(_) => TransportErrorKind::InvalidRequest,
        ChannelError::AuthFailed { .. } => TransportErrorKind::Unauthorized,
        ChannelError::Http(_) => TransportErrorKind::DeliveryFailed,
    };
    TransportError::new(kind, error.to_string())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use ironclaw_transport::{
        AttachmentKind as TransportAttachmentKind, TransportAdapter, TransportApprovalPrompt,
        TransportEgress, TransportError, TransportErrorKind, TransportIngress,
        TransportIngressSink, TransportMetadata, TransportReply, TransportRoute, TransportStatus,
        TransportStatusUpdate, TransportSubmission, TransportThreadId,
    };
    use serde_json::json;
    use tokio::sync::{Mutex, mpsc};

    use super::ChannelTransportAdapter;
    use crate::channels::{
        AttachmentKind, Channel, IncomingAttachment, IncomingMessage, MessageStream,
        OutgoingResponse, StatusUpdate,
    };
    use crate::error::ChannelError;
    use crate::testing::StubChannel;

    struct SinkRecorder {
        tx: mpsc::Sender<TransportIngress>,
    }

    #[async_trait]
    impl TransportIngressSink for SinkRecorder {
        async fn submit_ingress(
            &self,
            ingress: TransportIngress,
        ) -> Result<TransportSubmission, TransportError> {
            self.tx
                .send(ingress)
                .await
                .map_err(|_| TransportError::new(TransportErrorKind::Unavailable, "sink closed"))?;
            Ok(TransportSubmission {
                accepted_at: chrono::Utc::now(),
                correlation_id: None,
            })
        }
    }

    type CapturedStatuses = Arc<Mutex<Vec<(StatusUpdate, serde_json::Value)>>>;

    struct MetadataCaptureChannel {
        name: &'static str,
        statuses: CapturedStatuses,
    }

    impl MetadataCaptureChannel {
        fn new(name: &'static str) -> (Self, CapturedStatuses) {
            let statuses = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    name,
                    statuses: statuses.clone(),
                },
                statuses,
            )
        }
    }

    #[async_trait]
    impl Channel for MetadataCaptureChannel {
        fn name(&self) -> &str {
            self.name
        }

        async fn start(&self) -> Result<MessageStream, ChannelError> {
            let (_tx, rx) = mpsc::channel(1);
            Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
        }

        async fn respond(
            &self,
            _msg: &IncomingMessage,
            _response: OutgoingResponse,
        ) -> Result<(), ChannelError> {
            Ok(())
        }

        async fn send_status(
            &self,
            status: StatusUpdate,
            metadata: &serde_json::Value,
        ) -> Result<(), ChannelError> {
            self.statuses.lock().await.push((status, metadata.clone()));
            Ok(())
        }

        async fn health_check(&self) -> Result<(), ChannelError> {
            Ok(())
        }
    }

    fn route(adapter: &str, user: &str) -> TransportRoute {
        TransportRoute {
            adapter_id: ironclaw_transport::TransportAdapterId::new(adapter)
                .expect("valid adapter id"),
            scope: ironclaw_host_api::ResourceScope::local_default(
                ironclaw_host_api::UserId::new(user).expect("valid user"),
                ironclaw_host_api::InvocationId::new(),
            )
            .expect("valid scope"),
            recipient: Some("C123".to_string()),
            conversation_id: Some("conversation-1".to_string()),
            thread_id: Some(TransportThreadId::new("thread-1").unwrap()),
            metadata: TransportMetadata::from([("channel_id".to_string(), json!("C123"))]),
        }
    }

    #[tokio::test]
    async fn start_submits_normalized_channel_ingress_to_transport_sink() {
        let (channel, sender) = StubChannel::new("gateway");
        let adapter = ChannelTransportAdapter::new(Arc::new(channel)).expect("adapter");
        let (tx, mut rx) = mpsc::channel(1);
        adapter
            .start(Arc::new(SinkRecorder { tx }))
            .await
            .expect("start adapter");

        let message = IncomingMessage::new("gateway", "alice", "hello")
            .with_sender_id("alice-device")
            .with_thread("thread-1")
            .with_metadata(json!({
                "channel_id": "C123",
                "user_id": "mallory",
                "thread_id": "spoofed-thread"
            }))
            .with_timezone("America/Los_Angeles")
            .with_attachments(vec![IncomingAttachment {
                id: "upload-1".to_string(),
                kind: AttachmentKind::Document,
                mime_type: "text/plain".to_string(),
                filename: Some("notes.txt".to_string()),
                size_bytes: Some(24),
                source_url: Some("https://files.example/notes.txt".to_string()),
                storage_key: Some("workspace://uploads/upload-1".to_string()),
                local_path: None,
                extracted_text: Some("hello".to_string()),
                data: b"raw bytes survive transport".to_vec(),
                duration_secs: None,
            }]);

        sender.send(message).await.expect("send message");
        let ingress = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("ingress forwarded")
            .expect("ingress present");

        assert_eq!(ingress.route.adapter_id.as_str(), "gateway");
        assert_eq!(ingress.route.scope.user_id.as_str(), "alice");
        assert_eq!(ingress.route.recipient.as_deref(), Some("C123"));
        assert_eq!(
            ingress.route.thread_id.as_ref().map(|id| id.as_str()),
            Some("thread-1")
        );
        assert_eq!(ingress.metadata["user_id"], json!("mallory"));
        assert_eq!(
            ingress.message.attachments[0].kind,
            TransportAttachmentKind::Document
        );
        assert_eq!(
            ingress.message.attachments[0].storage_ref.as_deref(),
            Some("workspace://uploads/upload-1")
        );
        assert_eq!(
            ingress.message.attachments[0].data,
            b"raw bytes survive transport".to_vec()
        );
    }

    #[tokio::test]
    async fn deliver_reply_routes_through_wrapped_channel() {
        let (channel, _sender) = StubChannel::new("gateway");
        let responses = channel.captured_responses_handle();
        let adapter = ChannelTransportAdapter::new(Arc::new(channel)).expect("adapter");

        let ack = adapter
            .deliver(TransportEgress::Reply(TransportReply {
                route: route("gateway", "alice"),
                content: "done".to_string(),
                attachments: Vec::new(),
                metadata: TransportMetadata::new(),
            }))
            .await
            .expect("reply delivered");

        assert_eq!(ack.adapter_id.as_str(), "gateway");
        let responses = responses.lock().expect("poisoned");
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].0.channel, "gateway");
        assert_eq!(responses[0].0.user_id, "alice");
        assert_eq!(responses[0].0.sender_id, "C123");
        assert_eq!(responses[0].1.content, "done");
        assert_eq!(
            responses[0].1.thread_id.as_ref().map(|id| id.as_str()),
            Some("thread-1")
        );
    }

    #[tokio::test]
    async fn deliver_status_maps_to_existing_channel_status_shape() {
        let (channel, _sender) = StubChannel::new("gateway");
        let statuses = channel.captured_statuses_handle();
        let adapter = ChannelTransportAdapter::new(Arc::new(channel)).expect("adapter");

        adapter
            .deliver(TransportEgress::Status(TransportStatusUpdate {
                route: route("gateway", "alice"),
                status: TransportStatus::ToolStarted {
                    tool_name: "search".to_string(),
                    call_id: Some("call-1".to_string()),
                },
                metadata: TransportMetadata::new(),
            }))
            .await
            .expect("status delivered");

        let statuses = statuses.lock().expect("poisoned");
        assert!(matches!(
            &statuses[0],
            StatusUpdate::ToolStarted { name, call_id, .. }
                if name == "search" && call_id.as_deref() == Some("call-1")
        ));
    }

    #[tokio::test]
    async fn deliver_approval_prompt_keeps_route_metadata_for_channel_routing() {
        let (channel, captures) = MetadataCaptureChannel::new("gateway");
        let adapter = ChannelTransportAdapter::new(Arc::new(channel)).expect("adapter");
        let mut prompt_metadata = TransportMetadata::new();
        prompt_metadata.insert("allow_always".to_string(), json!(true));
        prompt_metadata.insert("parameters".to_string(), json!({"path": "/tmp/example"}));

        adapter
            .deliver(TransportEgress::ApprovalPrompt(TransportApprovalPrompt {
                route: route("gateway", "alice"),
                request_id: ironclaw_host_api::ApprovalRequestId::new(),
                title: "shell".to_string(),
                summary: "Run command".to_string(),
                metadata: prompt_metadata,
            }))
            .await
            .expect("approval prompt delivered");

        let captures = captures.lock().await;
        assert_eq!(captures[0].1["channel_id"], json!("C123"));
        assert!(matches!(
            &captures[0].0,
            StatusUpdate::ApprovalNeeded {
                tool_name,
                allow_always,
                parameters,
                ..
            } if tool_name == "shell"
                && *allow_always
                && parameters == &json!({"path": "/tmp/example"})
        ));
    }

    #[tokio::test]
    async fn direct_delivery_rejects_route_for_a_different_adapter() {
        let (channel, _sender) = StubChannel::new("gateway");
        let adapter = ChannelTransportAdapter::new(Arc::new(channel)).expect("adapter");

        let error = adapter
            .deliver(TransportEgress::Reply(TransportReply {
                route: route("slack", "alice"),
                content: "wrong adapter".to_string(),
                attachments: Vec::new(),
                metadata: TransportMetadata::new(),
            }))
            .await
            .expect_err("mismatched route must fail");

        assert_eq!(error.kind(), TransportErrorKind::InvalidRequest);
    }
}
