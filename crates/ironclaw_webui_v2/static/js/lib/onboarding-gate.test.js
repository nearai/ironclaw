// Unit tests for the first-run onboarding gate decision.
//
// Run with Node's built-in test runner (no extra deps):
//   node --test crates/ironclaw_webui_v2/static/js/lib/onboarding-gate.test.js
//
// NOTE: `build.rs` deliberately excludes `*.test.js` from the embedded
// asset bundle, so this file is never served to the browser.

import assert from "node:assert/strict";
import { test } from "node:test";
import { shouldRouteToOnboarding } from "./onboarding-gate.js";

test("a failed providers query (e.g. gated 404 under SSO) does NOT force onboarding", () => {
  // The regression this fix exists for: SSO users with a boot-configured
  // provider were trapped on /welcome because /llm/providers 404s and the
  // gatewayStatus.llm_backend stub is null, so hasActiveProvider is false.
  // A failed query can't prove "no provider", so we must not onboard.
  assert.equal(
    shouldRouteToOnboarding({
      isLoading: false,
      hasActiveProvider: false,
      isError: true,
    }),
    false,
    "must not redirect to onboarding when the providers query errored"
  );
});

test("no active provider on a successful query DOES force onboarding", () => {
  // env-bearer / single-operator: route is mounted, query succeeds, and
  // there is genuinely no provider configured → first-run onboarding is the
  // correct destination.
  assert.equal(
    shouldRouteToOnboarding({
      isLoading: false,
      hasActiveProvider: false,
      isError: false,
    }),
    true
  );
});

test("an active provider never forces onboarding", () => {
  assert.equal(
    shouldRouteToOnboarding({
      isLoading: false,
      hasActiveProvider: true,
      isError: false,
    }),
    false
  );
});

test("onboarding is deferred while the providers query is still loading", () => {
  assert.equal(
    shouldRouteToOnboarding({
      isLoading: true,
      hasActiveProvider: false,
      isError: false,
    }),
    false,
    "must wait for the query to settle before redirecting"
  );
});

test("loading is still deferred even when the query has errored", () => {
  // Pins the short-circuit: a future reorder of the `&&` chain must not let
  // an in-flight retry (isLoading true) slip through to a redirect decision.
  assert.equal(
    shouldRouteToOnboarding({
      isLoading: true,
      hasActiveProvider: false,
      isError: true,
    }),
    false
  );
});

test("an omitted isError key is treated as no error", () => {
  // Makes the implicit `!undefined === true` contract explicit: a caller
  // that forgets to pass `isError` falls back to the reachable-route path.
  assert.equal(
    shouldRouteToOnboarding({ isLoading: false, hasActiveProvider: false }),
    true
  );
});
