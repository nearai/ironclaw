// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";
import { productAuthOAuthEventsSource } from "../../../lib/product-auth-oauth-events.vm-inline.mjs";

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
  return `${productAuthOAuthEventsSource()}\n${lines.join("\n")}\nglobalThis.__testExports = { useExtensions };`;
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
