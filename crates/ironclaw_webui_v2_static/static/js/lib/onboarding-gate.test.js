// Unit tests for the first-run onboarding gate decision.
//
// Run with Node's built-in test runner (no extra deps):
//   node --test crates/ironclaw_webui_v2_static/static/js/lib/
//
// NOTE: `build.rs` deliberately excludes `*.test.js` from the embedded
// asset bundle, so this file is never served to the browser.

import assert from "node:assert/strict";
import { test } from "node:test";
import {
  isProviderConfigRouteUnavailable,
  shouldRouteToOnboarding,
} from "./onboarding-gate.js";

test("404 from the providers route is treated as config-route-unavailable", () => {
  assert.equal(isProviderConfigRouteUnavailable({ status: 404 }), true);
});

test("non-404 errors are not config-route-unavailable", () => {
  assert.equal(isProviderConfigRouteUnavailable({ status: 401 }), false);
  assert.equal(isProviderConfigRouteUnavailable({ status: 500 }), false);
  assert.equal(isProviderConfigRouteUnavailable(undefined), false);
  assert.equal(isProviderConfigRouteUnavailable(null), false);
});

test("a gated providers route (404) does NOT force onboarding", () => {
  // The regression this fix exists for: SSO users with a boot-configured
  // provider were trapped on /welcome because /llm/providers 404s and the
  // gatewayStatus.llm_backend stub is null, so hasActiveProvider is false.
  assert.equal(
    shouldRouteToOnboarding({
      isLoading: false,
      hasActiveProvider: false,
      providerConfigUnavailable: true,
    }),
    false,
    "must not redirect to onboarding when the config route is gated"
  );
});

test("no active provider on a reachable route DOES force onboarding", () => {
  // env-bearer / single-operator: route is mounted, genuinely no provider
  // configured yet → first-run onboarding is the correct destination.
  assert.equal(
    shouldRouteToOnboarding({
      isLoading: false,
      hasActiveProvider: false,
      providerConfigUnavailable: false,
    }),
    true
  );
});

test("an active provider never forces onboarding", () => {
  assert.equal(
    shouldRouteToOnboarding({
      isLoading: false,
      hasActiveProvider: true,
      providerConfigUnavailable: false,
    }),
    false
  );
});

test("onboarding is deferred while the providers query is still loading", () => {
  assert.equal(
    shouldRouteToOnboarding({
      isLoading: true,
      hasActiveProvider: false,
      providerConfigUnavailable: false,
    }),
    false,
    "must wait for the query to settle before redirecting"
  );
});
