import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function useExtensionsOauthSourceForTest() {
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
  return `${lines.join("\n")}\nglobalThis.__testExports = { useOauthSetup };`;
}

test("useOauthSetup exposes the popup-watcher phase as authorizing", () => {
  const stateUpdates = [];
  const intervals = [];
  let mutationConfig = null;
  let stateIndex = 0;
  const popup = { closed: false, location: { href: "about:blank" } };
  const context = {
    Date,
    Error,
    React: {
      useCallback: (fn) => fn,
      useEffect: () => {},
      useRef: (initial) => ({ current: initial }),
      useState: (initial) => {
        const index = stateIndex++;
        return [
          typeof initial === "function" ? initial() : initial,
          (value) => stateUpdates.push({ index, value }),
        ];
      },
    },
    URL,
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
      mutationConfig = config;
      return { isPending: false, mutate: () => {}, error: null };
    },
    useQuery: () => ({ data: {}, isLoading: false }),
    useQueryClient: () => ({
      getQueryData: () => null,
      invalidateQueries: () => {},
    }),
    useT: () => (key) => key,
    window: {
      clearInterval: () => {},
      open: () => popup,
      setInterval: (callback) => {
        intervals.push(callback);
        return intervals.length;
      },
    },
  };
  vm.runInNewContext(useExtensionsOauthSourceForTest(), context);

  const result = context.globalThis.__testExports.useOauthSetup({ id: "slack" });
  assert.equal(result.isAuthorizing, false);

  mutationConfig.onSuccess({
    res: { authorization_url: "https://slack.com/oauth/v2/authorize" },
    popup,
  });

  assert.deepEqual(stateUpdates, [{ index: 0, value: true }]);
  popup.closed = true;
  intervals[0]();
  assert.deepEqual(stateUpdates, [
    { index: 0, value: true },
    { index: 0, value: false },
  ]);
});

test("useOauthSetup waits for the matching Slack OAuth callback when reconnecting an existing token", () => {
  const stateUpdates = [];
  const intervals = [];
  const storage = new Map();
  let mutationConfig = null;
  let stateIndex = 0;
  let configuredCount = 0;
  const popup = { closed: false, location: { href: "about:blank" } };
  const context = {
    Date,
    Error,
    React: {
      useCallback: (fn) => fn,
      useEffect: () => {},
      useRef: (initial) => ({ current: initial }),
      useState: (initial) => {
        const index = stateIndex++;
        return [
          typeof initial === "function" ? initial() : initial,
          (value) => stateUpdates.push({ index, value }),
        ];
      },
    },
    URL,
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
      mutationConfig = config;
      return { isPending: false, mutate: () => {}, error: null };
    },
    useQuery: () => ({ data: {}, isLoading: false }),
    useQueryClient: () => ({
      getQueryData: (queryKey) => {
        if (JSON.stringify(queryKey) === JSON.stringify(["extension-setup", "slack"])) {
          return {
            secrets: [
              {
                name: "slack_personal_oauth",
                provider: "slack_personal",
                provided: true,
              },
            ],
          };
        }
        return null;
      },
      invalidateQueries: () => {},
    }),
    useT: () => (key) => key,
    window: {
      clearInterval: () => {},
      localStorage: {
        getItem: (key) => (storage.has(key) ? storage.get(key) : null),
        setItem: (key, value) => storage.set(key, String(value)),
      },
      open: () => popup,
      addEventListener: () => {},
      removeEventListener: () => {},
      setInterval: (callback) => {
        intervals.push(callback);
        return intervals.length;
      },
    },
  };
  vm.runInNewContext(useExtensionsOauthSourceForTest(), context);

  context.globalThis.__testExports.useOauthSetup(
    { id: "slack" },
    {
      onConfigured: () => {
        configuredCount += 1;
      },
    },
  );

  mutationConfig.onSuccess(
    {
      res: {
        authorization_url: "https://slack.com/oauth/v2/authorize",
        flow_id: "flow-new",
      },
      popup,
    },
    {
      secret: {
        provider: "slack_personal",
        provided: true,
      },
    },
  );

  intervals[0]();
  assert.equal(configuredCount, 0);

  storage.set(
    "ironclaw:product-auth:oauth-complete",
    JSON.stringify({
      type: "ironclaw:product-auth:oauth-complete",
      status: "completed",
      flowId: "flow-new",
    }),
  );
  intervals[0]();

  assert.equal(configuredCount, 1);
  assert.deepEqual(stateUpdates, [
    { index: 0, value: true },
    { index: 0, value: false },
  ]);
});

test("useOauthSetup completes reconnect when polling sees Slack become configured without a callback event", () => {
  const stateUpdates = [];
  const intervals = [];
  const storage = new Map();
  let mutationConfig = null;
  let stateIndex = 0;
  let configuredCount = 0;
  let extensionState = {
    package_ref: { id: "slack" },
    active: true,
    authenticated: false,
    needs_setup: true,
    activation_status: "active",
    onboarding_state: "setup_required",
  };
  const popup = { closed: false, location: { href: "about:blank" } };
  const context = {
    Date,
    Error,
    React: {
      useCallback: (fn) => fn,
      useEffect: () => {},
      useRef: (initial) => ({ current: initial }),
      useState: (initial) => {
        const index = stateIndex++;
        return [
          typeof initial === "function" ? initial() : initial,
          (value) => stateUpdates.push({ index, value }),
        ];
      },
    },
    URL,
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
      mutationConfig = config;
      return { isPending: false, mutate: () => {}, error: null };
    },
    useQuery: () => ({ data: {}, isLoading: false }),
    useQueryClient: () => ({
      getQueryData: (queryKey) => {
        if (JSON.stringify(queryKey) === JSON.stringify(["extension-setup", "slack"])) {
          return {
            secrets: [
              {
                name: "slack_personal_oauth",
                provider: "slack_personal",
                provided: true,
              },
            ],
          };
        }
        if (JSON.stringify(queryKey) === JSON.stringify(["extensions"])) {
          return { extensions: [extensionState] };
        }
        return null;
      },
      invalidateQueries: () => {},
    }),
    useT: () => (key) => key,
    window: {
      clearInterval: () => {},
      localStorage: {
        getItem: (key) => (storage.has(key) ? storage.get(key) : null),
        setItem: (key, value) => storage.set(key, String(value)),
      },
      open: () => popup,
      addEventListener: () => {},
      removeEventListener: () => {},
      setInterval: (callback) => {
        intervals.push(callback);
        return intervals.length;
      },
    },
  };
  vm.runInNewContext(useExtensionsOauthSourceForTest(), context);

  context.globalThis.__testExports.useOauthSetup(
    { id: "slack" },
    {
      onConfigured: () => {
        configuredCount += 1;
      },
    },
  );

  mutationConfig.onSuccess(
    {
      res: {
        authorization_url: "https://slack.com/oauth/v2/authorize",
        flow_id: "flow-new",
      },
      popup,
    },
    {
      secret: {
        provider: "slack_personal",
        provided: true,
      },
    },
  );

  intervals[0]();
  assert.equal(configuredCount, 0);

  extensionState = {
    package_ref: { id: "slack" },
    active: true,
    authenticated: true,
    needs_setup: false,
    activation_status: "active",
    onboarding_state: null,
  };
  intervals[0]();

  assert.equal(configuredCount, 1);
  assert.deepEqual(stateUpdates, [
    { index: 0, value: true },
    { index: 0, value: false },
  ]);
});

test("useOauthSetup keeps polling reconnect after Slack closes the OAuth popup", () => {
  const stateUpdates = [];
  const intervals = [];
  const storage = new Map();
  let mutationConfig = null;
  let stateIndex = 0;
  let configuredCount = 0;
  let extensionState = {
    package_ref: { id: "slack" },
    active: true,
    authenticated: false,
    needs_setup: true,
    activation_status: "active",
    onboarding_state: "setup_required",
  };
  const popup = { closed: false, location: { href: "about:blank" } };
  const context = {
    Date,
    Error,
    React: {
      useCallback: (fn) => fn,
      useEffect: () => {},
      useRef: (initial) => ({ current: initial }),
      useState: (initial) => {
        const index = stateIndex++;
        return [
          typeof initial === "function" ? initial() : initial,
          (value) => stateUpdates.push({ index, value }),
        ];
      },
    },
    URL,
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
      mutationConfig = config;
      return { isPending: false, mutate: () => {}, error: null };
    },
    useQuery: () => ({ data: {}, isLoading: false }),
    useQueryClient: () => ({
      getQueryData: (queryKey) => {
        if (JSON.stringify(queryKey) === JSON.stringify(["extension-setup", "slack"])) {
          return {
            secrets: [
              {
                name: "slack_personal_oauth",
                provider: "slack_personal",
                provided: true,
              },
            ],
          };
        }
        if (JSON.stringify(queryKey) === JSON.stringify(["extensions"])) {
          return { extensions: [extensionState] };
        }
        return null;
      },
      invalidateQueries: () => {},
    }),
    useT: () => (key) => key,
    window: {
      clearInterval: () => {},
      localStorage: {
        getItem: (key) => (storage.has(key) ? storage.get(key) : null),
        setItem: (key, value) => storage.set(key, String(value)),
      },
      open: () => popup,
      addEventListener: () => {},
      removeEventListener: () => {},
      setInterval: (callback) => {
        intervals.push(callback);
        return intervals.length;
      },
    },
  };
  vm.runInNewContext(useExtensionsOauthSourceForTest(), context);

  context.globalThis.__testExports.useOauthSetup(
    { id: "slack" },
    {
      onConfigured: () => {
        configuredCount += 1;
      },
    },
  );

  mutationConfig.onSuccess(
    {
      res: {
        authorization_url: "https://slack.com/oauth/v2/authorize",
        flow_id: "flow-new",
      },
      popup,
    },
    {
      secret: {
        provider: "slack_personal",
        provided: true,
      },
    },
  );

  popup.closed = true;
  intervals[0]();
  assert.equal(configuredCount, 0);

  extensionState = {
    package_ref: { id: "slack" },
    active: true,
    authenticated: true,
    needs_setup: false,
    activation_status: "active",
    onboarding_state: null,
  };
  intervals[0]();

  assert.equal(configuredCount, 1);
  assert.deepEqual(stateUpdates, [
    { index: 0, value: true },
    { index: 0, value: false },
  ]);
});
