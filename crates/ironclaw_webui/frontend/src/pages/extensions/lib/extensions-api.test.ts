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
  return `${lines.join("\n")}\nglobalThis.__testExports = { installExtension, activateExtension, removeExtension, submitExtensionSetup, startExtensionOauth };`;
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
    clientActionId: () => "client-action-test",
    setupExtension: () => {},
  };
  vm.runInNewContext(extensionsApiSourceForTest(), context);

  const before = Date.now();
  await context.globalThis.__testExports.startExtensionOauth(
    { kind: "extension", id: "slack" },
    {
      provider: "slack",
      setup: {
        account_label: "slack slack",
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

test("extension lifecycle mutations include a client action id", async () => {
  const apiCalls = [];
  const setupCalls = [];
  const context = {
    apiFetch: async (url, options) => {
      apiCalls.push({ url, options });
      return { success: true };
    },
    encodeURIComponent,
    globalThis: {},
    redeemPairingCode: () => {},
    clientActionId: () => "client-action-test",
    setupExtension: async (extensionName, options) => {
      setupCalls.push({ extensionName, options });
      return { success: true };
    },
  };
  vm.runInNewContext(extensionsApiSourceForTest(), context);

  const packageRef = { kind: "extension", id: "web-access" };
  await context.globalThis.__testExports.installExtension(packageRef);
  await context.globalThis.__testExports.activateExtension(packageRef);
  await context.globalThis.__testExports.removeExtension(packageRef);
  await context.globalThis.__testExports.submitExtensionSetup(packageRef, {}, {});

  assert.deepEqual(JSON.parse(apiCalls[0].options.body), {
    package_ref: packageRef,
    client_action_id: "client-action-test",
  });
  assert.deepEqual(JSON.parse(apiCalls[1].options.body), {
    client_action_id: "client-action-test",
  });
  assert.deepEqual(JSON.parse(apiCalls[2].options.body), {
    client_action_id: "client-action-test",
  });
  assert.equal(setupCalls[0].extensionName, "web-access");
  assert.deepEqual(JSON.parse(JSON.stringify(setupCalls[0].options)), {
    action: "submit",
    payload: { secrets: {}, fields: {} },
  });
});
