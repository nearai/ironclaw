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

function visit(node, fn, seen = new Set()) {
  if (Array.isArray(node)) {
    for (const item of node) visit(item, fn, seen);
    return;
  }
  if (!node || typeof node !== "object" || seen.has(node)) return;
  seen.add(node);
  fn(node);
  if (Array.isArray(node.values)) {
    for (const value of node.values) visit(value, fn, seen);
  }
  if (Array.isArray(node.children)) {
    for (const child of node.children) visit(child, fn, seen);
  }
  if (node.props && typeof node.props === "object") {
    visit(node.props.children, fn, seen);
  }
}

function componentProps(root, type) {
  const props = [];
  visit(root, (node) => {
    if (node.type === type && node.props) props.push(node.props);
  });
  return props;
}

function t(key, vars = {}) {
  return (COPY[key] || key).replace(/\{(\w+)\}/g, (_, name) => String(vars[name] ?? ""));
}

function loadComponent() {
  function StatStrip() {}
  function StatTile() {}
  const context = {
    globalThis: {},
    StatStrip,
    StatTile,
    cn: (...parts) => parts.filter(Boolean).join(" "),
    html,
    useT: () => t,
  };
  vm.runInNewContext(sourceForTest(), context);
  return { AutomationsSummaryStrip: context.globalThis.__testExports.AutomationsSummaryStrip, StatTile };
}

test("summary tiles filter all, active, running, and nonzero failures", () => {
  const { AutomationsSummaryStrip, StatTile } = loadComponent();
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

  const tiles = componentProps(rendered, StatTile);
  assert.equal(tiles.length, 5);

  const interactive = tiles.filter((tile) => typeof tile.onSelect === "function");
  assert.equal(interactive.length, 4);
  assert.deepEqual(
    interactive.map((tile) => tile.isActive),
    [false, false, true, false]
  );
  assert.deepEqual(
    interactive.map((tile) => tile.selectTitle),
    [
      "Filter by Scheduled",
      "Filter by Active",
      "Filter by Running",
      "Filter by Failures",
    ]
  );

  // The NEXT RUN tile is informational: no filter, smaller value type.
  const nextRun = tiles[4];
  assert.equal(nextRun.onSelect, undefined);
  assert.equal(nextRun.value, "Jun 24");
  assert.match(nextRun.valueClassName, /text-lg/);

  for (const tile of interactive) tile.onSelect();
  assert.deepEqual(selected, ["all", "active", "running", "failures"]);
});

test("zero-failure summary tile is not interactive", () => {
  const { AutomationsSummaryStrip, StatTile } = loadComponent();
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

  const tiles = componentProps(rendered, StatTile);
  const interactive = tiles.filter((tile) => typeof tile.onSelect === "function");
  assert.equal(interactive.length, 3);

  for (const tile of interactive) tile.onSelect();
  assert.deepEqual(selected, ["all", "active", "running"]);
});
