import assert from "node:assert/strict";
import { test } from "vitest";

import { runVmModuleForTest } from "../../../test-support/vm-module-harness";
import {
  SettingsImportError,
  SettingsImportFailureReason,
} from "../lib/settings-api";

test("settings import mutation rejects when no supported settings were imported", async () => {
  let importMutationOptions;
  const context = {
    React: {
      useCallback: (fn) => fn,
      useState: (initial) => [initial, () => {}],
    },
    RESTART_REQUIRED_KEYS: new Set(),
    SettingsImportError,
    fetchSettingsExport: () => {},
    importSettingsPayload: async () => ({
      success: false,
      imported: 0,
      results: [],
      reason: SettingsImportFailureReason.NoSupportedSettings,
      message: "No supported settings were found in the selected file",
    }),
    updateSetting: () => {},
    useMutation: (options) => {
      if (!importMutationOptions) {
        importMutationOptions = options;
        return { mutate: () => {}, error: null };
      }
      importMutationOptions = options;
      return {
        mutateAsync: options.mutationFn,
        isPending: false,
        error: null,
      };
    },
    useQuery: () => ({ data: { settings: {} } }),
    useQueryClient: () => ({
      invalidateQueries: () => {},
      setQueryData: () => {},
    }),
  };

  const { useSettings } = runVmModuleForTest(
    "./useSettings.ts",
    ["useSettings"],
    context,
    import.meta.url
  );

  const settings = useSettings();
  await assert.rejects(
    () => settings.importSettings({ settings: {} }),
    (error) => {
      assert.ok(error instanceof SettingsImportError);
      assert.equal(error.reason, SettingsImportFailureReason.NoSupportedSettings);
      assert.match(error.message, /No supported settings/);
      return true;
    }
  );
});
