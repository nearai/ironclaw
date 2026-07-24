// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";
import { productAuthOAuthEventsSource } from "../../../lib/product-auth-oauth-events.vm-inline";
import { hasChannelSurface } from "../lib/extensions-schema";

// Wire-shaped surface fixtures for the surfaces/runtime extension model.
const channelSurfaces = [{ kind: "channel", inbound: true, outbound: true }];
const toolSurfaces = [{ kind: "tool" }];

function useExtensionsSourceForTest() {
  const extensionActions = readFileSync(
    new URL("../lib/extension-actions.ts", import.meta.url),
    "utf8",
  ).replaceAll("export function ", "function ");
  const source = readFileSync(new URL("./useExtensions.ts", import.meta.url), "utf8");
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
  return `${extensionActions}\n${productAuthOAuthEventsSource()}\n${lines.join("\n")}\nglobalThis.__testExports = { useExtensions };`;
}

function contextFor(mutationState, queryCalls) {
  return {
    React: { useCallback: (fn) => fn, useEffect: () => {}, useRef: () => ({ current: null }), useState: () => [null, () => {}] },
    fetchExtensionRegistry: () => {},
    fetchExtensionSetup: () => {},
    fetchExtensions: () => {},
    gatewayStatus: () => {},
    globalThis: {},
    // The real surface-taxonomy helper: grouping and install routing must key
    // off declared channel surfaces, exactly as production does.
    hasChannelSurface,
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

test("useExtensions preserves the server install result message", async () => {
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
        return {
          data: { extensions: [] },
          isLoading: false,
          refetch: () => Promise.resolve({ data: { extensions: [] } }),
        };
      }
      if (queryKey[0] === "extension-registry") {
        return {
          data: { entries: [] },
          isLoading: false,
          refetch: () => Promise.resolve({ data: { entries: [] } }),
        };
      }
      return {
        data: {},
        isLoading: false,
        refetch: () => Promise.resolve({ data: {} }),
      };
    },
  };
  vm.runInNewContext(useExtensionsSourceForTest(), context);

  context.globalThis.__testExports.useExtensions();
  await mutationConfigs[0].onSuccess(
    { success: true, message: "Slack installed. Complete setup to publish its tools." },
    { displayName: "Slack", surfaces: channelSurfaces }
  );

  assert.deepEqual(JSON.parse(JSON.stringify(actionResults[0])), {
    type: "success",
    message: "Slack installed. Complete setup to publish its tools.",
  });
});

test("useExtensions hands setup the authoritative installed channel projection", async () => {
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
      if (queryKey[0] === "extensions") {
        return {
          data: { extensions: [] },
          isLoading: false,
          refetch: () =>
            Promise.resolve({
              data: {
                extensions: [
                  {
                    package_ref: { kind: "extension", id: "slack" },
                    display_name: "Slack",
                    installation_state: "setup_needed",
                    surfaces: channelSurfaces,
                  },
                ],
              },
            }),
        };
      }
      if (queryKey[0] === "extension-registry") {
        return {
          data: { entries: [] },
          isLoading: false,
          refetch: () => Promise.resolve({ data: { entries: [] } }),
        };
      }
      return {
        data: {},
        isLoading: false,
        refetch: () => Promise.resolve({ data: {} }),
      };
    },
  };
  vm.runInNewContext(useExtensionsSourceForTest(), context);

  context.globalThis.__testExports.useExtensions();
  await mutationConfigs[0].onSuccess(
    { success: true },
    {
      displayName: "Slack",
      packageRef: { kind: "extension", id: "slack" },
      onNeedsSetup: (payload) => needsSetupPayloads.push(payload),
    }
  );

  assert.equal(needsSetupPayloads.length, 1, "install-configure must open the modal");
  assert.deepEqual(needsSetupPayloads[0].surfaces, channelSurfaces);
  assert.equal(needsSetupPayloads[0].installation_state, "setup_needed");
});

test("useExtensions places uninstalled wasm channel-surface registry entry in channelRegistry not toolRegistry", () => {
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
              {
                runtime: "wasm",
                surfaces: channelSurfaces,
                package_ref: { id: "telegram" },
                installed: false,
              },
            ],
          },
          isLoading: false,
        };
      }
      return { data: {}, isLoading: false };
    },
  };
  vm.runInNewContext(useExtensionsSourceForTest(), context);

  const extensions = context.globalThis.__testExports.useExtensions();

  assert.deepEqual(
    extensions.channelRegistry.map((entry) => entry.package_ref.id),
    ["telegram"],
    "wasm channel-surface registry entry must appear in channelRegistry"
  );
  assert.deepEqual(
    extensions.toolRegistry.map((entry) => entry.package_ref.id),
    [],
    "wasm channel-surface registry entry must NOT appear in toolRegistry"
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
              {
                runtime: "first_party",
                surfaces: channelSurfaces,
                package_ref: { id: "slack" },
              },
              {
                runtime: "wasm",
                surfaces: channelSurfaces,
                package_ref: { id: "telegram" },
              },
              { runtime: "wasm", surfaces: toolSurfaces, package_ref: { id: "github" } },
              { runtime: "mcp", surfaces: toolSurfaces, package_ref: { id: "notion" } },
            ],
          },
          isLoading: false,
        };
      }
      if (queryKey[0] === "extension-registry") {
        return {
          data: {
            entries: [
              {
                runtime: "first_party",
                surfaces: channelSurfaces,
                package_ref: { id: "slack" },
                installed: false,
              },
              {
                runtime: "wasm",
                surfaces: toolSurfaces,
                package_ref: { id: "web-access" },
                installed: false,
              },
            ],
          },
          isLoading: false,
        };
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
    ["github", "notion"],
    "tools = every non-channel extension; MCP-backed tools sit beside wasm ones"
  );
  assert.equal(
    extensions.mcpServers,
    undefined,
    "runtime is a badge, never a grouping axis — no mcpServers rail"
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
