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
  return `${lines.join("\n")}\nglobalThis.__testExports = { installExtension, removeExtension, submitExtensionSetup, startExtensionOauth };`;
}

test("installExtension assigns a fresh client idempotency key to each gesture", async () => {
  const apiCalls = [];
  const ids = ["install-gesture-one", "install-gesture-two"];
  const context = {
    apiFetch: async (url, options) => {
      apiCalls.push({ url, options });
      return { success: true };
    },
    clientActionId: () => ids.shift(),
    encodeURIComponent,
    globalThis: {},
    setupExtension: () => {},
  };
  vm.runInNewContext(extensionsApiSourceForTest(), context);

  const packageRef = { kind: "extension", id: "notion" };
  await context.globalThis.__testExports.installExtension(packageRef);
  await context.globalThis.__testExports.installExtension(packageRef);

  assert.equal(apiCalls.length, 2);
  assert.equal(JSON.parse(apiCalls[0].options.body).client_action_id, "install-gesture-one");
  assert.equal(JSON.parse(apiCalls[1].options.body).client_action_id, "install-gesture-two");
  assert.deepEqual(JSON.parse(apiCalls[0].options.body).package_ref, packageRef);
});

test("startExtensionOauth sends an expiry safely below the backend max TTL", async () => {
  const apiCalls = [];
  const context = {
    apiFetch: async (url, options) => {
      apiCalls.push({ url, options });
      return { success: true };
    },
    encodeURIComponent,
    globalThis: {},
    clientActionId: () => "client-action-test",
    setupExtension: () => {},
  };
  vm.runInNewContext(extensionsApiSourceForTest(), context);

  const before = Date.now();
  await context.globalThis.__testExports.startExtensionOauth(
    { kind: "extension", id: "slack" },
    {
      name: "slack_personal_oauth",
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
  assert.equal(payload.requirement, "slack_personal_oauth");
  assert.equal(payload.invocation_id, "invocation-alpha");
  assert.equal("provider" in payload, false);
  assert.equal("account_label" in payload, false);
  assert.equal("scopes" in payload, false);
});

test("startExtensionOauth does not crash while a requirement projection is refreshing", async () => {
  const apiCalls = [];
  const context = {
    apiFetch: async (url, options) => {
      apiCalls.push({ url, options });
      return { success: false };
    },
    encodeURIComponent,
    globalThis: {},
    setupExtension: () => {},
  };
  vm.runInNewContext(extensionsApiSourceForTest(), context);

  await context.globalThis.__testExports.startExtensionOauth(
    { kind: "extension", id: "slack" },
    undefined,
  );

  assert.equal(apiCalls.length, 1);
  const payload = JSON.parse(apiCalls[0].options.body);
  assert.equal("requirement" in payload, false);
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
    clientActionId: () => "client-action-test",
    setupExtension: async (extensionName, options) => {
      setupCalls.push({ extensionName, options });
      return { success: true };
    },
  };
  vm.runInNewContext(extensionsApiSourceForTest(), context);

  const packageRef = { kind: "extension", id: "web-access" };
  await context.globalThis.__testExports.installExtension(packageRef, {
    clientActionId: "stable-install-action",
  });
  await context.globalThis.__testExports.removeExtension(packageRef, {
    clientActionId: "stable-remove-action",
  });
  await context.globalThis.__testExports.submitExtensionSetup(packageRef, {}, {
    clientActionId: "stable-setup-action",
  });

  assert.deepEqual(JSON.parse(apiCalls[0].options.body), {
    package_ref: packageRef,
    client_action_id: "stable-install-action",
  });
  assert.deepEqual(JSON.parse(apiCalls[1].options.body), {
    client_action_id: "stable-remove-action",
  });
  assert.equal(setupCalls[0].extensionName, "web-access");
  assert.deepEqual(JSON.parse(JSON.stringify(setupCalls[0].options)), {
    action: "submit",
    payload: { secrets: {} },
    clientActionId: "stable-setup-action",
  });
});
