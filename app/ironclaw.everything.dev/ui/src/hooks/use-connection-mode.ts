import { useState, useEffect } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { ironclawStatusQueryKey } from "@/hooks/use-ironclaw-status";
import { useApiClient } from "@/app";

const STORAGE_KEY = "ironclaw-mode";

export type ConnectionMode = "auto" | "hosted" | "local";

export function getConnectionMode(): ConnectionMode {
  if (typeof window === "undefined") return "auto";
  return (sessionStorage.getItem(STORAGE_KEY) as ConnectionMode) ?? "auto";
}

export function setConnectionMode(mode: ConnectionMode) {
  if (typeof window === "undefined") return;
  sessionStorage.setItem(STORAGE_KEY, mode);
}

export function useConnectionMode(): {
  connectionMode: ConnectionMode;
  switchMode: (mode: ConnectionMode) => Promise<void>;
} {
  const apiClient = useApiClient();
  const queryClient = useQueryClient();
  const [connectionMode, setConnectionModeState] = useState<ConnectionMode>(() =>
    getConnectionMode(),
  );

  useEffect(() => {
    apiClient.ironclaw.settings
      .getMode()
      .then((result) => {
        const mode = (result as { mode: ConnectionMode }).mode;
        setConnectionModeState(mode);
        setConnectionMode(mode);
      })
      .catch(() => {
        setConnectionModeState(getConnectionMode());
      });
  }, [apiClient]);

  const switchMode = async (mode: ConnectionMode) => {
    setConnectionModeState(mode);
    setConnectionMode(mode);
    try {
      await apiClient.ironclaw.settings.setMode({ mode });
    } catch {}
    queryClient.invalidateQueries({ queryKey: ironclawStatusQueryKey });
  };

  return { connectionMode, switchMode };
}
