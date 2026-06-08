// Pure decision logic for the first-run onboarding gate in
// `layout/gateway-layout.js`. Extracted as dependency-free functions so
// the routing decision can be unit-tested without a React renderer or
// react-query (see `onboarding-gate.test.js`).

// The operator LLM-config routes (`/api/webchat/v2/llm/*`) are
// intentionally unmounted for multi-user / SSO authenticators — the
// gateway returns 404 until an admin-role boundary exists (see
// `ironclaw_reborn_composition` `allows_operator_*_config`). In that mode
// the active provider is configured operator-side at boot
// (`config.toml [llm.default]` / `LLM_*` env), NOT through this UI. So a
// 404 from the providers route means "not surfaced via this UI", not "no
// LLM configured", and must NOT be treated as a reason to onboard.
export function isProviderConfigRouteUnavailable(error) {
  return error?.status === 404;
}

// Whether to force the first-run onboarding redirect. We only redirect
// once the providers query has settled (`!isLoading`), there is no active
// provider, AND the config route is actually reachable. When the route is
// gated (404), the `/welcome` flow can't configure anything anyway and the
// backend already has a boot-configured provider — redirecting there would
// trap the user on a dead end.
export function shouldRouteToOnboarding({
  isLoading,
  hasActiveProvider,
  providerConfigUnavailable,
}) {
  return !isLoading && !hasActiveProvider && !providerConfigUnavailable;
}
