import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useApiClient } from "@/app";
import { getConnectionMode } from "@/hooks/use-connection-mode";

export type IronclawConnectionStatus = "connected" | "disconnected" | "never-connected" | "checking";

export const ironclawStatusQueryKey = ["ironclaw-status"] as const;

const SESSION_KEY = "ironclaw-was-connected";

function markWasConnected() {
  try { sessionStorage.setItem(SESSION_KEY, "1"); } catch { }
}

function getWasConnected(): boolean {
  try { return sessionStorage.getItem(SESSION_KEY) === "1"; } catch { return false; }
}

function clearWasConnected() {
  try { sessionStorage.removeItem(SESSION_KEY); } catch { }
}

export function useIronclawStatus(): {
  status: IronclawConnectionStatus;
  refetch: () => void;
  disconnect: () => Promise<void>;
  connectionMode: "local" | "hosted";
} {
  const apiClient = useApiClient();
  const queryClient = useQueryClient();

  const { data, isFetching, isError, refetch } = useQuery({
    queryKey: ironclawStatusQueryKey,
    queryFn: async () => {
      await apiClient.ironclaw.ping();
      markWasConnected();
      return true;
    },
    refetchInterval: 10_000,
    retry: false,
    staleTime: 8_000,
  });

  const connectionMode = getConnectionMode();

  let status: IronclawConnectionStatus;
  if (isFetching && data === undefined) {
    status = "checking";
  } else if (isError || data === undefined) {
    status = getWasConnected() ? "disconnected" : "never-connected";
  } else {
    status = "connected";
  }

  const disconnect = async () => {
    clearWasConnected();
    queryClient.setQueryData(ironclawStatusQueryKey, undefined);
  };

  return {
    status,
    refetch: () => { refetch(); },
    disconnect,
    connectionMode,
  };
}
