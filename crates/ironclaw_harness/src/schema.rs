//! The typed harness-manifest AST (`api_version = "ironclaw.harness/v1"`).
//!
//! A harness is a *named composition* of extensions, skills, a prompt overlay,
//! runtime constraints, and a capability-surface filter that together define a
//! use-case mode. It reuses the blueprint crate's shared sub-shapes
//! ([`CapabilitySurface`], [`PromptOverlay`], [`RequiredRef`]) rather than
//! redefining parallel types: the same building blocks, composed one level up.

use std::collections::BTreeMap;

use ironclaw_blueprint::{CapabilitySurface, PromptOverlay, RequiredRef};
use serde::{Deserialize, Serialize};

/// Root harness manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessManifest {
    pub api_version: String,
    pub kind: HarnessKind,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust: Option<String>,
    /// Composes on top of the resolved scope system prompt; never replaces
    /// identity files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_overlay: Option<PromptOverlay>,
    /// Authority constraints. A harness may only *reduce* authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_constraints: Option<RuntimeConstraints>,
    /// Extensions that must be installed & authenticated for activation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_extensions: Vec<RequiredRef>,
    /// Skills that must be present for activation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_skills: Vec<RequiredRef>,
    /// Visibility filter applied before the model call (not authorization).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_surface: Option<CapabilitySurface>,
    /// Pre-seeded memory layout — typed path templates (e.g.
    /// `findings_root = "/memory/projects/${project}/findings"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_schema: Option<BTreeMap<String, String>>,
    /// Required outputs at end-of-engagement; activation completion fails if
    /// missing (e.g. `report = "/artifacts/${run}/engagement-report.md"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_artifacts: Option<BTreeMap<String, String>>,
}

/// Document-kind discriminant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HarnessKind {
    Harness,
}

/// Authority constraints a harness imposes. All fields *narrow* — they cannot
/// grant authority the deployment/profile does not already allow.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConstraints {
    /// Caps the runtime profile (e.g. `Sandboxed`). Activation fails closed if
    /// honoring it would require *raising* the current profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_profile: Option<String>,
    /// Deployment modes in which this harness may activate. Empty = any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub require_deployment_mode: Vec<String>,
    /// Network brokering mode (e.g. `Brokered`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_mode: Option<String>,
}
