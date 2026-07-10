import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

import { GATE_KIND } from "./gate-kinds.js";

function loadGates() {
  // Strip ES `import` lines (the vm context has no module loader) and
  // inject the imported symbols as context globals — gates.js imports
  // `GATE_KIND` from ./gate-kinds.js.
  const source = readFileSync(new URL("./gates.js", import.meta.url), "utf8")
    .split("\n")
    .filter((line) => !line.startsWith("import "))
    .join("\n")
    .replace(
      /export function (gateFromEvent|gateFromProjectionGate|gateDisplayParameters)/g,
      "function $1",
    );
  const context = { globalThis: {}, GATE_KIND };
  vm.runInNewContext(
    `${source}\nglobalThis.__testExports = { gateFromEvent, gateFromProjectionGate, gateDisplayParameters };`,
    context,
  );
  return context.globalThis.__testExports;
}

function plain(value) {
  return JSON.parse(JSON.stringify(value));
}

test("gateFromEvent maps approval always-allow affordance", () => {
  const { gateFromEvent } = loadGates();

  assert.deepEqual(
    plain(gateFromEvent("gate", {
      turn_run_id: "run-1",
      gate_ref: "gate:approval",
      headline: "Approval required",
      body: "Review the action.",
      allow_always: true,
    })),
    {
      kind: "gate",
      gateKind: "approval",
      runId: "run-1",
      gateRef: "gate:approval",
      invocationId: null,
      headline: "Approval required",
      body: "Review the action.",
      description: "Review the action.",
      allowAlways: true,
    },
  );
});

test("gateFromEvent defaults missing always-allow affordance to false", () => {
  const { gateFromEvent } = loadGates();

  assert.deepEqual(
    plain(gateFromEvent("gate", {
      turn_run_id: "run-1",
      gate_ref: "gate:resource",
      headline: "Resource unavailable",
      body: "Try later.",
    })),
    {
      kind: "gate",
      gateKind: "approval",
      runId: "run-1",
      gateRef: "gate:resource",
      invocationId: null,
      headline: "Resource unavailable",
      body: "Try later.",
      description: "Try later.",
      allowAlways: false,
    },
  );
});

test("gateFromEvent keeps a readable approval description when context lookup is missing", () => {
  const { gateFromEvent } = loadGates();

  const gate = plain(gateFromEvent("gate", {
    turn_run_id: "run-1",
    gate_ref: "gate:approval-1",
    headline: "Approval required",
    body: "capability requires approval",
    allow_always: true,
  }));

  assert.equal(gate.description, "capability requires approval");
  assert.equal(gate.toolName, undefined);
});

test("gateFromEvent maps approval context into readable approval card props", () => {
  const { gateFromEvent } = loadGates();

  const gate = plain(gateFromEvent("gate", {
    turn_run_id: "run-1",
    gate_ref: "gate:approval-1",
    headline: "Approval required",
    body: "capability requires approval",
    allow_always: true,
    approval_context: {
      tool_name: "builtin.http",
      action: { label: "Run tool" },
      scope: { label: "This request only", reusable: false },
      reason: "approval required for Dispatch of builtin.http",
      destination: {
        label: "GET https://example.com",
        url: "https://example.com",
        domain: "example.com",
      },
      details: [
        { label: "Capability", value: "builtin.http" },
        { label: "Estimated network egress", value: "4096 bytes" },
      ],
    },
  }));

  assert.equal(gate.allowAlways, true);
  assert.equal(gate.toolName, "builtin.http");
  assert.equal(gate.description, "approval required for Dispatch of builtin.http");
  assert.equal(gate.destination.domain, "example.com");
  assert.deepEqual(gate.approvalScope, {
    label: "This request only",
    reusable: false,
  });
  assert.deepEqual(gate.approvalDetails, [
    { label: "Action", value: "Run tool" },
    { label: "Destination", value: "GET https://example.com" },
    { label: "Scope", value: "This request only" },
    { label: "Capability", value: "builtin.http" },
    { label: "Estimated network egress", value: "4096 bytes" },
  ]);
  assert.equal(gate.parameters, null);
});

test("gateFromEvent merges top-level details with approval context details", () => {
  const { gateFromEvent } = loadGates();

  // The event carries top-level `details` AND an approval_context. Both
  // sources must reach the approval card; the top-level rows must not be
  // dropped when an approval context is present.
  const gate = plain(gateFromEvent("gate", {
    turn_run_id: "run-1",
    gate_ref: "gate:approval-1",
    headline: "Approval required",
    allow_always: false,
    details: [{ label: "Estimated cost", value: "$0.02" }],
    approval_context: {
      tool_name: "builtin.http",
      action: { label: "Run tool" },
      scope: { label: "This request only", reusable: false },
      reason: "approval required",
    },
  }));

  assert.deepEqual(gate.approvalDetails, [
    { label: "Action", value: "Run tool" },
    { label: "Scope", value: "This request only" },
    { label: "Estimated cost", value: "$0.02" },
  ]);
});

test("gateFromProjectionGate maps approval context from durable projection", () => {
  const { gateFromProjectionGate } = loadGates();

  const gate = plain(gateFromProjectionGate({
    run_id: "run-1",
    gate_kind: "approval",
    gate_ref: "gate:approval-1",
    invocation_id: "invocation-1",
    headline: "Approval required",
    body: "capability requires approval",
    allow_always: true,
    approval_context: {
      tool_name: "builtin.http",
      action: { label: "Network request" },
      scope: { label: "This request only", reusable: false },
      details: [{ label: "Secret", value: "<redacted>" }],
    },
  }));

  assert.equal(gate.toolName, "builtin.http");
  assert.equal(gate.description, "capability requires approval");
  assert.deepEqual(gate.approvalDetails, [
    { label: "Action", value: "Network request" },
    { label: "Scope", value: "This request only" },
    { label: "Secret", value: "<redacted>" },
  ]);
  assert.equal(gate.parameters, null);
});

test("gateFromEvent keeps modern auth prompts without challenge kind off token card", () => {
  const { gateFromEvent } = loadGates();

  assert.deepEqual(
    plain(gateFromEvent("auth_required", {
      turn_run_id: "run-auth",
      auth_request_ref: "gate:auth",
      headline: "Authentication required",
      body: "Google authentication required",
      provider: "google",
    })),
    {
      kind: "auth_required",
      gateKind: "auth",
      challengeKind: "other",
      runId: "run-auth",
      gateRef: "gate:auth",
      invocationId: null,
      provider: "google",
      accountLabel: "",
      authorizationUrl: null,
      expiresAt: null,
      headline: "Authentication required",
      body: "Google authentication required",
    },
  );
});

test("gateFromEvent preserves explicit oauth prompts without authorization URL", () => {
  const { gateFromEvent } = loadGates();

  assert.deepEqual(
    plain(gateFromEvent("auth_required", {
      turn_run_id: "run-auth",
      auth_request_ref: "gate:auth",
      headline: "Authentication required",
      body: "Google authentication required",
      challenge_kind: "oauth_url",
      provider: "google",
    })),
    {
      kind: "auth_required",
      gateKind: "auth",
      challengeKind: "oauth_url",
      runId: "run-auth",
      gateRef: "gate:auth",
      invocationId: null,
      provider: "google",
      accountLabel: "",
      authorizationUrl: null,
      expiresAt: null,
      headline: "Authentication required",
      body: "Google authentication required",
    },
  );
});

test("gateFromEvent preserves legacy auth prompts as manual token prompts", () => {
  const { gateFromEvent } = loadGates();

  assert.equal(
    gateFromEvent("auth_required", {
      turn_run_id: "run-auth",
      auth_request_ref: "gate:auth",
    }).challengeKind,
    "manual_token",
  );
});

test("gateDisplayParameters joins approval details as label: value lines", () => {
  const { gateDisplayParameters } = loadGates();

  assert.equal(
    gateDisplayParameters({
      approvalDetails: [
        { label: "Method", value: "GET" },
        { label: "Destination", value: "https://example.com" },
      ],
    }),
    "Method: GET\nDestination: https://example.com",
  );
});

test("gateDisplayParameters prefers an explicit parameters string", () => {
  const { gateDisplayParameters } = loadGates();

  assert.equal(
    gateDisplayParameters({
      parameters: "query: deploy status",
      approvalDetails: [{ label: "Method", value: "GET" }],
    }),
    "query: deploy status",
  );
});

test("gateDisplayParameters returns null when there is nothing to show", () => {
  const { gateDisplayParameters } = loadGates();

  assert.equal(gateDisplayParameters({ approvalDetails: [] }), null);
  assert.equal(gateDisplayParameters(null), null);
});
