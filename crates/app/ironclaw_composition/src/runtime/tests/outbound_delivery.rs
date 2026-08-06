use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_host_api::ids::CapabilityId;
use ironclaw_loop_contracts::{
    LoopCapabilityPort, ProviderToolCall, RegisterProviderToolCallRequest,
};
use ironclaw_loop_host::{
    HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
    HostManagedModelMessageRole, HostManagedModelRequest, HostManagedModelResponse,
};
use ironclaw_threads::{MessageKind, ThreadHistoryRequest};
use ironclaw_turns::TurnStatus;

use crate::runtime_input::{PollSettings, RebornRuntimeIdentity, RebornRuntimeInput};

use super::build_reborn_runtime;

const RUNTIME_SEND_TIMEOUT: Duration = Duration::from_secs(20);

fn provider_tool_call(
    tool_definitions: &[ironclaw_loop_contracts::ProviderToolDefinition],
    capability_id: &str,
    call_id: &str,
    arguments: serde_json::Value,
) -> ProviderToolCall {
    let capability_id = CapabilityId::new(capability_id).expect("capability id");
    let tool = tool_definitions
        .iter()
        .find(|definition| definition.capability_id == capability_id)
        .unwrap_or_else(|| panic!("{capability_id} provider tool definition should exist"));
    ProviderToolCall {
        provider_id: "test-provider".to_string(),
        provider_model_id: "test-model".to_string(),
        turn_id: Some("provider-turn-1".to_string()),
        id: call_id.to_string(),
        name: tool.name.clone(),
        arguments,
        response_reasoning: None,
        reasoning: None,
        signature: None,
    }
}

fn model_capability_error(error: impl std::fmt::Display) -> HostManagedModelError {
    let safe_summary = error.to_string();
    HostManagedModelError::safe(HostManagedModelErrorKind::Unavailable, safe_summary)
}

#[derive(Debug, Default)]
struct ReplyAttachmentGateway {
    calls: StdMutex<usize>,
}

#[async_trait]
impl HostManagedModelGateway for ReplyAttachmentGateway {
    async fn stream_model(
        &self,
        _request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::InvalidRequest,
            "expected capability-aware model path",
        ))
    }

    async fn stream_model_with_capabilities(
        &self,
        request: HostManagedModelRequest,
        capabilities: Arc<dyn LoopCapabilityPort>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        let call_index = {
            let mut calls = self
                .calls
                .lock()
                .expect("reply attachment gateway call lock poisoned");
            let current = *calls;
            *calls += 1;
            current
        };
        if call_index >= 2 {
            let tool_result_count = request
                .messages
                .iter()
                .filter(|message| message.role == HostManagedModelMessageRole::ToolResult)
                .count();
            assert_eq!(tool_result_count, 2);
            return Ok(HostManagedModelResponse::assistant_reply(
                "The CSV is attached.",
            ));
        }

        let tool_definitions = capabilities
            .tool_definitions()
            .map_err(model_capability_error)?;
        let call = match call_index {
            0 => provider_tool_call(
                &tool_definitions,
                "builtin.write_file",
                "call-write-reply-attachment",
                serde_json::json!({
                    "path": "/workspace/reports/result.csv",
                    "content": "name,value\nalpha,1\n"
                }),
            ),
            1 => provider_tool_call(
                &tool_definitions,
                "builtin.attach_workspace_file_to_reply",
                "call-attach-workspace-file",
                serde_json::json!({
                    "path": "/workspace/reports/result.csv"
                }),
            ),
            _ => unreachable!("handled above"),
        };
        let candidate = capabilities
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(call))
            .await
            .map_err(model_capability_error)?;
        Ok(HostManagedModelResponse::capability_calls(
            vec![candidate],
            "",
        ))
    }
}

#[tokio::test]
async fn production_reply_attachment_capability_registers_durable_run_intent() {
    let root = tempfile::tempdir().expect("tempdir");
    let gateway: Arc<dyn HostManagedModelGateway> = Arc::new(ReplyAttachmentGateway::default());
    let input =
        RebornRuntimeInput::from_build_input(crate::deployment::local_filesystem_build_input(
            "runtime-reply-attachment-owner",
            root.path().join("standalone"),
        ))
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-reply-attachment-tenant".to_string(),
            agent_id: "runtime-reply-attachment-agent".to_string(),
            source_binding_id: "runtime-reply-attachment-source".to_string(),
            reply_target_binding_id: "runtime-reply-attachment-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: RUNTIME_SEND_TIMEOUT,
        })
        .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let conversation = runtime.new_conversation().await.expect("conversation");
    runtime
        .enable_global_auto_approve_for_test(&conversation)
        .await;
    let reply = tokio::time::timeout(
        RUNTIME_SEND_TIMEOUT,
        runtime.send_user_message(
            &conversation,
            "Create a CSV report and attach it to your reply.",
        ),
    )
    .await
    .expect("runtime send should finish")
    .expect("runtime send should succeed");
    assert_eq!(reply.status, TurnStatus::Completed);

    let history = runtime
        .thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: runtime.thread_scope.clone(),
            thread_id: conversation.0.clone(),
        })
        .await
        .expect("thread history");
    let assistant = history
        .messages
        .iter()
        .find(|message| message.kind == MessageKind::Assistant)
        .expect("finalized assistant reply");
    assert_eq!(assistant.attachments.len(), 1);
    assert_eq!(
        assistant.attachments[0].storage_key.as_deref(),
        Some("/workspace/reports/result.csv")
    );
    assert_eq!(
        assistant.attachments[0].filename.as_deref(),
        Some("result.csv")
    );
    assert_eq!(assistant.attachments[0].mime_type, "text/csv");
    assert_eq!(assistant.attachments[0].size_bytes, Some(19));

    runtime.shutdown().await.expect("runtime shutdown");
}
