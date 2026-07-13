// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

import { CONNECTION_STATUS } from "../lib/connection-status.js";

function loadConnectionStatusForTest() {
  const source = readFileSync(new URL("./connection-status.tsx", import.meta.url), "utf8");
  const body = source
    .split("\n")
    .filter((line) => !line.startsWith("import "))
    .join("\n")
    .replace("export function ConnectionStatus", "function ConnectionStatus");
  const context = {
    CONNECTION_STATUS,
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
    assert.equal(rendered.props.role, "status", status);
    assert.equal(rendered.props["aria-label"], status, status);
    assert.doesNotMatch(rendered.props.className, /\babsolute\b/, status);
    assert.match(rendered.props.className, /\bmax-w-32\b/, status);
    assert.match(rendered.props.className, /\bsm:max-w-48\b/, status);
    assert.doesNotMatch(rendered.props.className, /\bsticky\b/, status);
    assert.ok(rendered.props.className.includes(style), status);
    assert.match(rendered.children[0].props.className, /\bshrink-0\b/, status);
    assert.match(rendered.children[1].props.className, /\btruncate\b/, status);
    assert.equal(rendered.children[1].children[0], status, status);
  }

  const unknown = ConnectionStatus({ status: "blocked" });
  assert.notEqual(unknown, null);
  assert.equal(typeof unknown, "object");
  assert.ok(unknown.props.className.includes("--v2-surface-soft"));
});
