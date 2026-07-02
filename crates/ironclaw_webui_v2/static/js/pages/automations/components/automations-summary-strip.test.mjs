import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
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
  "automations.summary.filterAction": "Filter by {label}",
  "automations.summary.nextRun": "Next run",
  "automations.summary.nextRunDetail": "Soonest scheduled fire",
  "automations.summary.none": "None",
  "automations.summary.running": "Running",
  "automations.summary.runningDetail": "Runs in progress",
  "automations.summary.scheduled": "Scheduled",
  "automations.summary.scheduledDetail": "Scheduled automations",
};

function sourceForTest() {
  const source = readFileSync(new URL("./automations-summary-strip.js", import.meta.url), "utf8");
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

function nativeProps(root, tagName) {
  const props = [];
  visit(root, (node) => {
    if (!Array.isArray(node.strings) || !node.strings.join("").includes(`<${tagName}`)) return;
    const current = {};
    node.strings.forEach((part, index) => {
      const name = part.match(/([A-Za-z][A-Za-z0-9-]*)=\s*$/)?.[1];
      if (name) current[name] = node.values[index];
    });
    props.push(current);
  });
  return props;
}

function t(key, vars = {}) {
  return (COPY[key] || key).replace(/\{(\w+)\}/g, (_, name) => String(vars[name] ?? ""));
}

function loadComponent() {
  function Panel() {}
  function StatCard() {}
  const context = {
    globalThis: {},
    Panel,
    StatCard,
    cn: (...parts) => parts.filter(Boolean).join(" "),
    html,
    useT: () => t,
  };
  vm.runInNewContext(sourceForTest(), context);
  return context.globalThis.__testExports.AutomationsSummaryStrip;
}

test("summary cards filter all, active, running, and nonzero failures", () => {
  const AutomationsSummaryStrip = loadComponent();
  const selected = [];

  const rendered = AutomationsSummaryStrip({
    summary: {
      scheduled: 5,
      active: 2,
      running: 1,
      failures: 3,
      nextRun: "Jun 24",
    },
    activeFilter: "running",
    onSelectFilter: (filter) => selected.push(filter),
  });

  const buttons = nativeProps(rendered, "button");
  assert.equal(buttons.length, 4);
  assert.deepEqual(buttons.map((button) => button["aria-pressed"]), [false, false, true, false]);
  assert.deepEqual(buttons.map((button) => button.title), [
    "Filter by Scheduled",
    "Filter by Active",
    "Filter by Running",
    "Filter by Failures",
  ]);

  for (const button of buttons) button.onClick();

  assert.deepEqual(selected, ["all", "active", "running", "failures"]);
});

test("zero-failure summary card is not interactive", () => {
  const AutomationsSummaryStrip = loadComponent();
  const selected = [];

  const rendered = AutomationsSummaryStrip({
    summary: {
      scheduled: 5,
      active: 2,
      running: 1,
      failures: 0,
      nextRun: "Jun 24",
    },
    activeFilter: "all",
    onSelectFilter: (filter) => selected.push(filter),
  });

  const buttons = nativeProps(rendered, "button");
  assert.equal(buttons.length, 3);

  for (const button of buttons) button.onClick();

  assert.deepEqual(selected, ["all", "active", "running"]);
});
