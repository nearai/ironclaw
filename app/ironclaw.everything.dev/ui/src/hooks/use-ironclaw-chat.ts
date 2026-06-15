import type { UIMessage } from "@tanstack/ai/client";
import { useChat } from "@tanstack/ai-react";
import { useCallback, useEffect, useRef, useState } from "react";
import type { ApiClient } from "@/app";
import type { StagedAttachment } from "@/lib/attachments";
import { createIronclawChatStream } from "@/lib/ironclaw-chat-stream";

interface RunState {
  phase:
    | "idle"
    | "submitted"
    | "running"
    | "finalizing"
    | "awaiting_approval"
    | "auth_required"
    | "failed"
    | "cancelled"
    | "disconnected";
  runId?: string;
  replyMessageId?: string;
  message?: string;
  gateRef?: string;
  gateHeadline?: string;
  gateBody?: string;
  authRequestRef?: string;
  authHeadline?: string;
  authBody?: string;
  authUrl?: string;
  activeToolName?: string;
}

export type { RunState };

function messageTextContent(message: UIMessage): string {
  return message.parts
    .filter((part): part is { type: "text"; content: string } => part.type === "text")
    .map((part) => part.content)
    .join(" ");
}

export function useIronclawChat(
  threadId: string,
  apiClient: ApiClient,
  initialMessages: Array<UIMessage>,
) {
  const activeRunIdRef = useRef<string | null>(null);
  const pendingAttachmentsRef = useRef<StagedAttachment[]>([]);
  const [runState, setRunState] = useState<RunState>({ phase: "idle" });
  const onRunStateChangeRef = useRef<(update: Partial<RunState>) => void>(() => {});

  onRunStateChangeRef.current = (update) => {
    setRunState((prev) => ({ ...prev, ...update }));
  };

  const initialMessagesRef = useRef(initialMessages);

  const chat = useChat({
    threadId,
    initialMessages,
    fetcher: async function* ({ messages }, { signal }) {
      const lastUser = [...messages].reverse().find((m) => m.role === "user");
      const content = lastUser ? messageTextContent(lastUser) : "";
      const attachments = pendingAttachmentsRef.current;
      pendingAttachmentsRef.current = [];

      onRunStateChangeRef.current({ phase: "submitted", message: undefined, replyMessageId: undefined, activeToolName: undefined });

      const accepted = await apiClient.conversation.sendMessage({
        threadId,
        content,
        clientActionId: `ui-${crypto.randomUUID()}`,
        attachments: attachments?.map((a) => ({
          mimeType: a.mimeType,
          filename: a.filename,
          dataBase64: a.dataBase64,
        })),
      });

      yield* createIronclawChatStream({
        threadId,
        accepted,
        apiClient,
        signal,
        onRunStarted: (runId) => {
          activeRunIdRef.current = runId;
        },
        onRunStateChange: (update) => {
          onRunStateChangeRef.current(update);
        },
      });
    },
    onError: (error) => {
      console.error("[useIronclawChat]", error);
      setRunState((prev) => ({
        ...prev,
        phase: "failed",
        message: error instanceof Error ? error.message : String(error),
      }));
    },
  });

  useEffect(() => {
    if (initialMessages.length > 0 && initialMessages !== initialMessagesRef.current) {
      initialMessagesRef.current = initialMessages;
      chat.setMessages(initialMessages);
    }
  }, [initialMessages, chat]);

  useEffect(() => {
    if (runState.phase !== "finalizing" || !runState.replyMessageId) return;

    const hasPaintedReply = chat.messages.some((message) => message.id === runState.replyMessageId);
    if (!hasPaintedReply) return;

    setRunState({ phase: "idle", replyMessageId: undefined, activeToolName: undefined });
  }, [chat.messages, runState.phase, runState.replyMessageId]);

  const sendMessageWithAttachments = useCallback(
    (content: string, attachments?: StagedAttachment[]) => {
      pendingAttachmentsRef.current = attachments ?? [];
      chat.sendMessage(content);
    },
    [chat.sendMessage],
  );

  const resolveGate = useCallback(
    async (gateRef: string, approved: boolean) => {
      const runId = activeRunIdRef.current;
      if (!runId) throw new Error("Missing run ID for gate resolution");
      await apiClient.ironclaw.threads.resolveGate({
        id: threadId,
        runId,
        gateRef,
        resolution: approved ? "approved" : "denied",
      });
    },
    [apiClient, threadId],
  );

  return {
    messages: chat.messages,
    sendMessage: sendMessageWithAttachments,
    status: chat.status,
    stop: chat.stop,
    setMessages: chat.setMessages,
    resolveGate,
    runState,
  };
}
