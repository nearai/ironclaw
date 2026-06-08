// Pure decision logic for the first-run onboarding gate in
// `layout/gateway-layout.js`. Extracted as a dependency-free function so
// the routing decision can be unit-tested without a React renderer or
// react-query (see `onboarding-gate.test.js`).

// Whether to force the first-run onboarding redirect. We only redirect
// once the providers query has settled (`!isLoading`), there is no active
// provider, AND the query did not error.
//
// `isError` is what unblocks the multi-user / SSO build: the operator
// LLM-config routes (`/api/webchat/v2/llm/*`) are intentionally unmounted
// for multi-user authenticators (the gateway 404s until an admin-role
// boundary exists — see `ironclaw_reborn_composition`
// `allows_operator_*_config`), so the providers query fails. More
// generally, a failed query (404 gated, transient 5xx, offline) tells us
// nothing reliable about whether a provider is configured — and the
// provider may already be set operator-side at boot
// (`config.toml [llm.default]` / `LLM_*` env), which the `/welcome` flow
// can't configure anyway. So on any query error we must not trap the user
// on onboarding.
export function shouldRouteToOnboarding({ isLoading, hasActiveProvider, isError }) {
  return !isLoading && !hasActiveProvider && !isError;
}
