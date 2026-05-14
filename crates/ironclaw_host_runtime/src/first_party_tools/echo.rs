use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_host_api::{CapabilityId, EffectKind, PermissionMode};
use serde_json::{Value, json};

use crate::FirstPartyCapabilityError;

use super::{input_error, resource_profile};

pub const ECHO_CAPABILITY_ID: &str = "builtin.echo";

pub(super) fn manifest() -> Result<CapabilityManifest, ExtensionError> {
    Ok(CapabilityManifest {
        id: CapabilityId::new(ECHO_CAPABILITY_ID)?,
        description: "Echo a message".to_string(),
        effects: vec![EffectKind::DispatchCapability],
        default_permission: PermissionMode::Allow,
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "message": { "type": "string", "description": "Message to echo" }
            },
            "required": ["message"]
        }),
        resource_profile: resource_profile(),
    })
}

pub(super) fn dispatch(input: &Value) -> Result<Value, FirstPartyCapabilityError> {
    let message = input
        .get("message")
        .and_then(Value::as_str)
        .ok_or_else(input_error)?;
    Ok(Value::String(message.to_string()))
}
