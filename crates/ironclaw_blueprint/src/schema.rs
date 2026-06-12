//! The typed blueprint AST (`api_version = "ironclaw.config/v1"`).
//!
//! Every struct uses `deny_unknown_fields` so an unrecognised key is a hard
//! parse error rather than a silently dropped value — the epic is explicit
//! that unknown top-level (and nested) keys must fail closed. The shapes here
//! are the *input* contract; reconciling them into the typed Reborn repos is a
//! separate apply slice and intentionally lives outside this crate.
//!
//! The v1 schema deliberately includes everything the epic body and its
//! follow-up comments specified — inline harness definitions (#3036 comment)
//! and the `[agent_loop]` driver-selection block (#3107) — because once
//! `ironclaw.config/v1` ships, the only way to add a field is a migration or a
//! major bump. Better to land the full surface now.
//!
//! Every type also derives [`JsonSchema`] so the per-domain JSON Schema
//! artifacts (see [`crate::json_schema`]) are generated from these exact
//! shapes and can never drift from what the parser accepts.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Root blueprint document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Blueprint {
    /// Locks the schema major. Validated against [`crate::SUPPORTED_API_VERSION`].
    pub api_version: String,
    /// Document kind discriminant. Only `Blueprint` is valid in this file.
    pub kind: BlueprintKind,
    /// Scope the blueprint applies to. An empty scope targets the system
    /// default layer; narrower scopes constrain the apply. Scope can never
    /// *widen* authority — that is enforced at apply time, not here.
    #[serde(default)]
    pub scope: Scope,
    /// Scoped system-prompt setting. Loaded as a setting, never pasted into
    /// agent-loop code.
    #[serde(default)]
    pub system_prompt: Option<SystemPrompt>,
    /// Provider/model selection plus per-provider config.
    #[serde(default)]
    pub providers: Option<Providers>,
    /// Runtime profile + approval policy. Constrained by deployment mode at
    /// apply time; declared, not resolved, here.
    #[serde(default)]
    pub runtime: Option<Runtime>,
    /// Optional loop-driver selection (#3107). Omitted means the
    /// deployment/session default applies.
    #[serde(default)]
    pub agent_loop: Option<AgentLoop>,
    /// Extensions to install/configure. References existing extension IDs;
    /// blueprints never embed extension binaries.
    #[serde(default)]
    pub extensions: Vec<Extension>,
    /// Skills to enable.
    #[serde(default)]
    pub skills: Vec<Skill>,
    /// Pre-seeded missions (reuse the existing routine engine; no new cron).
    #[serde(default)]
    pub missions: Vec<Mission>,
    /// Pre-seeded projects.
    #[serde(default)]
    pub projects: Vec<Project>,
    /// Optional capability-surface visibility filter. UX/visibility only —
    /// action-time authorization is still mandatory.
    #[serde(default)]
    pub capability_surface: Option<CapabilitySurface>,
    /// Bind a registered harness by id, or define one inline (#3036 comment).
    #[serde(default)]
    pub harness: Option<HarnessBinding>,
}

/// Document-kind discriminant for the blueprint file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum BlueprintKind {
    Blueprint,
}

/// Apply scope. All fields optional; absence means "do not narrow on this
/// axis".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

/// Scoped system-prompt setting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SystemPrompt {
    /// Root-relative path to the prompt body. Read once, hashed into the
    /// lockfile.
    pub text_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applies_to: Option<AppliesTo>,
}

/// Narrowing selector for where a scoped setting applies.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AppliesTo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

/// `[providers]` — a `default_llm` selector plus arbitrarily-named
/// per-provider config tables (`[providers.anthropic]`, …).
///
/// This struct cannot use `deny_unknown_fields` because the named provider
/// tables are flattened into `entries`; unknown *provider config* keys are
/// still rejected by [`ProviderEntry`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Providers {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_llm: Option<String>,
    #[serde(flatten)]
    pub entries: BTreeMap<String, ProviderEntry>,
}

/// Per-provider configuration. `api_key` accepts only a `${secret:<name>}`
/// handle (enforced by the secret scan); inline material is rejected.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProviderEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
}

/// `[runtime]` — declared profile + approval policy.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Runtime {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_policy: Option<String>,
}

/// `[agent_loop]` — optional loop-driver selection (#3107).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentLoop {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_driver: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steering_policy: Option<String>,
}

/// `[[extensions]]` entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Extension {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust: Option<String>,
    /// Opaque per-extension config. Validated against the extension's own
    /// schema by the apply reconciler, not here — which is also why the JSON
    /// Schema view of this field is deliberately permissive (`any`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<serde_json::Value>")]
    pub config: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<ExtensionAuth>,
}

/// Extension auth binding. References an account, never embeds credentials.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExtensionAuth {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_ref: Option<String>,
}

/// `[[skills]]` entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Skill {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

/// `[[missions]]` entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Mission {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<String>,
    /// Root-relative path to the mission brief. Hashed into the lockfile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brief_ref: Option<String>,
}

/// `[[projects]]` entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<ProjectSeed>,
}

/// Seed source for a pre-seeded project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectSeed {
    pub from: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
}

/// Capability-surface visibility filter. Glob entries like `github-mcp.*` are
/// allowed; this is matched before the model call, not at authorization time.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilitySurface {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny: Vec<String>,
}

/// `[harness]` — either bind a registered harness by id, define one inline, or
/// both (a registered id with inline overrides is rejected at validation; the
/// two are mutually exclusive).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HarnessBinding {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline: Option<InlineHarness>,
}

/// Inline harness definition (#3036 comment). Registered as project-scoped in
/// the typed harness repo at apply time; behaviourally identical to a separate
/// manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InlineHarness {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_overlay: Option<PromptOverlay>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_extensions: Vec<RequiredRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_skills: Vec<RequiredRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_surface: Option<CapabilitySurface>,
}

/// Prompt overlay composed on top of the resolved scope system prompt — never
/// a replacement for identity files.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PromptOverlay {
    /// Root-relative path to the overlay body. Hashed into the lockfile.
    pub text_ref: String,
}

/// A `{ id = "…" }` reference to a required extension or skill.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequiredRef {
    pub id: String,
}
