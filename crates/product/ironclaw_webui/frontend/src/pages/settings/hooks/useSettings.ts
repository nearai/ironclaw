import React from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  fetchSettingsExport,
  importSettings as importSettingsPayload,
  NoSupportedSettingsImportError,
  updateSetting,
} from "../lib/settings-api";
import { throwIfApiFailed } from "../lib/api-result";
import { RESTART_REQUIRED_KEYS } from "../lib/settings-schema";

type SettingsExport = {
  settings: Record<string, unknown>;
  diagnostics: unknown[];
  precedence: unknown[];
};

type SettingMutationVariables = { key: string; value: unknown };
type SettingsImportPayload = { settings?: Record<string, unknown> };

export function useSettings() {
  const queryClient = useQueryClient();
  const query = useQuery<SettingsExport>({
    queryKey: ["settings-export"],
    queryFn: fetchSettingsExport,
    staleTime: 30_000,
  });

  const settings = query.data?.settings || {};

  const [savedKeys, setSavedKeys] = React.useState<Record<string, boolean>>({});
  const [needsRestart, setNeedsRestart] = React.useState(false);

  const mutation = useMutation<unknown, Error, SettingMutationVariables>({
    // A resolved response with `success: false` is a failed save, not a
    // success — surface it so the UI shows the error rather than a fake
    // "Saved" indicator (and never flips `needsRestart`).
    mutationFn: async ({ key, value }) =>
      throwIfApiFailed(await updateSetting(key, value), "Save failed"),
    onSuccess: (_data, { key, value }) => {
      queryClient.setQueryData<SettingsExport>(["settings-export"], (old) => {
        if (!old) return old;
        const next = { ...old, settings: { ...old.settings } };
        if (value === null || value === undefined) {
          delete next.settings[key];
        } else {
          next.settings[key] = value;
        }
        return next;
      });

      setSavedKeys((prev) => ({ ...prev, [key]: true }));
      setTimeout(() => setSavedKeys((prev) => ({ ...prev, [key]: false })), 2000);

      if (RESTART_REQUIRED_KEYS.has(key)) {
        setNeedsRestart(true);
      }
      if (key === "agent.auto_approve_tools") {
        queryClient.invalidateQueries({ queryKey: ["settings-tools"] });
      }
    },
  });

  const save = React.useCallback(
    (key: string, value: unknown) => mutation.mutate({ key, value }),
    [mutation]
  );

  const importMutation = useMutation<unknown, Error, SettingsImportPayload>({
    mutationFn: async (payload) => {
      const result = await importSettingsPayload(payload);
      if (result.success === false) {
        throw new NoSupportedSettingsImportError(result);
      }
      return result;
    },
    onSuccess: (_data, payload) => {
      queryClient.invalidateQueries({ queryKey: ["settings-export"] });
      const importedKeys = Object.keys(payload?.settings || {});
      if (importedKeys.includes("agent.auto_approve_tools")) {
        queryClient.invalidateQueries({ queryKey: ["settings-tools"] });
      }
      if (importedKeys.some((key) => RESTART_REQUIRED_KEYS.has(key))) {
        setNeedsRestart(true);
      }
    },
  });

  const importSettings = React.useCallback(
    (payload: SettingsImportPayload) => importMutation.mutateAsync(payload),
    [importMutation]
  );

  return {
    settings,
    query,
    save,
    savedKeys,
    needsRestart,
    importSettings,
    isImporting: importMutation.isPending,
    saveError: mutation.error,
  };
}
