//! Product-facing lifecycle contract for Reborn package UX.
//!
//! This module deliberately models package/install lifecycle separately from
//! auth, approval, pairing, and policy gates. Those remain owned by their
//! dedicated services; lifecycle projections may only carry redacted refs to
//! the owning interaction.

use std::fmt;

use async_trait::async_trait;
use ironclaw_host_api::{
    AgentId, CapabilitySurfaceKind, ChannelPresentation, InstallationState, ProjectId, TenantId,
    UserId,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

use crate::{ProductCommandContext, ProductWorkflowError, RebornChannelConnectStrategy};

pub(crate) const LIFECYCLE_ID_MAX_BYTES: usize = 256;
const LIFECYCLE_REF_MAX_BYTES: usize = 512;

macro_rules! bounded_lifecycle_string {
    ($name:ident, $label:literal, $max:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, ProductWorkflowError> {
                validate_lifecycle_string(value.into(), $label, $max).map(Self)
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(de::Error::custom)
            }
        }
    };
}

bounded_lifecycle_string!(
    LifecyclePackageId,
    "lifecycle package id",
    LIFECYCLE_ID_MAX_BYTES
);
bounded_lifecycle_string!(
    LifecycleBlockerRef,
    "lifecycle blocker ref",
    LIFECYCLE_REF_MAX_BYTES
);

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
    pub fn new(
        kind: LifecyclePackageKind,
        id: impl Into<String>,
    ) -> Result<Self, ProductWorkflowError> {
        Ok(Self {
            kind,
            id: LifecyclePackageId::new(id)?,
        })
    }

    pub fn require_kind(&self, expected: LifecyclePackageKind) -> Result<(), ProductWorkflowError> {
        if self.kind == expected {
            return Ok(());
        }
        Err(ProductWorkflowError::InvalidBindingRequest {
            reason: format!(
                "lifecycle package kind mismatch: expected {:?}, got {:?}",
                expected, self.kind
            ),
        })
    }

    pub fn require_extension(self) -> Result<Self, ProductWorkflowError> {
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
    pub fn runtime(ref_id: impl Into<Option<String>>) -> Result<Self, ProductWorkflowError> {
        Ok(Self::Runtime {
            ref_id: validate_optional_ref(ref_id.into())?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum LifecycleProductAction {
    ExtensionSearch {
        query: String,
    },
    ExtensionList,
    ExtensionInstall {
        package_ref: LifecyclePackageRef,
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
    ExtensionSearch,
    ExtensionList,
    ExtensionInstall,
    ExtensionRemove,
    SkillSearch,
    SkillInstall,
    SkillRemove,
}

impl LifecycleCommandKind {
    pub const ALL: [Self; 7] = [
        Self::ExtensionSearch,
        Self::ExtensionList,
        Self::ExtensionInstall,
        Self::ExtensionRemove,
        Self::SkillSearch,
        Self::SkillInstall,
        Self::SkillRemove,
    ];

    pub const fn command_name(self) -> &'static str {
        match self {
            Self::ExtensionSearch => "extension_search",
            Self::ExtensionList => "extension_list",
            Self::ExtensionInstall => "extension_install",
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
            Self::ExtensionSearch { .. } => LifecycleCommandKind::ExtensionSearch,
            Self::ExtensionList => LifecycleCommandKind::ExtensionList,
            Self::ExtensionInstall { .. } => LifecycleCommandKind::ExtensionInstall,
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
            | Self::ExtensionRemove { package_ref }
            | Self::SkillRemove { package_ref } => Some(package_ref),
            Self::ExtensionSearch { .. } | Self::SkillSearch { .. } | Self::SkillInstall { .. } => {
                None
            }
            Self::ExtensionList => None,
        }
    }
}

/// Structured "the caller must connect this channel" affordance attached to a
/// channel-extension activation result. Carried verbatim (snake_case) to the
/// WebChat as a capability display preview so the in-chat pairing panel is
/// driven by structured state, never by parsing the activation message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelConnectionRequirement {
    pub channel: String,
    /// User-facing channel name from the manifest (S5 wire gap). The frontend
    /// renders this instead of deriving a label from the channel id, so the
    /// connect affordance carries no per-extension copy.
    pub display_name: String,
    pub strategy: RebornChannelConnectStrategy,
    pub instructions: String,
    pub input_placeholder: String,
    pub submit_label: String,
    pub error_message: String,
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

/// Directional shape of an extension's channel surface, derived from the
/// manifest's product-adapter capability flags: `inbound` when the surface
/// receives external messages (`inbound_messages`), `outbound` when the host
/// can push final replies/notifications to it (`external_final_reply_push`).
/// The agent-facing rule this pins: final answers are delivered by the host
/// on outbound channel surfaces; model tools never deliver them.
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
    /// Present iff `surface_kinds` contains [`CapabilitySurfaceKind::Channel`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_directions: Option<LifecycleChannelDirections>,
    /// Connect affordance for the channel surface (strategy + copy), present
    /// when the surface requires a caller-scoped account binding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_connection: Option<ChannelConnectionRequirement>,
    /// The channel surface's declared `[channel.presentation]` facts (markdown
    /// support, message length cap), present iff `surface_kinds` contains
    /// `Channel`. Fed into prompt construction so the model formats replies to
    /// fit the channel it is answering on (OUT-11).
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
    /// The installed state of this catalog result for the caller, or `None`
    /// when the caller has no visible installation of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installation_phase: Option<LifecyclePublicState>,
}

/// Compatibility projection of persisted installation ownership.
///
/// Current lifecycle operations always return `Private`: tenant-level admin
/// configuration is not user membership. `Shared` remains only so a legacy
/// tenant-owned row can be read and narrowed during restore without changing
/// the durable wire enum in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleInstallScope {
    /// Legacy tenant-owned installation; never created by current lifecycle.
    Shared,
    /// Installed privately by the caller — visible only to them.
    Private,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleInstalledExtensionSummary {
    pub summary: LifecycleExtensionSummary,
    /// The projected installation state (§6.1) for the caller's installation.
    pub phase: LifecyclePublicState,
    /// `None` only when the caller has no visible installation (projection of
    /// an uninstalled package); list responses always carry `Some`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_scope: Option<LifecycleInstallScope>,
}

/// The complete public extension lifecycle vocabulary.
///
/// Host/runtime checkpoints such as `Installed`, `Configured`, `Disabled`,
/// `Failed`, and `Removed` are implementation details. Product surfaces must
/// never expose them as additional user actions or resting states: membership
/// is absent (`uninstalled`), present but not ready (`setup_needed`), or
/// present and callable (`active`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecyclePublicState {
    Uninstalled,
    SetupNeeded,
    Active,
}

impl LifecyclePublicState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Uninstalled => "uninstalled",
            Self::SetupNeeded => "setup_needed",
            Self::Active => "active",
        }
    }

    /// Collapse a host-owned internal checkpoint onto the product contract.
    pub fn from_host_checkpoint(state: InstallationState) -> Self {
        match state {
            InstallationState::Active => Self::Active,
            InstallationState::Removed => Self::Uninstalled,
            InstallationState::Installed
            | InstallationState::Configured
            | InstallationState::Disabled
            | InstallationState::Failed
            | InstallationState::Unsupported => Self::SetupNeeded,
        }
    }
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
    /// Channel pairing (host-issued code consumed on the external side).
    Pairing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleExtensionSource {
    HostBundled,
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
    /// Honest runtime name for the wire: implementation detail, clearly
    /// labeled — never product taxonomy (surfaces carry that).
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleProductResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_ref: Option<LifecyclePackageRef>,
    /// The package's public lifecycle state. Internal installation/runtime
    /// checkpoints are deliberately collapsed before crossing this boundary.
    pub phase: LifecyclePublicState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<LifecycleReadinessBlocker>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<LifecycleProductPayload>,
}

impl LifecycleProductResponse {
    pub fn projection(
        package_ref: Option<LifecyclePackageRef>,
        phase: LifecyclePublicState,
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

    /// Return the caller-visible extension packages that are currently
    /// callable.
    ///
    /// This deliberately derives authority from the public, caller-scoped
    /// lifecycle projection. A provider-global runtime catalog may contain
    /// tools discovered by another ready member, but only packages projected
    /// as [`LifecyclePublicState::Active`] for this caller may cross into that
    /// caller's model-visible or executable capability surface.
    pub fn callable_extension_package_refs(
        &self,
    ) -> Result<Vec<LifecyclePackageRef>, ProductWorkflowError> {
        let Some(LifecycleProductPayload::ExtensionList { extensions, .. }) = &self.payload else {
            return Err(ProductWorkflowError::Transient {
                reason: "caller extension readiness projection is unavailable".to_string(),
            });
        };
        Ok(extensions
            .iter()
            .filter(|extension| extension.phase == LifecyclePublicState::Active)
            .map(|extension| extension.summary.package_ref.clone())
            .collect())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LifecycleProductSurfaceContext {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<ProjectId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum LifecycleProductContext {
    Command(Box<ProductCommandContext>),
    Surface(LifecycleProductSurfaceContext),
}

#[async_trait]
pub trait LifecycleProductFacade: Send + Sync {
    async fn execute(
        &self,
        context: LifecycleProductContext,
        action: LifecycleProductAction,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError>;

    async fn project_package(
        &self,
        context: LifecycleProductContext,
        package_ref: LifecyclePackageRef,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError>;

    /// Import a standalone extension from an uploaded bundle (zip bytes) — the
    /// WebUI "Install Tool" path. Default is unavailable; only the local runtime
    /// facade implements it.
    async fn import_extension_bundle(
        &self,
        _context: LifecycleProductContext,
        _bundle: Vec<u8>,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        Err(ProductWorkflowError::InvalidBindingRequest {
            reason: "extension import is not supported by this runtime".to_string(),
        })
    }

    /// Redacted activation error for each installed extension whose activation
    /// failed, keyed by extension id — sourced from the durable installation
    /// record's typed `last_error`. The extensions-list facade threads this
    /// into `RebornExtensionInfo::activation_error` so a failed extension shows
    /// *why* it failed instead of collapsing to a bare `installed`/`failed`
    /// state with no reason.
    ///
    /// Default: none. A facade that does not surface durable installation
    /// errors reports no reason and the wire's `activation_error` stays absent;
    /// the production extension-host facade overrides this to read the
    /// installation records' `last_error`.
    async fn installed_activation_errors(
        &self,
        _context: LifecycleProductContext,
    ) -> Result<std::collections::HashMap<String, String>, ProductWorkflowError> {
        Ok(std::collections::HashMap::new())
    }
}

#[derive(Debug, Clone)]
pub struct UnsupportedLifecycleProductFacade {
    runtime_ref: String,
}

impl UnsupportedLifecycleProductFacade {
    pub fn new(runtime_ref: impl Into<String>) -> Result<Self, ProductWorkflowError> {
        Ok(Self {
            runtime_ref: validate_lifecycle_string(
                runtime_ref.into(),
                "unsupported lifecycle runtime ref",
                LIFECYCLE_REF_MAX_BYTES,
            )?,
        })
    }

    pub fn new_static(runtime_ref: &'static str) -> Self {
        debug_assert!(
            validate_lifecycle_string(
                runtime_ref.to_string(),
                "unsupported lifecycle runtime ref",
                LIFECYCLE_REF_MAX_BYTES,
            )
            .is_ok()
        );
        Self {
            runtime_ref: runtime_ref.to_string(),
        }
    }

    fn unsupported_projection(
        &self,
        package_ref: Option<LifecyclePackageRef>,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        Ok(LifecycleProductResponse::projection(
            package_ref,
            LifecyclePublicState::SetupNeeded,
            vec![LifecycleReadinessBlocker::runtime(Some(
                self.runtime_ref.clone(),
            ))?],
        ))
    }
}

#[async_trait]
impl LifecycleProductFacade for UnsupportedLifecycleProductFacade {
    async fn execute(
        &self,
        _context: LifecycleProductContext,
        action: LifecycleProductAction,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        self.unsupported_projection(action.package_ref().cloned())
    }

    async fn project_package(
        &self,
        _context: LifecycleProductContext,
        package_ref: LifecyclePackageRef,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        self.unsupported_projection(Some(package_ref))
    }
}

/// Validates a lifecycle string: non-empty, within byte limit, with optional
/// control-character filtering.
pub(crate) fn validate_lifecycle_string(
    value: String,
    label: &'static str,
    max_bytes: usize,
) -> Result<String, ProductWorkflowError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ProductWorkflowError::InvalidBindingRequest {
            reason: format!("{label} must not be empty"),
        });
    }
    if value.len() > max_bytes {
        return Err(ProductWorkflowError::InvalidBindingRequest {
            reason: format!("{label} must be at most {max_bytes} bytes"),
        });
    }
    if trimmed.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(ProductWorkflowError::InvalidBindingRequest {
            reason: format!("{label} must not contain NUL/control characters"),
        });
    }
    Ok(trimmed.to_string())
}

/// Validates free-form lifecycle text that may contain control characters
/// (e.g. newlines in skill markdown) but still blocks NUL.
pub(crate) fn validate_lifecycle_text(
    value: String,
    label: &'static str,
    max_bytes: usize,
) -> Result<String, ProductWorkflowError> {
    if value.trim().is_empty() {
        return Err(ProductWorkflowError::InvalidBindingRequest {
            reason: format!("{label} must not be empty"),
        });
    }
    if value.len() > max_bytes {
        return Err(ProductWorkflowError::InvalidBindingRequest {
            reason: format!("{label} must be at most {max_bytes} bytes"),
        });
    }
    if value.chars().any(|c| c == '\0') {
        return Err(ProductWorkflowError::InvalidBindingRequest {
            reason: format!("{label} must not contain NUL characters"),
        });
    }
    Ok(value)
}

fn validate_optional_ref(
    value: Option<String>,
) -> Result<Option<LifecycleBlockerRef>, ProductWorkflowError> {
    value.map(LifecycleBlockerRef::new).transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_response_owns_and_serializes_only_public_states() {
        for (state, expected) in [
            (LifecyclePublicState::Uninstalled, "uninstalled"),
            (LifecyclePublicState::SetupNeeded, "setup_needed"),
            (LifecyclePublicState::Active, "active"),
        ] {
            let response = LifecycleProductResponse::projection(None, state, Vec::new());
            assert_eq!(response.phase, state);
            assert_eq!(state.as_str(), expected);
            let wire = serde_json::to_value(response).expect("lifecycle response serializes");
            assert_eq!(wire["phase"], expected, "public state {state:?}");
        }
    }

    #[test]
    fn public_state_wire_round_trips_as_the_same_public_state() {
        for (wire, expected_state) in [
            ("uninstalled", LifecyclePublicState::Uninstalled),
            ("setup_needed", LifecyclePublicState::SetupNeeded),
            ("active", LifecyclePublicState::Active),
        ] {
            let response: LifecycleProductResponse = serde_json::from_value(serde_json::json!({
                "phase": wire,
                "blockers": []
            }))
            .expect("public lifecycle response deserializes");
            assert_eq!(response.phase, expected_state);
            let serialized = serde_json::to_value(response).expect("response reserializes");
            assert_eq!(serialized["phase"], wire);
        }
    }

    #[test]
    fn host_checkpoints_collapse_to_the_public_state_vocabulary() {
        for (checkpoint, expected_state) in [
            (
                InstallationState::Removed,
                LifecyclePublicState::Uninstalled,
            ),
            (
                InstallationState::Installed,
                LifecyclePublicState::SetupNeeded,
            ),
            (
                InstallationState::Configured,
                LifecyclePublicState::SetupNeeded,
            ),
            (
                InstallationState::Disabled,
                LifecyclePublicState::SetupNeeded,
            ),
            (InstallationState::Failed, LifecyclePublicState::SetupNeeded),
            (
                InstallationState::Unsupported,
                LifecyclePublicState::SetupNeeded,
            ),
            (InstallationState::Active, LifecyclePublicState::Active),
        ] {
            assert_eq!(
                LifecyclePublicState::from_host_checkpoint(checkpoint),
                expected_state
            );
        }
    }

    #[test]
    fn callable_extension_packages_are_derived_only_from_active_list_rows() {
        let response: LifecycleProductResponse = serde_json::from_value(serde_json::json!({
            "phase": "active",
            "blockers": [],
            "payload": {
                "kind": "extension_list",
                "extensions": [
                    {
                        "summary": {
                            "package_ref": {"kind": "extension", "id": "ready"},
                            "name": "Ready",
                            "version": "1",
                            "description": "ready",
                            "source": "host_bundled",
                            "runtime_kind": "first_party",
                            "visible_read_only_capability_ids": [],
                            "credential_requirements": []
                        },
                        "phase": "active",
                        "install_scope": "private"
                    },
                    {
                        "summary": {
                            "package_ref": {"kind": "extension", "id": "setup-needed"},
                            "name": "Setup needed",
                            "version": "1",
                            "description": "setup needed",
                            "source": "host_bundled",
                            "runtime_kind": "first_party",
                            "visible_read_only_capability_ids": [],
                            "credential_requirements": []
                        },
                        "phase": "setup_needed",
                        "install_scope": "private"
                    }
                ],
                "count": 2
            }
        }))
        .expect("extension list response");

        assert_eq!(
            response
                .callable_extension_package_refs()
                .expect("callable packages")
                .into_iter()
                .map(|package_ref| package_ref.id.into_inner())
                .collect::<Vec<_>>(),
            vec!["ready"]
        );
    }
}
