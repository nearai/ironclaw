//! Scope-routing gateway for Reborn group integration tests: routes model calls to
//! per-thread scripted gateways by `TurnScope` via `resolve_for_scope`, called at
//! host-construction time (off the model hot path). Its own `stream_model` is
//! unreachable on success — fails loudly with `ConfigurationError` on an unregistered
//! scope, preserving the single-fake-at-vendor-SDK-seam invariant (CLAUDE.md §5-8, §28):
//! the only fake is the `TraceLlm` at the bottom of each registered gateway's chain.

// Shared by all group test binaries; symbols read as dead when a binary
// does not exercise every variant.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_loop_host::{
    HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
    HostManagedModelRequest, HostManagedModelResponse,
};
use ironclaw_turns::TurnScope;

/// Scope-keyed gateway registry for Reborn group integration tests.
///
/// `stream_model` is a sentinel: always returns `ConfigurationError` on a routing miss so
/// it can't be confused with `TraceLlm` exhaustion (`Unavailable`) or `driver_protocol_violation`.
#[derive(Default)]
pub struct ScopeRegistryGateway {
    map: Mutex<HashMap<TurnScope, Arc<dyn HostManagedModelGateway>>>,
    /// Thread-id-prefix fallbacks, consulted only when the exact map misses.
    ///
    /// For runs whose thread id a test cannot know in advance because it is
    /// derived from the run that triggers them — a background pass keyed on its
    /// triggering run id, say. The exact map stays authoritative, so an
    /// explicitly registered scope is never shadowed by a prefix.
    prefixes: Mutex<Vec<(String, Arc<dyn HostManagedModelGateway>)>>,
}

impl ScopeRegistryGateway {
    pub fn new() -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
            prefixes: Mutex::new(Vec::new()),
        }
    }

    /// Register `gateway` for every scope whose thread id starts with
    /// `thread_id_prefix`. Same `&self` reason as `register` below.
    pub fn register_prefix(
        &self,
        thread_id_prefix: impl Into<String>,
        gateway: Arc<dyn HostManagedModelGateway>,
    ) {
        self.prefixes
            .lock()
            .expect("ScopeRegistryGateway prefix lock poisoned")
            .push((thread_id_prefix.into(), gateway));
    }

    /// Register `gateway` for `scope`. `&self` (not `&mut self`) so callers holding an
    /// `Arc<ScopeRegistryGateway>` can register threads one-by-one before any turn is submitted.
    pub fn register(&self, scope: TurnScope, gateway: Arc<dyn HostManagedModelGateway>) {
        let replaced = self
            .map
            .lock()
            .expect("ScopeRegistryGateway map lock poisoned")
            .insert(scope, gateway);
        assert!(
            replaced.is_none(),
            "duplicate scope registration in ScopeRegistryGateway"
        );
    }
}

#[async_trait]
impl HostManagedModelGateway for ScopeRegistryGateway {
    /// Sentinel — never reached when routing succeeds; execution here means a scope was not
    /// registered. Fails with `ConfigurationError`, distinct from `Unavailable` (model
    /// exhaustion) or `driver_protocol_violation`.
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

    /// Returns the gateway registered for `scope`, or `None`. Called at host-construction
    /// time (off the model hot path); `None` makes the host fall back to `Arc::clone(self)`,
    /// so the next `stream_model` call emits the sentinel error above.
    fn resolve_for_scope(&self, scope: &TurnScope) -> Option<Arc<dyn HostManagedModelGateway>> {
        if let Some(gateway) = self
            .map
            .lock()
            .expect("ScopeRegistryGateway map lock poisoned")
            .get(scope)
            .cloned()
        {
            return Some(gateway);
        }
        self.prefixes
            .lock()
            .expect("ScopeRegistryGateway prefix lock poisoned")
            .iter()
            .find(|(prefix, _)| scope.thread_id.as_str().starts_with(prefix.as_str()))
            .map(|(_, gateway)| Arc::clone(gateway))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stub gateway (reuses `ScopeRegistryGateway`) for `Arc::ptr_eq` identity checks.
    fn stub_gateway() -> Arc<dyn HostManagedModelGateway> {
        Arc::new(ScopeRegistryGateway::new())
    }

    fn make_scope(thread_id: &str) -> TurnScope {
        TurnScope::new(
            ironclaw_host_api::ids::TenantId::from_trusted("tenant:test".to_string()),
            None,
            None,
            ironclaw_host_api::ids::ThreadId::from_trusted(thread_id.to_string()),
        )
    }

    /// Mutation guards: always-None `resolve_for_scope` fails `is_some()` here; wrong-entry
    /// resolution fails `ptr_eq` in `two_scopes_route_to_distinct_gateways`.
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

    /// A run whose thread id a test cannot know in advance still needs a
    /// scripted model. The prefix fallback is how it gets one — and the exact
    /// map must keep winning, so a prefix can never shadow a thread a test
    /// registered by hand.
    #[test]
    fn a_prefix_matches_unregistered_scopes_without_shadowing_exact_ones() {
        let registry = ScopeRegistryGateway::new();
        let exact_scope = make_scope("pass-abc");
        let exact = stub_gateway();
        let by_prefix = stub_gateway();

        registry.register(exact_scope.clone(), Arc::clone(&exact));
        registry.register_prefix("pass-", Arc::clone(&by_prefix));

        let resolved = registry
            .resolve_for_scope(&make_scope("pass-xyz"))
            .expect("a prefix match resolves");
        assert!(
            Arc::ptr_eq(&by_prefix, &resolved),
            "an unregistered scope under the prefix routes to the prefix gateway"
        );

        let resolved = registry
            .resolve_for_scope(&exact_scope)
            .expect("the exact registration still resolves");
        assert!(
            Arc::ptr_eq(&exact, &resolved),
            "an exactly registered scope must never be shadowed by a prefix"
        );

        assert!(
            registry
                .resolve_for_scope(&make_scope("other-xyz"))
                .is_none(),
            "a scope outside the prefix stays unregistered"
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

    /// Fail-loud guard: re-registering a scope must panic, not silently repoint the first
    /// registration's callers at the second gateway.
    #[test]
    #[should_panic(expected = "duplicate scope registration")]
    fn duplicate_register_for_same_scope_panics() {
        let registry = ScopeRegistryGateway::new();
        let scope = make_scope("thread:a");
        let gw_first = stub_gateway();
        let gw_second = stub_gateway();

        registry.register(scope.clone(), gw_first);
        registry.register(scope, gw_second);
    }

    /// Minimal request for exercising the `stream_model` sentinel; only `run_id`/`turn_id`
    /// matter (the sentinel only echoes them into the error message).
    fn make_request() -> HostManagedModelRequest {
        HostManagedModelRequest {
            model_profile_id: ironclaw_loop_contracts::ModelProfileId::new("interactive_model")
                .expect("valid model profile id"),
            fallback_index: 0,
            messages: Vec::new(),
            surface_version: None,
            resolved_model_route: None,
            run_id: ironclaw_turns::TurnRunId::new(),
            turn_id: ironclaw_turns::TurnId::new(),
            tool_choice: None,
            response_format: None,
        }
    }

    /// De-mask guard: a routing miss must fail as the distinct `ConfigurationError` sentinel,
    /// not resemble model exhaustion (`Unavailable`) or `driver_protocol_violation`.
    #[tokio::test]
    async fn stream_model_sentinel_reports_configuration_error_on_routing_miss() {
        let registry = ScopeRegistryGateway::new();

        let error = registry
            .stream_model(make_request())
            .await
            .expect_err("stream_model on an unrouted registry must fail");

        assert_eq!(
            error.kind,
            HostManagedModelErrorKind::ConfigurationError,
            "routing-miss sentinel must report ConfigurationError, not be confusable \
             with model exhaustion (Unavailable) or driver_protocol_violation"
        );
        assert!(
            error
                .safe_summary
                .contains("ScopeRegistryGateway: stream_model called directly"),
            "sentinel error message must identify itself so a routing miss is \
             diagnosable, got: {:?}",
            error.safe_summary
        );
        assert!(
            error
                .safe_summary
                .contains("no per-scope gateway was registered"),
            "sentinel error message must explain the routing miss, got: {:?}",
            error.safe_summary
        );
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
