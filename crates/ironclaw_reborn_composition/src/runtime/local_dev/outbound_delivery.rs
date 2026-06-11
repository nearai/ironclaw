use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{InvocationId, UserId};
use ironclaw_loop_support::CapabilityResultWrite;
use ironclaw_product_workflow::{
    OutboundPreferencesProductFacade, RebornOutboundDeliveryTargetId, RebornServicesError,
    RebornServicesErrorCode, RebornSetOutboundPreferencesRequest, WebUiAuthenticatedCaller,
};
use ironclaw_turns::run_profile::{
    AgentLoopHostError, AgentLoopHostErrorKind, CapabilityOutcome, CapabilityProgress,
    CapabilityResultMessage, ConcurrencyHint,
};

use crate::runtime::local_dev::synthetic_capability::{
    LocalDevSyntheticCapability, LocalDevSyntheticCapabilityDescriptor,
    LocalDevSyntheticCapabilityHandler, LocalDevSyntheticCapabilityInvocation,
};

pub(crate) const OUTBOUND_DELIVERY_TARGETS_LIST_CAPABILITY_ID: &str =
    "builtin.outbound_delivery_targets_list";
pub(crate) const OUTBOUND_DELIVERY_TARGET_SET_CAPABILITY_ID: &str =
    "builtin.outbound_delivery_target_set";
const OUTBOUND_DELIVERY_TARGETS_LIST_PROVIDER_TOOL_NAME: &str =
    "builtin__outbound_delivery_targets_list";
const OUTBOUND_DELIVERY_TARGET_SET_PROVIDER_TOOL_NAME: &str =
    "builtin__outbound_delivery_target_set";
const OUTBOUND_DELIVERY_TARGETS_LIST_DESCRIPTION: &str = "List available outbound delivery targets for final replies and routine/trigger results, such as Slack DMs or Slack channels. Use before saying a delivery product is unavailable or asking the user to reconnect it.";
const OUTBOUND_DELIVERY_TARGET_SET_DESCRIPTION: &str = "Set the current user's final-reply delivery target to an id returned by builtin__outbound_delivery_targets_list. Use only after the user asks to send replies or routine/trigger results through that product or channel.";

pub(super) fn outbound_delivery_capabilities(
    facade: Arc<dyn OutboundPreferencesProductFacade>,
    fallback_user_id: UserId,
) -> Result<Vec<LocalDevSyntheticCapability>, AgentLoopHostError> {
    Ok(vec![
        LocalDevSyntheticCapability::new(
            LocalDevSyntheticCapabilityDescriptor::new(
                OUTBOUND_DELIVERY_TARGETS_LIST_CAPABILITY_ID,
                OUTBOUND_DELIVERY_TARGETS_LIST_PROVIDER_TOOL_NAME,
                OUTBOUND_DELIVERY_TARGETS_LIST_DESCRIPTION,
                ConcurrencyHint::SafeForParallel,
                outbound_delivery_targets_list_input_schema(),
            )?,
            Arc::new(OutboundDeliveryTargetsListHandler {
                facade: Arc::clone(&facade),
                fallback_user_id: fallback_user_id.clone(),
            }),
        ),
        LocalDevSyntheticCapability::new(
            LocalDevSyntheticCapabilityDescriptor::new(
                OUTBOUND_DELIVERY_TARGET_SET_CAPABILITY_ID,
                OUTBOUND_DELIVERY_TARGET_SET_PROVIDER_TOOL_NAME,
                OUTBOUND_DELIVERY_TARGET_SET_DESCRIPTION,
                ConcurrencyHint::Exclusive,
                outbound_delivery_target_set_input_schema(),
            )?,
            Arc::new(OutboundDeliveryTargetSetHandler {
                facade,
                fallback_user_id,
            }),
        ),
    ])
}

struct OutboundDeliveryTargetsListHandler {
    facade: Arc<dyn OutboundPreferencesProductFacade>,
    fallback_user_id: UserId,
}

#[async_trait]
impl LocalDevSyntheticCapabilityHandler for OutboundDeliveryTargetsListHandler {
    fn validate_provider_arguments(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<(), AgentLoopHostError> {
        parse_optional_channel(arguments).map(|_| ())
    }

    async fn invoke(
        &self,
        invocation: LocalDevSyntheticCapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        let channel_filter = parse_optional_channel(&invocation.input)?;
        let caller = caller_for_run(&invocation, &self.fallback_user_id);
        let mut response = self
            .facade
            .list_outbound_delivery_targets(caller)
            .await
            .map_err(|error| outbound_delivery_host_error("list_targets", error))?;
        if let Some(channel_filter) = channel_filter {
            response.targets.retain(|option| {
                option
                    .target
                    .channel
                    .as_str()
                    .eq_ignore_ascii_case(channel_filter.as_str())
            });
        }
        let count = response.targets.len();
        let output = serde_json::to_value(response).map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                "outbound delivery target list output serialization failed",
            )
        })?;
        write_completed_result(
            invocation,
            output,
            format!("found {count} delivery target(s)"),
        )
        .await
    }
}

struct OutboundDeliveryTargetSetHandler {
    facade: Arc<dyn OutboundPreferencesProductFacade>,
    fallback_user_id: UserId,
}

#[async_trait]
impl LocalDevSyntheticCapabilityHandler for OutboundDeliveryTargetSetHandler {
    fn validate_provider_arguments(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<(), AgentLoopHostError> {
        parse_target_id(arguments).map(|_| ())
    }

    async fn invoke(
        &self,
        invocation: LocalDevSyntheticCapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        let target_id = parse_target_id(&invocation.input)?;
        let caller = caller_for_run(&invocation, &self.fallback_user_id);
        let response = self
            .facade
            .set_outbound_preferences(
                caller,
                RebornSetOutboundPreferencesRequest {
                    final_reply_target_id: Some(target_id),
                },
            )
            .await
            .map_err(|error| outbound_delivery_host_error("set_target", error))?;
        let target_name = response
            .final_reply_target
            .as_ref()
            .map(|target| target.display_name.as_str())
            .unwrap_or("delivery target")
            .to_string();
        let output = serde_json::to_value(response).map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                "outbound delivery target set output serialization failed",
            )
        })?;
        write_completed_result(
            invocation,
            output,
            format!("final replies will be delivered to {target_name}"),
        )
        .await
    }
}

async fn write_completed_result(
    invocation: LocalDevSyntheticCapabilityInvocation,
    output: serde_json::Value,
    safe_summary: String,
) -> Result<CapabilityOutcome, AgentLoopHostError> {
    let (result_ref, byte_len) = invocation
        .result_writer
        .write_capability_result(CapabilityResultWrite {
            run_context: &invocation.run_context,
            input_ref: &invocation.request.input_ref,
            invocation_id: InvocationId::new(),
            capability_id: &invocation.request.capability_id,
            output,
            display_preview: None,
        })
        .await?;
    Ok(CapabilityOutcome::Completed(CapabilityResultMessage {
        result_ref,
        safe_summary,
        progress: CapabilityProgress::MadeProgress,
        terminate_hint: false,
        byte_len,
    }))
}

fn caller_for_run(
    invocation: &LocalDevSyntheticCapabilityInvocation,
    fallback_user_id: &UserId,
) -> WebUiAuthenticatedCaller {
    let user_id = invocation
        .run_context
        .scope
        .explicit_owner_user_id()
        .cloned()
        .or_else(|| {
            invocation
                .run_context
                .actor
                .as_ref()
                .map(|actor| actor.user_id.clone())
        })
        .unwrap_or_else(|| fallback_user_id.clone());
    WebUiAuthenticatedCaller::new(
        invocation.run_context.scope.tenant_id.clone(),
        user_id,
        invocation.run_context.scope.agent_id.clone(),
        invocation.run_context.scope.project_id.clone(),
    )
}

fn outbound_delivery_targets_list_input_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "channel": {
                "type": "string",
                "description": "Optional product/channel filter such as slack."
            }
        },
        "additionalProperties": false
    })
}

fn outbound_delivery_target_set_input_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "target_id": {
                "type": "string",
                "description": "Opaque target_id returned by builtin__outbound_delivery_targets_list."
            }
        },
        "required": ["target_id"],
        "additionalProperties": false
    })
}

fn parse_optional_channel(input: &serde_json::Value) -> Result<Option<String>, AgentLoopHostError> {
    match input.get("channel") {
        None => Ok(None),
        Some(value) => value
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| Some(value.to_string()))
            .ok_or_else(|| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "outbound delivery target list channel must be a non-empty string",
                )
            }),
    }
}

fn parse_target_id(
    input: &serde_json::Value,
) -> Result<RebornOutboundDeliveryTargetId, AgentLoopHostError> {
    let target_id = input
        .get("target_id")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "outbound delivery target set requires a target_id string",
            )
        })?;
    RebornOutboundDeliveryTargetId::new(target_id).map_err(|_| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "outbound delivery target_id is invalid",
        )
    })
}

fn outbound_delivery_host_error(
    operation: &'static str,
    error: RebornServicesError,
) -> AgentLoopHostError {
    let kind = match error.code {
        RebornServicesErrorCode::InvalidRequest | RebornServicesErrorCode::NotFound => {
            AgentLoopHostErrorKind::InvalidInvocation
        }
        RebornServicesErrorCode::Unauthenticated | RebornServicesErrorCode::Forbidden => {
            AgentLoopHostErrorKind::Unauthorized
        }
        RebornServicesErrorCode::Conflict | RebornServicesErrorCode::RateLimited => {
            AgentLoopHostErrorKind::Unavailable
        }
        RebornServicesErrorCode::Unavailable => AgentLoopHostErrorKind::Unavailable,
        RebornServicesErrorCode::Internal => AgentLoopHostErrorKind::Internal,
    };
    ironclaw_loop_support::raw_agent_loop_host_error(
        "local_dev_outbound_delivery",
        operation,
        kind,
        "outbound delivery target operation failed",
        error,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_target_id_rejects_missing_target_id() {
        let error =
            parse_target_id(&serde_json::json!({})).expect_err("missing target id should fail");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    }

    #[test]
    fn parse_optional_channel_rejects_empty_channel() {
        let error = parse_optional_channel(&serde_json::json!({"channel": "  "}))
            .expect_err("empty channel should fail");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    }
}
