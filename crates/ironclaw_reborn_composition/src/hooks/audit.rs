//! Quarantine audit emission for the third-party hook projection path.

use std::sync::Arc;

use ironclaw_events::{SecurityAuditEvent, SecurityAuditSink, SecurityBoundary, SecurityDecision};
use ironclaw_host_api::{InvocationId, ResourceScope, SYSTEM_RESERVED_ID, TenantId, UserId};

/// Stable security-audit code for hook projection/install quarantine.
pub(super) const HOOK_QUARANTINED_CODE: &str = "hook_quarantined";

/// Structured target for the security-audit `tracing` channel. Composition is a
/// pre-run phase (no run-scoped `LoopHostMilestoneSink` exists yet), so
/// quarantine decisions are also surfaced via `tracing` at this stable target.
pub(super) const SECURITY_AUDIT_TARGET: &str = "security_audit";

/// Emit a `hook.quarantined` security-audit event for an extension whose hooks
/// were dropped during projection.
///
/// This is a background / composition path, so per the REPL/TUI logging rule it
/// uses `debug!` (never `info!`/`warn!`, which corrupt the interactive
/// display). When a [`SecurityAuditSink`] is wired, it also records a
/// payload-free durable event carrying the tenant-scoped synthetic system scope.
pub(super) fn emit_hook_quarantined(
    tenant_id: &ironclaw_host_api::TenantId,
    extension_id: &str,
    reason: &str,
    hooks_dropped: usize,
    audit_sink: Option<&Arc<dyn SecurityAuditSink>>,
) {
    #[cfg(test)]
    test_capture::record(tenant_id, extension_id);

    if let Some(sink) = audit_sink {
        sink.record(
            SecurityAuditEvent::new(
                SecurityBoundary::HookQuarantine,
                SecurityDecision::Blocked,
                HOOK_QUARANTINED_CODE,
            )
            .with_scope(quarantine_scope(tenant_id)),
        );
    }

    tracing::debug!(
        target: SECURITY_AUDIT_TARGET,
        event = "hook.quarantined",
        tenant_id = %tenant_id.as_str(),
        extension_id = %extension_id,
        reason = %reason,
        hooks_dropped = hooks_dropped,
        "third-party extension hooks quarantined during projection"
    );
}

fn quarantine_scope(tenant_id: &TenantId) -> ResourceScope {
    ResourceScope {
        tenant_id: tenant_id.clone(),
        user_id: UserId::from_trusted(SYSTEM_RESERVED_ID.to_string()),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

/// Deterministic, thread-local capture of quarantine-audit attribution for
/// tests. The `tracing` channel is the production observability surface, but
/// asserting on it from a unit test is racy: a sibling test that installs a
/// global subscriber raises `tracing`'s process-wide max-level hint and can
/// suppress the `debug!` event before any thread-local subscriber sees it.
/// This thread-local sink records the `(tenant_id, extension_id)` pairs emitted
/// on the CURRENT thread regardless of any global `tracing` filter, so the
/// caller-driven attribution test is deterministic under parallel `cargo test`.
#[cfg(test)]
pub(super) mod test_capture {
    use std::cell::RefCell;

    thread_local! {
        static CAPTURED: RefCell<Option<Vec<(String, String)>>> = const { RefCell::new(None) };
    }

    /// Run `body` with capture armed on this thread; returns the recorded
    /// `(tenant_id, extension_id)` quarantine pairs. Nesting is not supported
    /// (a single test scope at a time), which matches the per-test usage.
    pub(in crate::hooks) fn with_capture<R>(
        body: impl FnOnce() -> R,
    ) -> (R, Vec<(String, String)>) {
        CAPTURED.with(|cell| *cell.borrow_mut() = Some(Vec::new()));
        let result = body();
        let captured = CAPTURED.with(|cell| cell.borrow_mut().take().unwrap_or_default());
        (result, captured)
    }

    pub(super) fn record(tenant_id: &ironclaw_host_api::TenantId, extension_id: &str) {
        CAPTURED.with(|cell| {
            if let Some(buffer) = cell.borrow_mut().as_mut() {
                buffer.push((tenant_id.as_str().to_string(), extension_id.to_string()));
            }
        });
    }
}
