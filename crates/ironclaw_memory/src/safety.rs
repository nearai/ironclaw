//! Prompt-write safety contract vocabulary for the memory crate.
//!
//! Protected memory documents (system prompt, identity, profile, hygiene
//! configuration) need a uniform safety boundary regardless of which write
//! surface — adapter, filesystem, indexer — performs the mutation. This
//! module owns the provider-neutral vocabulary (operation, source, severity,
//! reason codes, event sink, policy trait) that the memory backend and the
//! filesystem adapters depend on. The default policy implementation and the
//! enforcement engine live in the `ironclaw_memory_native` provider crate.

use std::collections::BTreeSet;

use async_trait::async_trait;
use ironclaw_host_api::{HostApiError, VirtualPath};

use crate::events::{MemoryAuditContext, MemoryEventSinkError};
use crate::path::{MemoryDocumentPath, MemoryDocumentScope, validated_memory_relative_path};

/// Version identifier for the protected prompt-path policy registry.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PromptSafetyPolicyVersion(String);

impl PromptSafetyPolicyVersion {
    pub fn new(value: impl Into<String>) -> Result<Self, HostApiError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(HostApiError::InvalidId {
                kind: "prompt safety policy version",
                value,
                reason: "policy version must not be empty".to_string(),
            });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for PromptSafetyPolicyVersion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Stable protected-path class emitted by prompt-write safety decisions.
///
/// `as_str()` returns a per-file stable identifier (e.g. `agents_md`,
/// `soul_md`, `heartbeat_md`) for the default registry paths so
/// telemetry can distinguish events by which protected file was
/// written. Custom paths added via [`PromptProtectedPathRegistry::with_additional_path`]
/// fall through to `custom_protected_path`; consumers that need the
/// exact path use [`relative_path`](Self::relative_path).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptProtectedPathClass {
    relative_path: String,
}

impl PromptProtectedPathClass {
    pub fn relative_path(&self) -> &str {
        &self.relative_path
    }

    pub fn as_str(&self) -> &'static str {
        // The match arms must stay aligned with `DEFAULT_PROMPT_PROTECTED_PATHS`
        // (lowercased, since `normalize_prompt_protected_path` lowercases on
        // insert). A new default path without a corresponding arm here
        // collapses into `custom_protected_path` and is invisible to
        // per-file telemetry — `default_paths_have_distinct_stable_class`
        // catches that.
        match self.relative_path.as_str() {
            "soul.md" => "soul_md",
            "agents.md" => "agents_md",
            "user.md" => "user_md",
            "identity.md" => "identity_md",
            "system.md" => "system_md",
            "memory.md" => "memory_md",
            "tools.md" => "tools_md",
            "heartbeat.md" => "heartbeat_md",
            "bootstrap.md" => "bootstrap_md",
            "context/assistant-directives.md" => "context_assistant_directives",
            "context/profile.json" => "context_profile",
            _ => "custom_protected_path",
        }
    }
}

/// Versioned registry of memory-relative files that may be injected into future prompts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptProtectedPathRegistry {
    policy_version: PromptSafetyPolicyVersion,
    protected_paths: BTreeSet<String>,
}

impl PromptProtectedPathRegistry {
    pub fn new(
        policy_version: PromptSafetyPolicyVersion,
        protected_paths: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Self, HostApiError> {
        let mut registry = Self {
            policy_version,
            protected_paths: BTreeSet::new(),
        };
        for path in protected_paths {
            registry = registry.with_additional_path(path)?;
        }
        Ok(registry)
    }

    pub fn policy_version(&self) -> &PromptSafetyPolicyVersion {
        &self.policy_version
    }

    pub fn classify_path(&self, path: &MemoryDocumentPath) -> Option<PromptProtectedPathClass> {
        self.classify_relative_path(path.relative_path())
    }

    pub fn classify_relative_path(&self, relative_path: &str) -> Option<PromptProtectedPathClass> {
        let normalized = normalize_prompt_protected_path(relative_path).ok()?;
        self.protected_paths
            .contains(&normalized)
            .then_some(PromptProtectedPathClass {
                relative_path: normalized,
            })
    }

    pub fn with_additional_path(mut self, path: impl Into<String>) -> Result<Self, HostApiError> {
        let normalized = normalize_prompt_protected_path(&path.into())?;
        self.protected_paths.insert(normalized);
        Ok(self)
    }
}

impl Default for PromptProtectedPathRegistry {
    fn default() -> Self {
        Self {
            policy_version: PromptSafetyPolicyVersion("prompt-protected-paths:v1".to_string()),
            protected_paths: DEFAULT_PROMPT_PROTECTED_PATHS
                .iter()
                .map(|path| path.to_ascii_lowercase())
                .collect(),
        }
    }
}

pub const DEFAULT_PROMPT_PROTECTED_PATHS: &[&str] = &[
    "SOUL.md",
    "AGENTS.md",
    "USER.md",
    "IDENTITY.md",
    "SYSTEM.md",
    "MEMORY.md",
    "TOOLS.md",
    "HEARTBEAT.md",
    "BOOTSTRAP.md",
    "context/assistant-directives.md",
    "context/profile.json",
];

fn normalize_prompt_protected_path(path: &str) -> Result<String, HostApiError> {
    validated_memory_relative_path(path.to_string()).map(|path| path.to_ascii_lowercase())
}

/// Operation type passed to prompt-write safety policy hooks.
///
/// This crate directly wires the hook through memory repository and filesystem write/append
/// paths. Other host services that implement patch, import, seed, profile, or admin prompt
/// mutations must pass their final resolved content through the same policy boundary before
/// persistence; the variants are shared vocabulary for those callers, not self-wiring magic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptWriteOperation {
    Write,
    Append,
    Patch,
    Import,
    Seed,
    ProfileUpdate,
    AdminSystemPromptUpdate,
}

impl std::fmt::Display for PromptWriteOperation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Write => "write",
            Self::Append => "append",
            Self::Patch => "patch",
            Self::Import => "import",
            Self::Seed => "seed",
            Self::ProfileUpdate => "profile_update",
            Self::AdminSystemPromptUpdate => "admin_system_prompt_update",
        })
    }
}

/// Caller surface that requested a protected prompt-file mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptWriteSource {
    MemoryBackend,
    MemoryFilesystemAdapter,
    MemoryDocumentFilesystem,
    Import,
    Seed,
    Profile,
    AdminSystemPrompt,
    Capability,
}

impl std::fmt::Display for PromptWriteSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::MemoryBackend => "memory_backend",
            Self::MemoryFilesystemAdapter => "memory_filesystem_adapter",
            Self::MemoryDocumentFilesystem => "memory_document_filesystem",
            Self::Import => "import",
            Self::Seed => "seed",
            Self::Profile => "profile",
            Self::AdminSystemPrompt => "admin_system_prompt",
            Self::Capability => "capability",
        })
    }
}

/// Named allowance required for policy-approved protected prompt-file bypasses.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PromptSafetyAllowanceId(String);

impl PromptSafetyAllowanceId {
    pub fn new(value: impl Into<String>) -> Result<Self, HostApiError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(HostApiError::InvalidId {
                kind: "prompt safety allowance",
                value,
                reason: "allowance id must not be empty".to_string(),
            });
        }
        Ok(Self(value))
    }

    pub fn empty_prompt_file_clear() -> Self {
        Self("empty_prompt_file_clear".to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for PromptSafetyAllowanceId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Stable severity bucket for sanitized prompt-write safety outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PromptSafetySeverity {
    Low,
    Medium,
    High,
    Critical,
}

impl PromptSafetySeverity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }
}

/// Sanitized finding summary. It never includes raw content, matched text, or detector descriptions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptSafetySummary {
    pub severity: PromptSafetySeverity,
    pub finding_count: usize,
}

/// Stable sanitized reason code for protected prompt-write outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptSafetyReasonCode {
    HighRiskPromptInjection,
    CriticalPromptInjection,
    PromptWritePolicyUnavailable,
    PromptWritePolicyMisconfigured,
    ProtectedPathRegistryUnavailable,
    PromptWriteBypassNotAllowed,
    PromptWriteSafetyEventUnavailable,
}

impl PromptSafetyReasonCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::HighRiskPromptInjection => "high_risk_prompt_injection",
            Self::CriticalPromptInjection => "critical_prompt_injection",
            Self::PromptWritePolicyUnavailable => "prompt_write_policy_unavailable",
            Self::PromptWritePolicyMisconfigured => "prompt_write_policy_misconfigured",
            Self::ProtectedPathRegistryUnavailable => "protected_path_registry_unavailable",
            Self::PromptWriteBypassNotAllowed => "prompt_write_bypass_not_allowed",
            Self::PromptWriteSafetyEventUnavailable => "prompt_write_safety_event_unavailable",
        }
    }
}

impl std::fmt::Display for PromptSafetyReasonCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Sanitized prompt-write rejection reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptSafetyReason {
    pub code: PromptSafetyReasonCode,
    pub severity: Option<PromptSafetySeverity>,
    pub finding_count: usize,
    pub protected_path_class: Option<PromptProtectedPathClass>,
}

impl PromptSafetyReason {
    pub fn new(code: PromptSafetyReasonCode) -> Self {
        Self {
            code,
            severity: None,
            finding_count: 0,
            protected_path_class: None,
        }
    }

    pub fn with_findings(
        code: PromptSafetyReasonCode,
        severity: PromptSafetySeverity,
        finding_count: usize,
        protected_path_class: Option<PromptProtectedPathClass>,
    ) -> Self {
        Self {
            code,
            severity: Some(severity),
            finding_count,
            protected_path_class,
        }
    }
}

/// Request passed to host-composed prompt-write safety policy hooks.
pub struct PromptWriteSafetyRequest<'a> {
    pub scope: &'a MemoryDocumentScope,
    pub path: &'a VirtualPath,
    pub relative_memory_path: Option<&'a str>,
    pub operation: PromptWriteOperation,
    pub source: PromptWriteSource,
    pub content: &'a str,
    pub previous_content_hash: Option<&'a str>,
    pub policy_version: PromptSafetyPolicyVersion,
    pub protected_path_class: Option<&'a PromptProtectedPathClass>,
    pub allowance: Option<&'a PromptSafetyAllowanceId>,
}

/// Decision returned by prompt-write safety policy hooks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptWriteSafetyDecision {
    Allow,
    Warn { findings: PromptSafetySummary },
    Reject { reason: PromptSafetyReason },
    BypassAllowed { allowance: PromptSafetyAllowanceId },
}

/// Durable redacted event class emitted for protected prompt-write checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptWriteSafetyEventKind {
    Checked,
    Warned,
    Rejected,
    BypassAllowed,
}

/// Redacted prompt-write safety event payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptWriteSafetyEvent {
    pub kind: PromptWriteSafetyEventKind,
    pub scope: MemoryDocumentScope,
    pub operation: PromptWriteOperation,
    pub source: PromptWriteSource,
    pub policy_version: PromptSafetyPolicyVersion,
    pub protected_path_class: Option<PromptProtectedPathClass>,
    /// SHA-256 of the memory-relative protected path, when known.
    pub relative_path_hash: Option<String>,
    pub reason_code: Option<PromptSafetyReasonCode>,
    pub severity: Option<PromptSafetySeverity>,
    pub finding_count: usize,
    pub allowance: Option<PromptSafetyAllowanceId>,
    pub audit_context: Option<MemoryAuditContext>,
}

/// Host-composed sink for durable redacted prompt-write safety events.
#[async_trait]
pub trait PromptWriteSafetyEventSink: Send + Sync {
    async fn record_prompt_write_safety_event(
        &self,
        event: PromptWriteSafetyEvent,
    ) -> Result<(), MemoryEventSinkError>;
}

/// Sanitized policy evaluation failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptWriteSafetyError {
    pub reason: PromptSafetyReason,
}

impl PromptWriteSafetyError {
    pub fn new(code: PromptSafetyReasonCode) -> Self {
        Self {
            reason: PromptSafetyReason::new(code),
        }
    }
}

/// Host-composed policy hook for protected prompt-file writes.
#[async_trait]
pub trait PromptWriteSafetyPolicy: Send + Sync {
    fn protected_path_registry(&self) -> Option<&PromptProtectedPathRegistry> {
        None
    }

    fn requires_previous_content_hash(&self) -> bool {
        false
    }

    async fn check_write(
        &self,
        request: PromptWriteSafetyRequest<'_>,
    ) -> Result<PromptWriteSafetyDecision, PromptWriteSafetyError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn default_paths_have_distinct_stable_class() {
        // Lock in zmanian's M2 fix: every default protected path gets a
        // distinct stable class string so telemetry can distinguish
        // AGENTS.md vs SOUL.md vs HEARTBEAT.md events. A regression that
        // collapses any two defaults into the same bucket trips the
        // duplicate check; a regression that drops the per-file mapping
        // entirely (everything → custom_protected_path) trips both
        // the count and the spot-check assertion below.
        let registry = PromptProtectedPathRegistry::default();
        let mut classes: Vec<&'static str> = Vec::new();
        for default_path in DEFAULT_PROMPT_PROTECTED_PATHS {
            let class = registry
                .classify_relative_path(default_path)
                .unwrap_or_else(|| panic!("default path {default_path} must classify"));
            classes.push(class.as_str());
        }
        let unique: HashSet<&'static str> = classes.iter().copied().collect();
        assert_eq!(
            unique.len(),
            classes.len(),
            "every default protected path must have a distinct stable class string; got {classes:?}"
        );
        assert!(
            !classes.contains(&"custom_protected_path"),
            "no default path may fall through to the custom bucket; got {classes:?}"
        );
        // Spot-check the names zmanian called out specifically.
        let agents = registry.classify_relative_path("AGENTS.md").unwrap();
        let soul = registry.classify_relative_path("SOUL.md").unwrap();
        let heartbeat = registry.classify_relative_path("HEARTBEAT.md").unwrap();
        assert_eq!(agents.as_str(), "agents_md");
        assert_eq!(soul.as_str(), "soul_md");
        assert_eq!(heartbeat.as_str(), "heartbeat_md");
    }

    #[test]
    fn custom_paths_use_generic_class_but_carry_specific_relative_path() {
        let registry = PromptProtectedPathRegistry::new(
            PromptSafetyPolicyVersion::new("test:v1").unwrap(),
            ["custom/playbook.md"],
        )
        .unwrap();
        let class = registry
            .classify_relative_path("custom/playbook.md")
            .expect("custom path must classify");
        assert_eq!(class.as_str(), "custom_protected_path");
        assert_eq!(
            class.relative_path(),
            "custom/playbook.md",
            "custom paths fall through to the generic class but consumers can still recover the path"
        );
    }
}
