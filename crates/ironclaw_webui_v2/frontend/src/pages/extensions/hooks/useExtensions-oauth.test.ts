// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";
import { productAuthOAuthEventsSource } from "../../../lib/product-auth-oauth-events.vm-inline";

// The origin-independent flow-status poll is fire-and-forget: the interval
// callback kicks off `fetchOauthFlowStatus(...)` without awaiting it, so the
// completion runs on a microtask after the synchronous interval returns. Yield
// to the event loop so those microtasks drain before asserting.
function flushAsyncWork() {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

function useExtensionsOauthSourceForTest() {
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
  return `${productAuthOAuthEventsSource()}\n${lines.join("\n")}\nglobalThis.__testExports = { useOauthSetup };`;
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
    fetchOauthFlowStatus: () => Promise.resolve(null),
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
    fetchOauthFlowStatus: () => Promise.resolve(null),
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

  // A completion from a DIFFERENT extension's flow (e.g. another tab) must not
  // satisfy this modal while it is still waiting on its own callback.
  storage.set(
    "ironclaw:product-auth:oauth-complete",
    JSON.stringify({
      type: "ironclaw:product-auth:oauth-complete",
      status: "completed",
      flowId: "flow-other",
    }),
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
    fetchOauthFlowStatus: () => Promise.resolve(null),
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
    fetchOauthFlowStatus: () => Promise.resolve(null),
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

test("useOauthSetup surfaces a flow-matched failure signal as a retryable error and stops the watcher", () => {
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
    fetchOauthFlowStatus: () => Promise.resolve(null),
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
        flow_id: "flow-fails",
      },
      popup,
    },
    {},
  );

  // The callback popup reported a FAILURE for this exact flow and closed.
  storage.set(
    "ironclaw:product-auth:oauth-complete",
    JSON.stringify({
      type: "ironclaw:product-auth:oauth-complete",
      status: "failed",
      flowId: "flow-fails",
    }),
  );
  intervals[0]();

  assert.equal(configuredCount, 0, "a failed flow must not report configured");
  // isAuthorizing (first useState) flips true -> false: the watcher stopped.
  assert.ok(
    stateUpdates.some((update) => update.index === 0 && update.value === false),
    "the watcher must stop on a flow-matched failure",
  );
  // authError (second useState) carries the retryable error for the modal.
  assert.ok(
    stateUpdates.some(
      (update) =>
        update.index === 1 &&
        typeof update.value === "string" &&
        /authorization failed/i.test(update.value),
    ),
    "a flow-matched failure must surface a retryable error",
  );

  // A failure for a DIFFERENT flow must not disturb a fresh watcher.
  const before = stateUpdates.length;
  mutationConfig.onSuccess(
    {
      res: {
        authorization_url: "https://slack.com/oauth/v2/authorize",
        flow_id: "flow-second",
      },
      popup,
    },
    {},
  );
  storage.set(
    "ironclaw:product-auth:oauth-complete",
    JSON.stringify({
      type: "ironclaw:product-auth:oauth-complete",
      status: "failed",
      flowId: "flow-other",
    }),
  );
  intervals.at(-1)();
  assert.ok(
    !stateUpdates
      .slice(before)
      .some(
        (update) =>
          update.index === 1 &&
          typeof update.value === "string" &&
          /authorization failed/i.test(update.value),
      ),
    "a foreign flow's failure must not error this watcher",
  );
});

test("useOauthSetup reconnect ignores the pre-flow configured snapshot and waits for a fresh signal", () => {
  const stateUpdates = [];
  const intervals = [];
  const storage = new Map();
  let mutationConfig = null;
  let stateIndex = 0;
  let configuredCount = 0;
  // The caller is ALREADY connected: a true reconnect. The extensions cache
  // reports configured before the OAuth flow even starts, so "configured" is
  // not evidence that THIS flow completed — the old watcher completed on the
  // first poll tick while the user was still on Slack's consent screen.
  const extensionState = {
    package_ref: { id: "slack" },
    active: true,
    authenticated: true,
    needs_setup: false,
    activation_status: "active",
    onboarding_state: null,
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
    fetchOauthFlowStatus: () => Promise.resolve(null),
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

  // Stale pre-flow "configured" must not complete the reconnect — the user is
  // still authorizing in the popup.
  intervals[0]();
  intervals[0]();
  assert.equal(configuredCount, 0);

  // The flow-id-matched callback signal is what proves THIS flow completed.
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

test("useOauthSetup ignores a stale OAuth callback when the flow response carried no flow id", () => {
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
    fetchOauthFlowStatus: () => Promise.resolve(null),
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
    // Nothing is configured server-side, so the callback fast-path is the only
    // possible completion signal — and it must reject a foreign/absent flow id.
    useQueryClient: () => ({
      getQueryData: () => null,
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

  // Another tab already wrote a completion for a DIFFERENT flow before this flow
  // even starts. The backend omitted `flow_id` on this flow's start response, so
  // the watcher's `flowId` is null — the exact case the old code treated as a
  // wildcard match.
  storage.set(
    "ironclaw:product-auth:oauth-complete",
    JSON.stringify({
      type: "ironclaw:product-auth:oauth-complete",
      status: "completed",
      flowId: "flow-from-another-tab",
    }),
  );

  mutationConfig.onSuccess(
    {
      res: { authorization_url: "https://slack.com/oauth/v2/authorize" },
      popup,
    },
    {},
  );

  // The initial storage read on watcher start must NOT treat the stale cross-tab
  // completion as this flow's completion.
  assert.equal(configuredCount, 0);

  // Neither must a subsequent poll — nor a completion carrying no flow id at all.
  intervals[0]();
  assert.equal(configuredCount, 0);

  storage.set(
    "ironclaw:product-auth:oauth-complete",
    JSON.stringify({
      type: "ironclaw:product-auth:oauth-complete",
      status: "completed",
    }),
  );
  intervals[0]();
  assert.equal(configuredCount, 0);

  // The watcher is still authorizing — it never falsely completed.
  assert.deepEqual(stateUpdates, [{ index: 0, value: true }]);
});

test("useOauthSetup completes reconnect from the origin-independent flow-status poll when no browser signal arrives", async () => {
  const stateUpdates = [];
  const intervals = [];
  const storage = new Map();
  let mutationConfig = null;
  let stateIndex = 0;
  let configuredCount = 0;
  const flowStatusCalls = [];
  const popup = { closed: false, location: { href: "about:blank" } };
  const context = {
    Date,
    Error,
    Promise,
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
    // Cross-origin callback: the same-origin localStorage/BroadcastChannel
    // signal never reaches this tab. The server-side flow-status poll is the
    // ONLY completion path.
    fetchOauthFlowStatus: (flowId, invocationId) => {
      flowStatusCalls.push({ flowId, invocationId });
      return Promise.resolve({ status: "completed" });
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
      mutationConfig = config;
      return { isPending: false, mutate: () => {}, error: null };
    },
    useQuery: () => ({ data: {}, isLoading: false }),
    // Already-configured reconnect: the setup cache reports the secret as
    // `provided`, so `requireCallbackCompletion` is true and the
    // provided-secret poll fallback is disabled — configured-state polling can
    // never complete this flow.
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
        callback_scope: { invocation_id: "invocation-new" },
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

  // No storage/BroadcastChannel signal is ever written. The poll fires on the
  // interval tick and resolves on a microtask.
  intervals[0]();
  await flushAsyncWork();

  assert.equal(configuredCount, 1, "the flow-status poll must complete the reconnect");
  // The poll must carry the flow id AND the invocation id the start response
  // minted, so the caller-scoped backend can locate its own flow.
  assert.deepEqual(flowStatusCalls[0], {
    flowId: "flow-new",
    invocationId: "invocation-new",
  });
  assert.deepEqual(stateUpdates, [
    { index: 0, value: true },
    { index: 0, value: false },
  ]);
});

test("useOauthSetup surfaces an expired flow-status poll as a retryable error when no browser signal arrives", async () => {
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
    Promise,
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
    fetchOauthFlowStatus: () => Promise.resolve({ status: "expired" }),
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
        flow_id: "flow-fails",
        callback_scope: { invocation_id: "invocation-fails" },
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
  await flushAsyncWork();

  assert.equal(configuredCount, 0, "an expired flow must not report configured");
  assert.ok(
    stateUpdates.some((update) => update.index === 0 && update.value === false),
    "the watcher must stop on an expired flow-status poll",
  );
  assert.ok(
    stateUpdates.some(
      (update) =>
        update.index === 1 &&
        typeof update.value === "string" &&
        /authorization expired/i.test(update.value),
    ),
    "an expired flow-status poll must surface a retryable error",
  );
});

test("useOauthSetup ignores flow A status after flow B becomes current", async () => {
  const stateUpdates = [];
  const intervals = [];
  const storage = new Map();
  const starts = [];
  let mutationConfig = null;
  let stateIndex = 0;
  let configuredCount = 0;
  let resolveFlowAStatus;
  const flowAStatus = new Promise((resolve) => {
    resolveFlowAStatus = resolve;
  });
  const popupA = {
    closed: false,
    close() {
      this.closed = true;
    },
    location: { href: "about:blank" },
  };
  const popupB = {
    closed: false,
    close() {
      this.closed = true;
    },
    location: { href: "about:blank" },
  };
  const context = {
    Date,
    Error,
    Promise,
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
    fetchOauthFlowStatus: (flowId) =>
      flowId === "flow-a" ? flowAStatus : Promise.resolve({ status: "completed" }),
    fetchPairingRequests: () => {},
    gatewayStatus: () => {},
    globalThis: {},
    installExtension: () => {},
    isChannelExtensionKind: () => false,
    listConnectableChannels: () => {},
    removeExtension: () => {},
    startExtensionOauth: () =>
      new Promise((resolve) => {
        starts.push(resolve);
      }),
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
      localStorage: {
        getItem: (key) => (storage.has(key) ? storage.get(key) : null),
        setItem: (key, value) => storage.set(key, String(value)),
      },
      open: () => popupA,
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

  const variablesA = { secret: { provided: true }, popup: popupA };
  const startA = mutationConfig.mutationFn(variablesA);
  starts.shift()({
    authorization_url: "https://slack.com/oauth/v2/authorize",
    flow_id: "flow-a",
  });
  mutationConfig.onSuccess(await startA, variablesA);
  intervals[0]();

  const variablesB = { secret: { provided: true }, popup: popupB };
  const startB = mutationConfig.mutationFn(variablesB);
  starts.shift()({
    authorization_url: "https://slack.com/oauth/v2/authorize",
    flow_id: "flow-b",
  });
  mutationConfig.onSuccess(await startB, variablesB);

  resolveFlowAStatus({ status: "completed" });
  await flushAsyncWork();
  assert.equal(configuredCount, 0, "flow A must not complete flow B's watcher");
  assert.equal(popupB.closed, false, "flow A must not close flow B's popup");

  intervals[1]();
  await flushAsyncWork();
  assert.equal(configuredCount, 1, "flow B can still complete normally");
  assert.ok(
    !stateUpdates.some(
      (update) => update.index === 1 && typeof update.value === "string",
    ),
    "flow A must not stamp an error onto flow B",
  );
});

test("useOauthSetup still times out when a matched failure signal cannot reach durable status", async () => {
  const stateUpdates = [];
  const intervals = [];
  const storage = new Map();
  let mutationConfig = null;
  let stateIndex = 0;
  let now = 0;
  const popup = { closed: false, location: { href: "about:blank" } };
  const context = {
    Date: { now: () => now },
    Error,
    Promise,
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
    fetchOauthFlowStatus: () => Promise.reject(new Error("status unavailable")),
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
  context.globalThis.__testExports.useOauthSetup({ id: "slack" });
  mutationConfig.onSuccess(
    {
      res: {
        authorization_url: "https://slack.com/oauth/v2/authorize",
        flow_id: "flow-pending-cleanup",
        callback_scope: { invocation_id: "invocation-pending-cleanup" },
      },
      popup,
    },
    {},
  );
  storage.set(
    "ironclaw:product-auth:oauth-complete",
    JSON.stringify({
      type: "ironclaw:product-auth:oauth-complete",
      status: "failed",
      flowId: "flow-pending-cleanup",
    }),
  );

  now = 10 * 60 * 1000 + 1;
  intervals[0]();
  await flushAsyncWork();

  assert.ok(
    stateUpdates.some((update) => update.index === 0 && update.value === false),
    "the watcher must stop after its bounded timeout",
  );
  assert.ok(
    stateUpdates.some(
      (update) =>
        update.index === 1 &&
        typeof update.value === "string" &&
        /timed out/i.test(update.value),
    ),
    "a persistent status outage must surface a retryable timeout",
  );
});
