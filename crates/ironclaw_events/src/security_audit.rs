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

use ironclaw_host_api::{CapabilityId, ResourceScope};

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
    /// MCP direct-lease boundary that gates raw protocol access.
    McpDirectLease,
    /// Attestation boundary: WebAuthn assertion / challenge verification in the
    /// custodial attested-signing path. (attested-signing stack.)
    Attestation,
    /// Custody key-access boundary: a custodial signing key is about to be used
    /// to produce a signature. (attested-signing stack.)
    CustodyKeyAccess,
    /// Chain-signing boundary: signing bytes are about to be signed for a
    /// specific chain/account. (attested-signing stack.)
    ChainSigning,
    /// Broadcast-submit boundary: a signed transaction is about to be submitted
    /// to a network. (attested-signing stack.)
    BroadcastSubmit,
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
            Self::McpDirectLease => "mcp_direct_lease",
            Self::Attestation => "attestation",
            Self::CustodyKeyAccess => "custody_key_access",
            Self::ChainSigning => "chain_signing",
            Self::BroadcastSubmit => "broadcast_submit",
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

/// Emits each event at `tracing::debug!`.
///
/// `debug!` is chosen deliberately: per the repo `CLAUDE.md` REPL/TUI rule,
/// `info!`/`warn!` corrupt the interactive display. Security-boundary
/// decisions are not user-facing status — they are diagnostics + retention
/// data, so they go to `debug!`. A real deployment will additionally wire a
/// durable sink behind this (or alongside it via a multi-adapter).
#[derive(Clone, Debug, Default)]
pub struct TracingSecurityAuditSink;

impl SecurityAuditSink for TracingSecurityAuditSink {
    fn record(&self, event: SecurityAuditEvent) {
        tracing::debug!(
            target: "ironclaw::security_audit",
            boundary = event.boundary.as_str(),
            decision = event.decision.as_str(),
            code = event.code,
            capability_id = event.capability_id.as_ref().map(|c| c.as_str()),
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

/// Error surfaced by a [`DurableSecurityAudit`] when a security-boundary
/// decision could NOT be durably recorded.
///
/// Unlike [`SecurityAuditSink`] (best-effort, infallible-by-contract), a
/// `DurableSecurityAudit` failure is **load-bearing**: the protected action
/// (pre-sign, pre-broadcast) MUST be refused when recording fails. See the
/// trait docs for the fail-closed contract.
#[derive(Debug, thiserror::Error)]
pub enum DurableAuditError {
    /// The durable backend failed to persist the event. Carries an opaque
    /// description only — never the rejected secret/header/path (the
    /// payload-free invariant of [`SecurityAuditEvent`] still holds).
    #[error("durable security-audit record failed: {reason}")]
    Backend {
        /// Human-readable description of the backend failure.
        reason: String,
    },
}

/// Durable, **fail-closed** security-audit contract for pre-sign / pre-broadcast
/// checkpoints.
///
/// This is the strict counterpart to [`SecurityAuditSink`]. The existing sink
/// is best-effort observability: it fires *after* a boundary has already
/// decided and its failure must never change the outcome. `DurableSecurityAudit`
/// is the opposite — it is awaited *before* a high-value, irreversible action
/// (using a custody key, broadcasting a signed transaction) and its failure
/// MUST block that action. "We could not write the audit record" is treated as
/// "do not perform the action": no un-audited custody-key use or broadcast.
///
/// Contract for implementations and call sites:
///
/// - `record` is `async` and returns `Result<(), DurableAuditError>`.
/// - A call site that gates a protected action MUST `.await` `record` and
///   refuse the action on `Err`.
/// - Implementations must persist durably before returning `Ok`. A backend
///   that merely buffers in volatile memory and could lose the record on crash
///   does NOT satisfy the durable contract for production use (the in-memory
///   impl below is test-only).
/// - The existing best-effort [`SecurityAuditSink`] is unchanged and its
///   callers are untouched; this is an additive, stricter path.
#[async_trait::async_trait]
pub trait DurableSecurityAudit: Send + Sync + std::fmt::Debug {
    /// Durably record a security-boundary decision. The caller awaits this and
    /// fails closed (refuses the protected action) on `Err`.
    async fn record(&self, event: SecurityAuditEvent) -> Result<(), DurableAuditError>;
}

#[async_trait::async_trait]
impl<T> DurableSecurityAudit for Arc<T>
where
    T: DurableSecurityAudit + ?Sized,
{
    async fn record(&self, event: SecurityAuditEvent) -> Result<(), DurableAuditError> {
        (**self).record(event).await
    }
}

/// Fail-closed pre-action checkpoint.
///
/// This is the single production entry point a high-value, irreversible
/// action (custody-key use, transaction broadcast) MUST funnel through before
/// proceeding. It durably records `event` via `audit` and returns `Ok(())`
/// **only** if the record succeeded. On any [`DurableAuditError`] it returns
/// `Err` and the caller MUST NOT perform the protected action — an un-audited
/// custody-key use or broadcast is forbidden ("we could not write the audit
/// record" ⇒ "do not perform the action").
///
/// The record happens *before* the action by construction: the action is only
/// reachable on the `Ok` path. Callers therefore cannot accidentally swallow a
/// failed audit and proceed anyway.
///
/// durable adapter: the real PG/libSQL-backed [`DurableSecurityAudit`]
/// implementation that this checkpoint awaits in production is delivered by the
/// attested-signing durable-store track (separate PR). Until then, wire a
/// production-durable adapter here; the in-memory impl below is test-only and
/// does NOT satisfy the durable contract for production use.
pub async fn checkpoint_or_refuse<A: DurableSecurityAudit + ?Sized>(
    audit: &A,
    event: SecurityAuditEvent,
) -> Result<(), DurableAuditError> {
    audit.record(event).await
}

/// Test-only durable audit that captures every successfully-recorded event in
/// memory. Not production-durable (volatile, unbounded). Use the `fail_after`
/// constructor to simulate a backend that starts failing, exercising the
/// fail-closed call-site contract.
#[derive(Debug, Default)]
pub struct InMemoryDurableSecurityAudit {
    events: std::sync::Mutex<Vec<SecurityAuditEvent>>,
    /// When `Some(n)`, `record` fails once `n` events have already been
    /// recorded — lets tests drive the `Err` branch deterministically.
    fail_after: Option<usize>,
}

impl InMemoryDurableSecurityAudit {
    /// An always-succeeding in-memory durable audit.
    pub fn new() -> Self {
        Self::default()
    }

    /// An audit that records the first `n` events then fails every subsequent
    /// `record` with [`DurableAuditError::Backend`].
    pub fn fail_after(n: usize) -> Self {
        Self {
            events: std::sync::Mutex::new(Vec::new()),
            fail_after: Some(n),
        }
    }

    /// Snapshot of durably-recorded events.
    pub fn snapshot(&self) -> Vec<SecurityAuditEvent> {
        self.events
            .lock()
            .expect("InMemoryDurableSecurityAudit mutex poisoned") // safety: test-only
            .clone()
    }

    /// Number of durably-recorded events.
    pub fn len(&self) -> usize {
        self.events
            .lock()
            .expect("InMemoryDurableSecurityAudit mutex poisoned") // safety: test-only
            .len()
    }

    /// Whether no events have been recorded.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[async_trait::async_trait]
impl DurableSecurityAudit for InMemoryDurableSecurityAudit {
    async fn record(&self, event: SecurityAuditEvent) -> Result<(), DurableAuditError> {
        let mut events = self.events.lock().map_err(|e| DurableAuditError::Backend {
            reason: e.to_string(),
        })?;
        if let Some(limit) = self.fail_after
            && events.len() >= limit
        {
            return Err(DurableAuditError::Backend {
                reason: "simulated durable-backend failure".to_string(),
            });
        }
        events.push(event);
        Ok(())
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
        assert_eq!(
            SecurityBoundary::McpDirectLease.as_str(),
            "mcp_direct_lease"
        );
        // attested-signing boundaries — wire-stable snake_case tokens.
        assert_eq!(SecurityBoundary::Attestation.as_str(), "attestation");
        assert_eq!(
            SecurityBoundary::CustodyKeyAccess.as_str(),
            "custody_key_access"
        );
        assert_eq!(SecurityBoundary::ChainSigning.as_str(), "chain_signing");
        assert_eq!(
            SecurityBoundary::BroadcastSubmit.as_str(),
            "broadcast_submit"
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

    /// Drive the *production* [`checkpoint_or_refuse`] and perform the protected
    /// action ONLY on its `Ok` path — exactly the real call-site contract.
    async fn guarded_action<A: DurableSecurityAudit>(
        audit: &A,
        event: SecurityAuditEvent,
    ) -> Result<&'static str, DurableAuditError> {
        checkpoint_or_refuse(audit, event).await?;
        Ok("signed")
    }

    #[tokio::test]
    async fn durable_audit_ok_allows_protected_action() {
        let audit = InMemoryDurableSecurityAudit::new();
        let event = SecurityAuditEvent::new(
            SecurityBoundary::CustodyKeyAccess,
            SecurityDecision::Allowed,
            "custody_key_use",
        );
        let outcome = guarded_action(&audit, event).await;
        assert_eq!(outcome.expect("audit ok must allow action"), "signed");
        assert_eq!(audit.len(), 1);
    }

    #[tokio::test]
    async fn durable_audit_err_blocks_protected_action() {
        // Backend fails on the very first record -> checkpoint_or_refuse returns
        // Err and the protected action must not run (fail-closed).
        let audit = InMemoryDurableSecurityAudit::fail_after(0);
        let event = SecurityAuditEvent::new(
            SecurityBoundary::BroadcastSubmit,
            SecurityDecision::Allowed,
            "broadcast_submit",
        );
        let outcome = guarded_action(&audit, event).await;
        assert!(
            matches!(outcome, Err(DurableAuditError::Backend { .. })),
            "a failed durable audit must block the protected action"
        );
        assert!(
            audit.is_empty(),
            "no event recorded and (critically) no action performed"
        );
    }

    #[tokio::test]
    async fn checkpoint_or_refuse_returns_err_on_audit_failure() {
        // Direct test of the production checkpoint API: a failing backend makes
        // the checkpoint itself refuse (the gate, not just a downstream caller).
        let audit = InMemoryDurableSecurityAudit::fail_after(0);
        let res = checkpoint_or_refuse(
            &audit,
            SecurityAuditEvent::new(
                SecurityBoundary::CustodyKeyAccess,
                SecurityDecision::Allowed,
                "custody_key_use",
            ),
        )
        .await;
        assert!(
            matches!(res, Err(DurableAuditError::Backend { .. })),
            "checkpoint must refuse when the durable audit write fails"
        );
        assert!(audit.is_empty());
    }

    #[tokio::test]
    async fn durable_audit_arc_passthrough() {
        let audit: Arc<InMemoryDurableSecurityAudit> =
            Arc::new(InMemoryDurableSecurityAudit::new());
        let handle: Arc<dyn DurableSecurityAudit> = audit.clone();
        handle
            .record(SecurityAuditEvent::new(
                SecurityBoundary::ChainSigning,
                SecurityDecision::Allowed,
                "chain_sign",
            ))
            .await
            .expect("record");
        assert_eq!(audit.len(), 1);
    }
}
