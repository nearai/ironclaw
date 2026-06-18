import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function extensionsPageSourceForTest() {
  const source = readFileSync(new URL("./extensions-page.js", import.meta.url), "utf8");
  const lines = [];
  for (const line of source.split("\n")) {
    if (line.startsWith("import ")) continue;
    lines.push(line.replace(/^export function /, "function "));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { ExtensionsPage };`;
}

function renderExtensionsPage(tab) {
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
      isBusy: false,
      actionResult: null,
      clearResult: () => {},
      install: () => {},
      activate: () => {},
      remove: () => {},
      invalidate: () => {},
    }),
    useParams: () => ({ tab }),
  };
  vm.runInNewContext(extensionsPageSourceForTest(), context);
  return {
    ...context,
    rendered: context.globalThis.__testExports.ExtensionsPage(),
  };
}

for (const tab of ["installed", "unknown"]) {
  test(`ExtensionsPage redirects ${tab} tab to registry`, () => {
    const { Navigate, rendered } = renderExtensionsPage(tab);

    assert.equal(rendered.values[0], Navigate);
    assert.match(rendered.strings.join(""), /to="\/extensions\/registry"/);
  });
}
