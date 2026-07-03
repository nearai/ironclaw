import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function channelConnectCardSourceForTest() {
  const source = readFileSync(new URL("./channel-connect-card.js", import.meta.url), "utf8");
  const lines = [];
  for (const line of source.split("\n")) {
    if (line.startsWith("import ")) continue;
    lines.push(line.replace(/^export function /, "function "));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { isSlackStrategy };`;
}

test("isSlackStrategy gates the Slack personal pairing renderer", () => {
  const context = { globalThis: {} };
  vm.runInNewContext(channelConnectCardSourceForTest(), context);
  const { isSlackStrategy } = context.globalThis.__testExports;

  assert.equal(
    isSlackStrategy(
      { channel: "slack", strategy: "inbound_proof_code" },
      "inbound_proof_code",
    ),
    true,
  );
  assert.equal(
    isSlackStrategy(
      { channel: "slack", strategy: "inbound_proof_code" },
      "admin_managed_channels",
    ),
    false,
  );
  assert.equal(
    isSlackStrategy(
      { channel: "teams", strategy: "inbound_proof_code" },
      "inbound_proof_code",
    ),
    false,
  );
});
