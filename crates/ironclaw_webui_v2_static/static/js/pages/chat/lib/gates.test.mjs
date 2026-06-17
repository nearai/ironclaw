import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function loadGates() {
  const source = readFileSync(new URL("./gates.js", import.meta.url), "utf8")
    .replace("export function gateFromEvent", "function gateFromEvent");
  const context = { globalThis: {} };
  vm.runInNewContext(
    `${source}\nglobalThis.__testExports = { gateFromEvent };`,
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
      runId: "run-1",
      gateRef: "gate:approval",
      headline: "Approval required",
      body: "Review the action.",
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
      runId: "run-1",
      gateRef: "gate:resource",
      headline: "Resource unavailable",
      body: "Try later.",
      allowAlways: false,
    },
  );
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
  assert.match(gate.parameters, /Estimated network egress: 4096 bytes/);
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
      challengeKind: "other",
      runId: "run-auth",
      gateRef: "gate:auth",
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
