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

function renderExtensionsPage(tab, { isBusy = false, isRemoving = false } = {}) {
  const hookValues = [];
  let hookCursor = 0;
  const removeCalls = [];
  function ConfirmDialog() {}
  function RegistryTab() {}
  const context = {
    ActionToast() {},
    ChannelsTab() {},
    ConfirmDialog,
    ConfigureModal() {},
    McpTab() {},
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
      mcpServers: [],
      channelRegistry: [],
      mcpRegistry: [],
      catalogEntries: [],
      connectableChannels: [],
      isLoading: false,
      isBusy,
      actionResult: null,
      clearResult: () => {},
      install: () => {},
      activate: () => {},
      remove: (...args) => removeCalls.push(args),
      isRemoving,
      invalidate: () => {},
    }),
    useParams: () => ({ tab }),
    useT: () => (key) => key,
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
    rendered: render(),
  };
}

for (const tab of ["installed", "unknown"]) {
  test(`ExtensionsPage redirects ${tab} tab to registry`, () => {
    const { Navigate, rendered } = renderExtensionsPage(tab);

    assert.equal(rendered.values[0], Navigate);
    assert.match(rendered.strings.join(""), /to="\/extensions\/registry"/);
  });
}

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
