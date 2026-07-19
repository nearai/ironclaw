// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";
import { productAuthOAuthEventsSource } from "../../../lib/product-auth-oauth-events.vm-inline";

function useExtensionsSourceForTest() {
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
  return `${productAuthOAuthEventsSource()}\n${lines.join("\n")}\nglobalThis.__testExports = { useExtensions, useOauthSetup };`;
}

function useExtensionsForTest({ extensions, registry }) {
  const queryData = new Map([
    ["extensions", { extensions }],
    ["extension-registry", { entries: registry }],
    ["connectable-channels", { channels: [] }],
    ["gateway-status-extensions", {}],
  ]);
  const context = {
    React: {
      useCallback: (fn) => fn,
      useEffect: () => {},
      useRef: () => ({ current: null }),
      useState: (initial) => [typeof initial === "function" ? initial() : initial, () => {}],
    },
    activateExtension: () => {},
    approvePairingCode: () => {},
    fetchExtensionRegistry: () => {},
    fetchExtensionSetup: () => {},
    fetchExtensions: () => {},
    fetchPairingRequests: () => {},
    gatewayStatus: () => {},
    globalThis: {},
    installExtension: () => {},
    isChannelExtensionKind: (kind) => kind === "wasm_channel" || kind === "channel",
    listConnectableChannels: () => {},
    removeExtension: () => {},
    startExtensionOauth: () => {},
    submitExtensionSetup: () => {},
    useMutation: () => ({ isPending: false, mutate: () => {} }),
    useQuery: (config) => ({
      data: queryData.get(config.queryKey[0]) || {},
      isLoading: false,
      error: null,
      isRefetching: false,
      refetch: () => Promise.resolve(),
    }),
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
        kind: "wasm_tool",
        active: true,
      },
      {
        package_ref: localRef,
        display_name: "Local Tool",
        kind: "wasm_tool",
        active: true,
      },
      {
        display_name: "Local No ID",
        kind: "wasm_tool",
        active: true,
      },
    ],
    registry: [
      {
        package_ref: googleRef,
        display_name: "Google Calendar",
        description: "Calendar access",
        keywords: ["calendar"],
        kind: "wasm_tool",
        installed: true,
      },
      {
        package_ref: githubRef,
        display_name: "GitHub",
        kind: "mcp_server",
        installed: false,
      },
      {
        display_name: "Registry No ID",
        kind: "wasm_tool",
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
    activateExtension: () => {},
    approvePairingCode: () => {},
    fetchExtensionRegistry: () => {},
    fetchExtensionSetup: () => {},
    fetchExtensions: () => {},
    fetchPairingRequests: () => {},
    gatewayStatus: () => {},
    globalThis: {},
    installExtension: () => {},
    isChannelExtensionKind: () => false,
    listConnectableChannels: () => {},
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

test("install/activate auth popups: noopener null is not an error; insecure URLs are", () => {
  const stateUpdates = [];
  const mutationConfigs = [];
  const openCalls = [];
  const context = {
    Date,
    Error,
    URL,
    React: {
      useCallback: (fn) => fn,
      useEffect: () => {},
      useRef: () => ({ current: null }),
      useState: (initial) => [
        typeof initial === "function" ? initial() : initial,
        (value) => stateUpdates.push(value),
      ],
    },
    activateExtension: () => {},
    approvePairingCode: () => {},
    fetchExtensionRegistry: () => {},
    fetchExtensionSetup: () => {},
    fetchExtensions: () => {},
    fetchPairingRequests: () => {},
    gatewayStatus: () => {},
    globalThis: {},
    installExtension: () => {},
    isChannelExtensionKind: () => false,
    listConnectableChannels: () => {},
    removeExtension: () => {},
    startExtensionOauth: () => {},
    submitExtensionSetup: () => {},
    useMutation: (config) => {
      mutationConfigs.push(config);
      return { isPending: false, mutate: () => {} };
    },
    useQuery: () => ({ data: {}, isLoading: false }),
    useQueryClient: () => ({ invalidateQueries: () => {} }),
    useT: () => (key) => key,
    // Spec-compliant browser: window.open with "noopener" returns null EVEN
    // when the popup opens, so null on this branch must not surface an error.
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

  // useExtensions declares its mutations in a fixed order: install first,
  // activate second (same order-coupling convention the other vm tests use).
  const [installConfig, activateConfig] = mutationConfigs;
  const lastError = () =>
    stateUpdates.filter((value) => value && value.type === "error").at(-1);

  installConfig.onSuccess(
    { success: true, auth_url: "https://slack.com/oauth/v2/authorize" },
    { displayName: "Slack", kind: "extension" },
  );
  assert.equal(lastError(), undefined, "noopener null must not read as a blocked popup");
  // The fresh open must pass the full hardened argument set (see
  // .claude/rules/testing.md mock-hygiene: assert EVERY argument the
  // production call passes — dropping "noopener" would be a security bug).
  assert.deepEqual(openCalls.at(-1), {
    url: "https://slack.com/oauth/v2/authorize",
    target: "_blank",
    features: "noopener,noreferrer",
  });

  activateConfig.onSuccess(
    { success: false, auth_url: "https://slack.com/oauth/v2/authorize" },
    { displayName: "Slack" },
  );
  assert.equal(lastError(), undefined);

  // A genuinely non-HTTPS URL still reports the HTTPS problem.
  activateConfig.onSuccess(
    { success: false, auth_url: "http://insecure.example/authorize" },
    { displayName: "Slack" },
  );
  assert.match(lastError().message, /HTTPS/);
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
            active: false,
            authenticated: false,
            needs_setup: true,
            has_auth: true,
            activation_status: "installed",
            onboarding_state: "auth_required",
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
    activateExtension: () => {},
    approvePairingCode: () => {},
    fetchExtensionRegistry: () => {},
    fetchExtensionSetup: () => {},
    fetchExtensions: () => {},
    fetchOauthFlowStatus: (flowId, invocationId) => {
      flowStatusRequests.push({ flowId, invocationId });
      return { status: "completed" };
    },
    fetchPairingRequests: () => {},
    gatewayStatus: () => {},
    globalThis: {},
    installExtension: () => {},
    isChannelExtensionKind: () => false,
    listConnectableChannels: () => {},
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
