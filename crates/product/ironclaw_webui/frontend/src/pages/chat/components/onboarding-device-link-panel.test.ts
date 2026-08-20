// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

function sourceForTest() {
  const source = readFileSync(
    new URL("./onboarding-device-link-panel.tsx", import.meta.url),
    "utf8",
  );
  const lines = [];
  let skippingImport = false;
  for (const line of source.split("\n")) {
    if (!skippingImport && line.startsWith("import ")) {
      skippingImport = !line.trimEnd().endsWith(";");
      continue;
    }
    if (skippingImport) {
      skippingImport = !line.trimEnd().endsWith(";");
      continue;
    }
    lines.push(
      line.replace(
        "export function OnboardingDeviceLinkPanel",
        "function OnboardingDeviceLinkPanel",
      ),
    );
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { OnboardingDeviceLinkPanel };`;
}

function findComponent(node, component) {
  if (!node || typeof node !== "object" || !Array.isArray(node.values)) return null;
  if (node.values.includes(component)) return node;
  for (const value of node.values) {
    const found = findComponent(value, component);
    if (found) return found;
  }
  return null;
}

function componentProps(node, component) {
  const props = {};
  const start = node.values.indexOf(component);
  for (let index = start + 1; index < node.values.length; index += 1) {
    const name = node.strings[index]?.match(/([A-Za-z][A-Za-z0-9]*)=\s*$/)?.[1];
    if (name) props[name] = node.values[index];
  }
  return props;
}

test("device-link onboarding resolves the declared provider and resumes chat on completion", async () => {
  const notifications = [];
  function DeviceLinkPanel() {}
  const context = {
    DeviceLinkPanel,
    deviceLinkSetupSecret: (secrets) =>
      secrets.find((secret) => secret?.setup?.kind === "device_link") || null,
    globalThis: {},
    html: (strings, ...values) => ({ strings: Array.from(strings), values }),
    notifyChannelConnected: async (payload) => notifications.push(payload),
    useExtensionSetup: () => ({
      secrets: [
        { provider: "telegram-user", setup: { kind: "device_link" } },
      ],
      isLoading: false,
      error: null,
    }),
    useT: () => (key) => key,
  };
  vm.runInNewContext(sourceForTest(), context);
  const rendered = context.globalThis.__testExports.OnboardingDeviceLinkPanel({
    onboarding: { extensionName: "telegram" },
    displayName: "Telegram",
    errorMessage: "Link failed.",
  });

  const panel = findComponent(rendered, DeviceLinkPanel);
  assert.ok(panel);
  const props = componentProps(panel, DeviceLinkPanel);
  assert.equal(props.provider, "telegram-user");
  assert.equal(props.extensionName, "telegram");
  assert.equal(props.displayName, "Telegram");
  await props.onCompleted();
  assert.deepEqual(JSON.parse(JSON.stringify(notifications)), [
    { channel: "telegram", source: "chat-device-link" },
  ]);
});
