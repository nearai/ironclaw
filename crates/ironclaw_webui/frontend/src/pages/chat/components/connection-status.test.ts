// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

import { CONNECTION_STATUS } from "../lib/connection-status";

const DISCLOSURE_ID = "connection-status-details";

function loadConnectionStatusForTest({ expanded = false } = {}) {
  const source = readFileSync(
    new URL("./connection-status.tsx", import.meta.url),
    "utf8",
  );
  const body = source
    .split("\n")
    .filter((line) => !line.startsWith("import "))
    .join("\n")
    .replace("export function ConnectionStatus", "function ConnectionStatus");
  const context = {
    CONNECTION_STATUS,
    React: {
      useEffect: (effect) => effect(),
      useId: () => DISCLOSURE_ID,
      useState: () => [expanded, () => {}],
    },
    html: (strings, ...values) => ({ strings, values }),
    useT: () => (key) => key,
    globalThis: {},
  };
  vm.runInNewContext(
    `${body}\nglobalThis.__testExports = { ConnectionStatus };`,
    context,
  );
  return context.globalThis.__testExports.ConnectionStatus;
}

function findNode(value, predicate, seen = new Set()) {
  if (!value || typeof value !== "object" || seen.has(value)) return null;
  seen.add(value);
  if (Array.isArray(value)) {
    for (const candidate of value) {
      const match = findNode(candidate, predicate, seen);
      if (match) return match;
    }
    return null;
  }
  if (predicate(value)) return value;

  for (const key of ["children", "values"]) {
    const candidates = Array.isArray(value[key]) ? value[key] : [];
    for (const candidate of candidates) {
      const match = findNode(candidate, predicate, seen);
      if (match) return match;
    }
  }
  return null;
}

function nodeByTestId(rendered, testId) {
  return findNode(
    rendered,
    (node) => node.props?.["data-testid"] === testId,
  );
}

test("ConnectionStatus keeps an empty live region mounted for routine states", () => {
  const ConnectionStatus = loadConnectionStatusForTest();

  for (const status of [
    undefined,
    CONNECTION_STATUS.IDLE,
    CONNECTION_STATUS.CONNECTING,
    CONNECTION_STATUS.CONNECTED,
  ]) {
    const rendered = ConnectionStatus({ status });
    assert.notEqual(rendered, null, status);
    assert.equal(rendered.props.className, "contents", status);

    const liveStatus = findNode(
      rendered,
      (node) => node.props?.role === "status",
    );
    assert.notEqual(liveStatus, null, status);
    assert.equal(liveStatus.props["aria-live"], "polite", status);
    assert.equal(liveStatus.props["aria-atomic"], "true", status);
    assert.equal(liveStatus.children[0], "", status);
    assert.equal(nodeByTestId(rendered, "connection-status"), null, status);
    assert.equal(
      nodeByTestId(rendered, "connection-status-toggle"),
      null,
      status,
    );
  }
});

test("ConnectionStatus renders static desktop status and mobile disclosure", () => {
  const ConnectionStatus = loadConnectionStatusForTest();

  for (const [status, style] of [
    [CONNECTION_STATUS.RECONNECTING, "--v2-warning-soft"],
    [CONNECTION_STATUS.DISCONNECTED, "--v2-danger-soft"],
    [CONNECTION_STATUS.PAUSED, "--v2-surface-soft"],
  ]) {
    const rendered = ConnectionStatus({ status });
    const liveStatus = findNode(
      rendered,
      (node) => node.props?.role === "status",
    );
    assert.equal(liveStatus.children[0], status, status);

    const desktopStatus = nodeByTestId(rendered, "connection-status");
    assert.equal(desktopStatus.type, "span", status);
    assert.equal("onClick" in desktopStatus.props, false, status);
    assert.match(desktopStatus.props.className, /\bhidden\b/, status);
    assert.match(desktopStatus.props.className, /\bsm:inline-flex\b/, status);
    assert.ok(desktopStatus.props.className.includes(style), status);
    assert.equal(desktopStatus.children[1].children[0], status, status);

    const toggle = nodeByTestId(rendered, "connection-status-toggle");
    assert.equal(toggle.type, "button", status);
    assert.equal(toggle.props["aria-label"], status, status);
    assert.equal(toggle.props["aria-expanded"], "false", status);
    assert.equal(toggle.props["aria-controls"], DISCLOSURE_ID, status);
    assert.match(toggle.props.className, /\bborder-transparent\b/, status);
    assert.match(toggle.props.className, /\bbg-transparent\b/, status);
    assert.match(toggle.props.className, /\bsm:hidden\b/, status);

    const floatingLabel = nodeByTestId(
      rendered,
      "connection-status-label",
    );
    assert.equal(floatingLabel.props.id, DISCLOSURE_ID, status);
    assert.equal(floatingLabel.props["aria-hidden"], "true", status);
    assert.match(floatingLabel.props.className, /\babsolute\b/, status);
    assert.match(floatingLabel.props.className, /\binvisible\b/, status);
    assert.match(floatingLabel.props.className, /\bsm:hidden\b/, status);
    assert.ok(floatingLabel.props.className.includes(style), status);
    assert.equal(floatingLabel.children[0], status, status);
  }
});

test("ConnectionStatus exposes the expanded mobile label state", () => {
  const ConnectionStatus = loadConnectionStatusForTest({ expanded: true });
  const rendered = ConnectionStatus({ status: CONNECTION_STATUS.RECONNECTING });
  const toggle = nodeByTestId(rendered, "connection-status-toggle");
  const floatingLabel = nodeByTestId(rendered, "connection-status-label");

  assert.equal(toggle.props["aria-expanded"], "true");
  assert.equal(toggle.props["aria-controls"], floatingLabel.props.id);
  assert.equal(floatingLabel.props["aria-hidden"], "false");
  assert.match(floatingLabel.props.className, /\bvisible\b/);
});

test("ConnectionStatus falls back safely for an unknown interruption", () => {
  const ConnectionStatus = loadConnectionStatusForTest();
  const rendered = ConnectionStatus({ status: "blocked" });
  const floatingLabel = nodeByTestId(rendered, "connection-status-label");

  assert.ok(floatingLabel.props.className.includes("--v2-surface-soft"));
  assert.equal(floatingLabel.children[0], "blocked");
});
