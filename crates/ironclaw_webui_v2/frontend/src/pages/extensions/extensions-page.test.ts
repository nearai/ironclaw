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
  const translations = {
    "ext.catalog.loadErrorTitle": "Extension catalog unavailable",
    "ext.catalog.loadErrorDesc": "The extension catalog could not be loaded.",
    "ext.catalog.retry": "Retry",
    "ext.catalog.retrying": "Retrying…",
  };
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
      isLoading: false,
      error: null,
      refetch: () => {},
      isRefetching: false,
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
    useT: () => (key) => translations[key] || key,
  };
  vm.runInNewContext(extensionsPageSourceForTest(), context);
  return {
    ...context,
    rendered: context.globalThis.__testExports.ExtensionsPage(),
  };
}

function templateText(node) {
  if (node == null) return "";
  if (typeof node !== "object") return String(node);
  return [node.strings || [], node.values || []]
    .flat()
    .map(templateText)
    .join(" ");
}

for (const tab of ["installed", "unknown"]) {
  test(`ExtensionsPage redirects ${tab} tab to registry`, () => {
    const { Navigate, rendered } = renderExtensionsPage(tab);

    assert.equal(rendered.values[0], Navigate);
    assert.match(rendered.strings.join(""), /to="\/extensions\/registry"/);
  });
}

test("ExtensionsPage replaces the empty catalog with a retryable error banner", () => {
  const refetch = () => {};
  const { rendered } = renderExtensionsPage("registry", {
    error: new Error("offline"),
    refetch,
  });
  const text = templateText(rendered);

  assert.match(text, /role="alert"/);
  assert.match(text, /Extension catalog unavailable/);
  assert.match(text, /The extension catalog could not be loaded\./);
  assert.match(text, /Retry/);
  assert.doesNotMatch(text, /Registry is empty/);
});
