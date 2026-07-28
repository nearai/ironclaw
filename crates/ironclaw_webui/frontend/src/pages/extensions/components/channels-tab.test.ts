// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

function sourceForTest() {
  const source = readFileSync(new URL("./channels-tab.tsx", import.meta.url), "utf8");
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
    lines.push(line.replace(/^export function /, "function "));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { ChannelsTab };`;
}

function renderTab(overrides = {}) {
  const context = {
    ExtensionCard() {},
    RegistryCard() {},
    globalThis: {},
    html: (strings, ...values) => ({ strings: Array.from(strings), values }),
    useT: () => (key) => key,
  };
  vm.runInNewContext(sourceForTest(), context);
  return {
    ExtensionCard: context.ExtensionCard,
    RegistryCard: context.RegistryCard,
    rendered: context.globalThis.__testExports.ChannelsTab({
      channels: [],
      channelRegistry: [],
      onConfigure() {},
      onRemove() {},
      onInstall() {},
      isBusy: false,
      ...overrides,
    }),
  };
}

function componentCount(node, component) {
  if (!node || typeof node !== "object") return node === component ? 1 : 0;
  if (Array.isArray(node)) {
    return node.reduce((sum, child) => sum + componentCount(child, component), 0);
  }
  if (Array.isArray(node.values)) {
    return node.values.reduce(
      (sum, child) => sum + componentCount(child, component),
      0,
    );
  }
  return 0;
}

test("ChannelsTab renders each caller-installed channel as one extension card", () => {
  const view = renderTab({
    channels: [
      { package_ref: { id: "channel-a" } },
      { package_ref: { id: "channel-b" } },
    ],
  });
  assert.equal(componentCount(view.rendered, view.ExtensionCard), 2);
  assert.equal(componentCount(view.rendered, view.RegistryCard), 0);
});

test("ChannelsTab renders uninstalled channel catalog entries as registry cards", () => {
  const view = renderTab({
    channelRegistry: [
      { package_ref: { id: "channel-a" } },
      { package_ref: { id: "channel-b" } },
    ],
  });
  assert.equal(componentCount(view.rendered, view.ExtensionCard), 0);
  assert.equal(componentCount(view.rendered, view.RegistryCard), 2);
});
