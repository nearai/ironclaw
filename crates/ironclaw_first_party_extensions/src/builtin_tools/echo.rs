use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_host_api::{EffectKind, PermissionMode};
use serde_json::Value;

use ironclaw_host_runtime::FirstPartyCapabilityError;

use super::{first_party_capability_manifest, input_error, resource_profile};

pub const ECHO_CAPABILITY_ID: &str = "builtin.echo";

pub(super) fn manifest() -> Result<CapabilityManifest, ExtensionError> {
    first_party_capability_manifest(
        ECHO_CAPABILITY_ID,
        "Echo a message",
        vec![EffectKind::DispatchCapability],
        PermissionMode::Allow,
        resource_profile(),
    )
}

pub(super) fn dispatch(input: &Value) -> Result<Value, FirstPartyCapabilityError> {
    let message = input
        .get("message")
        .and_then(Value::as_str)
        .ok_or_else(input_error)?;
    Ok(Value::String(message.to_string()))
}
