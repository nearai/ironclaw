// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

function extensionsApiSourceForTest() {
  const source = readFileSync(new URL("./extensions-api.ts", import.meta.url), "utf8");
  const lines = [];
  for (const line of source.split("\n")) {
    if (line.startsWith("import ")) continue;
    lines.push(line.replace(/^export function /, "function "));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { startExtensionOauth };`;
}

test("startExtensionOauth sends an expiry safely below the backend max TTL", async () => {
  const apiCalls = [];
  const context = {
    apiFetch: async (url, options) => {
      apiCalls.push({ url, options });
      return { success: true };
    },
    encodeURIComponent,
    globalThis: {},
    redeemPairingCode: () => {},
    setupExtension: () => {},
  };
  vm.runInNewContext(extensionsApiSourceForTest(), context);

  const before = Date.now();
  await context.globalThis.__testExports.startExtensionOauth(
    { kind: "extension", id: "slack" },
    {
      provider: "slack_personal",
      setup: {
        account_label: "slack slack_personal",
        scopes: ["users:read"],
        invocation_id: "invocation-alpha",
      },
    }
  );

  assert.equal(apiCalls.length, 1);
  const payload = JSON.parse(apiCalls[0].options.body);
  const ttlMs = Date.parse(payload.expires_at) - before;
  assert.ok(ttlMs > 4 * 60 * 1000, "expiry should leave enough time to authorize");
  assert.ok(ttlMs <= 6 * 60 * 1000, "expiry should not sit on the 10 minute backend limit");
  assert.equal(payload.invocation_id, "invocation-alpha");
});
