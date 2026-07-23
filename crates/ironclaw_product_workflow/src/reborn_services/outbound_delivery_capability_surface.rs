use std::sync::Arc;

use super::{
    OutboundPreferencesProductFacade, RebornOperatorToolInfo, RebornOutboundDeliveryTargetId,
    RebornOutboundDeliveryTargetListResponse, RebornOutboundPreferencesResponse,
    RebornServicesError, RebornSetOutboundPreferencesRequest, WebUiAuthenticatedCaller,
};
use ironclaw_host_api::{EffectKind, ExtensionId, HostApiError, PermissionMode};
use thiserror::Error;

pub const OUTBOUND_DELIVERY_SYNTHETIC_PROVIDER_ID: &str = "builtin";

pub const OUTBOUND_DELIVERY_TARGETS_LIST_CAPABILITY_ID: &str =
    "builtin.outbound_delivery_targets_list";
pub const OUTBOUND_DELIVERY_TARGETS_LIST_PROVIDER_TOOL_NAME: &str =
    "builtin__outbound_delivery_targets_list";
pub const OUTBOUND_DELIVERY_TARGETS_LIST_DESCRIPTION: &str = "List available outbound delivery targets, such as direct messages or channels exposed by installed integrations. These targets route only final replies and routine/trigger results. This delivery-routing tool cannot read conversations, message content, membership, status, or profiles; use the corresponding integration's read capabilities for those tasks. When the user asks to send routine or trigger results through an external product or channel, call this before builtin__trigger_create and before saying a delivery product is unavailable or asking the user to reconnect it. Pass a listed id as builtin__trigger_create's delivery_target_id to route that trigger, or to builtin__outbound_delivery_target_set to change the user-wide default.";

pub const OUTBOUND_DELIVERY_TARGET_SET_CAPABILITY_ID: &str = "builtin.outbound_delivery_target_set";
pub const OUTBOUND_DELIVERY_TARGET_SET_PROVIDER_TOOL_NAME: &str =
    "builtin__outbound_delivery_target_set";
pub const OUTBOUND_DELIVERY_TARGET_SET_DESCRIPTION: &str = "Set the current user's DEFAULT final-reply outbound delivery target, such as a direct message or channel exposed by an installed integration, to an id returned by builtin__outbound_delivery_targets_list. This default is shared by replies and by every trigger without its own delivery_target_id \u{2014} changing it re-routes all of them, so to route a single trigger's results pass delivery_target_id to builtin__trigger_create instead of changing this. Approval may be required before the preference is changed.";

pub fn outbound_delivery_synthetic_provider() -> Result<ExtensionId, HostApiError> {
    ExtensionId::new(OUTBOUND_DELIVERY_SYNTHETIC_PROVIDER_ID)
}

pub fn outbound_delivery_target_set_operator_tool_info(
    provider: ExtensionId,
) -> Result<RebornOperatorToolInfo, HostApiError> {
    Ok(RebornOperatorToolInfo {
        capability_id: ironclaw_host_api::CapabilityId::new(
            OUTBOUND_DELIVERY_TARGET_SET_CAPABILITY_ID,
        )?,
        provider,
        description: Arc::from(OUTBOUND_DELIVERY_TARGET_SET_DESCRIPTION),
        default_permission: PermissionMode::Ask,
        effects: Arc::from([EffectKind::DispatchCapability, EffectKind::ExternalWrite]),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundDeliveryTargetsListInput {
    channel: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundDeliveryTargetSetInput {
    target_id: RebornOutboundDeliveryTargetId,
}

impl OutboundDeliveryTargetSetInput {
    pub fn target_id(&self) -> &RebornOutboundDeliveryTargetId {
        &self.target_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{reason}")]
pub struct OutboundDeliveryCapabilityInputError {
    reason: String,
}

impl OutboundDeliveryCapabilityInputError {
    fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

pub async fn list_outbound_delivery_targets_for_model(
    facade: &dyn OutboundPreferencesProductFacade,
    caller: WebUiAuthenticatedCaller,
    input: OutboundDeliveryTargetsListInput,
) -> Result<RebornOutboundDeliveryTargetListResponse, RebornServicesError> {
    let mut response = facade.list_outbound_delivery_targets(caller).await?;
    if let Some(channel_filter) = input.channel {
        response.targets.retain(|option| {
            option
                .target
                .channel
                .as_str()
                .eq_ignore_ascii_case(channel_filter.as_str())
        });
    }
    Ok(response)
}

pub async fn set_outbound_delivery_target_for_model(
    facade: &dyn OutboundPreferencesProductFacade,
    caller: WebUiAuthenticatedCaller,
    input: OutboundDeliveryTargetSetInput,
) -> Result<RebornOutboundPreferencesResponse, RebornServicesError> {
    facade
        .set_outbound_preferences(
            caller,
            RebornSetOutboundPreferencesRequest {
                final_reply_target_id: Some(input.target_id),
            },
        )
        .await
}

pub fn outbound_delivery_targets_list_input_schema() -> serde_json::Value {
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

pub fn outbound_delivery_target_set_input_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "target_id": {
                "type": "string",
                "description": "Target id returned by builtin__outbound_delivery_targets_list."
            }
        },
        "required": ["target_id"],
        "additionalProperties": false
    })
}

pub fn parse_outbound_delivery_targets_list_input(
    input: &serde_json::Value,
) -> Result<OutboundDeliveryTargetsListInput, OutboundDeliveryCapabilityInputError> {
    let input = input_object(input, "outbound delivery target list", &["channel"])?;
    let channel = match input.get("channel") {
        None => None,
        Some(value) => Some(
            value
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .ok_or_else(|| {
                    OutboundDeliveryCapabilityInputError::new(
                        "outbound delivery target list channel must be a non-empty string",
                    )
                })?,
        ),
    };
    Ok(OutboundDeliveryTargetsListInput { channel })
}

pub fn parse_outbound_delivery_target_set_input(
    input: &serde_json::Value,
) -> Result<OutboundDeliveryTargetSetInput, OutboundDeliveryCapabilityInputError> {
    let input = input_object(input, "outbound delivery target set", &["target_id"])?;
    let value = input
        .get("target_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            OutboundDeliveryCapabilityInputError::new(
                "outbound delivery target set target_id must be a string",
            )
        })?;
    let target_id = RebornOutboundDeliveryTargetId::new(value).map_err(|reason| {
        OutboundDeliveryCapabilityInputError::new(format!(
            "outbound delivery target set target_id is invalid: {reason}"
        ))
    })?;
    Ok(OutboundDeliveryTargetSetInput { target_id })
}

fn input_object<'a>(
    input: &'a serde_json::Value,
    capability_name: &'static str,
    allowed_fields: &[&str],
) -> Result<&'a serde_json::Map<String, serde_json::Value>, OutboundDeliveryCapabilityInputError> {
    let object = input.as_object().ok_or_else(|| {
        OutboundDeliveryCapabilityInputError::new(format!(
            "{capability_name} input must be an object"
        ))
    })?;
    if let Some(field) = object
        .keys()
        .find(|field| !allowed_fields.contains(&field.as_str()))
    {
        return Err(OutboundDeliveryCapabilityInputError::new(format!(
            "{capability_name} input contains unsupported field `{field}`"
        )));
    }
    Ok(object)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_input_rejects_unknown_fields() {
        let err = parse_outbound_delivery_targets_list_input(&serde_json::json!({
            "channel": "slack",
            "extra": true
        }))
        .expect_err("unknown field should fail");

        assert!(err.to_string().contains("unsupported field `extra`"));
    }

    #[test]
    fn set_input_validates_target_id_shape() {
        let err = parse_outbound_delivery_target_set_input(&serde_json::json!({
            "target_id": "bad\nid"
        }))
        .expect_err("invalid target id should fail");

        assert!(err.to_string().contains("target_id is invalid"));
    }
}
