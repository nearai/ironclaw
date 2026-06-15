import { useQueryClient } from "@tanstack/react-query";
import { ironclawStatusQueryKey } from "@/hooks/use-ironclaw-status";

const STORAGE_KEY = "ironclaw-mode";

export function getConnectionMode(): "local" | "hosted" {
  if (typeof window === "undefined") return "local";
  return (sessionStorage.getItem(STORAGE_KEY) as "local" | "hosted") ?? "local";
}

export function setConnectionMode(mode: "local" | "hosted") {
  if (typeof window === "undefined") return;
  sessionStorage.setItem(STORAGE_KEY, mode);
}

export function useConnectionMode() {
  const queryClient = useQueryClient();

  const current = getConnectionMode();

  const switchMode = (mode: "local" | "hosted") => {
    setConnectionMode(mode);
    queryClient.invalidateQueries({ queryKey: ironclawStatusQueryKey });
  };

  return { connectionMode: current, switchMode };
}
