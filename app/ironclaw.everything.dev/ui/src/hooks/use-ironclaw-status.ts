import { useEffect, useRef } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import type { z } from "every-plugin/zod";
import { useApiClient, useAuthClient } from "@/app";
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
  const auth = useAuthClient();

  const { data, isFetching, isError, refetch } = useQuery({
    queryKey: ironclawStatusQueryKey,
    queryFn: async () => {
      const authSession = await auth.getSession();
      if (!authSession?.data) {
        return { connected: false, session: null, attachmentCapabilities: null };
      }
      try {
        const apiSession = await apiClient.ironclaw.session();
        markWasConnected();
        return {
          connected: true,
          session: apiSession,
          attachmentCapabilities: apiSession?.capabilities?.attachments ?? null,
        };
      } catch (err: any) {
        const isIronclaw404 = (typeof err?.f === "string" && err.f.includes("Ironclaw API error")) || (err?.message && String(err.message).includes("Ironclaw API error"));
        const isNotConfigured =
          err?.code === "PRECONDITION_FAILED" || err?.message?.includes("No IronClaw connection configured") || isIronclaw404;
        if (isNotConfigured) {
          clearWasConnected();
          return { connected: false, session: null, attachmentCapabilities: null };
        }
        let pingOk = false;
        try {
          await apiClient.ironclaw.ping();
          pingOk = true;
        } catch {}
        if (pingOk) {
          markWasConnected();
          const capsStr = sessionStorage.getItem("ironclaw-attachment-caps");
          return {
            connected: true,
            session: null,
            attachmentCapabilities: capsStr ? JSON.parse(capsStr) : null,
          };
        }
        throw err;
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
  } else if (isError || data === undefined || data === null) {
    status = getWasConnected() ? "disconnected" : "never-connected";
  } else if (!data.connected) {
    status = "never-connected";
  } else {
    status = "connected";
  }

  const prevStatus = useRef<IronclawConnectionStatus>(status);
  useEffect(() => {
    if (prevStatus.current === "checking" && status === "disconnected") {
      prevStatus.current = status;
      return;
    }
    if (prevStatus.current === "checking" && status === "never-connected") {
      if (!isToastShown()) {
        markToastShown();
        toast("IronClaw agent not connected", {
          description: "Set up a connection in Settings to get started.",
          action: {
            label: "Settings",
            onClick: () => window.location.href = "/settings/ironclaw",
          },
          duration: 8000,
        });
      }
      prevStatus.current = status;
      return;
    }
    if (prevStatus.current === "connected" && status === "disconnected") {
      if (!isToastShown()) {
        markToastShown();
        toast.error("Lost connection to your IronClaw instance. Check Settings \u2192 IronClaw or set up a new connection.");
      }
    }
    if (status === "connected") {
      clearToastShown();
    }
    prevStatus.current = status;
  }, [status]);

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

function isToastShown(): boolean {
  try {
    return sessionStorage.getItem("ironclaw-toast") === "1";
  } catch {
    return false;
  }
}

function markToastShown() {
  try {
    sessionStorage.setItem("ironclaw-toast", "1");
  } catch {}
}

function clearToastShown() {
  try {
    sessionStorage.removeItem("ironclaw-toast");
  } catch {}
}
