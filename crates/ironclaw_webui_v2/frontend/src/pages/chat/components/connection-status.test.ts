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

test("ConnectionStatus suppresses non-actionable transport states", () => {
  const ConnectionStatus = loadConnectionStatusForTest();

  for (const status of [
    undefined,
    CONNECTION_STATUS.IDLE,
    CONNECTION_STATUS.CONNECTING,
    CONNECTION_STATUS.CONNECTED,
    CONNECTION_STATUS.RECONNECTING,
    CONNECTION_STATUS.DISCONNECTED,
    CONNECTION_STATUS.PAUSED,
  ]) {
    assert.equal(ConnectionStatus({ status }), null, status);
  }

  const unknown = ConnectionStatus({ status: "blocked" });
  assert.notEqual(unknown, null);
  assert.equal(typeof unknown, "object");
});
