//! Scope-routing gateway for Reborn group integration tests.
//!
//! `ScopeRegistryGateway` multiplexes model calls to per-thread scripted
//! gateways by `TurnScope`. The loop-driver host calls
//! `resolve_for_scope(&scope)` at host-construction time (not on the model
//! hot path) and wraps the returned gateway in
//! `ThreadResolvingLoopModelGateway`. The registry's own `stream_model` is
//! therefore **never called** when routing succeeds; it exists only to satisfy
//! the trait contract and fails **loudly** so a routing miss (unregistered
//! scope) surfaces as `ConfigurationError` rather than masking as the original
//! flake.
//!
//! ## Invariant
//! This dispatcher sits at the `HostManagedModelGateway` seam but routes to
//! REAL `LlmProviderModelGateway` instances over the `ironclaw_llm` decorator
//! chain. The single-fake-at-the-vendor-SDK-seam invariant (CLAUDE.md §5–8,
//! §28) is preserved: the only fake is the `TraceLlm` at the bottom of each
//! registered gateway's chain, not this dispatcher.

// Shared by all group test binaries; symbols read as dead when a binary
// does not exercise every variant.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_loop_support::{
    HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
    HostManagedModelRequest, HostManagedModelResponse,
};
use ironclaw_turns::TurnScope;

/// Scope-keyed gateway registry for Reborn group integration tests.
///
/// Call [`register`](Self::register) (with `&self`) before submitting any
/// turns; the loop-driver calls
/// [`resolve_for_scope`][HostManagedModelGateway::resolve_for_scope] at
/// host-construction time to obtain the per-thread scripted gateway.
///
/// The registry's own `stream_model` is a deliberate sentinel: it always
/// returns [`HostManagedModelErrorKind::ConfigurationError`] so a routing
/// miss fails legibly and cannot be confused with `TraceLlm` deque exhaustion
/// (which surfaces as `Unavailable`) or `driver_protocol_violation`.
#[derive(Default)]
pub struct ScopeRegistryGateway {
    map: Mutex<HashMap<TurnScope, Arc<dyn HostManagedModelGateway>>>,
}

impl ScopeRegistryGateway {
    /// Construct an empty registry.
    pub fn new() -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
        }
    }

    /// Register `gateway` for `scope`.
    ///
    /// Interior-mutable (`&self`) so callers can hold an
    /// `Arc<ScopeRegistryGateway>` and register threads one-by-one before any
    /// turn is submitted. This is called from `.thread(conv).script().build()`
    /// before any turn reaches the scheduler.
    pub fn register(&self, scope: TurnScope, gateway: Arc<dyn HostManagedModelGateway>) {
        self.map
            .lock()
            .expect("ScopeRegistryGateway map lock poisoned")
            .insert(scope, gateway);
    }
}

#[async_trait]
impl HostManagedModelGateway for ScopeRegistryGateway {
    /// Sentinel — never reached when routing succeeds.
    ///
    /// The loop-driver host resolves the per-scope gateway via
    /// [`resolve_for_scope`](Self::resolve_for_scope) and calls `stream_model`
    /// on **that** gateway. If execution reaches here it means a scope was not
    /// registered — failing with `ConfigurationError` makes the miss impossible
    /// to confuse with model exhaustion (`Unavailable`) or
    /// `driver_protocol_violation`.
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::ConfigurationError,
            format!(
                "ScopeRegistryGateway: stream_model called directly \
                 (run_id={:?}, turn_id={:?}); \
                 this gateway only routes via resolve_for_scope — \
                 no per-scope gateway was registered for the active scope. \
                 Register every thread scope before submitting turns.",
                request.run_id, request.turn_id,
            ),
        ))
    }

    /// Return the gateway registered for `scope`, or `None` if no match.
    ///
    /// The loop-driver host calls this at host-construction time (off the
    /// model hot path). `None` → the host falls back to `Arc::clone(self)`,
    /// causing the next `stream_model` call to emit the sentinel error above,
    /// making the routing miss immediately visible.
    fn resolve_for_scope(&self, scope: &TurnScope) -> Option<Arc<dyn HostManagedModelGateway>> {
        self.map
            .lock()
            .expect("ScopeRegistryGateway map lock poisoned")
            .get(scope)
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trivial gateway stub: another `ScopeRegistryGateway` satisfies the trait
    /// bound and lets us check `Arc::ptr_eq` identity without touching any model
    /// logic.
    fn stub_gateway() -> Arc<dyn HostManagedModelGateway> {
        Arc::new(ScopeRegistryGateway::new())
    }

    fn make_scope(thread_id: &str) -> TurnScope {
        TurnScope::new(
            ironclaw_host_api::TenantId::from_trusted("tenant:test".to_string()),
            None,
            None,
            ironclaw_host_api::ThreadId::from_trusted(thread_id.to_string()),
        )
    }

    /// Mutation guard (a): if `resolve_for_scope` is changed to always return
    /// `None`, this test goes RED on the `is_some()` assertion.
    /// Mutation guard (b): if it returns the wrong entry or ignores scope,
    /// `two_scopes_route_to_distinct_gateways` goes RED on `ptr_eq`.
    #[test]
    fn registered_scope_returns_some() {
        let registry = ScopeRegistryGateway::new();
        let scope = make_scope("thread:a");
        let gw = stub_gateway();

        registry.register(scope.clone(), Arc::clone(&gw));

        let resolved = registry.resolve_for_scope(&scope);
        assert!(resolved.is_some(), "registered scope must resolve to Some");
        assert!(
            Arc::ptr_eq(&gw, resolved.as_ref().unwrap()),
            "resolved gateway must be the exact Arc that was registered"
        );
    }

    #[test]
    fn unregistered_scope_returns_none() {
        let registry = ScopeRegistryGateway::new();
        let scope_a = make_scope("thread:a");
        let scope_b = make_scope("thread:b");
        let gw = stub_gateway();

        registry.register(scope_a, Arc::clone(&gw));

        let resolved = registry.resolve_for_scope(&scope_b);
        assert!(resolved.is_none(), "unregistered scope must return None");
    }

    #[test]
    fn two_scopes_route_to_distinct_gateways() {
        let registry = ScopeRegistryGateway::new();
        let scope_a = make_scope("thread:a");
        let scope_b = make_scope("thread:b");
        let gw_a = stub_gateway();
        let gw_b = stub_gateway();

        registry.register(scope_a.clone(), Arc::clone(&gw_a));
        registry.register(scope_b.clone(), Arc::clone(&gw_b));

        let resolved_a = registry
            .resolve_for_scope(&scope_a)
            .expect("scope_a must resolve");
        let resolved_b = registry
            .resolve_for_scope(&scope_b)
            .expect("scope_b must resolve");

        assert!(
            Arc::ptr_eq(&gw_a, &resolved_a),
            "scope_a must resolve to gw_a"
        );
        assert!(
            Arc::ptr_eq(&gw_b, &resolved_b),
            "scope_b must resolve to gw_b"
        );
        assert!(
            !Arc::ptr_eq(&resolved_a, &resolved_b),
            "two distinct scopes must NOT resolve to the same gateway"
        );
    }
}
