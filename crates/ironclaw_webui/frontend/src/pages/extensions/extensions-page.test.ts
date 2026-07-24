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

function visit(node, fn) {
  if (Array.isArray(node)) {
    for (const item of node) visit(item, fn);
    return;
  }
  if (!node || typeof node !== "object") return;
  fn(node);
  visit(node.values, fn);
}

function componentProps(root, component) {
  const props = [];
  visit(root, (node) => {
    if (!Array.isArray(node.values)) return;
    for (let index = 0; index < node.values.length; index += 1) {
      if (node.values[index] !== component) continue;
      const current = {};
      for (let propIndex = index + 1; propIndex < node.values.length; propIndex += 1) {
        const name = node.strings[propIndex]?.match(/([A-Za-z][A-Za-z0-9-]*)=\s*$/)?.[1];
        if (name) current[name] = node.values[propIndex];
      }
      props.push(current);
    }
  });
  return props;
}

function renderExtensionsPage(tab, extensionState = {}) {
  const hookValues = [];
  let hookCursor = 0;
  const removeCalls = [];
  function ConfirmDialog() {}
  function RegistryTab() {}
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
    ConfirmDialog,
    ConfigureModal() {},
    ToolsTab() {},
    Navigate() {},
    React: {
      useCallback: (fn) => fn,
      useState: (initial) => {
        const index = hookCursor;
        hookCursor += 1;
        if (!(index in hookValues)) {
          hookValues[index] = typeof initial === "function" ? initial() : initial;
        }
        return [hookValues[index], (next) => {
          hookValues[index] = typeof next === "function" ? next(hookValues[index]) : next;
        }];
      },
    },
    RegistryTab,
    globalThis: {},
    html(strings, ...values) {
      return { strings: Array.from(strings), values };
    },
    useExtensions: () => ({
      status: {},
      channels: [],
      tools: [],
      channelRegistry: [],
      toolRegistry: [],
      catalogEntries: [],
      isExtensionsLoading: false,
      isRegistryLoading: false,
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
      remove: (...args) => removeCalls.push(args),
      isRemoving: false,
      invalidate: () => {},
      ...extensionState,
    }),
    useParams: () => ({ tab }),
    useT: () => (key) => translations[key] || key,
  };
  vm.runInNewContext(extensionsPageSourceForTest(), context);
  const render = () => {
    hookCursor = 0;
    return context.globalThis.__testExports.ExtensionsPage();
  };
  return {
    ...context,
    removeCalls,
    render,
    CatalogErrorBanner: context.globalThis.__testExports.CatalogErrorBanner,
    rendered: render(),
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
  test(`ExtensionsPage redirects ${tab} tab before waiting for data`, () => {
    const { Navigate, rendered } = renderExtensionsPage(tab, {
      isExtensionsLoading: true,
      isRegistryLoading: true,
    });

    assert.equal(rendered.values[0], Navigate);
    assert.match(rendered.strings.join(""), /to="\/extensions\/registry"/);
  });
}

test("ExtensionsPage redirects the legacy mcp tab to the tools view", () => {
  const { Navigate, rendered } = renderExtensionsPage("mcp", {
    isExtensionsLoading: true,
    isRegistryLoading: true,
  });

  assert.equal(rendered.values[0], Navigate);
  assert.match(rendered.strings.join(""), /to="\/extensions\/tools"/);
});

test("ExtensionsPage renders the tools view for the tools tab", () => {
  const { ToolsTab, rendered } = renderExtensionsPage("tools", {
    isExtensionsLoading: false,
    isRegistryLoading: false,
  });

  const toolsTab = findComponent(rendered, ToolsTab) || componentProps(rendered, ToolsTab)[0];
  assert.ok(toolsTab, "the tools tab content must be rendered");
});

test("ExtensionsPage removes an extension only after confirming the shared dialog", () => {
  const harness = renderExtensionsPage("registry", { isBusy: true, isRemoving: false });
  const [registry] = componentProps(harness.rendered, harness.RegistryTab);
  const extension = {
    displayName: "GitHub",
    packageRef: { kind: "extension", id: "github" },
  };

  registry.onRemove(extension);
  assert.deepEqual(harness.removeCalls, []);

  const rendered = harness.render();
  const [dialog] = componentProps(rendered, harness.ConfirmDialog);
  assert.equal(dialog.open, true);
  assert.equal(dialog.title, "common.remove: GitHub");
  assert.equal(dialog.isConfirming, false);

  dialog.onConfirm();
  assert.equal(harness.removeCalls.length, 1);
  assert.equal(harness.removeCalls[0][0], extension);
  assert.equal(typeof harness.removeCalls[0][1].onSettled, "function");
});

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
  const refetch = () => {};
  const { CatalogErrorBanner, ChannelsTab, rendered } = renderExtensionsPage("channels", {
    registryError: new Error("offline"),
    refetch,
  });
  const values = templateValues(rendered);

  assert.ok(values.includes(CatalogErrorBanner));
  assert.ok(values.includes(ChannelsTab));

  // A catalog (registry) failure must surface the full "Extension catalog
  // unavailable" message even on a non-registry tab — the banner text follows
  // the failure cause, not the tab. Regression: previously the inline banner on
  // the channels tab hardcoded the partial "Some extension data" text.
  const [bannerProps] = componentProps(rendered, CatalogErrorBanner);
  assert.equal(bannerProps.isCatalogError, true);
  const text = templateText(
    CatalogErrorBanner({ isCatalogError: true, isRefetching: false, onRetry: refetch }),
  );
  assert.match(text, /Extension catalog unavailable/);
  assert.match(text, /--v2-danger-text/);
  assert.doesNotMatch(text, /Some extension data is unavailable/);
});

test("ExtensionsPage keeps the registry visible when installed-extension enrichment fails", () => {
  const refetch = () => {};
  const { CatalogErrorBanner, RegistryTab, rendered } = renderExtensionsPage("registry", {
    extensionsError: new Error("offline"),
    refetch,
  });
  const values = templateValues(rendered);
  const banner = CatalogErrorBanner({
    isCatalogError: false,
    isRefetching: false,
    onRetry: refetch,
  });
  const text = templateText(banner);

  assert.ok(values.includes(CatalogErrorBanner));
  assert.ok(values.includes(RegistryTab));
  // The inline banner on the registry tab reflects the enrichment failure cause.
  const [bannerProps] = componentProps(rendered, CatalogErrorBanner);
  assert.equal(bannerProps.isCatalogError, false);
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
