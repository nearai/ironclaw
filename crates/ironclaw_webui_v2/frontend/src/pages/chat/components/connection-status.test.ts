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
    [CONNECTION_STATUS.RECONNECTING, "bg-copper/20"],
    [CONNECTION_STATUS.DISCONNECTED, "bg-red-500/20"],
    [CONNECTION_STATUS.PAUSED, "bg-iron-700/50"],
  ]) {
    const rendered = ConnectionStatus({ status });
    assert.notEqual(rendered, null, status);
    assert.equal(rendered.props.role, "status", status);
    assert.match(rendered.props.className, /\babsolute\b/, status);
    assert.match(rendered.props.className, /\bw-max\b/, status);
    assert.doesNotMatch(rendered.props.className, /\bsticky\b/, status);
    assert.doesNotMatch(rendered.props.className, /\binset-x-4\b/, status);
    assert.match(rendered.props.className, new RegExp(style.replace("/", "\\/")), status);
    assert.equal(rendered.children[0], status, status);
  }

  const unknown = ConnectionStatus({ status: "blocked" });
  assert.notEqual(unknown, null);
  assert.equal(typeof unknown, "object");
  assert.match(unknown.props.className, /bg-iron-700\/50/);
});
