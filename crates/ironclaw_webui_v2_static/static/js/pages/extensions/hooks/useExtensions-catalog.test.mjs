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
