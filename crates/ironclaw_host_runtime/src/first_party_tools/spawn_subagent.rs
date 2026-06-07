use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_host_api::{EffectKind, PermissionMode};

pub const SPAWN_SUBAGENT_CAPABILITY_ID: &str = "builtin.spawn_subagent";

pub(crate) fn manifest() -> Result<CapabilityManifest, ExtensionError> {
    super::first_party_capability_manifest(
        SPAWN_SUBAGENT_CAPABILITY_ID,
        "Authorize a scoped child subagent run",
        vec![EffectKind::DispatchCapability, EffectKind::SpawnProcess],
        PermissionMode::Ask,
        super::resource_profile(),
    )
}

pub(crate) fn dispatch() -> serde_json::Value {
    serde_json::json!({
        "authorized": true,
    })
}
