import assert from "node:assert/strict";
import { test } from "vitest";

import { runVmModuleForTest } from "../../../test-support/vm-module-harness";

function loadUseSettings() {
  const calls = [];
  let mutationOptions;
  const context = {
    React: {
      useCallback: (fn) => fn,
      useState: (initial) => [initial, () => {}],
    },
    RESTART_REQUIRED_KEYS: new Set(),
    fetchSettingsExport: async () => ({ settings: {}, diagnostics: [], precedence: [] }),
    importSettingsPayload: async () => ({ success: true, imported: 0, results: [] }),
    NoSupportedSettingsImportError: class extends Error {},
    throwIfApiFailed: (data, fallbackMessage) => {
      if (data?.success === false) throw new Error(data.message || fallbackMessage);
      return data;
    },
    updateSetting: async () => ({ success: true, value: null }),
    useMutation: (options) => {
      if (!mutationOptions) mutationOptions = options;
      return {
        error: null,
        isPending: false,
        mutate: () => {},
        mutateAsync: async () => {},
      };
    },
    useQuery: () => ({ data: { settings: {}, diagnostics: [], precedence: [] } }),
    useQueryClient: () => ({
      invalidateQueries: (...args) => calls.push({ type: "invalidateQueries", args }),
      setQueryData: (...args) => calls.push({ type: "setQueryData", args }),
    }),
    setTimeout: () => 0,
  };
  const exports = runVmModuleForTest("./useSettings.ts", ["useSettings"], context, import.meta.url);
  exports.useSettings();
  return { calls, mutationOptions };
}

test("useSettings caches the backend-confirmed value after a save", () => {
  const { calls, mutationOptions } = loadUseSettings();

  mutationOptions.onSuccess(
    { success: true, value: "normalized-provider" },
    { key: "provider.default", value: " Requested Provider " },
  );

  const cacheWrite = calls.find((call) => call.type === "setQueryData");
  const next = cacheWrite.args[1]({
    settings: { "provider.default": "old-provider" },
    diagnostics: [],
    precedence: [],
  });
  assert.equal(next.settings["provider.default"], "normalized-provider");
});
