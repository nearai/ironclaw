//! Hook-lifecycle provider-ownership resolution.
//!
//! Extracted from the dispatcher (PR #3931 P2) so the dense security policy
//! that decides *who owns a hook-lifecycle event* lives in one focused,
//! directly-tested place rather than inline in the oversized dispatch module.
//!
//! # Security policy (PR #3640 Bug 3 / PR #3931 Hole 2)
//!
//! For hook-lifecycle event kinds (`HookDispatched` / `HookDecisionEmitted` /
//! `HookFailed`) the authoritative owner is the extension that *registered* the
//! hook named by `event.hook_id` — NOT the `event.provider` payload, which is
//! attacker-controllable when a hook synthesizes a lifecycle event for another
//! hook. `event.provider` is a plain serialized field; a synthesized event can
//! set it to any extension id, and there is no unforgeable host-stamped owner
//! source distinct from the payload today. So for lifecycle events the carried
//! provider is NEVER trusted: the only path that yields an owner is a registry
//! hit on `hook_id`.
//!
//! Non-lifecycle events carry no `hook_id` anchor; their `provider` claim names
//! the capability provider established by the host and is not spoofable in the
//! same way, so it is used as-is.
//!
//! Every other branch fails closed to `None` (the hook is inert for the
//! event), and the distinct failure modes must NOT be collapsed into a
//! carried-provider fallback:
//!
//!   1. Poisoned registry — the registry cannot be trusted at all.
//!   2. Unknown hook_id — no local binding owns this hook; trusting the claim
//!      would let a synthesized event (unknown hook_id + provider=ext-a)
//!      activate ext-a's `OwnCapabilities` hooks.
//!   3. Missing hook_id on a lifecycle event — cannot be anchored to an owner.

use ironclaw_events::{RuntimeEvent, RuntimeEventKind};
use ironclaw_host_api::ExtensionId;

/// Result of probing the hook registry for the owner of a `hook_id`.
///
/// Lifting the registry probe to a value lets the ownership policy be tested
/// directly — including the poisoned-registry path — without constructing a
/// real poisoned `Mutex`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LifecycleOwnerLookup {
    /// The registry resolved `hook_id` to this owning extension.
    Owner(ExtensionId),
    /// No local binding owns `hook_id`.
    Unknown,
    /// The registry lock was poisoned and cannot be trusted.
    Poisoned,
}

/// Returns `true` for the hook-lifecycle event kinds whose owner must be
/// resolved from the registry rather than the carried provider.
///
/// This is an exhaustive `match` with NO wildcard arm by design: every
/// `RuntimeEventKind` variant is classified explicitly so that adding a new
/// variant fails to compile until it is deliberately placed on the lifecycle
/// or non-lifecycle side. A `matches!` (or a `_ => false` arm) would let a
/// future lifecycle variant silently fall through into the non-lifecycle
/// branch of [`resolve_event_owner`], which trusts the forgeable
/// `event.provider` field — reopening PR #3931 Hole 2 with no compile-time
/// signal.
pub(crate) fn is_lifecycle_kind(kind: RuntimeEventKind) -> bool {
    match kind {
        RuntimeEventKind::HookDispatched
        | RuntimeEventKind::HookDecisionEmitted
        | RuntimeEventKind::HookFailed => true,
        RuntimeEventKind::DispatchRequested
        | RuntimeEventKind::RuntimeSelected
        | RuntimeEventKind::DispatchSucceeded
        | RuntimeEventKind::DispatchFailed
        | RuntimeEventKind::CapabilityActivityRequested
        | RuntimeEventKind::CapabilityActivitySucceeded
        | RuntimeEventKind::CapabilityActivityFailed
        | RuntimeEventKind::ModelStarted
        | RuntimeEventKind::ModelCompleted
        | RuntimeEventKind::ModelFailed
        | RuntimeEventKind::AssistantReplyFinalized
        | RuntimeEventKind::LoopCompleted
        | RuntimeEventKind::LoopCancelled
        | RuntimeEventKind::LoopFailed
        | RuntimeEventKind::ProcessStarted
        | RuntimeEventKind::ProcessCompleted
        | RuntimeEventKind::ProcessFailed
        | RuntimeEventKind::ProcessKilled => false,
    }
}

/// Resolve the authoritative scope-provider for a runtime event.
///
/// `probe` is the registry lookup for the event's `hook_id` (only consulted for
/// lifecycle events). Keeping it as an already-computed value rather than
/// taking the registry directly makes this function pure and table-testable.
///
/// See the module docs for the full security policy. The short version:
///
/// - Non-lifecycle event → trust `event.provider` (host-established).
/// - Lifecycle event, registry hit → use the resolved owner; if the carried
///   provider disagrees, that is a spoof — log and still use the resolved
///   owner.
/// - Lifecycle event, unknown hook_id / poisoned registry / missing hook_id →
///   fail closed to `None`. Never fall back to the forgeable carried provider.
pub(crate) fn resolve_event_owner(
    event: &RuntimeEvent,
    probe: impl FnOnce(&str) -> LifecycleOwnerLookup,
) -> Option<ExtensionId> {
    if !is_lifecycle_kind(event.kind) {
        // Non-lifecycle events carry no hook-id anchor; the provider claim is
        // the only available signal and names the host-established capability
        // provider, so it is not spoofable in the same way.
        return event.provider.clone();
    }
    let Some(hook_id) = event.hook_id.as_deref() else {
        // A hook-lifecycle event with no hook_id cannot be anchored to an
        // owner. Fail-closed: do not trust the payload provider.
        return None;
    };
    match probe(hook_id) {
        LifecycleOwnerLookup::Poisoned => {
            // Fail-closed: an untrusted registry resolves to no owner, so
            // providerless OwnCapabilities hooks remain inert. We must not
            // substitute the spoofable carried provider here.
            None
        }
        LifecycleOwnerLookup::Owner(resolved_owner) => {
            if let Some(claimed) = event.provider.as_ref()
                && &resolved_owner != claimed
            {
                tracing::warn!(
                    hook_id = hook_id,
                    claimed_provider = claimed.as_str(),
                    resolved_provider = resolved_owner.as_str(),
                    "hook-lifecycle event provider claim disagrees with \
                     registry-resolved owner; using resolved owner (possible \
                     provider spoof)"
                );
            }
            Some(resolved_owner)
        }
        LifecycleOwnerLookup::Unknown => {
            // Unknown hook_id: no local binding owns it. The carried provider
            // is forgeable and cannot be authenticated, so it is NOT used — a
            // synthesized lifecycle event must never be able to target another
            // extension's OwnCapabilities hooks. (If a future design adds an
            // unforgeable host-stamped owner field distinct from the payload, a
            // legitimate foreign-host lifecycle path could be honored here;
            // until then, fail closed.)
            if event.provider.is_some() {
                tracing::debug!(
                    hook_id = hook_id,
                    "hook-lifecycle event names an unknown hook_id while carrying a \
                     provider claim; ignoring the unauthenticated claim and resolving \
                     to no owner (fail-closed)"
                );
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_events::RuntimeEvent;
    use ironclaw_host_api::{
        AgentId, CapabilityId, ProjectId, ResourceScope, TenantId, ThreadId, UserId,
    };

    fn ext(id: &str) -> ExtensionId {
        ExtensionId::new(id).expect("valid extension id literal")
    }

    fn scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant-a").expect("tenant"),
            user_id: UserId::new("user-a").expect("user"),
            agent_id: Some(AgentId::new("agent-a").expect("agent")),
            project_id: Some(ProjectId::new("project-a").expect("project")),
            mission_id: None,
            thread_id: Some(ThreadId::new("thread-a").expect("thread")),
            invocation_id: ironclaw_host_api::InvocationId::new(),
        }
    }

    /// Build a `HookFailed` lifecycle event carrying an optional provider claim
    /// and an optional hook_id anchor.
    fn lifecycle_event(provider: Option<&str>, hook_id_hex: Option<&str>) -> RuntimeEvent {
        let mut event = RuntimeEvent::hook_failed(
            scope(),
            CapabilityId::new("test.cap").expect("cap"),
            // a placeholder hook-id-hex on the payload; overwritten below
            hook_id_hex.unwrap_or("00"),
            "panic",
            "test_failure",
            provider.map(ext),
        );
        // `hook_failed` always sets hook_id; clear it for the missing-anchor case.
        if hook_id_hex.is_none() {
            event.hook_id = None;
        }
        event
    }

    /// A non-lifecycle event used to exercise the provider-passthrough branch.
    fn non_lifecycle_event(provider: Option<&str>) -> RuntimeEvent {
        let mut event = lifecycle_event(provider, Some("aa"));
        event.kind = RuntimeEventKind::DispatchRequested;
        event
    }

    // probe sentinels --------------------------------------------------------

    fn probe_owner(owner: &'static str) -> impl FnOnce(&str) -> LifecycleOwnerLookup {
        move |_| LifecycleOwnerLookup::Owner(ext(owner))
    }
    fn probe_unknown(_: &str) -> LifecycleOwnerLookup {
        LifecycleOwnerLookup::Unknown
    }
    fn probe_poisoned(_: &str) -> LifecycleOwnerLookup {
        LifecycleOwnerLookup::Poisoned
    }
    fn probe_unreached(_: &str) -> LifecycleOwnerLookup {
        panic!("probe must not be consulted for this case");
    }

    #[test]
    fn is_lifecycle_kind_classifies_every_variant() {
        // Every `RuntimeEventKind` is listed here explicitly. If a variant is
        // added to the enum, this test must be updated alongside the exhaustive
        // `match` in `is_lifecycle_kind` — the goal is that a new lifecycle
        // variant is never silently treated as non-lifecycle (which would trust
        // the forgeable `event.provider`).
        let lifecycle = [
            RuntimeEventKind::HookDispatched,
            RuntimeEventKind::HookDecisionEmitted,
            RuntimeEventKind::HookFailed,
        ];
        let non_lifecycle = [
            RuntimeEventKind::DispatchRequested,
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchSucceeded,
            RuntimeEventKind::DispatchFailed,
            RuntimeEventKind::CapabilityActivityRequested,
            RuntimeEventKind::CapabilityActivitySucceeded,
            RuntimeEventKind::CapabilityActivityFailed,
            RuntimeEventKind::ModelStarted,
            RuntimeEventKind::ModelCompleted,
            RuntimeEventKind::ModelFailed,
            RuntimeEventKind::AssistantReplyFinalized,
            RuntimeEventKind::LoopCompleted,
            RuntimeEventKind::LoopCancelled,
            RuntimeEventKind::LoopFailed,
            RuntimeEventKind::ProcessStarted,
            RuntimeEventKind::ProcessCompleted,
            RuntimeEventKind::ProcessFailed,
            RuntimeEventKind::ProcessKilled,
        ];
        for kind in lifecycle {
            assert!(
                is_lifecycle_kind(kind),
                "{kind:?} must be classified as a hook-lifecycle kind"
            );
        }
        for kind in non_lifecycle {
            assert!(
                !is_lifecycle_kind(kind),
                "{kind:?} must NOT be classified as a hook-lifecycle kind"
            );
        }
    }

    #[test]
    fn non_lifecycle_event_trusts_carried_provider() {
        let event = non_lifecycle_event(Some("ext-host"));
        // Non-lifecycle: provider is used as-is, registry never probed.
        let owner = resolve_event_owner(&event, probe_unreached);
        assert_eq!(owner, Some(ext("ext-host")));
    }

    #[test]
    fn lifecycle_registered_owner_is_used() {
        let event = lifecycle_event(Some("ext-claimed"), Some("aa"));
        let owner = resolve_event_owner(&event, probe_owner("ext-real"));
        // Registry hit wins; the matching/mismatching claim does not change it.
        assert_eq!(owner, Some(ext("ext-real")));
    }

    #[test]
    fn lifecycle_spoofed_provider_is_overridden_by_resolved_owner() {
        // Provider claims ext-attacker; registry says ext-victim owns the hook.
        let event = lifecycle_event(Some("ext-attacker"), Some("aa"));
        let owner = resolve_event_owner(&event, probe_owner("ext-victim"));
        assert_eq!(
            owner,
            Some(ext("ext-victim")),
            "the forgeable provider claim must never override the resolved owner"
        );
    }

    #[test]
    fn lifecycle_unknown_hook_id_fails_closed() {
        // Synthesized event: unknown hook_id + a provider claim. Must NOT
        // activate the claimed extension's hooks.
        let event = lifecycle_event(Some("ext-attacker"), Some("deadbeef"));
        let owner = resolve_event_owner(&event, probe_unknown);
        assert_eq!(owner, None);
    }

    #[test]
    fn lifecycle_poisoned_registry_fails_closed() {
        let event = lifecycle_event(Some("ext-attacker"), Some("aa"));
        let owner = resolve_event_owner(&event, probe_poisoned);
        assert_eq!(owner, None);
    }

    #[test]
    fn lifecycle_missing_hook_id_fails_closed_without_probing() {
        // No hook_id anchor: cannot resolve an owner; provider claim ignored.
        let event = lifecycle_event(Some("ext-attacker"), None);
        let owner = resolve_event_owner(&event, probe_unreached);
        assert_eq!(owner, None);
    }
}
