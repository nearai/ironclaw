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
  return `${lines.join("\n")}\nglobalThis.__testExports = { usePairing };`;
}

function contextFor(mutationState, queryCalls) {
  return {
    React: { useCallback: (fn) => fn, useEffect: () => {}, useRef: () => ({ current: null }), useState: () => [null, () => {}] },
    activateExtension: () => {},
    approvePairingCode: () => {},
    fetchExtensionRegistry: () => {},
    fetchExtensionSetup: () => {},
    fetchExtensions: () => {},
    fetchPairingRequests: () => {},
    gatewayStatus: () => {},
    globalThis: {},
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
  };
}

test("usePairing only exposes result on success and error on failure", () => {
  for (const [name, mutationState, expected] of [
    ["idle", { mutate: () => {}, isPending: false, isSuccess: false, isError: false }, { result: null, error: null }],
    ["success", { mutate: () => {}, isPending: false, isSuccess: true, isError: false, data: { success: true } }, { result: { success: true }, error: null }],
    ["error", { mutate: () => {}, isPending: false, isSuccess: false, isError: true, error: new Error("bad") }, { result: null, errorMessage: "bad" }],
  ]) {
    const queryCalls = [];
    const context = contextFor(mutationState, queryCalls);
    vm.runInNewContext(useExtensionsSourceForTest(), context);

    const pairing = context.globalThis.__testExports.usePairing("slack");

    assert.deepEqual(pairing.result, expected.result, name);
    assert.equal(pairing.error?.message || null, expected.errorMessage || null, name);
  }
});

test("usePairing can disable the legacy pairing request query for custom redeemers", () => {
  const queryCalls = [];
  const context = contextFor(
    { mutate: () => {}, isPending: false, isSuccess: false, isError: false },
    queryCalls
  );
  vm.runInNewContext(useExtensionsSourceForTest(), context);

  context.globalThis.__testExports.usePairing("slack", { enabled: false });

  assert.equal(queryCalls.length, 1);
  assert.equal(queryCalls[0].enabled, false);
});
