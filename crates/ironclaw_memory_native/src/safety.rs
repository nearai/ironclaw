//! Prompt-write safety enforcement for the memory crate.
//!
//! The prompt-write-safety *vocabulary* (operation, source, severity, reason
//! codes, event payload/sink, policy trait, protected-path registry) moved to
//! `ironclaw_memory` and is re-exported below. The *enforcement engine*
//! — the default policy (which depends on `ironclaw_safety`'s sanitizer), the
//! protected-path classification, and the `enforce_prompt_write_safety` helper
//! the memory backend and filesystem adapters call — stays here.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_filesystem::{FilesystemError, FilesystemOperation};
use ironclaw_host_api::VirtualPath;
use ironclaw_safety::{Sanitizer, Severity};

use crate::chunking::content_sha256;
use crate::events::MemoryAuditContext;
use crate::path::{MemoryDocumentPath, MemoryDocumentScope, memory_error, valid_memory_path};

pub use ironclaw_memory::{
    DEFAULT_PROMPT_PROTECTED_PATHS, PromptProtectedPathClass, PromptProtectedPathRegistry,
    PromptSafetyAllowanceId, PromptSafetyPolicyVersion, PromptSafetyReason, PromptSafetyReasonCode,
    PromptSafetySeverity, PromptSafetySummary, PromptWriteOperation, PromptWriteSafetyDecision,
    PromptWriteSafetyError, PromptWriteSafetyEvent, PromptWriteSafetyEventKind,
    PromptWriteSafetyEventSink, PromptWriteSafetyPolicy, PromptWriteSafetyRequest,
    PromptWriteSource,
};

/// Map a sanitizer severity to the contract's sanitized severity bucket.
///
/// Replaces the former `From<Severity> for PromptSafetySeverity` impl: now that
/// `PromptSafetySeverity` lives in `ironclaw_memory` (a foreign type)
/// and `Severity` is foreign too, the orphan rule forbids the `From` impl here.
fn severity_from(severity: Severity) -> PromptSafetySeverity {
    match severity {
        Severity::Low => PromptSafetySeverity::Low,
        Severity::Medium => PromptSafetySeverity::Medium,
        Severity::High => PromptSafetySeverity::High,
        Severity::Critical => PromptSafetySeverity::Critical,
    }
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
        let severity = severity_from(max_severity);
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
    pub audit_context: Option<&'a MemoryAuditContext>,
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
        relative_path_hash: Some(content_sha256(check.path.relative_path())),
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
        audit_context: check.audit_context.cloned(),
    };
    if let Err(error) = event_sink.record_prompt_write_safety_event(event).await {
        tracing::debug!(
            target: "ironclaw::memory::prompt_write_safety",
            error = %error,
            operation = %check.operation,
            source = %check.source,
            "failed to record prompt write safety event"
        );
        if parts.require_sink {
            return Err(prompt_write_safety_error(
                check
                    .path
                    .virtual_path()
                    .unwrap_or_else(|_| valid_memory_path()),
                check.filesystem_operation,
                PromptSafetyReason::new(PromptSafetyReasonCode::PromptWriteSafetyEventUnavailable),
            ));
        }
        return Ok(());
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
