//! Security-boundary audit primitive.
//!
//! [`SecurityAuditSink`] is a payload-free recording trait for *security
//! decisions* — the moments when a defense-in-depth boundary in IronClaw
//! Reborn allows, blocks, or otherwise re-shapes a request. It is intentionally
//! distinct from [`AuditSink`](crate::AuditSink) (which carries control-plane
//! `AuditEnvelope` records with full correlation context) and from
//! [`EventSink`](crate::EventSink) (which carries runtime event transitions).
//!
//! # Why a dedicated sink?
//!
//! Across recent PR review on hooks (#3573), no-exposure guard (#3767),
//! product-auth continuations (#3888), and the credential boundary (#3903),
//! the same pattern keeps recurring: security boundaries emit their decisions
//! via `tracing::warn!` or `tracing::error!`. The repo `CLAUDE.md` explicitly
//! forbids `warn!`/`info!` for non-user-facing diagnostics — those levels
//! corrupt the REPL/TUI rendering. CLAUDE.md *also* says:
//!
//! > LLM data is never deleted. All LLM output … is the most valuable data in
//! > the system. Never strip, truncate, or delete it from the database. Mark
//! > with timestamps, make filterable, but always retain.
//!
//! Security-boundary decisions are exactly this class of data: they must be
//! retained, filterable by boundary/decision/scope, and never be the only
//! signal carried on a `warn!` line.
//!
//! # Payload-free invariant
//!
//! [`SecurityAuditEvent`] deliberately has **no free-form `String` payload
//! field** — there is no `details`, `message`, `value`, or `reason: String`
//! into which a careless caller could stuff the secret/header/path that the
//! boundary just rejected. The event records only:
//!
//! - which boundary fired ([`SecurityBoundary`])
//! - what the decision was ([`SecurityDecision`])
//! - a `'static` reason **code** (grep target — see below)
//! - optional capability id and resource scope (already redaction-safe types)
//! - a timestamp
//!
//! This invariant is **load-bearing**. Do not add a `String` field. If you
//! need a new dimension, extend [`SecurityBoundary`] or [`SecurityDecision`],
//! or add a `&'static str` code constant.
//!
//! # The `code` convention
//!
//! `SecurityAuditEvent::code` is a short, stable, `&'static str`. It exists
//! so that SRE/ops can grep durable logs for a specific decision class
//! (`leak_redact_failed`, `no_exposure_block_header`, `hook_deny_predicate`,
//! `auth_continuation_replay`, `mcp_direct_lease_deny`, ...). Treat it like a
//! metric name: lowercase, snake_case, never user-derived, never PII, never a
//! secret. New codes should be added as `pub const` items in the module that
//! owns the boundary so they are visible to consumers.

use std::sync::Arc;
use std::time::SystemTime;

use chrono::Utc;
use ironclaw_host_api::{
    ActionSummary, AuditEnvelope, AuditEventId, AuditStage, CapabilityId, CorrelationId,
    DecisionSummary, EffectKind, ResourceScope,
};

use crate::sink::DurableAuditLog;

/// Identifies *which* security boundary produced an audit event.
///
/// New variants should be added when a new defense-in-depth surface is
/// introduced. Prefer extending this enum over reusing an existing variant
/// for an unrelated boundary — downstream filters and dashboards key off it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SecurityBoundary {
    /// Output-redaction / leak-detection guard around obligation completion.
    LeakDetector,
    /// Egress guard that blocks redacted host paths / sensitive headers from
    /// leaving the runtime. (PR #3767 follow-up adoption target.)
    NoExposureGuard,
    /// Credential channel boundary that gates secret-material handoff to
    /// runtime adapters. (PR #3903 follow-up adoption target.)
    CredentialChannel,
    /// Auth continuation dispatcher that re-enters a paused product-auth
    /// workflow. (PR #3888 follow-up adoption target.)
    AuthContinuation,
    /// Hook predicate / envelope rejection. (PR #3573 follow-up adoption
    /// target.)
    HookDeny,
    /// Hook activation/projection quarantine boundary. Used when an installed
    /// extension's hook set is dropped during projection or install-time
    /// validation.
    HookQuarantine,
    /// MCP direct-lease boundary that gates raw protocol access.
    McpDirectLease,
}

impl SecurityBoundary {
    /// Stable short token for logging / metrics. Does not depend on
    /// `Debug` formatting.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LeakDetector => "leak_detector",
            Self::NoExposureGuard => "no_exposure_guard",
            Self::CredentialChannel => "credential_channel",
            Self::AuthContinuation => "auth_continuation",
            Self::HookDeny => "hook_deny",
            Self::HookQuarantine => "hook_quarantine",
            Self::McpDirectLease => "mcp_direct_lease",
        }
    }
}

/// What the boundary decided.
///
/// New variants should describe *categories* of decision (blocked, allowed,
/// scope-mismatched, replay-rejected), not specific reasons — the specific
/// reason goes in [`SecurityAuditEvent::code`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SecurityDecision {
    /// Boundary denied the operation.
    Blocked,
    /// Boundary allowed the operation (recorded for retention/forensics).
    Allowed,
    /// Boundary observed a scope mismatch (e.g. wrong tenant/project).
    ScopeMismatch,
    /// Boundary rejected a replay or stale nonce.
    ReplayRejected,
}

impl SecurityDecision {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Blocked => "blocked",
            Self::Allowed => "allowed",
            Self::ScopeMismatch => "scope_mismatch",
            Self::ReplayRejected => "replay_rejected",
        }
    }
}

/// A single security-boundary decision.
///
/// **Payload-free by construction.** See module-level docs for the invariant.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct SecurityAuditEvent {
    pub boundary: SecurityBoundary,
    pub decision: SecurityDecision,
    pub capability_id: Option<CapabilityId>,
    pub scope: Option<ResourceScope>,
    pub timestamp: SystemTime,
    /// Stable, lowercase, snake_case reason code. Grep target for SRE.
    ///
    /// MUST NOT be derived from user input or contain secret material.
    /// See the module-level docs for the `code` convention.
    pub code: &'static str,
}

impl SecurityAuditEvent {
    /// Construct a new event with `timestamp = SystemTime::now()`.
    ///
    /// Callers must supply `code` as a `&'static str` literal or `pub const`.
    /// This is the only way to construct the event, which (together with the
    /// type system) enforces the no-free-form-payload invariant.
    pub fn new(boundary: SecurityBoundary, decision: SecurityDecision, code: &'static str) -> Self {
        Self {
            boundary,
            decision,
            capability_id: None,
            scope: None,
            timestamp: SystemTime::now(),
            code,
        }
    }

    pub fn with_capability_id(mut self, capability_id: CapabilityId) -> Self {
        self.capability_id = Some(capability_id);
        self
    }

    pub fn with_scope(mut self, scope: ResourceScope) -> Self {
        self.scope = Some(scope);
        self
    }
}

/// Recording surface for [`SecurityAuditEvent`].
///
/// **Best-effort observability.** Implementations must not panic. A sink
/// failure must not change the outcome of the surrounding security decision;
/// the boundary has already decided (block/allow) by the time it records.
///
/// `record` is **sync** by design — unlike [`AuditSink`](crate::AuditSink),
/// security boundaries frequently fire from sync paths (predicate gates,
/// header filters, output redaction) and we do not want adoption to require
/// `.await` plumbing. Durable adapters can buffer / forward to async logs.
pub trait SecurityAuditSink: Send + Sync + std::fmt::Debug {
    fn record(&self, event: SecurityAuditEvent);
}

impl<T> SecurityAuditSink for Arc<T>
where
    T: SecurityAuditSink + ?Sized,
{
    fn record(&self, event: SecurityAuditEvent) {
        (**self).record(event);
    }
}

/// Drops every event. Suitable for tests and contexts that do not yet need
/// durable security-audit recording.
#[derive(Clone, Debug, Default)]
pub struct NoopSecurityAuditSink;

impl SecurityAuditSink for NoopSecurityAuditSink {
    fn record(&self, _event: SecurityAuditEvent) {}
}

/// Records each event to every configured sink.
///
/// Best-effort: child sinks must not panic by contract, and this fanout does
/// not retry. It exists so production composition can keep the tracing surface
/// while also wiring a durable sink.
#[derive(Clone, Debug, Default)]
pub struct FanoutSecurityAuditSink {
    sinks: Vec<Arc<dyn SecurityAuditSink>>,
}

impl FanoutSecurityAuditSink {
    pub fn new(sinks: Vec<Arc<dyn SecurityAuditSink>>) -> Self {
        Self { sinks }
    }
}

impl SecurityAuditSink for FanoutSecurityAuditSink {
    fn record(&self, event: SecurityAuditEvent) {
        for sink in &self.sinks {
            sink.record(event.clone());
        }
    }
}

/// Best-effort adapter that persists payload-free security-audit events into
/// the durable control-plane audit log.
///
/// [`SecurityAuditSink::record`] is synchronous because several security
/// boundaries fire from sync paths. This adapter schedules the durable append
/// on the current Tokio runtime. If no runtime is present, it emits a debug
/// diagnostic and preserves the surrounding security decision.
#[derive(Clone)]
pub struct DurableSecurityAuditSink {
    log: Arc<dyn DurableAuditLog>,
}

impl DurableSecurityAuditSink {
    pub fn new(log: Arc<dyn DurableAuditLog>) -> Self {
        Self { log }
    }
}

impl std::fmt::Debug for DurableSecurityAuditSink {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DurableSecurityAuditSink")
            .field("log", &"<durable_audit_log>")
            .finish()
    }
}

impl SecurityAuditSink for DurableSecurityAuditSink {
    fn record(&self, event: SecurityAuditEvent) {
        let Some(record) = audit_envelope_from_security_event(event) else {
            tracing::debug!(
                target: "ironclaw::security_audit",
                "security audit event omitted from durable log because it had no scope"
            );
            return;
        };
        let log = Arc::clone(&self.log);
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move {
                    if let Err(error) = log.append(record).await {
                        tracing::debug!(
                            target: "ironclaw::security_audit",
                            error = %error,
                            "failed to persist security audit event"
                        );
                    }
                });
            }
            Err(error) => {
                tracing::debug!(
                    target: "ironclaw::security_audit",
                    error = %error,
                    "security audit event omitted from durable log because no Tokio runtime is active"
                );
            }
        }
    }
}

fn audit_envelope_from_security_event(event: SecurityAuditEvent) -> Option<AuditEnvelope> {
    let scope = event.scope?;
    Some(AuditEnvelope {
        event_id: AuditEventId::new(),
        correlation_id: CorrelationId::new(),
        stage: audit_stage_for_decision(event.decision),
        timestamp: Utc::now(),
        tenant_id: scope.tenant_id,
        user_id: scope.user_id,
        agent_id: scope.agent_id,
        project_id: scope.project_id,
        mission_id: scope.mission_id,
        thread_id: scope.thread_id,
        invocation_id: scope.invocation_id,
        process_id: None,
        approval_request_id: None,
        extension_id: None,
        action: ActionSummary {
            kind: "security_boundary_decision".to_string(),
            target: Some(format!("{}:{}", event.boundary.as_str(), event.code)),
            effects: Vec::<EffectKind>::new(),
        },
        decision: DecisionSummary {
            kind: event.decision.as_str().to_string(),
            reason: None,
            actor: None,
        },
        result: None,
    })
}

fn audit_stage_for_decision(decision: SecurityDecision) -> AuditStage {
    match decision {
        SecurityDecision::Allowed => AuditStage::After,
        SecurityDecision::Blocked
        | SecurityDecision::ScopeMismatch
        | SecurityDecision::ReplayRejected => AuditStage::Denied,
    }
}

/// Emits each event at `tracing::debug!`.
///
/// `debug!` is chosen deliberately: per the repo `CLAUDE.md` REPL/TUI rule,
/// `info!`/`warn!` corrupt the interactive display. Security-boundary
/// decisions are not user-facing status — they are diagnostics + retention
/// data, so they go to `debug!`.
///
/// # Retention status (PR #3922 review)
///
/// This sink is the **observability** shape, not the durable retention store.
/// It emits every recorded field — `boundary`, `decision`, `code`,
/// `capability_id`, **and `scope`** — at `debug!` so SRE can grep/filter the
/// full dimension set the module promises. It does **not** itself persist
/// events to a queryable store; that durable backing is provided by wiring a
/// persistent [`SecurityAuditSink`] alongside this one (e.g. via a
/// multi-adapter) at composition time. Production composition installs this
/// sink today so security-boundary decisions are never dropped silently; the
/// durable store is a tracked follow-up and is intentionally decoupled from
/// this primitive.
#[derive(Clone, Debug, Default)]
pub struct TracingSecurityAuditSink;

impl SecurityAuditSink for TracingSecurityAuditSink {
    fn record(&self, event: SecurityAuditEvent) {
        // Emit the *full* recorded dimension set. The module docs promise
        // events are "filterable by boundary/decision/scope"; omitting `scope`
        // here would silently break that filterability for any boundary that
        // sets it (e.g. scope-mismatch decisions). `ResourceScope` is a
        // redaction-safe type, so logging it does not violate the
        // payload-free invariant.
        tracing::debug!(
            target: "ironclaw::security_audit",
            boundary = event.boundary.as_str(),
            decision = event.decision.as_str(),
            code = event.code,
            capability_id = event.capability_id.as_ref().map(|c| c.as_str()),
            scope = ?event.scope,
            "security boundary decision"
        );
    }
}

/// Test-only recording sink that captures every event. Not intended for
/// production: unbounded growth, no eviction.
#[derive(Debug, Default)]
pub struct InMemorySecurityAuditSink {
    events: std::sync::Mutex<Vec<SecurityAuditEvent>>,
}

impl InMemorySecurityAuditSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&self) -> Vec<SecurityAuditEvent> {
        self.events
            .lock()
            .expect("InMemorySecurityAuditSink mutex poisoned") // safety: test-only sink; poisoning means a test thread already panicked
            .clone()
    }

    pub fn len(&self) -> usize {
        self.events
            .lock()
            .expect("InMemorySecurityAuditSink mutex poisoned") // safety: test-only sink; poisoning means a test thread already panicked
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl SecurityAuditSink for InMemorySecurityAuditSink {
    fn record(&self, event: SecurityAuditEvent) {
        self.events
            .lock()
            .expect("InMemorySecurityAuditSink mutex poisoned") // safety: test-only sink; poisoning means a test thread already panicked
            .push(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundary_decision_codes_are_stable_tokens() {
        // These string forms are part of the public surface (used in logs,
        // dashboards, durable records). Locking them down here prevents an
        // accidental rename from silently breaking SRE pattern-matching.
        assert_eq!(SecurityBoundary::LeakDetector.as_str(), "leak_detector");
        assert_eq!(
            SecurityBoundary::NoExposureGuard.as_str(),
            "no_exposure_guard"
        );
        assert_eq!(
            SecurityBoundary::CredentialChannel.as_str(),
            "credential_channel"
        );
        assert_eq!(
            SecurityBoundary::AuthContinuation.as_str(),
            "auth_continuation"
        );
        assert_eq!(SecurityBoundary::HookDeny.as_str(), "hook_deny");
        assert_eq!(SecurityBoundary::HookQuarantine.as_str(), "hook_quarantine");
        assert_eq!(
            SecurityBoundary::McpDirectLease.as_str(),
            "mcp_direct_lease"
        );

        assert_eq!(SecurityDecision::Blocked.as_str(), "blocked");
        assert_eq!(SecurityDecision::Allowed.as_str(), "allowed");
        assert_eq!(SecurityDecision::ScopeMismatch.as_str(), "scope_mismatch");
        assert_eq!(SecurityDecision::ReplayRejected.as_str(), "replay_rejected");
    }

    #[test]
    fn noop_sink_drops_events() {
        let sink = NoopSecurityAuditSink;
        sink.record(SecurityAuditEvent::new(
            SecurityBoundary::LeakDetector,
            SecurityDecision::Blocked,
            "test_code",
        ));
    }

    #[test]
    fn in_memory_sink_captures_events_in_order() {
        let sink = InMemorySecurityAuditSink::new();
        assert!(sink.is_empty());

        sink.record(SecurityAuditEvent::new(
            SecurityBoundary::LeakDetector,
            SecurityDecision::Blocked,
            "first",
        ));
        sink.record(SecurityAuditEvent::new(
            SecurityBoundary::NoExposureGuard,
            SecurityDecision::Allowed,
            "second",
        ));

        let snapshot = sink.snapshot();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].code, "first");
        assert_eq!(snapshot[0].boundary, SecurityBoundary::LeakDetector);
        assert_eq!(snapshot[0].decision, SecurityDecision::Blocked);
        assert_eq!(snapshot[1].code, "second");
        assert_eq!(snapshot[1].boundary, SecurityBoundary::NoExposureGuard);
        assert_eq!(snapshot[1].decision, SecurityDecision::Allowed);
    }

    #[test]
    fn event_can_be_enriched_with_scope_and_capability() {
        use ironclaw_host_api::{InvocationId, UserId};

        let scope = ResourceScope::local_default(
            UserId::new("alice").expect("valid user id"),
            InvocationId::new(),
        )
        .expect("valid scope");
        let cap = CapabilityId::new("ironclaw.echo".to_string()).expect("valid capability id");

        let event = SecurityAuditEvent::new(
            SecurityBoundary::LeakDetector,
            SecurityDecision::Blocked,
            "leak_redact_failed",
        )
        .with_capability_id(cap.clone())
        .with_scope(scope.clone());

        assert_eq!(event.capability_id.as_ref(), Some(&cap));
        assert_eq!(event.scope.as_ref(), Some(&scope));
    }

    #[test]
    fn tracing_sink_records_scope_bearing_event() {
        // Regression guard (PR #3922 review): the production tracing sink must
        // accept and emit the *full* dimension set — including `scope` — not
        // just boundary/decision/code/capability. A scope-bearing event must
        // record without panicking through both the concrete type and the
        // `Arc<dyn>` boundary used in composition.
        use ironclaw_host_api::{InvocationId, UserId};

        let scope = ResourceScope::local_default(
            UserId::new("alice").expect("valid user id"),
            InvocationId::new(),
        )
        .expect("valid scope");
        let event = SecurityAuditEvent::new(
            SecurityBoundary::HookDeny,
            SecurityDecision::Blocked,
            "hook_deny_predicate",
        )
        .with_scope(scope);

        let sink: Arc<dyn SecurityAuditSink> = Arc::new(TracingSecurityAuditSink);
        sink.record(event);
    }

    #[test]
    fn arc_passthrough_records_on_inner() {
        let sink: Arc<InMemorySecurityAuditSink> = Arc::new(InMemorySecurityAuditSink::new());
        let handle: Arc<dyn SecurityAuditSink> = sink.clone();
        handle.record(SecurityAuditEvent::new(
            SecurityBoundary::HookDeny,
            SecurityDecision::Blocked,
            "passthrough",
        ));
        assert_eq!(sink.len(), 1);
    }

    #[tokio::test]
    async fn durable_sink_appends_scope_bearing_event() {
        use crate::{DurableAuditLog, EventCursor, EventStreamKey, InMemoryDurableAuditLog};
        use ironclaw_host_api::{InvocationId, TenantId, UserId};

        let log = Arc::new(InMemoryDurableAuditLog::new());
        let sink = DurableSecurityAuditSink::new(log.clone());
        let scope = ResourceScope {
            tenant_id: TenantId::new("tenant-a").expect("tenant"),
            user_id: UserId::new("system").expect("user"),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        };

        sink.record(
            SecurityAuditEvent::new(
                SecurityBoundary::HookQuarantine,
                SecurityDecision::Blocked,
                "hook_quarantined",
            )
            .with_scope(scope.clone()),
        );
        tokio::task::yield_now().await;

        let stream = EventStreamKey::new(scope.tenant_id, scope.user_id, None);
        let replay = log
            .read_after_cursor(
                &stream,
                &crate::ReadScope::any(),
                Some(EventCursor::origin()),
                10,
            )
            .await
            .expect("read audit log");
        assert_eq!(replay.entries.len(), 1);
        let record = &replay.entries[0].record;
        assert_eq!(record.stage, AuditStage::Denied);
        assert_eq!(record.action.kind, "security_boundary_decision");
        assert_eq!(
            record.action.target.as_deref(),
            Some("hook_quarantine:hook_quarantined")
        );
        assert_eq!(record.decision.kind, "blocked");
    }
}
