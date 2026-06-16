import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { z } from "every-plugin/zod";
import { useApiClient } from "@/app";
import { getConnectionMode, type ConnectionMode } from "@/hooks/use-connection-mode";
import type { SessionSchema } from "../../../plugins/ironclaw/src/contract.ts";

type SessionData = z.infer<typeof SessionSchema>;
type AttachmentCapabilities = NonNullable<SessionData["capabilities"]["attachments"]> | null;

export type IronclawConnectionStatus =
  | "connected"
  | "disconnected"
  | "never-connected"
  | "checking";

export const ironclawStatusQueryKey = ["ironclaw-status"] as const;

const SESSION_KEY = "ironclaw-was-connected";

function markWasConnected() {
  try {
    sessionStorage.setItem(SESSION_KEY, "1");
  } catch {}
}

function getWasConnected(): boolean {
  try {
    return sessionStorage.getItem(SESSION_KEY) === "1";
  } catch {
    return false;
  }
}

function clearWasConnected() {
  try {
    sessionStorage.removeItem(SESSION_KEY);
  } catch {}
}

export function useIronclawStatus(): {
  status: IronclawConnectionStatus;
  refetch: () => void;
  disconnect: () => Promise<void>;
  connectionMode: ConnectionMode;
  session: SessionData | null;
  attachmentCapabilities: AttachmentCapabilities;
} {
  const apiClient = useApiClient();
  const queryClient = useQueryClient();

  const { data, isFetching, isError, refetch } = useQuery({
    queryKey: ironclawStatusQueryKey,
    queryFn: async () => {
      try {
        const session = await apiClient.ironclaw.session();
        markWasConnected();
        return {
          connected: true,
          session,
          attachmentCapabilities: session?.capabilities?.attachments ?? null,
        };
      } catch {
        const capsStr = sessionStorage.getItem("ironclaw-attachment-caps");
        await apiClient.ironclaw.ping();
        markWasConnected();
        return {
          connected: true,
          session: null,
          attachmentCapabilities: capsStr ? JSON.parse(capsStr) : null,
        };
      }
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
    refetch: () => {
      refetch();
    },
    disconnect,
    connectionMode,
    session: data?.session ?? null,
    attachmentCapabilities: data?.attachmentCapabilities ?? null,
  };
}
