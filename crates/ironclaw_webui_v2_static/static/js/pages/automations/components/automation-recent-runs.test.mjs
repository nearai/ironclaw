import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

import { runSummaryView } from "../lib/automations-presenters.js";

const COPY = {
  "automations.table.noRuns": "No runs",
  "automations.runs.showingOf": "Showing {shown} of {total} recent runs",
  "automations.runs.total": "{count} runs",
  "automations.runs.ok": "{count} OK",
  "automations.runs.error": "{count} failed",
  "automations.runs.running": "{count} running",
  "automations.runs.unknown": "{count} unknown",
};

function sourceForTest() {
  const source = readFileSync(new URL("./automation-recent-runs.js", import.meta.url), "utf8");
  const lines = [];
  let skippingImport = false;
  for (const line of source.split("\n")) {
    if (!skippingImport && line.startsWith("import ")) {
      skippingImport = !line.trimEnd().endsWith(";");
      continue;
    }
    if (skippingImport) {
      skippingImport = !line.trimEnd().endsWith(";");
      continue;
    }
    lines.push(line.replace(/^export function /, "function "));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { RunDots, RunHistorySummary };`;
}

function html(strings, ...values) {
  return { strings: Array.from(strings), values };
}

function visit(node, fn) {
  if (Array.isArray(node)) {
    for (const item of node) visit(item, fn);
    return;
  }
  if (!node || typeof node !== "object") return;
  fn(node);
  if (Array.isArray(node.values)) {
    for (const value of node.values) visit(value, fn);
  }
}

function collectScalars(root) {
  const scalars = [];
  visit(root, (node) => {
    if (!Array.isArray(node.values)) return;
    for (const value of node.values) {
      if (typeof value === "string" || typeof value === "number" || typeof value === "boolean") {
        scalars.push(value);
      }
    }
  });
  return scalars;
}

function deepValuesAfter(root, fragment) {
  const values = [];
  visit(root, (node) => {
    if (!Array.isArray(node.strings) || !Array.isArray(node.values)) return;
    node.strings.forEach((part, index) => {
      if (part.includes(fragment)) values.push(node.values[index]);
    });
  });
  return values;
}

function t(key, vars = {}) {
  return (COPY[key] || key).replace(/\{(\w+)\}/g, (_, name) => String(vars[name] ?? ""));
}

function loadComponents() {
  function Button() {}
  function Icon() {}
  function StatusPill() {}
  const context = {
    globalThis: {},
    Button,
    Icon,
    StatusPill,
    buildScopedLogsPath: ({ threadId, runId }) => `/logs?thread=${threadId}&run=${runId}`,
    cn: (...parts) => parts.filter(Boolean).join(" "),
    html,
    runSummaryView,
    useT: () => t,
  };
  vm.runInNewContext(sourceForTest(), context);
  return context.globalThis.__testExports;
}

function runs(count, status = "ok") {
  return Array.from({ length: count }, (_, index) => ({
    run_id: `run-${index}`,
    status,
    status_label: status,
    fired_label: `fire-${index}`,
  }));
}

test("RunDots renders the empty state when there are no recent runs", () => {
  const { RunDots } = loadComponents();

  const rendered = RunDots({ runs: [] });

  assert.ok(collectScalars(rendered).includes("No runs"));
});

test("RunDots renders the empty state when recent runs are omitted or null", () => {
  const { RunDots } = loadComponents();

  assert.ok(collectScalars(RunDots({})).includes("No runs"));
  assert.ok(collectScalars(RunDots({ runs: null })).includes("No runs"));
});

test("RunDots renders exactly eight runs without an overflow chip", () => {
  const { RunDots } = loadComponents();

  const rendered = RunDots({ runs: runs(8) });

  assert.equal(deepValuesAfter(rendered, "aria-label=")[0], "Showing 8 of 8 recent runs");
  assert.equal(collectScalars(rendered).includes("+1"), false);
});

test("RunDots renders an overflow chip and caps very large hidden counts", () => {
  const { RunDots } = loadComponents();

  const tenRuns = RunDots({ runs: runs(10) });
  assert.equal(deepValuesAfter(tenRuns, "aria-label=")[0], "Showing 8 of 10 recent runs");
  assert.ok(collectScalars(tenRuns).includes("+2"));

  const manyRuns = RunDots({ runs: runs(1200) });
  assert.equal(deepValuesAfter(manyRuns, "aria-label=")[0], "Showing 8 of 1200 recent runs");
  assert.ok(collectScalars(manyRuns).includes("+999"));
});

test("RunHistorySummary renders every status chip from the presenter", () => {
  const { RunHistorySummary } = loadComponents();

  const rendered = RunHistorySummary({
    runs: [
      { status: "ok" },
      { status: "error" },
      { status: "running" },
      { status: "mystery" },
    ],
  });

  const scalars = collectScalars(rendered);
  for (const label of ["1 OK", "1 failed", "1 running", "1 unknown"]) {
    assert.ok(scalars.includes(label), `expected rendered summary to include ${label}`);
  }
});
