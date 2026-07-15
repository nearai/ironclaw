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
  return `${lines.join("\n")}\nglobalThis.__testExports = { ExtensionsPage, CatalogErrorBanner };`;
}

function renderExtensionsPage(tab, extensionState = {}) {
  const translations = {
    "ext.catalog.loadErrorTitle": "Extension catalog unavailable",
    "ext.catalog.loadErrorDesc": "The extension catalog could not be loaded.",
    "ext.catalog.partialErrorTitle": "Some extension data is unavailable",
    "ext.catalog.partialErrorDesc":
      "The available extension data is shown, but some details could not be loaded.",
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
      extensionsError: null,
      registryError: null,
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
    CatalogErrorBanner: context.globalThis.__testExports.CatalogErrorBanner,
    rendered: context.globalThis.__testExports.ExtensionsPage(),
  };
}

function templateText(node) {
  if (node == null) return "";
  if (Array.isArray(node)) return node.map(templateText).join(" ");
  if (typeof node !== "object") return String(node);
  return [node.strings || [], node.values || []]
    .flat()
    .map(templateText)
    .join(" ");
}

function templateValues(node) {
  if (node == null) return [];
  if (Array.isArray(node)) return node.flatMap(templateValues);
  if (typeof node !== "object") return [node];
  return [node, ...templateValues(node.values || [])];
}

for (const tab of ["installed", "unknown"]) {
  test(`ExtensionsPage redirects ${tab} tab to registry`, () => {
    const { Navigate, rendered } = renderExtensionsPage(tab);

    assert.equal(rendered.values[0], Navigate);
    assert.match(rendered.strings.join(""), /to="\/extensions\/registry"/);
  });
}

test("templateText includes text nested inside arrays", () => {
  assert.equal(
    templateText(["first", { strings: ["second"], values: [["third"]] }]),
    "first second third",
  );
});

test("ExtensionsPage replaces a failed registry with a retryable error banner", () => {
  const refetch = () => {};
  const { CatalogErrorBanner, RegistryTab, rendered } = renderExtensionsPage("registry", {
    registryError: new Error("offline"),
    refetch,
  });
  const values = templateValues(rendered);
  const banner = CatalogErrorBanner({ isRefetching: false, onRetry: refetch });
  const text = templateText(banner);

  assert.ok(values.includes(CatalogErrorBanner));
  assert.ok(!values.includes(RegistryTab));
  assert.match(text, /role="alert"/);
  assert.match(text, /Extension catalog unavailable/);
  assert.match(text, /The extension catalog could not be loaded\./);
  assert.match(text, /Retry/);
  assert.doesNotMatch(text, /Registry is empty/);
});

test("ExtensionsPage keeps installed channels visible when only the registry fails", () => {
  const { CatalogErrorBanner, ChannelsTab, rendered } = renderExtensionsPage("channels", {
    registryError: new Error("offline"),
  });
  const values = templateValues(rendered);

  assert.ok(values.includes(CatalogErrorBanner));
  assert.ok(values.includes(ChannelsTab));
});

test("ExtensionsPage keeps the registry visible when installed-extension enrichment fails", () => {
  const refetch = () => {};
  const { CatalogErrorBanner, RegistryTab, rendered } = renderExtensionsPage("registry", {
    extensionsError: new Error("offline"),
    refetch,
  });
  const values = templateValues(rendered);
  const banner = CatalogErrorBanner({
    isPartial: true,
    isRefetching: false,
    onRetry: refetch,
  });
  const text = templateText(banner);

  assert.ok(values.includes(CatalogErrorBanner));
  assert.ok(values.includes(RegistryTab));
  assert.match(text, /Some extension data is unavailable/);
  assert.match(text, /The available extension data is shown/);
  assert.match(text, /--v2-warning-text/);
  assert.doesNotMatch(text, /Extension catalog unavailable/);
});

test("ExtensionsPage blocks installed tabs when the installed-extension query fails", () => {
  const { CatalogErrorBanner, ChannelsTab, rendered } = renderExtensionsPage("channels", {
    extensionsError: new Error("offline"),
  });
  const values = templateValues(rendered);

  assert.ok(values.includes(CatalogErrorBanner));
  assert.ok(!values.includes(ChannelsTab));
});
