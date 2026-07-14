// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";
import vm from "node:vm";
import { productAuthOAuthEventsSource } from "./product-auth-oauth-events.vm-inline";

function libForTest(windowObject) {
  const context = {
    URL,
    window: windowObject,
    globalThis: {},
  };
  vm.runInNewContext(
    `${productAuthOAuthEventsSource()}\nglobalThis.__testExports = { openAuthPopup, completionMatchesFlow, failureMatchesFlow, completionMatchesGate };`,
    context,
  );
  return context.globalThis.__testExports;
}

test("openAuthPopup fresh-opens with the hardened argument set and tolerates the spec's noopener null", () => {
  const openCalls = [];
  const lib = libForTest({
    open: (url, target, features) => {
      openCalls.push({ url, target, features });
      return null;
    },
  });

  const result = lib.openAuthPopup("https://slack.com/oauth/v2/authorize");
  // Mock-hygiene rule: assert EVERY argument the production call passes —
  // losing "noopener" would hand the OAuth provider page window.opener access
  // to the authenticated SPA.
  assert.deepEqual(openCalls, [
    {
      url: "https://slack.com/oauth/v2/authorize",
      target: "_blank",
      features: "noopener,noreferrer",
    },
  ]);
  // Per spec, noopener opens return null even on success: not a failure.
  assert.equal(result.ok, true);
  assert.equal(result.popup, null);

  const insecure = lib.openAuthPopup("http://insecure.example/authorize");
  assert.equal(insecure.ok, false);
  assert.equal(insecure.reason, "insecure_url");
  assert.equal(openCalls.length, 1, "insecure URLs must never reach window.open");
});

test("openAuthPopup navigates a pre-opened placeholder popup instead of fresh-opening", () => {
  const openCalls = [];
  const popup = { closed: false, location: { href: "about:blank" } };
  const lib = libForTest({
    open: (...args) => {
      openCalls.push(args);
      return null;
    },
  });

  const result = lib.openAuthPopup("https://slack.com/oauth/v2/authorize", popup);
  assert.equal(result.ok, true);
  assert.equal(result.popup, popup);
  assert.equal(popup.location.href, "https://slack.com/oauth/v2/authorize");
  assert.equal(openCalls.length, 0);
});

test("flow matchers require type, status, and a flow-id match", () => {
  const lib = libForTest({});
  const completed = {
    type: "ironclaw:product-auth:oauth-complete",
    status: "completed",
    flowId: "flow-a",
  };
  assert.equal(lib.completionMatchesFlow(completed, "flow-a"), true);
  assert.equal(lib.completionMatchesFlow(completed, "flow-b"), false);
  assert.equal(lib.completionMatchesFlow({ ...completed, type: "other" }, "flow-a"), false);
  assert.equal(lib.completionMatchesFlow(completed, null), false);
  assert.equal(
    lib.completionMatchesFlow({ ...completed, flowId: undefined, flow_id: "flow-a" }, "flow-a"),
    true,
    "snake_case flow ids must keep matching",
  );

  const failed = { ...completed, status: "failed" };
  assert.equal(lib.failureMatchesFlow(failed, "flow-a"), true);
  assert.equal(lib.failureMatchesFlow(failed, "flow-b"), false);
  assert.equal(lib.failureMatchesFlow(completed, "flow-a"), false);
  assert.equal(lib.failureMatchesFlow({ ...failed, type: "other" }, "flow-a"), false);
});

test("completionMatchesGate refuses non-gate continuations instead of wildcarding", () => {
  const lib = libForTest({});
  const gate = { runId: "run-1", gateRef: "gate:auth" };
  const base = {
    type: "ironclaw:product-auth:oauth-complete",
    status: "completed",
    completedAt: 2000,
  };

  // A setup-only completion from ANOTHER extension's flow must not clear a
  // pending chat gate.
  assert.equal(
    lib.completionMatchesGate({ ...base, continuation: { type: "setup_only" } }, gate, 1000),
    false,
  );
  // A matching turn-gate continuation resolves the gate.
  assert.equal(
    lib.completionMatchesGate(
      {
        ...base,
        continuation: {
          type: "turn_gate_resume",
          turn_run_ref: "run-1",
          gate_ref: "gate:auth",
        },
      },
      gate,
      1000,
    ),
    true,
  );
  // A foreign gate's continuation does not.
  assert.equal(
    lib.completionMatchesGate(
      {
        ...base,
        continuation: {
          type: "turn_gate_resume",
          turn_run_ref: "run-2",
          gate_ref: "gate:auth",
        },
      },
      gate,
      1000,
    ),
    false,
  );
  // Legacy payloads with no continuation at all keep the timestamp fallback.
  assert.equal(lib.completionMatchesGate({ ...base }, gate, 1000), true);
  assert.equal(lib.completionMatchesGate({ ...base, completedAt: 500 }, gate, 1000), false);
});
