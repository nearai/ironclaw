use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_host_api::CapabilityId;
use ironclaw_loop_support::{
    HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
    HostManagedModelMessageRole, HostManagedModelRequest, HostManagedModelResponse,
};
use ironclaw_product_workflow::{
    RebornOutboundDeliveryTargetCapabilities, RebornOutboundDeliveryTargetId,
    RebornOutboundDeliveryTargetSummary, RebornServicesError, WebUiAuthenticatedCaller,
};
use ironclaw_threads::{LoadContextMessagesRequest, MessageKind, ThreadHistoryRequest};
use ironclaw_turns::{
    ReplyTargetBindingRef, TurnStatus,
    run_profile::{LoopCapabilityPort, ProviderToolCall},
};

use crate::RebornCompositionProfile;
use crate::input::RebornBuildInput;
use crate::outbound_preferences::{
    OutboundDeliveryTargetEntry, OutboundDeliveryTargetProvider,
    OutboundDeliveryTargetRegistrationOutcome,
};
use crate::runtime_input::{PollSettings, RebornRuntimeIdentity, RebornRuntimeInput};

use super::build_reborn_runtime;

const RUNTIME_SEND_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Default)]
struct OutboundDeliveryTriggerGateway {
    calls: StdMutex<usize>,
    requests: StdMutex<Vec<HostManagedModelRequest>>,
}

#[derive(Clone)]
struct StaticOutboundDeliveryTargetProvider {
    entry: OutboundDeliveryTargetEntry,
}

#[async_trait]
impl OutboundDeliveryTargetProvider for StaticOutboundDeliveryTargetProvider {
    async fn list_outbound_delivery_targets(
        &self,
        _caller: &WebUiAuthenticatedCaller,
    ) -> Result<Vec<OutboundDeliveryTargetEntry>, RebornServicesError> {
        Ok(vec![self.entry.clone()])
    }
}

#[async_trait]
impl HostManagedModelGateway for OutboundDeliveryTriggerGateway {
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.requests
            .lock()
            .expect("outbound trigger gateway requests lock poisoned")
            .push(request);
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
                .expect("outbound trigger gateway call lock poisoned");
            let call_index = *calls;
            *calls += 1;
            call_index
        };
        self.requests
            .lock()
            .expect("outbound trigger gateway requests lock poisoned")
            .push(request.clone());

        if call_index >= 3 {
            let tool_result_count = request
                .messages
                .iter()
                .filter(|message| message.role == HostManagedModelMessageRole::ToolResult)
                .count();
            assert_eq!(
                tool_result_count, 3,
                "final model request should observe list, set, and trigger_create results"
            );
            return Ok(HostManagedModelResponse::assistant_reply(
                "trigger delivery target selected",
            ));
        }

        let tool_definitions = capabilities
            .tool_definitions()
            .map_err(model_capability_error)?;
        let call = match call_index {
            0 => provider_tool_call(
                &tool_definitions,
                "builtin.outbound_delivery_targets_list",
                "call-list-outbound-delivery-targets",
                serde_json::json!({"channel": "slack"}),
            ),
            1 => provider_tool_call(
                &tool_definitions,
                "builtin.outbound_delivery_target_set",
                "call-set-outbound-delivery-target",
                serde_json::json!({"target_id": "slack:test-dm"}),
            ),
            2 => provider_tool_call(
                &tool_definitions,
                "builtin.trigger_create",
                "call-trigger-create",
                serde_json::json!({
                    "name": "Slack status digest",
                    "prompt": "Send the status digest to Slack.",
                    "cron": "0 9 * * *",
                    "timezone": "UTC"
                }),
            ),
            _ => unreachable!("handled above"),
        };
        let candidate = capabilities
            .register_provider_tool_call(call)
            .await
            .map_err(model_capability_error)?;
        Ok(HostManagedModelResponse::capability_calls(
            vec![candidate],
            "",
        ))
    }
}

fn provider_tool_call(
    tool_definitions: &[ironclaw_turns::run_profile::ProviderToolDefinition],
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

#[tokio::test]
async fn local_dev_runtime_selects_outbound_delivery_target_before_trigger_create() {
    let root = tempfile::tempdir().expect("tempdir");
    let host_home = root.path().join("host-home");
    std::fs::create_dir_all(&host_home).expect("host home");
    let gateway = Arc::new(OutboundDeliveryTriggerGateway::default());
    let gateway_for_runtime: Arc<dyn HostManagedModelGateway> = gateway.clone();
    let input = RebornRuntimeInput::from_services(
        RebornBuildInput::local_dev_with_profile(
            RebornCompositionProfile::LocalDevYolo,
            "runtime-outbound-trigger-owner",
            root.path().join("local-dev"),
        )
        .with_runtime_policy(
            crate::local_dev_yolo_runtime_policy(true).expect("local-yolo policy resolves"),
        )
        .with_local_dev_confirmed_host_home_root(host_home),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-outbound-trigger-tenant".to_string(),
        agent_id: "runtime-outbound-trigger-agent".to_string(),
        source_binding_id: "runtime-outbound-trigger-source".to_string(),
        reply_target_binding_id: "runtime-outbound-trigger-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: Duration::from_secs(3),
    })
    .with_model_gateway_override(gateway_for_runtime);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let slack_target_id = RebornOutboundDeliveryTargetId::new("slack:test-dm").expect("target id");
    let registered = runtime.register_outbound_delivery_target_provider(
        "slack:test",
        Arc::new(StaticOutboundDeliveryTargetProvider {
            entry: OutboundDeliveryTargetEntry {
                summary: RebornOutboundDeliveryTargetSummary::new(
                    slack_target_id,
                    "slack",
                    "Slack DM",
                    Some("Personal Slack direct message".to_string()),
                )
                .expect("target summary"),
                capabilities: RebornOutboundDeliveryTargetCapabilities {
                    final_replies: true,
                    gate_prompts: false,
                    auth_prompts: false,
                },
                reply_target_binding_ref: ReplyTargetBindingRef::new("reply:test:slack-dm")
                    .expect("reply target"),
            },
        }),
    );
    assert_eq!(
        registered.expect("test Slack target provider should register"),
        OutboundDeliveryTargetRegistrationOutcome::Registered
    );

    let conversation = runtime.new_conversation().await.expect("conversation");
    let reply = tokio::time::timeout(
        RUNTIME_SEND_TIMEOUT,
        runtime.send_user_message(
            &conversation,
            "Create a daily trigger and send the result to my Slack DM.",
        ),
    )
    .await
    .expect("runtime send should finish")
    .expect("runtime send should succeed");

    assert_eq!(reply.status, TurnStatus::Completed);
    assert_eq!(
        reply.text.as_deref(),
        Some("trigger delivery target selected")
    );
    let history = runtime
        .thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: runtime.thread_scope.clone(),
            thread_id: conversation.0.clone(),
        })
        .await
        .expect("thread history");
    let tool_result_ids = history
        .messages
        .iter()
        .filter(|message| message.kind == MessageKind::ToolResultReference)
        .map(|message| message.message_id)
        .collect::<Vec<_>>();
    assert_eq!(
        tool_result_ids.len(),
        3,
        "runtime should persist list, set, and trigger_create tool results"
    );
    let context = runtime
        .thread_service
        .load_context_messages(LoadContextMessagesRequest {
            scope: runtime.thread_scope.clone(),
            thread_id: conversation.0.clone(),
            message_ids: tool_result_ids,
        })
        .await
        .expect("tool result context");
    let invoked_capability_ids = context
        .messages
        .iter()
        .map(|message| {
            message
                .tool_result_provider_call
                .as_ref()
                .expect("provider replay metadata")
                .capability_id
                .as_str()
                .to_string()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        invoked_capability_ids,
        vec![
            "builtin.outbound_delivery_targets_list",
            "builtin.outbound_delivery_target_set",
            "builtin.trigger_create",
        ],
        "Slack trigger delivery should list targets, select one, then create the trigger"
    );

    runtime.shutdown().await.expect("runtime shutdown");
}
