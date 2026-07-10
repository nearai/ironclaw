// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

const COPY = {
  "automations.badge.danger": "danger",
  "automations.badge.info": "info",
  "automations.badge.muted": "muted",
  "automations.badge.signal": "signal",
  "automations.badge.success": "success",
  "automations.summary.active": "Active",
  "automations.summary.activeDetail": "Active automations",
  "automations.summary.failures": "Failures",
  "automations.summary.failuresDetail": "Failed recent runs",
  "automations.summary.nextRun": "Next run",
  "automations.summary.nextRunDetail": "Soonest scheduled fire",
  "automations.summary.nextRunDue": "Due now",
  "automations.summary.none": "None",
  "automations.summary.running": "Running",
  "automations.summary.runningDetail": "Runs in progress",
  "automations.summary.scheduled": "Scheduled",
  "automations.summary.scheduledDetail": "Scheduled automations",
};

function sourceForTest() {
  const source = readFileSync(new URL("./automations-summary-strip.tsx", import.meta.url), "utf8");
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
  return `${lines.join("\n")}\nglobalThis.__testExports = { AutomationsSummaryStrip };`;
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

function collectStrings(root) {
  const parts = [];
  visit(root, (node) => {
    if (Array.isArray(node.strings)) parts.push(node.strings.join(""));
  });
  return parts.join("");
}

function collectValues(root) {
  const values = [];
  visit(root, (node) => {
    if (!Array.isArray(node.values)) return;
    for (const value of node.values) {
      if (typeof value === "string" || typeof value === "number") values.push(String(value));
    }
  });
  return values;
}

function badgeTones(root) {
  const tones = [];
  visit(root, (node) => {
    if (!Array.isArray(node.strings)) return;
    node.strings.forEach((part, index) => {
      if (part.match(/tone=\s*$/)) tones.push(node.values[index]);
    });
  });
  return tones;
}

function t(key, vars = {}) {
  return (COPY[key] || key).replace(/\{(\w+)\}/g, (_, name) => String(vars[name] ?? ""));
}

function loadComponent() {
  function Badge() {}
  function Card() {}
  const React = {
    useState: (init) => [typeof init === "function" ? init() : init, () => {}],
    useEffect: () => {},
  };
  const context = {
    globalThis: {},
    Badge,
    Card,
    React,
    cn: (...parts) => parts.filter(Boolean).join(" "),
    html,
    useT: () => t,
  };
  vm.runInNewContext(sourceForTest(), context);
  return context.globalThis.__testExports.AutomationsSummaryStrip;
}

test("strip reflows two/three/five columns and renders every summary cell", () => {
  const AutomationsSummaryStrip = loadComponent();

  const rendered = AutomationsSummaryStrip({
    summary: { scheduled: 5, active: 2, running: 1, failures: 0, nextRun: "Jun 24" },
    nextRunAt: undefined,
  });

  const markup = collectStrings(rendered);
  assert.ok(
    markup.includes("grid-cols-2 gap-3 md:grid-cols-3 xl:grid-cols-5"),
    "cells must reflow to fewer columns below large screens"
  );

  const values = collectValues(rendered);
  for (const label of ["Scheduled", "Active", "Running", "Failures", "Next run"]) {
    assert.ok(values.includes(label), `expected the ${label} cell label`);
  }
  // No next run scheduled -> the headline falls back to "None".
  assert.ok(values.includes("None"));
});

test("failures cell flips from success to danger tone when failures exist", () => {
  const AutomationsSummaryStrip = loadComponent();

  const clean = AutomationsSummaryStrip({
    summary: { scheduled: 5, active: 2, running: 1, failures: 0 },
  });
  assert.ok(badgeTones(clean).includes("success"));
  assert.ok(!badgeTones(clean).includes("danger"));

  const failing = AutomationsSummaryStrip({
    summary: { scheduled: 5, active: 2, running: 1, failures: 3 },
  });
  assert.ok(badgeTones(failing).includes("danger"));
});

test("next-run cell counts down to a future fire and reports overdue fires as due", () => {
  const AutomationsSummaryStrip = loadComponent();

  const future = AutomationsSummaryStrip({
    summary: { scheduled: 1, active: 1, running: 0, failures: 0 },
    nextRunAt: Date.now() + 90_000,
  });
  const countdown = collectValues(future).find((value) => /^1:(2[89]|30)$/.test(value));
  assert.ok(countdown, "a fire 90s out must render as a compact m:ss countdown");

  const overdue = AutomationsSummaryStrip({
    summary: { scheduled: 1, active: 1, running: 0, failures: 0 },
    nextRunAt: Date.now() - 1_000,
  });
  assert.ok(collectValues(overdue).includes("Due now"));
});
