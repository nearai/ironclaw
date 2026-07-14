// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

function extensionsPageSourceForTest() {
  const source = readFileSync(new URL("./extensions-page.tsx", import.meta.url), "utf8");
  const lines = [];
  for (const line of source.split("\n")) {
    if (line.startsWith("import ")) continue;
    lines.push(line.replace(/^export function /, "function "));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { ExtensionsPage };`;
}

function renderExtensionsPage(tab, extensionState = {}) {
  const context = {
    ActionToast() {},
    ChannelsTab() {},
    ConfigureModal() {},
    McpTab() {},
    Navigate() {},
    React: {
      useCallback: (fn) => fn,
      useState: (initial) => [typeof initial === "function" ? initial() : initial, () => {}],
    },
    RegistryTab() {},
    globalThis: {},
    html(strings, ...values) {
      return { strings: Array.from(strings), values };
    },
    useExtensions: () => ({
      status: {},
      channels: [],
      mcpServers: [],
      channelRegistry: [],
      mcpRegistry: [],
      catalogEntries: [],
      connectableChannels: [],
      isExtensionsLoading: false,
      isRegistryLoading: false,
      isBusy: false,
      actionResult: null,
      clearResult: () => {},
      install: () => {},
      activate: () => {},
      remove: () => {},
      invalidate: () => {},
      ...extensionState,
    }),
    useParams: () => ({ tab }),
  };
  vm.runInNewContext(extensionsPageSourceForTest(), context);
  return {
    ...context,
    rendered: context.globalThis.__testExports.ExtensionsPage(),
  };
}

function findComponent(node, component) {
  if (Array.isArray(node)) {
    for (const child of node) {
      const match = findComponent(child, component);
      if (match) return match;
    }
    return null;
  }
  if (!node || typeof node !== "object") return null;
  if (node.type === component) return node;
  return findComponent(node.children, component);
}

test("ExtensionsPage renders registry data while installed extensions are still loading", () => {
  const catalogEntries = [{ id: "registry-tool" }];
  const { RegistryTab, rendered } = renderExtensionsPage("registry", {
    catalogEntries,
    isExtensionsLoading: true,
    isRegistryLoading: false,
  });

  const renderedJson = JSON.stringify(rendered);
  assert.doesNotMatch(
    renderedJson,
    /v2-skeleton/,
    "the registry must not remain behind the installed-extension skeleton",
  );
  const registryTab = findComponent(rendered, RegistryTab);
  assert.ok(registryTab, "the registry tab content must be rendered");
  assert.equal(registryTab.props.catalogEntries, catalogEntries);
});

for (const tab of ["installed", "unknown"]) {
  test(`ExtensionsPage redirects ${tab} tab before waiting for data`, () => {
    const { Navigate, rendered } = renderExtensionsPage(tab, {
      isExtensionsLoading: true,
      isRegistryLoading: true,
    });

    assert.equal(rendered.values[0], Navigate);
    assert.match(rendered.strings.join(""), /to="\/extensions\/registry"/);
  });
}
