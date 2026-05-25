//! Product-facing lifecycle contract for Reborn package UX.
//!
//! This module deliberately models package/install lifecycle separately from
//! auth, approval, pairing, and policy gates. Those remain owned by their
//! dedicated services; lifecycle projections may only carry redacted refs to
//! the owning interaction.

use std::{fmt, sync::Arc};

use async_trait::async_trait;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_product_adapters::{
    InboundCommandPayload, ProductInboundAck, ProductRejection, ProductRejectionKind,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use serde_json::Value;

use crate::{ProductCommand, ProductCommandContext, ProductCommandService, ProductWorkflowError};

const LIFECYCLE_ID_MAX_BYTES: usize = 256;
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecyclePhase {
    Discovered,
    Installing,
    Installed,
    Configured,
    Activating,
    Active,
    Disabled,
    UpgradeRequired,
    Failed,
    Removing,
    Removed,
    UnsupportedOrLegacy,
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
        name: Option<String>,
        content: String,
    },
    SkillRemove {
        package_ref: LifecyclePackageRef,
    },
}

impl LifecycleProductAction {
    pub fn command_name(&self) -> &'static str {
        match self {
            Self::ExtensionSearch { .. } => "extension_search",
            Self::ExtensionInstall { .. } => "extension_install",
            Self::ExtensionAuth { .. } => "extension_auth",
            Self::ExtensionActivate { .. } => "extension_activate",
            Self::ExtensionConfigure { .. } => "extension_configure",
            Self::ExtensionRemove { .. } => "extension_remove",
            Self::SkillSearch { .. } => "skill_search",
            Self::SkillInstall { .. } => "skill_install",
            Self::SkillRemove { .. } => "skill_remove",
        }
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
            Self::ExtensionSearch { .. } | Self::SkillSearch { .. } | Self::SkillInstall { .. } => {
                None
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleProductResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_ref: Option<LifecyclePackageRef>,
    pub phase: LifecyclePhase,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<LifecycleReadinessBlocker>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
}

impl LifecycleProductResponse {
    pub fn projection(
        package_ref: Option<LifecyclePackageRef>,
        phase: LifecyclePhase,
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
    Command(ProductCommandContext),
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
        _context: LifecycleProductContext,
        package_ref: LifecyclePackageRef,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        Err(ProductWorkflowError::UnsupportedActionKind {
            kind: format!("lifecycle_project_package:{:?}", package_ref.kind),
        })
    }
}

pub struct LifecycleProductCommandService {
    facade: Arc<dyn LifecycleProductFacade>,
}

impl LifecycleProductCommandService {
    pub fn new(facade: Arc<dyn LifecycleProductFacade>) -> Self {
        Self { facade }
    }
}

#[async_trait]
impl ProductCommandService for LifecycleProductCommandService {
    async fn execute(
        &self,
        context: ProductCommandContext,
        command: ProductCommand,
    ) -> Result<ProductInboundAck, ProductWorkflowError> {
        let ProductCommand::Lifecycle { action } = command else {
            return Ok(ProductInboundAck::Rejected(ProductRejection::permanent(
                ProductRejectionKind::PolicyDenied,
                format!("command routing unavailable: {}", command.name()),
            )));
        };
        // Lifecycle commands are admitted and executed by the facade;
        // the structured response (phase, blockers, payload) belongs to
        // the lifecycle projection stream, not the command ack channel.
        // TODO: once the product surface surfaces lifecycle projections
        // to the caller, wire the response into the projection stream.
        self.facade
            .execute(LifecycleProductContext::Command(context), action)
            .await?;
        Ok(ProductInboundAck::NoOp)
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

    fn unsupported_projection(
        &self,
        package_ref: Option<LifecyclePackageRef>,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        Ok(LifecycleProductResponse::projection(
            package_ref,
            LifecyclePhase::UnsupportedOrLegacy,
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

pub fn parse_lifecycle_command_payload(payload: &InboundCommandPayload) -> ProductCommand {
    match payload.command.as_str() {
        "extension_search" => ProductCommand::Lifecycle {
            action: LifecycleProductAction::ExtensionSearch {
                query: payload.arguments.trim().to_string(),
            },
        },
        "extension_install" => extension_package_command(payload, |package_ref| {
            LifecycleProductAction::ExtensionInstall { package_ref }
        }),
        "extension_auth" => extension_package_command(payload, |package_ref| {
            LifecycleProductAction::ExtensionAuth { package_ref }
        }),
        "extension_activate" => extension_package_command(payload, |package_ref| {
            LifecycleProductAction::ExtensionActivate { package_ref }
        }),
        "extension_configure" => parse_extension_configure_command(payload),
        "extension_remove" => extension_package_command(payload, |package_ref| {
            LifecycleProductAction::ExtensionRemove { package_ref }
        }),
        "skill_search" => ProductCommand::Lifecycle {
            action: LifecycleProductAction::SkillSearch {
                query: payload.arguments.trim().to_string(),
            },
        },
        "skill_install" => parse_skill_install_command(payload),
        "skill_remove" => parse_skill_remove_command(payload),
        _ => unknown_lifecycle_command(payload),
    }
}

fn parse_extension_configure_command(payload: &InboundCommandPayload) -> ProductCommand {
    let args = payload.arguments.trim();
    let (id, config_payload) = match serde_json::from_str::<Value>(args) {
        Ok(json) => {
            let id = json
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            (id, json.get("payload").cloned())
        }
        Err(_) => (first_argument(args).to_string(), None),
    };
    match lifecycle_package_ref(LifecyclePackageKind::Extension, id) {
        Ok(package_ref) => ProductCommand::Lifecycle {
            action: LifecycleProductAction::ExtensionConfigure {
                package_ref,
                payload: config_payload,
            },
        },
        Err(_) => unknown_lifecycle_command(payload),
    }
}

fn parse_skill_install_command(payload: &InboundCommandPayload) -> ProductCommand {
    let args = payload.arguments.trim();
    let Ok(json) = serde_json::from_str::<Value>(args) else {
        return unknown_lifecycle_command(payload);
    };
    let content = match json.get("content").and_then(Value::as_str) {
        Some(content) => content,
        None => return unknown_lifecycle_command(payload),
    };
    let content = match validate_lifecycle_text(content.to_string(), "skill content", 64 * 1024) {
        Ok(content) => content,
        Err(_) => return unknown_lifecycle_command(payload),
    };
    let name = match json.get("name").and_then(Value::as_str) {
        Some(name) => {
            match validate_lifecycle_string(name.to_string(), "skill name", LIFECYCLE_ID_MAX_BYTES)
            {
                Ok(name) => Some(name),
                Err(_) => return unknown_lifecycle_command(payload),
            }
        }
        None => None,
    };
    ProductCommand::Lifecycle {
        action: LifecycleProductAction::SkillInstall { name, content },
    }
}

fn parse_skill_remove_command(payload: &InboundCommandPayload) -> ProductCommand {
    let args = payload.arguments.trim();
    let id = serde_json::from_str::<Value>(args)
        .ok()
        .and_then(|json| {
            json.get("id")
                .or_else(|| json.get("name"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| first_argument(args).to_string());
    match lifecycle_package_ref(LifecyclePackageKind::Skill, id) {
        Ok(package_ref) => ProductCommand::Lifecycle {
            action: LifecycleProductAction::SkillRemove { package_ref },
        },
        Err(_) => unknown_lifecycle_command(payload),
    }
}

fn extension_package_command(
    payload: &InboundCommandPayload,
    build: fn(LifecyclePackageRef) -> LifecycleProductAction,
) -> ProductCommand {
    let id = lifecycle_ref_argument(payload);
    match lifecycle_package_ref(LifecyclePackageKind::Extension, id) {
        Ok(package_ref) => ProductCommand::Lifecycle {
            action: build(package_ref),
        },
        Err(_) => unknown_lifecycle_command(payload),
    }
}

fn lifecycle_ref_argument(payload: &InboundCommandPayload) -> String {
    let args = payload.arguments.trim();
    serde_json::from_str::<Value>(args)
        .ok()
        .and_then(|json| json.get("id").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_else(|| first_argument(args).to_string())
}

fn first_argument(args: &str) -> &str {
    args.split_whitespace().next().unwrap_or("")
}

fn unknown_lifecycle_command(payload: &InboundCommandPayload) -> ProductCommand {
    ProductCommand::Unknown {
        name: payload.command.clone(),
        arguments: payload.arguments.clone(),
    }
}

pub fn lifecycle_package_ref(
    kind: LifecyclePackageKind,
    id: impl Into<String>,
) -> Result<LifecyclePackageRef, ProductWorkflowError> {
    LifecyclePackageRef::new(kind, id)
}

/// Validates a lifecycle string: non-empty, within byte limit, with optional
/// control-character filtering.
pub fn validate_lifecycle_string(
    value: String,
    label: &'static str,
    max_bytes: usize,
) -> Result<String, ProductWorkflowError> {
    validate_lifecycle_value(value, label, max_bytes, true)
}

/// Validates free-form lifecycle text that may contain control characters
/// (e.g. newlines in skill markdown) but still blocks NUL.
pub fn validate_lifecycle_text(
    value: String,
    label: &'static str,
    max_bytes: usize,
) -> Result<String, ProductWorkflowError> {
    validate_lifecycle_value(value, label, max_bytes, false)
}

fn validate_lifecycle_value(
    value: String,
    label: &'static str,
    max_bytes: usize,
    reject_control: bool,
) -> Result<String, ProductWorkflowError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ProductWorkflowError::InvalidBindingRequest {
            reason: format!("{label} must not be empty"),
        });
    }
    if trimmed.len() > max_bytes {
        return Err(ProductWorkflowError::InvalidBindingRequest {
            reason: format!("{label} must be at most {max_bytes} bytes"),
        });
    }
    let has_bad_char = if reject_control {
        trimmed.chars().any(|c| c == '\0' || c.is_control())
    } else {
        trimmed.chars().any(|c| c == '\0')
    };
    if has_bad_char {
        return Err(ProductWorkflowError::InvalidBindingRequest {
            reason: format!(
                "{label} must not contain NUL{} characters",
                if reject_control { "/control" } else { "" }
            ),
        });
    }
    Ok(trimmed.to_string())
}

fn validate_optional_ref(
    value: Option<String>,
) -> Result<Option<LifecycleBlockerRef>, ProductWorkflowError> {
    value.map(LifecycleBlockerRef::new).transpose()
}
