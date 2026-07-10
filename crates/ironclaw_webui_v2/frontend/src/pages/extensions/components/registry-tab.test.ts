// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

function registryTabSourceForTest() {
  const source = readFileSync(new URL("./registry-tab.tsx", import.meta.url), "utf8");
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
  return `${lines.join("\n")}\nglobalThis.__testExports = { RegistryTab };`;
}

function renderRegistryTab(props, filter = "") {
  const context = {
    ExtensionCard() {},
    RegistryCard() {},
    React: {
      useState: () => [filter, () => {}],
    },
    globalThis: {},
    html(strings, ...values) {
      return { strings: Array.from(strings), values };
    },
    useT: () => (key) => {
      if (key === "extensions.installed") return "Installed";
      if (key === "ext.registry.availableTitle") return "Available extensions";
      if (key === "ext.registry.searchPlaceholder") return "Search extensions...";
      return key;
    },
  };
  vm.runInNewContext(registryTabSourceForTest(), context);
  return {
    ...context,
    rendered: context.globalThis.__testExports.RegistryTab(props),
  };
}

function collectComponentValues(node, component, matches = []) {
  if (Array.isArray(node)) {
    for (const item of node) collectComponentValues(item, component, matches);
    return matches;
  }
  if (!node || typeof node !== "object" || !Array.isArray(node.values)) {
    return matches;
  }
  for (let index = 0; index < node.values.length; index += 1) {
    if (node.values[index] === component) {
      matches.push(node.values.slice(index, index + 7));
    }
    collectComponentValues(node.values[index], component, matches);
  }
  return matches;
}

test("RegistryTab renders only real installed extensions with management actions", () => {
  const onInstall = () => {};
  const installedExtension = {
    package_ref: { kind: "extension", id: "google-calendar" },
    display_name: "Google Runtime",
    kind: "wasm_tool",
    active: true,
  };
  const registryOnlyInstalled = {
    package_ref: { kind: "extension", id: "calendar-registry-only" },
    display_name: "Calendar Registry Only",
    kind: "wasm_tool",
    installed: true,
  };
  const availableEntry = {
    package_ref: { kind: "extension", id: "github" },
    display_name: "GitHub",
    kind: "mcp_server",
  };
  const { ExtensionCard, RegistryCard, rendered } = renderRegistryTab({
    catalogEntries: [
      {
        id: "google-calendar",
        installed: true,
        entry: {
          package_ref: installedExtension.package_ref,
          display_name: "Google Calendar",
          description: "Calendar metadata",
          kind: "wasm_tool",
        },
        extension: installedExtension,
      },
      {
        id: "calendar-registry-only",
        installed: true,
        entry: registryOnlyInstalled,
        extension: null,
      },
      {
        id: "github",
        installed: false,
        entry: availableEntry,
        extension: null,
      },
    ],
    onInstall,
    onActivate: () => {},
    onConfigure: () => {},
    onRemove: () => {},
    connectableChannels: [],
    isBusy: false,
  });

  const extensionCards = collectComponentValues(rendered, ExtensionCard);
  assert.equal(extensionCards.length, 1);
  assert.equal(extensionCards[0][2], installedExtension);

  const registryCards = collectComponentValues(rendered, RegistryCard);
  assert.equal(registryCards.length, 2);
  assert.equal(registryCards[0][2], registryOnlyInstalled);
  assert.equal(registryCards[0][3], "Installed");
  assert.equal(registryCards[1][2], availableEntry);
  assert.equal(registryCards[1][3], onInstall);
});

test("RegistryTab searches installed entries using registry metadata", () => {
  const installedExtension = {
    package_ref: { kind: "extension", id: "google-calendar" },
    display_name: "Runtime Name",
    kind: "wasm_tool",
    active: true,
  };
  const { ExtensionCard, rendered } = renderRegistryTab(
    {
      catalogEntries: [
        {
          id: "google-calendar",
          installed: true,
          entry: {
            package_ref: installedExtension.package_ref,
            display_name: "Google Calendar",
            description: "Calendar integration",
            keywords: ["schedule"],
            kind: "wasm_tool",
          },
          extension: installedExtension,
        },
      ],
      onInstall: () => {},
      onActivate: () => {},
      onConfigure: () => {},
      onRemove: () => {},
      connectableChannels: [],
      isBusy: false,
    },
    "calendar",
  );

  assert.equal(collectComponentValues(rendered, ExtensionCard).length, 1);
});
