// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

import { CONNECTION_STATUS } from "../lib/connection-status";

function loadConnectionStatusForTest() {
  const source = readFileSync(new URL("./connection-status.tsx", import.meta.url), "utf8");
  const body = source
    .split("\n")
    .filter((line) => !line.startsWith("import "))
    .join("\n")
    .replace("export function ConnectionStatus", "function ConnectionStatus");
  const context = {
    CONNECTION_STATUS,
    React: {
      useEffect: (effect) => effect(),
      useState: (initial) => [initial, () => {}],
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

test("ConnectionStatus hides routine states and renders connection interruptions", () => {
  const ConnectionStatus = loadConnectionStatusForTest();

  for (const status of [
    undefined,
    CONNECTION_STATUS.IDLE,
    CONNECTION_STATUS.CONNECTING,
    CONNECTION_STATUS.CONNECTED,
  ]) {
    assert.equal(ConnectionStatus({ status }), null, status);
  }

  for (const [status, style] of [
    [CONNECTION_STATUS.RECONNECTING, "--v2-warning-soft"],
    [CONNECTION_STATUS.DISCONNECTED, "--v2-danger-soft"],
    [CONNECTION_STATUS.PAUSED, "--v2-surface-soft"],
  ]) {
    const rendered = ConnectionStatus({ status });
    assert.notEqual(rendered, null, status);
    const liveStatus = rendered.children[0];
    const button = rendered.children[1];
    assert.equal(liveStatus.props.role, "status", status);
    assert.equal(liveStatus.children[0], status, status);
    assert.equal(button.props["aria-label"], status, status);
    assert.equal(button.props["data-testid"], "connection-status", status);
    assert.match(button.props.className, /\bw-8\b/, status);
    assert.match(button.props.className, /\bborder-transparent\b/, status);
    assert.match(button.props.className, /\bbg-transparent\b/, status);
    assert.match(button.props.className, /\bsm:h-7\b/, status);
    assert.match(button.props.className, /\bsm:w-auto\b/, status);
    assert.match(button.props.className, /\bsm:max-w-48\b/, status);
    assert.ok(button.props.className.includes(style), status);
    assert.match(button.children[0].props.className, /\bshrink-0\b/, status);
    assert.match(button.children[1].props.className, /\bhidden\b/, status);
    assert.match(button.children[1].props.className, /\bsm:block\b/, status);
    assert.equal(button.children[1].children[0], status, status);
    const floatingLabel = rendered.children[2];
    assert.equal(floatingLabel.props["data-testid"], "connection-status-label", status);
    assert.match(floatingLabel.props.className, /\babsolute\b/, status);
    assert.match(floatingLabel.props.className, /\binvisible\b/, status);
    assert.match(floatingLabel.props.className, /\bsm:hidden\b/, status);
    assert.ok(floatingLabel.props.className.includes(style), status);
    assert.equal(floatingLabel.children[0], status, status);
  }

  const unknown = ConnectionStatus({ status: "blocked" });
  assert.notEqual(unknown, null);
  assert.equal(typeof unknown, "object");
  assert.ok(unknown.children[2].props.className.includes("--v2-surface-soft"));
});
