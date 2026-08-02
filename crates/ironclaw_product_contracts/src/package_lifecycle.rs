//! Neutral package lifecycle vocabulary.
//!
//! These are value types for extension/package lifecycle projections and
//! commands. Product-facing services may wrap or project them, but the values
//! themselves are host API vocabulary so generic extension services can share
//! them without depending on product workflow.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use ironclaw_host_api::error::HostApiError;

use ironclaw_extension_contracts::{
    channel::ChannelPresentation,
    hosted_mcp::RegisterHostedMcpRequest,
    lifecycle_id::{LifecycleBlockerRef, LifecyclePackageId},
    state::{InstallationState, LifecyclePublicState},
    surface::CapabilitySurfaceKind,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecyclePackageKind {
    Extension,
    Skill,
    Mcp,
    Wasm,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecyclePackageRef {
    pub kind: LifecyclePackageKind,
    pub id: LifecyclePackageId,
}

impl LifecyclePackageRef {
    pub fn new(kind: LifecyclePackageKind, id: impl Into<String>) -> Result<Self, HostApiError> {
        Ok(Self {
            kind,
            id: LifecyclePackageId::new(id)?,
        })
    }

    pub fn require_kind(&self, expected: LifecyclePackageKind) -> Result<(), HostApiError> {
        if self.kind == expected {
            return Ok(());
        }
        Err(HostApiError::InvariantViolation {
            reason: format!(
                "lifecycle package kind mismatch: expected {:?}, got {:?}",
                expected, self.kind
            ),
        })
    }

    pub fn require_extension(self) -> Result<Self, HostApiError> {
        self.require_kind(LifecyclePackageKind::Extension)?;
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LifecycleReadinessBlocker {
    Setup { ref_id: Option<LifecycleBlockerRef> },
    Auth { ref_id: Option<LifecycleBlockerRef> },
    Pairing { ref_id: Option<LifecycleBlockerRef> },
    Approval { ref_id: Option<LifecycleBlockerRef> },
    Policy { ref_id: Option<LifecycleBlockerRef> },
    Credential { ref_id: Option<LifecycleBlockerRef> },
    Runtime { ref_id: Option<LifecycleBlockerRef> },
}

impl LifecycleReadinessBlocker {
    pub fn runtime(ref_id: impl Into<Option<String>>) -> Result<Self, HostApiError> {
        Ok(Self::Runtime {
            ref_id: validate_optional_ref(ref_id.into())?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum LifecycleProductAction {
    ExtensionRegisterHostedMcp {
        request: RegisterHostedMcpRequest,
    },
    ExtensionSearch {
        query: String,
    },
    ExtensionList,
    ExtensionInstall {
        package_ref: LifecyclePackageRef,
    },
    ExtensionAuth {
        package_ref: LifecyclePackageRef,
    },
    ExtensionActivate {
        package_ref: LifecyclePackageRef,
    },
    ExtensionConfigure {
        package_ref: LifecyclePackageRef,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        payload: Option<Value>,
    },
    ExtensionRemove {
        package_ref: LifecyclePackageRef,
    },
    SkillSearch {
        query: String,
    },
    SkillInstall {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<LifecyclePackageId>,
        content: String,
    },
    SkillRemove {
        package_ref: LifecyclePackageRef,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleCommandKind {
    /// Kept only so `LifecycleProductAction::command_kind()` stays total; deliberately absent from `ALL`.
    /// Not reachable as a chat command — registration is WebUI-only via `EXTENSION_REGISTER_HOSTED_MCP_CAPABILITY`.
    ExtensionRegisterHostedMcp,
    ExtensionSearch,
    ExtensionList,
    ExtensionInstall,
    ExtensionAuth,
    ExtensionActivate,
    ExtensionConfigure,
    ExtensionRemove,
    SkillSearch,
    SkillInstall,
    SkillRemove,
}

impl LifecycleCommandKind {
    pub const ALL: [Self; 10] = [
        Self::ExtensionSearch,
        Self::ExtensionList,
        Self::ExtensionInstall,
        Self::ExtensionAuth,
        Self::ExtensionActivate,
        Self::ExtensionConfigure,
        Self::ExtensionRemove,
        Self::SkillSearch,
        Self::SkillInstall,
        Self::SkillRemove,
    ];

    pub const fn command_name(self) -> &'static str {
        match self {
            Self::ExtensionRegisterHostedMcp => "extension_register_hosted_mcp",
            Self::ExtensionSearch => "extension_search",
            Self::ExtensionList => "extension_list",
            Self::ExtensionInstall => "extension_install",
            Self::ExtensionAuth => "extension_auth",
            Self::ExtensionActivate => "extension_activate",
            Self::ExtensionConfigure => "extension_configure",
            Self::ExtensionRemove => "extension_remove",
            Self::SkillSearch => "skill_search",
            Self::SkillInstall => "skill_install",
            Self::SkillRemove => "skill_remove",
        }
    }

    pub fn from_command_name(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|kind| kind.command_name() == name)
    }
}

impl LifecycleProductAction {
    pub fn command_kind(&self) -> LifecycleCommandKind {
        match self {
            Self::ExtensionRegisterHostedMcp { .. } => {
                LifecycleCommandKind::ExtensionRegisterHostedMcp
            }
            Self::ExtensionSearch { .. } => LifecycleCommandKind::ExtensionSearch,
            Self::ExtensionList => LifecycleCommandKind::ExtensionList,
            Self::ExtensionInstall { .. } => LifecycleCommandKind::ExtensionInstall,
            Self::ExtensionAuth { .. } => LifecycleCommandKind::ExtensionAuth,
            Self::ExtensionActivate { .. } => LifecycleCommandKind::ExtensionActivate,
            Self::ExtensionConfigure { .. } => LifecycleCommandKind::ExtensionConfigure,
            Self::ExtensionRemove { .. } => LifecycleCommandKind::ExtensionRemove,
            Self::SkillSearch { .. } => LifecycleCommandKind::SkillSearch,
            Self::SkillInstall { .. } => LifecycleCommandKind::SkillInstall,
            Self::SkillRemove { .. } => LifecycleCommandKind::SkillRemove,
        }
    }

    pub fn command_name(&self) -> &'static str {
        self.command_kind().command_name()
    }

    /// Returns the `LifecyclePackageRef` when this action targets a single
    /// package, otherwise `None`.
    pub fn package_ref(&self) -> Option<&LifecyclePackageRef> {
        match self {
            Self::ExtensionInstall { package_ref }
            | Self::ExtensionAuth { package_ref }
            | Self::ExtensionActivate { package_ref }
            | Self::ExtensionConfigure { package_ref, .. }
            | Self::ExtensionRemove { package_ref }
            | Self::SkillRemove { package_ref } => Some(package_ref),
            Self::ExtensionRegisterHostedMcp { .. }
            | Self::ExtensionSearch { .. }
            | Self::SkillSearch { .. }
            | Self::SkillInstall { .. } => None,
            Self::ExtensionList => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelConnectStrategy {
    InboundProofCode,
    AdminManagedChannels,
    WebGeneratedCode,
    QrCode,
    #[serde(rename = "oauth")]
    OAuth,
}

impl ChannelConnectStrategy {
    /// The wire form, identical to the serde representation. Product surfaces
    /// render the strategy as a string (the connect affordance) and must not
    /// re-derive it with `format!("{:?}")`, which would emit `WebGeneratedCode`
    /// where the wire contract is `web_generated_code`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InboundProofCode => "inbound_proof_code",
            Self::AdminManagedChannels => "admin_managed_channels",
            Self::WebGeneratedCode => "web_generated_code",
            Self::QrCode => "qr_code",
            Self::OAuth => "oauth",
        }
    }
}

/// Structured "the caller must connect this channel" affordance attached to a
/// channel-extension activation result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelConnectionRequirement {
    pub channel: String,
    pub display_name: String,
    pub strategy: ChannelConnectStrategy,
    pub instructions: String,
    pub input_placeholder: String,
    pub submit_label: String,
    pub error_message: String,
}

/// Presence-only projection of one manifest-declared channel-config field.
/// Secret fields report `provided` only; stored values are never echoed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelConfigField {
    /// The manifest-declared field handle (the submit key).
    pub name: String,
    /// Operator-facing label from the manifest.
    pub label: String,
    pub secret: bool,
    pub provided: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LifecycleProductPayload {
    ExtensionSearch {
        extensions: Vec<LifecycleSearchExtensionSummary>,
        count: usize,
    },
    ExtensionList {
        extensions: Vec<LifecycleInstalledExtensionSummary>,
        count: usize,
    },
    ExtensionInstall {
        installed: bool,
        visible_capability_ids: Vec<String>,
        #[serde(default)]
        next_step: String,
    },
    ExtensionActivate {
        activated: bool,
        #[serde(default)]
        visible_capability_ids: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        connection_required: Option<ChannelConnectionRequirement>,
    },
    ExtensionRemove {
        removed: bool,
    },
    SkillSearch {
        skills: Vec<LifecycleSkillSummary>,
        count: usize,
        limit: usize,
        truncated: bool,
    },
    SkillInstall {
        installed: bool,
        name: LifecyclePackageId,
    },
    SkillRemove {
        removed: bool,
        name: LifecyclePackageId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleChannelDirections {
    pub inbound: bool,
    pub outbound: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleExtensionSummary {
    pub package_ref: LifecyclePackageRef,
    pub name: String,
    pub version: String,
    pub description: String,
    pub source: LifecycleExtensionSource,
    pub runtime_kind: LifecycleExtensionRuntimeKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub surface_kinds: Vec<CapabilitySurfaceKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_directions: Option<LifecycleChannelDirections>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_connection: Option<ChannelConnectionRequirement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_presentation: Option<ChannelPresentation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub visible_capability_ids: Vec<String>,
    pub visible_read_only_capability_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub credential_requirements: Vec<LifecycleExtensionCredentialRequirement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub onboarding: Option<LifecycleExtensionOnboarding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleSearchExtensionSummary {
    #[serde(flatten)]
    pub summary: LifecycleExtensionSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installation_phase: Option<InstallationState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleInstallScope {
    Shared,
    Private,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleInstalledExtensionSummary {
    pub summary: LifecycleExtensionSummary,
    pub phase: InstallationState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_scope: Option<LifecycleInstallScope>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleExtensionCredentialRequirement {
    pub name: String,
    pub provider: String,
    pub required: bool,
    pub setup: LifecycleExtensionCredentialSetup,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleExtensionOnboarding {
    pub instructions: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_next_step: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LifecycleExtensionCredentialSetup {
    ManualToken,
    #[serde(rename = "oauth")]
    OAuth {
        scopes: Vec<String>,
    },
    Pairing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleExtensionSource {
    HostBundled,
    Installed,
    Registry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleExtensionRuntimeKind {
    WasmTool,
    McpServer,
    FirstParty,
    System,
    Script,
}

impl LifecycleExtensionRuntimeKind {
    pub fn runtime_wire_name(self) -> &'static str {
        match self {
            Self::McpServer => "mcp",
            Self::FirstParty => "first_party",
            Self::System => "system",
            Self::WasmTool => "wasm",
            Self::Script => "script",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleSkillSummary {
    pub name: LifecyclePackageId,
    pub version: String,
    pub description: String,
    pub source: LifecycleSkillSource,
    pub keywords: Vec<String>,
    pub tags: Vec<String>,
    pub requires_skills: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleSkillSource {
    System,
    User,
    Installed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleProductResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_ref: Option<LifecyclePackageRef>,
    pub phase: InstallationState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<LifecycleReadinessBlocker>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<LifecycleProductPayload>,
}

/// Project internal lifecycle checkpoints to the public extension lifecycle
/// vocabulary used by CLI, WebUI setup responses, and model-visible lifecycle
/// capability output.
///
/// Internally the host distinguishes `Installed`, `Configured`, `Disabled`,
/// `Failed`, and `Removed` to drive recovery and diagnostics. Product surfaces
/// expose only the user-actionable state machine: `uninstalled`,
/// `setup_needed`, and `active`.
pub fn public_lifecycle_response_json(
    response: &LifecycleProductResponse,
) -> Result<Value, serde_json::Error> {
    let mut value = serde_json::to_value(response)?;
    project_public_lifecycle_states(&mut value);
    Ok(value)
}

pub fn project_public_lifecycle_states(value: &mut Value) {
    match value {
        Value::Array(values) => {
            for value in values {
                project_public_lifecycle_states(value);
            }
        }
        Value::Object(values) => {
            for (key, value) in values {
                if matches!(
                    key.as_str(),
                    "phase" | "installation_phase" | "installation_state"
                ) && let Value::String(text) = value
                    && let Some(projected) = public_lifecycle_state(text)
                {
                    *text = projected.to_string();
                    continue;
                }
                project_public_lifecycle_states(value);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn public_lifecycle_state(state: &str) -> Option<&'static str> {
    // One definition of the checkpoint→public collapse: `LifecyclePublicState`.
    // This string form exists only to re-project already-serialized payloads.
    InstallationState::from_wire(state)
        .map(|state| LifecyclePublicState::from_host_checkpoint(state).as_str())
}

impl LifecycleProductResponse {
    pub fn projection(
        package_ref: Option<LifecyclePackageRef>,
        phase: InstallationState,
        blockers: Vec<LifecycleReadinessBlocker>,
    ) -> Self {
        Self {
            package_ref,
            phase,
            blockers,
            message: None,
            payload: None,
        }
    }
}

fn validate_optional_ref(
    value: Option<String>,
) -> Result<Option<LifecycleBlockerRef>, HostApiError> {
    value.map(LifecycleBlockerRef::new).transpose()
}

#[cfg(test)]
mod tests {
    #[test]
    fn channel_connect_strategy_as_str_matches_the_serde_wire_form() {
        for strategy in [
            ChannelConnectStrategy::InboundProofCode,
            ChannelConnectStrategy::AdminManagedChannels,
            ChannelConnectStrategy::WebGeneratedCode,
            ChannelConnectStrategy::QrCode,
            ChannelConnectStrategy::OAuth,
        ] {
            assert_eq!(
                serde_json::to_value(strategy).expect("serialize"),
                serde_json::Value::String(strategy.as_str().to_string()),
                "as_str must not drift from the serde wire form"
            );
        }
    }

    use super::*;

    #[test]
    fn lifecycle_package_ref_rejects_empty_id() {
        let error = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "")
            .expect_err("empty package id rejected");
        assert!(error.to_string().contains("lifecycle_package"));
    }

    #[test]
    fn lifecycle_package_ref_requires_kind() {
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Skill, "demo").expect("valid ref");
        let error = package_ref
            .require_kind(LifecyclePackageKind::Extension)
            .expect_err("wrong kind rejected");
        assert!(error.to_string().contains("kind mismatch"));
    }
}
