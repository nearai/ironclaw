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
