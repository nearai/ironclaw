// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";
import { productAuthOAuthEventsSource } from "../../../lib/product-auth-oauth-events.vm-inline";
import { hasChannelSurface } from "../lib/extensions-schema";

// Wire-shaped tool-surface fixture for the surfaces/runtime extension model.
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
  return `${extensionActions}\n${productAuthOAuthEventsSource()}\n${lines.join("\n")}\nglobalThis.__testExports = { useExtensions, useOauthSetup, useSetupSubmit };`;
}

function useExtensionsForTest({ extensions, registry }) {
  const queryData = new Map([
    ["extensions", { extensions }],
    ["extension-registry", { entries: registry }],
    ["gateway-status-extensions", {}],
  ]);
  const context = {
    React: {
      useCallback: (fn) => fn,
      useEffect: () => {},
      useRef: () => ({ current: null }),
      useState: (initial) => [typeof initial === "function" ? initial() : initial, () => {}],
    },
    fetchExtensionRegistry: () => {},
    fetchExtensionSetup: () => {},
    fetchExtensions: () => {},
    gatewayStatus: () => {},
    globalThis: {},
    installExtension: () => {},
    // The real surface-taxonomy helper, so grouping matches production.
    hasChannelSurface,
    removeExtension: () => {},
    startExtensionOauth: () => {},
    submitExtensionSetup: () => {},
    useMutation: () => ({ isPending: false, mutate: () => {} }),
    useQuery: (config) => {
      // Channel discovery rides on the extensions snapshot's `surfaces`; the
      // hook must not resurrect a separate connectable-channels query.
      assert.ok(
        queryData.has(config.queryKey[0]),
        `useExtensions created an unexpected query: ${config.queryKey[0]}`,
      );
      return {
        data: queryData.get(config.queryKey[0]),
        isLoading: false,
        error: null,
        isRefetching: false,
        refetch: () => Promise.resolve(),
      };
    },
    useQueryClient: () => ({ invalidateQueries: () => {} }),
    useT: () => (key, params = {}) =>
      `${key}${params.name ? `:${params.name}` : ""}`,
    window: { clearInterval: () => {}, setInterval: () => 1 },
  };
  vm.runInNewContext(useExtensionsSourceForTest(), context);
  return context.globalThis.__testExports.useExtensions();
}

test("useExtensions merges registry and installed entries with installed first", () => {
  const googleRef = { kind: "extension", id: "google-calendar" };
  const githubRef = { kind: "extension", id: "github" };
  const localRef = { kind: "extension", id: "local-tool" };

  const result = useExtensionsForTest({
    extensions: [
      {
        package_ref: googleRef,
        display_name: "Google Runtime",
        runtime: "wasm",
        surfaces: toolSurfaces,
        installation_state: "active",
      },
      {
        package_ref: localRef,
        display_name: "Local Tool",
        runtime: "wasm",
        surfaces: toolSurfaces,
        installation_state: "active",
      },
      {
        display_name: "Local No ID",
        runtime: "wasm",
        surfaces: toolSurfaces,
        installation_state: "active",
      },
    ],
    registry: [
      {
        package_ref: googleRef,
        display_name: "Google Calendar",
        description: "Calendar access",
        keywords: ["calendar"],
        runtime: "wasm",
        surfaces: toolSurfaces,
        installed: true,
      },
      {
        package_ref: githubRef,
        display_name: "GitHub",
        runtime: "mcp",
        surfaces: toolSurfaces,
        installed: false,
      },
      {
        display_name: "Registry No ID",
        runtime: "wasm",
        surfaces: toolSurfaces,
        installed: false,
      },
    ],
  });

  const { catalogEntries } = result;
  assert.deepEqual(
    Array.from(catalogEntries, (entry) => Boolean(entry.installed)),
    [true, true, true, false, false],
    "installed entries sort ahead of available registry entries",
  );
  assert.equal(
    catalogEntries.filter((entry) => entry.id === "google-calendar").length,
    1,
    "matching registry/runtime entries are de-duplicated",
  );
  const google = catalogEntries.find((entry) => entry.id === "google-calendar");
  assert.equal(google.entry.display_name, "Google Calendar");
  assert.equal(google.extension.display_name, "Google Runtime");
  assert.ok(
    catalogEntries.some((entry) => entry.extension?.package_ref?.id === "local-tool" && !entry.entry),
    "installed entries missing from the registry are retained",
  );
  assert.equal(
    new Set(catalogEntries.map((entry) => entry.id)).size,
    catalogEntries.length,
    "id-less registry and installed entries receive stable fallback ids",
  );
});

test("useExtensions exposes catalog errors and refetches both catalog queries", async () => {
  const catalogError = new Error("catalog unavailable");
  const refetched = [];
  const queryConfigs = [];
  const context = {
    React: {
      useCallback: (fn) => fn,
      useEffect: () => {},
      useRef: () => ({ current: null }),
      useState: (initial) => [typeof initial === "function" ? initial() : initial, () => {}],
    },
    fetchExtensionRegistry: () => {},
    fetchExtensionSetup: () => {},
    fetchExtensions: () => {},
    gatewayStatus: () => {},
    globalThis: {},
    installExtension: () => {},
    // The real surface-taxonomy helper, so grouping matches production.
    hasChannelSurface,
    removeExtension: () => {},
    startExtensionOauth: () => {},
    submitExtensionSetup: () => {},
    useMutation: () => ({ isPending: false, mutate: () => {} }),
    useQuery: (config) => {
      queryConfigs.push(config);
      const { queryKey } = config;
      const key = queryKey[0];
      return {
        data: key === "extensions" ? { extensions: [] } : key === "extension-registry" ? { entries: [] } : {},
        error: key === "extension-registry" ? catalogError : null,
        isLoading: false,
        isRefetching: false,
        refetch: () => {
          refetched.push(key);
          return Promise.resolve();
        },
      };
    },
    useQueryClient: () => ({ invalidateQueries: () => {} }),
    useT: () => (key) => key,
    window: { clearInterval: () => {}, confirm: () => true, setInterval: () => 1 },
  };
  vm.runInNewContext(useExtensionsSourceForTest(), context);
  const result = context.globalThis.__testExports.useExtensions();

  assert.equal(result.extensionsError, null);
  assert.equal(result.registryError, catalogError);
  assert.equal(result.error, catalogError);
  assert.equal(
    queryConfigs.find(({ queryKey }) => queryKey[0] === "extensions")?.networkMode,
    "always",
  );
  assert.equal(
    queryConfigs.find(({ queryKey }) => queryKey[0] === "extension-registry")?.networkMode,
    "always",
  );
  await result.refetch();
  assert.deepEqual(refetched, ["extensions", "extension-registry"]);
});

test("extension mutations preserve stable client action ids through hook payloads", async () => {
  const calls = [];
  let generated = 0;
  const context = {
    React: {
      useCallback: (fn) => fn,
      useEffect: () => {},
      useRef: () => ({ current: null }),
      useState: (initial) => [typeof initial === "function" ? initial() : initial, () => {}],
    },
    approvePairingCode: () => {},
    clientActionId: () => `generated-action-${++generated}`,
    fetchExtensionRegistry: () => {},
    fetchExtensionSetup: () => {},
    fetchExtensions: () => {},
    fetchPairingRequests: () => {},
    gatewayStatus: () => {},
    globalThis: {},
    hasChannelSurface,
    installExtension: (packageRef, options) => {
      calls.push(["install", packageRef, options]);
      return Promise.resolve({ success: true });
    },
    removeExtension: (packageRef, options) => {
      calls.push(["remove", packageRef, options]);
      return Promise.resolve({ success: true });
    },
    startExtensionOauth: () => {},
    submitExtensionSetup: (packageRef, secrets, options) => {
      calls.push(["setup", packageRef, secrets, options]);
      return Promise.resolve({ success: true });
    },
    useMutation: (config) => ({
      isPending: false,
      mutate: (payload, options) => config.mutationFn(payload, options),
      mutateAsync: (payload, options) => config.mutationFn(payload, options),
    }),
    useQuery: (config) => ({
      data:
        config.queryKey[0] === "extensions"
          ? { extensions: [] }
          : config.queryKey[0] === "extension-registry"
            ? { entries: [] }
            : {},
      isLoading: false,
      error: null,
      isRefetching: false,
      refetch: () => Promise.resolve(),
    }),
    useQueryClient: () => ({
      invalidateQueries: () => {},
    }),
    useT: () => (key) => key,
    window: { clearInterval: () => {}, setInterval: () => 1 },
  };
  vm.runInNewContext(useExtensionsSourceForTest(), context);

  const packageRef = { kind: "extension", id: "github" };
  const extensions = context.globalThis.__testExports.useExtensions();
  await extensions.install({ packageRef });
  await extensions.remove({
    packageRef,
    clientActionId: "caller-remove-action",
  });

  const setup = context.globalThis.__testExports.useSetupSubmit(packageRef);
  await setup.mutate({ secrets: {} });

  assert.deepEqual(JSON.parse(JSON.stringify(calls)), [
    ["install", packageRef, { clientActionId: "generated-action-1" }],
    ["remove", packageRef, { clientActionId: "caller-remove-action" }],
    ["setup", packageRef, {}, { clientActionId: "generated-action-2" }],
  ]);
});

function installMutationHarness(authoritativeExtension) {
  const mutationConfigs = [];
  const openCalls = [];
  const setupRequests = [];
  const refetches = [];
  const context = {
    Date,
    Error,
    URL,
    React: {
      useCallback: (fn) => fn,
      useEffect: () => {},
      useRef: () => ({ current: null }),
      useState: (initial) => [typeof initial === "function" ? initial() : initial, () => {}],
    },
    fetchExtensionRegistry: () => {},
    fetchExtensionSetup: () => {},
    fetchExtensions: () => {},
    gatewayStatus: () => {},
    globalThis: {},
    installExtension: () => {},
    hasChannelSurface,
    removeExtension: () => {},
    startExtensionOauth: () => {},
    submitExtensionSetup: () => {},
    useMutation: (config) => {
      mutationConfigs.push(config);
      return { isPending: false, mutate: () => {} };
    },
    useQuery: (config) => ({
      data: config.queryKey[0] === "extension-registry" ? { entries: [] } : {},
      isLoading: false,
      error: null,
      isRefetching: false,
      refetch: () => {
        refetches.push(config.queryKey[0]);
        return Promise.resolve(
          config.queryKey[0] === "extensions"
            ? { data: { extensions: authoritativeExtension ? [authoritativeExtension] : [] } }
            : { data: config.queryKey[0] === "extension-registry" ? { entries: [] } : {} },
        );
      },
    }),
    useQueryClient: () => ({ invalidateQueries: () => {} }),
    useT: () => (key) => key,
    window: {
      clearInterval: () => {},
      setInterval: () => 1,
      open: (url, target, features) => {
        openCalls.push({ url, target, features });
        return null;
      },
    },
  };
  vm.runInNewContext(useExtensionsSourceForTest(), context);
  context.globalThis.__testExports.useExtensions();
  return {
    installConfig: mutationConfigs[0],
    openCalls,
    refetches,
    setupRequests,
    onNeedsSetup: (request) => setupRequests.push(request),
  };
}

test("install refreshes the authoritative projection before opening setup", async () => {
  const packageRef = { kind: "extension", id: "notion" };
  const authoritativeExtension = {
    package_ref: packageRef,
    display_name: "Notion",
    installation_state: "setup_needed",
    surfaces: [{ kind: "tool" }, { kind: "auth" }],
  };
  const harness = installMutationHarness(authoritativeExtension);

  await harness.installConfig.onSuccess(
    { success: true },
    {
      packageRef,
      displayName: "Notion",
      surfaces: toolSurfaces,
      onNeedsSetup: harness.onNeedsSetup,
    },
  );

  assert.deepEqual(harness.refetches, [
    "extensions",
    "extension-registry",
  ]);
  assert.equal(harness.openCalls.length, 0, "install responses must never launch OAuth");
  assert.equal(harness.setupRequests.length, 1);
  assert.equal(
    harness.setupRequests[0].installation_state,
    "setup_needed",
  );
  assert.deepEqual(harness.setupRequests[0].packageRef, packageRef);
});

test("install does not infer setup from catalog surfaces when the projection is active", async () => {
  const packageRef = { kind: "extension", id: "notion" };
  const harness = installMutationHarness({
    package_ref: packageRef,
    display_name: "Notion",
    installation_state: "active",
    surfaces: [{ kind: "tool" }, { kind: "auth" }],
  });

  await harness.installConfig.onSuccess(
    { success: true },
    {
      packageRef,
      displayName: "Notion",
      surfaces: [{ kind: "tool" }, { kind: "auth" }],
      onNeedsSetup: harness.onNeedsSetup,
    },
  );

  assert.equal(harness.setupRequests.length, 0);
  assert.equal(harness.openCalls.length, 0);
});

test("install does not fabricate setup when the authoritative extension is absent", async () => {
  const packageRef = { kind: "extension", id: "notion" };
  const harness = installMutationHarness(null);

  await harness.installConfig.onSuccess(
    { success: true },
    {
      packageRef,
      displayName: "Notion",
      surfaces: [{ kind: "tool" }, { kind: "auth" }],
      onNeedsSetup: harness.onNeedsSetup,
    },
  );

  assert.equal(harness.setupRequests.length, 0);
  assert.equal(harness.openCalls.length, 0);
});

test("useOauthSetup waits for flow completion after the OAuth popup closes", async () => {
  const mutationConfigs = [];
  const intervalCallbacks = [];
  const invalidations = [];
  const flowStatusRequests = [];
  let configuredCalls = 0;
  const queryData = new Map([
    [
      "extension-setup",
      {
        secrets: [{ provided: false }],
      },
    ],
    [
      "extensions",
      {
        extensions: [
          {
            package_ref: { id: "slack" },
            installation_state: "setup_needed",
          },
        ],
      },
    ],
  ]);
  const popup = {
    closed: false,
    close() {
      this.closed = true;
    },
    location: { href: "about:blank" },
  };
  const context = {
    Date,
    Error,
    URL,
    React: {
      useCallback: (fn) => fn,
      useEffect: () => {},
      useRef: (initial) => ({ current: initial ?? null }),
      useState: (initial) => [
        typeof initial === "function" ? initial() : initial,
        () => {},
      ],
    },
    fetchExtensionRegistry: () => {},
    fetchExtensionSetup: () => {},
    fetchExtensions: () => {},
    fetchOauthFlowStatus: (flowId, invocationId) => {
      flowStatusRequests.push({ flowId, invocationId });
      return { status: "completed" };
    },
    gatewayStatus: () => {},
    globalThis: {},
    hasChannelSurface,
    installExtension: () => {},
    removeExtension: () => {},
    startExtensionOauth: () => {},
    submitExtensionSetup: () => {},
    useMutation: (config) => {
      mutationConfigs.push(config);
      return { isPending: false, mutate: () => {} };
    },
    useQuery: () => ({ data: {}, isLoading: false }),
    useQueryClient: () => ({
      getQueryData: (queryKey) => queryData.get(queryKey[0]),
      invalidateQueries: ({ queryKey }) => invalidations.push(queryKey),
    }),
    useT: () => (key) => key,
    window: {
      clearInterval: () => {},
      setInterval: (callback) => {
        intervalCallbacks.push(callback);
        return intervalCallbacks.length;
      },
      addEventListener: () => {},
      removeEventListener: () => {},
      localStorage: { getItem: () => null },
    },
  };
  vm.runInNewContext(useExtensionsSourceForTest(), context);

  context.globalThis.__testExports.useOauthSetup(
    { id: "slack" },
    {
      onConfigured: () => {
        configuredCalls += 1;
      },
    },
  );
  assert.equal(mutationConfigs.length, 1);

  mutationConfigs[0].onSuccess(
    {
      res: {
        authorization_url: "https://slack.com/oauth/v2/authorize",
        flow_id: "flow-slack-1",
        callback_scope: { invocation_id: "invocation-slack-1" },
      },
      popup,
    },
    { secret: { provided: false } },
  );
  popup.closed = true;

  assert.equal(intervalCallbacks.length, 1);
  intervalCallbacks[0]();
  await Promise.resolve();
  await Promise.resolve();

  assert.deepEqual(flowStatusRequests, [
    { flowId: "flow-slack-1", invocationId: "invocation-slack-1" },
  ]);
  assert.equal(
    configuredCalls,
    1,
    "durable flow completion should trigger activation even after the callback popup closes",
  );
});
