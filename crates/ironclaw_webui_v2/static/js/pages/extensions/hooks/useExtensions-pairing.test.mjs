import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function useExtensionsSourceForTest() {
  const source = readFileSync(new URL("./useExtensions.js", import.meta.url), "utf8");
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
  return `${lines.join("\n")}\nglobalThis.__testExports = { useExtensions };`;
}

function contextFor(mutationState, queryCalls) {
  return {
    React: { useCallback: (fn) => fn, useEffect: () => {}, useRef: () => ({ current: null }), useState: () => [null, () => {}] },
    activateExtension: () => {},
    approvePairingCode: () => {},
    fetchExtensionRegistry: () => {},
    fetchExtensionSetup: () => {},
    fetchExtensions: () => {},
    listConnectableChannels: () => {},
    fetchPairingRequests: () => {},
    gatewayStatus: () => {},
    globalThis: {},
    isChannelExtensionKind: (kind) => kind === "wasm_channel" || kind === "channel",
    installExtension: () => {},
    removeExtension: () => {},
    startExtensionOauth: () => {},
    submitExtensionSetup: () => {},
    useMutation: () => mutationState,
    useQuery: (config) => {
      queryCalls.push(config);
      return { data: { requests: [] }, isLoading: false };
    },
    useQueryClient: () => ({ invalidateQueries: () => {} }),
    useT: () => (key, params = {}) =>
      `${key}${params.name ? `:${params.name}` : ""}`,
  };
}

test("useExtensions shows setup-panel copy after channel install succeeds", () => {
  const mutationConfigs = [];
  const actionResults = [];
  const context = {
    ...contextFor(
      { mutate: () => {}, isPending: false, isSuccess: false, isError: false },
      []
    ),
    React: {
      useCallback: (fn) => fn,
      useEffect: () => {},
      useRef: () => ({ current: null }),
      useState: (initial) => [initial, (value) => actionResults.push(value)],
    },
    useMutation: (config) => {
      mutationConfigs.push(config);
      return { mutate: () => {}, isPending: false, isSuccess: false, isError: false };
    },
    useQuery: ({ queryKey }) => {
      if (queryKey[0] === "extensions") {
        return { data: { extensions: [] }, isLoading: false };
      }
      if (queryKey[0] === "extension-registry") {
        return { data: { entries: [] }, isLoading: false };
      }
      if (queryKey[0] === "connectable-channels") {
        return { data: { channels: [] }, isLoading: false };
      }
      return { data: {}, isLoading: false };
    },
  };
  vm.runInNewContext(useExtensionsSourceForTest(), context);

  context.globalThis.__testExports.useExtensions();
  mutationConfigs[0].onSuccess(
    { success: true, message: "Slack is installed. Activate it to make its tools available." },
    { displayName: "Slack", kind: "channel" }
  );

  assert.deepEqual(JSON.parse(JSON.stringify(actionResults[0])), {
    type: "success",
    message: "Slack installed. Connect the account using the setup panel below.",
  });
});

test("useExtensions install→configure hands the modal the channel kind (so it shows Connect, not 'no config')", () => {
  const mutationConfigs = [];
  const needsSetupPayloads = [];
  const context = {
    ...contextFor(
      { mutate: () => {}, isPending: false, isSuccess: false, isError: false },
      []
    ),
    React: {
      useCallback: (fn) => fn,
      useEffect: () => {},
      useRef: () => ({ current: null }),
      useState: (initial) => [initial, () => {}],
    },
    useMutation: (config) => {
      mutationConfigs.push(config);
      return { mutate: () => {}, isPending: false, isSuccess: false, isError: false };
    },
    useQuery: ({ queryKey }) => {
      if (queryKey[0] === "extensions") return { data: { extensions: [] }, isLoading: false };
      if (queryKey[0] === "extension-registry") return { data: { entries: [] }, isLoading: false };
      if (queryKey[0] === "connectable-channels") return { data: { channels: [] }, isLoading: false };
      return { data: {}, isLoading: false };
    },
  };
  vm.runInNewContext(useExtensionsSourceForTest(), context);

  context.globalThis.__testExports.useExtensions();
  // Install a connectable channel with auto-configure — the modal is opened via
  // onNeedsSetup. Its payload MUST carry `kind` or the modal cannot tell it is a
  // channel and wrongly renders "No configuration required".
  mutationConfigs[0].onSuccess(
    { success: true },
    {
      displayName: "Slack",
      kind: "channel",
      packageRef: { kind: "extension", id: "slack" },
      configureAfterInstall: true,
      onNeedsSetup: (payload) => needsSetupPayloads.push(payload),
    }
  );

  assert.equal(needsSetupPayloads.length, 1, "install-configure must open the modal");
  assert.equal(needsSetupPayloads[0].kind, "channel");
  assert.equal(needsSetupPayloads[0].authenticated, false);
});

test("useExtensions places uninstalled wasm_channel registry entry in channelRegistry not toolRegistry", () => {
  const context = {
    ...contextFor(
      { mutate: () => {}, isPending: false, isSuccess: false, isError: false },
      []
    ),
    React: {
      useCallback: (fn) => fn,
      useEffect: () => {},
      useRef: () => ({ current: null }),
      useState: (initial) => [initial, () => {}],
    },
    useQuery: ({ queryKey }) => {
      if (queryKey[0] === "extensions") {
        return { data: { extensions: [] }, isLoading: false };
      }
      if (queryKey[0] === "extension-registry") {
        return {
          data: {
            entries: [
              { kind: "wasm_channel", package_ref: { id: "telegram" }, installed: false },
            ],
          },
          isLoading: false,
        };
      }
      if (queryKey[0] === "connectable-channels") {
        return { data: { channels: [] }, isLoading: false };
      }
      return { data: {}, isLoading: false };
    },
  };
  vm.runInNewContext(useExtensionsSourceForTest(), context);

  const extensions = context.globalThis.__testExports.useExtensions();

  assert.deepEqual(
    extensions.channelRegistry.map((entry) => entry.package_ref.id),
    ["telegram"],
    "wasm_channel registry entry must appear in channelRegistry"
  );
  assert.deepEqual(
    extensions.toolRegistry.map((entry) => entry.package_ref.id),
    [],
    "wasm_channel registry entry must NOT appear in toolRegistry"
  );
});

test("useExtensions groups manifest-backed channels with channel entries", () => {
  const context = {
    ...contextFor(
      { mutate: () => {}, isPending: false, isSuccess: false, isError: false },
      []
    ),
    React: {
      useCallback: (fn) => fn,
      useEffect: () => {},
      useRef: () => ({ current: null }),
      useState: (initial) => [initial, () => {}],
    },
    useQuery: ({ queryKey }) => {
      if (queryKey[0] === "extensions") {
        return {
          data: {
            extensions: [
              { kind: "channel", package_ref: { id: "slack" } },
              { kind: "wasm_channel", package_ref: { id: "telegram" } },
              { kind: "wasm_tool", package_ref: { id: "github" } },
            ],
          },
          isLoading: false,
        };
      }
      if (queryKey[0] === "extension-registry") {
        return {
          data: {
            entries: [
              { kind: "channel", package_ref: { id: "slack" }, installed: false },
              { kind: "wasm_tool", package_ref: { id: "web-access" }, installed: false },
            ],
          },
          isLoading: false,
        };
      }
      if (queryKey[0] === "connectable-channels") {
        return { data: { channels: [] }, isLoading: false };
      }
      return { data: {}, isLoading: false };
    },
  };
  vm.runInNewContext(useExtensionsSourceForTest(), context);

  const extensions = context.globalThis.__testExports.useExtensions();

  assert.deepEqual(
    extensions.channels.map((entry) => entry.package_ref.id),
    ["slack", "telegram"]
  );
  assert.deepEqual(
    extensions.tools.map((entry) => entry.package_ref.id),
    ["github"]
  );
  assert.deepEqual(
    extensions.channelRegistry.map((entry) => entry.package_ref.id),
    ["slack"]
  );
  assert.deepEqual(
    extensions.toolRegistry.map((entry) => entry.package_ref.id),
    ["web-access"]
  );
});
