//! Product-facing lifecycle contract for Reborn package UX.
//!
//! This module deliberately models package/install lifecycle separately from
//! auth, approval, pairing, and policy gates. Those remain owned by their
//! dedicated services; lifecycle projections may only carry redacted refs to
//! the owning interaction.

use std::fmt;

use async_trait::async_trait;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use serde_json::Value;

use crate::{ProductCommandContext, ProductWorkflowError};

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
}

/// Browser lifecycle contract phases.
///
/// Some phases are forward-declared. The first local facades currently emit
/// only the states they can prove from their backing systems; future
/// extension/skill stores may make the remaining states reachable without
/// changing the wire enum.
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
        name: Option<LifecyclePackageId>,
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
