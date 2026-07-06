import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function loadConnectionStatusForTest() {
  const source = readFileSync(new URL("./connection-status.js", import.meta.url), "utf8");
  const body = source
    .split("\n")
    .filter((line) => !line.startsWith("import "))
    .join("\n")
    .replace("export function ConnectionStatus", "function ConnectionStatus");
  const context = {
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

test("ConnectionStatus suppresses transient initial connecting state", () => {
  const ConnectionStatus = loadConnectionStatusForTest();

  assert.equal(ConnectionStatus({ status: "connecting" }), null);
  const reconnecting = ConnectionStatus({ status: "reconnecting" });
  assert.notEqual(reconnecting, null);
  assert.equal(typeof reconnecting, "object");
});
