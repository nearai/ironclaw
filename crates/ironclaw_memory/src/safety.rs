//! Prompt-write safety policy primitives for the memory crate.
//!
//! Protected memory documents (system prompt, identity, profile, hygiene
//! configuration) need a uniform safety boundary regardless of which write
//! surface — adapter, filesystem, indexer — performs the mutation. This
//! module owns the vocabulary (operation, source, severity, reason codes,
//! event sink) and the enforcement helpers that the memory backend and the
//! filesystem adapters call.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_filesystem::{FilesystemError, FilesystemOperation};
use ironclaw_host_api::{HostApiError, VirtualPath};
use ironclaw_safety::{Sanitizer, Severity};

use crate::path::{
    MemoryDocumentPath, MemoryDocumentScope, memory_error, valid_memory_path,
    validated_memory_relative_path,
};

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

const DEFAULT_PROMPT_PROTECTED_PATHS: &[&str] = &[
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

impl From<Severity> for PromptSafetySeverity {
    fn from(severity: Severity) -> Self {
        match severity {
            Severity::Low => Self::Low,
            Severity::Medium => Self::Medium,
            Severity::High => Self::High,
            Severity::Critical => Self::Critical,
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
    fn new(code: PromptSafetyReasonCode) -> Self {
        Self {
            code,
            severity: None,
            finding_count: 0,
            protected_path_class: None,
        }
    }

    fn with_findings(
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
    pub reason_code: Option<PromptSafetyReasonCode>,
    pub severity: Option<PromptSafetySeverity>,
    pub finding_count: usize,
    pub allowance: Option<PromptSafetyAllowanceId>,
}

/// Host-composed sink for durable redacted prompt-write safety events.
#[async_trait]
pub trait PromptWriteSafetyEventSink: Send + Sync {
    async fn record_prompt_write_safety_event(
        &self,
        event: PromptWriteSafetyEvent,
    ) -> Result<(), FilesystemError>;
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

/// Default prompt-write safety policy preserving current workspace scanner behavior.
pub struct DefaultPromptWriteSafetyPolicy {
    registry: PromptProtectedPathRegistry,
    sanitizer: Sanitizer,
}

impl DefaultPromptWriteSafetyPolicy {
    pub fn new() -> Self {
        Self::with_registry(PromptProtectedPathRegistry::default())
    }

    pub fn with_registry(registry: PromptProtectedPathRegistry) -> Self {
        Self {
            registry,
            sanitizer: Sanitizer::new(),
        }
    }
}

impl Default for DefaultPromptWriteSafetyPolicy {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PromptWriteSafetyPolicy for DefaultPromptWriteSafetyPolicy {
    fn protected_path_registry(&self) -> Option<&PromptProtectedPathRegistry> {
        Some(&self.registry)
    }

    async fn check_write(
        &self,
        request: PromptWriteSafetyRequest<'_>,
    ) -> Result<PromptWriteSafetyDecision, PromptWriteSafetyError> {
        let protected_path_class = request.protected_path_class.cloned().or_else(|| {
            request
                .relative_memory_path
                .and_then(|path| self.registry.classify_relative_path(path))
        });
        let Some(protected_path_class) = protected_path_class else {
            return Ok(PromptWriteSafetyDecision::Allow);
        };

        if request.content.trim().is_empty() {
            if let Some(allowance) = request.allowance
                && *allowance == PromptSafetyAllowanceId::empty_prompt_file_clear()
            {
                return Ok(PromptWriteSafetyDecision::BypassAllowed {
                    allowance: allowance.clone(),
                });
            }
            return Ok(PromptWriteSafetyDecision::Reject {
                reason: PromptSafetyReason {
                    protected_path_class: Some(protected_path_class),
                    ..PromptSafetyReason::new(PromptSafetyReasonCode::PromptWriteBypassNotAllowed)
                },
            });
        }

        let warnings = self.sanitizer.detect(request.content);
        let Some(max_severity) = warnings.iter().map(|warning| warning.severity).max() else {
            return Ok(PromptWriteSafetyDecision::Allow);
        };
        let severity = PromptSafetySeverity::from(max_severity);
        let finding_count = warnings.len();

        if max_severity >= Severity::Critical {
            return Ok(PromptWriteSafetyDecision::Reject {
                reason: PromptSafetyReason::with_findings(
                    PromptSafetyReasonCode::CriticalPromptInjection,
                    severity,
                    finding_count,
                    Some(protected_path_class),
                ),
            });
        }
        if max_severity >= Severity::High {
            return Ok(PromptWriteSafetyDecision::Reject {
                reason: PromptSafetyReason::with_findings(
                    PromptSafetyReasonCode::HighRiskPromptInjection,
                    severity,
                    finding_count,
                    Some(protected_path_class),
                ),
            });
        }

        Ok(PromptWriteSafetyDecision::Warn {
            findings: PromptSafetySummary {
                severity,
                finding_count,
            },
        })
    }
}

pub(crate) fn prompt_write_protected_classification(
    policy: Option<&Arc<dyn PromptWriteSafetyPolicy>>,
    registry: &PromptProtectedPathRegistry,
    path: &MemoryDocumentPath,
) -> Option<(PromptProtectedPathClass, PromptSafetyPolicyVersion)> {
    if let Some(path_class) = registry.classify_path(path) {
        return Some((path_class, registry.policy_version().clone()));
    }
    policy
        .and_then(|policy| policy.protected_path_registry())
        .and_then(|registry| {
            registry
                .classify_path(path)
                .map(|path_class| (path_class, registry.policy_version().clone()))
        })
}

pub(crate) fn prompt_write_policy_requires_previous_content_hash(
    policy: Option<&Arc<dyn PromptWriteSafetyPolicy>>,
) -> bool {
    policy
        .map(|policy| policy.requires_previous_content_hash())
        .unwrap_or(false)
}

pub(crate) struct PromptWriteSafetyCheck<'a> {
    pub scope: &'a MemoryDocumentScope,
    pub path: &'a MemoryDocumentPath,
    pub operation: PromptWriteOperation,
    pub source: PromptWriteSource,
    pub content: &'a str,
    pub previous_content_hash: Option<&'a str>,
    pub allowance: Option<&'a PromptSafetyAllowanceId>,
    pub filesystem_operation: FilesystemOperation,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PromptWriteSafetyEnforcement {
    pub allowance: Option<PromptSafetyAllowanceId>,
}

pub(crate) async fn enforce_prompt_write_safety(
    policy: Option<&Arc<dyn PromptWriteSafetyPolicy>>,
    event_sink: Option<&Arc<dyn PromptWriteSafetyEventSink>>,
    registry: &PromptProtectedPathRegistry,
    check: PromptWriteSafetyCheck<'_>,
) -> Result<PromptWriteSafetyEnforcement, FilesystemError> {
    let Some((protected_path_class, policy_version)) =
        prompt_write_protected_classification(policy, registry, check.path)
    else {
        return Ok(PromptWriteSafetyEnforcement::default());
    };
    let virtual_path = check
        .path
        .virtual_path()
        .unwrap_or_else(|_| valid_memory_path());
    let Some(policy) = policy else {
        let reason = PromptSafetyReason::new(PromptSafetyReasonCode::PromptWritePolicyUnavailable);
        emit_prompt_write_safety_event(
            event_sink,
            &check,
            PromptWriteSafetyEventParts {
                kind: PromptWriteSafetyEventKind::Rejected,
                policy_version: &policy_version,
                protected_path_class: &protected_path_class,
                reason: Some(&reason),
                findings: None,
                allowance: None,
                require_sink: false,
            },
        )
        .await?;
        return Err(prompt_write_safety_error(
            virtual_path,
            check.filesystem_operation,
            reason,
        ));
    };

    let request = PromptWriteSafetyRequest {
        scope: check.scope,
        path: &virtual_path,
        relative_memory_path: Some(check.path.relative_path()),
        operation: check.operation,
        source: check.source,
        content: check.content,
        previous_content_hash: check.previous_content_hash,
        policy_version: policy_version.clone(),
        protected_path_class: Some(&protected_path_class),
        allowance: check.allowance,
    };

    match policy.check_write(request).await {
        Ok(PromptWriteSafetyDecision::Allow) => {
            emit_prompt_write_safety_event(
                event_sink,
                &check,
                PromptWriteSafetyEventParts {
                    kind: PromptWriteSafetyEventKind::Checked,
                    policy_version: &policy_version,
                    protected_path_class: &protected_path_class,
                    reason: None,
                    findings: None,
                    allowance: None,
                    require_sink: false,
                },
            )
            .await?;
            Ok(PromptWriteSafetyEnforcement::default())
        }
        Ok(PromptWriteSafetyDecision::BypassAllowed { allowance }) => {
            emit_prompt_write_safety_event(
                event_sink,
                &check,
                PromptWriteSafetyEventParts {
                    kind: PromptWriteSafetyEventKind::BypassAllowed,
                    policy_version: &policy_version,
                    protected_path_class: &protected_path_class,
                    reason: None,
                    findings: None,
                    allowance: Some(&allowance),
                    require_sink: true,
                },
            )
            .await?;
            tracing::debug!(
                target: "ironclaw::memory::prompt_write_safety",
                operation = %check.operation,
                source = %check.source,
                protected_path_class = %protected_path_class.as_str(),
                policy_version = %policy_version,
                allowance = %allowance,
                "protected prompt write bypass allowed"
            );
            Ok(PromptWriteSafetyEnforcement {
                allowance: Some(allowance),
            })
        }
        Ok(PromptWriteSafetyDecision::Warn { findings }) => {
            emit_prompt_write_safety_event(
                event_sink,
                &check,
                PromptWriteSafetyEventParts {
                    kind: PromptWriteSafetyEventKind::Warned,
                    policy_version: &policy_version,
                    protected_path_class: &protected_path_class,
                    reason: None,
                    findings: Some(&findings),
                    allowance: None,
                    require_sink: true,
                },
            )
            .await?;
            tracing::debug!(
                target: "ironclaw::memory::prompt_write_safety",
                operation = %check.operation,
                source = %check.source,
                protected_path_class = %protected_path_class.as_str(),
                policy_version = %policy_version,
                severity = %findings.severity.as_str(),
                finding_count = findings.finding_count,
                "protected prompt write allowed with sanitized safety warning"
            );
            Ok(PromptWriteSafetyEnforcement::default())
        }
        Ok(PromptWriteSafetyDecision::Reject { reason }) => {
            emit_prompt_write_safety_event(
                event_sink,
                &check,
                PromptWriteSafetyEventParts {
                    kind: PromptWriteSafetyEventKind::Rejected,
                    policy_version: &policy_version,
                    protected_path_class: &protected_path_class,
                    reason: Some(&reason),
                    findings: None,
                    allowance: None,
                    require_sink: false,
                },
            )
            .await?;
            Err(prompt_write_safety_error(
                virtual_path,
                check.filesystem_operation,
                reason,
            ))
        }
        Err(error) => {
            let reason = error.reason;
            emit_prompt_write_safety_event(
                event_sink,
                &check,
                PromptWriteSafetyEventParts {
                    kind: PromptWriteSafetyEventKind::Rejected,
                    policy_version: &policy_version,
                    protected_path_class: &protected_path_class,
                    reason: Some(&reason),
                    findings: None,
                    allowance: None,
                    require_sink: false,
                },
            )
            .await?;
            Err(prompt_write_safety_error(
                virtual_path,
                check.filesystem_operation,
                reason,
            ))
        }
    }
}

struct PromptWriteSafetyEventParts<'a> {
    kind: PromptWriteSafetyEventKind,
    policy_version: &'a PromptSafetyPolicyVersion,
    protected_path_class: &'a PromptProtectedPathClass,
    reason: Option<&'a PromptSafetyReason>,
    findings: Option<&'a PromptSafetySummary>,
    allowance: Option<&'a PromptSafetyAllowanceId>,
    // Outcomes that would still persist with a non-clean safety result (warn/bypass)
    // require a durable redacted audit seam before persistence.
    require_sink: bool,
}

async fn emit_prompt_write_safety_event(
    event_sink: Option<&Arc<dyn PromptWriteSafetyEventSink>>,
    check: &PromptWriteSafetyCheck<'_>,
    parts: PromptWriteSafetyEventParts<'_>,
) -> Result<(), FilesystemError> {
    let Some(event_sink) = event_sink else {
        return if parts.require_sink {
            Err(prompt_write_safety_error(
                check
                    .path
                    .virtual_path()
                    .unwrap_or_else(|_| valid_memory_path()),
                check.filesystem_operation,
                PromptSafetyReason::new(PromptSafetyReasonCode::PromptWriteSafetyEventUnavailable),
            ))
        } else {
            Ok(())
        };
    };
    let event = PromptWriteSafetyEvent {
        kind: parts.kind,
        scope: check.scope.clone(),
        operation: check.operation,
        source: check.source,
        policy_version: parts.policy_version.clone(),
        protected_path_class: Some(parts.protected_path_class.clone()),
        reason_code: parts.reason.map(|reason| reason.code),
        severity: parts
            .reason
            .and_then(|reason| reason.severity)
            .or_else(|| parts.findings.map(|findings| findings.severity)),
        finding_count: parts
            .reason
            .map(|reason| reason.finding_count)
            .or_else(|| parts.findings.map(|findings| findings.finding_count))
            .unwrap_or(0),
        allowance: parts.allowance.cloned(),
    };
    if let Err(error) = event_sink.record_prompt_write_safety_event(event).await {
        tracing::debug!(
            target: "ironclaw::memory::prompt_write_safety",
            error = %error,
            operation = %check.operation,
            source = %check.source,
            "failed to record prompt write safety event"
        );
        return Err(prompt_write_safety_error(
            check
                .path
                .virtual_path()
                .unwrap_or_else(|_| valid_memory_path()),
            check.filesystem_operation,
            PromptSafetyReason::new(PromptSafetyReasonCode::PromptWriteSafetyEventUnavailable),
        ));
    }
    Ok(())
}

fn prompt_write_safety_error(
    path: VirtualPath,
    operation: FilesystemOperation,
    reason: PromptSafetyReason,
) -> FilesystemError {
    memory_error(path, operation, reason.code.as_str())
}

pub(crate) fn take_prompt_safety_allowance(
    allowance: &Mutex<Option<PromptSafetyAllowanceId>>,
    path: &VirtualPath,
    operation: FilesystemOperation,
) -> Result<Option<PromptSafetyAllowanceId>, FilesystemError> {
    let mut allowance = allowance.lock().map_err(|_| {
        memory_error(
            path.clone(),
            operation,
            "prompt write safety allowance lock poisoned",
        )
    })?;
    Ok(allowance.take())
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
