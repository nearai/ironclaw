import { consumeEventIterator } from "@orpc/client";
import type { StreamChunk, UIMessage } from "@tanstack/ai/client";
import { useChat } from "@tanstack/ai-react";
import { useCallback, useEffect, useRef, useState } from "react";
import type { ApiClient } from "@/app";
import type { StagedAttachment } from "@/lib/attachments";

interface RunState {
  phase:
    | "idle"
    | "submitted"
    | "running"
    | "awaiting_approval"
    | "auth_required"
    | "failed"
    | "cancelled"
    | "disconnected";
  runId?: string;
  message?: string;
  gateRef?: string;
  gateHeadline?: string;
  gateBody?: string;
  authRequestRef?: string;
  authHeadline?: string;
  authBody?: string;
  authUrl?: string;
}

export type { RunState };

function messageTextContent(message: UIMessage): string {
  return message.parts
    .filter((part): part is { type: "text"; content: string } => part.type === "text")
    .map((part) => part.content)
    .join(" ");
}

function createIronclawStream({
  apiClient,
  threadId,
  content,
  clientActionId,
  attachments,
  onRunStarted,
  onRunStateChange,
  signal,
}: {
  apiClient: ApiClient;
  threadId: string;
  content: string;
  clientActionId: string;
  attachments?: StagedAttachment[];
  onRunStarted: (runId: string) => void;
  onRunStateChange: (update: Partial<RunState>) => void;
  signal: AbortSignal;
}): AsyncIterable<StreamChunk> {
  return {
    [Symbol.asyncIterator]() {
      const queue: Array<Record<string, unknown>> = [];
      const waiters: Array<(result: IteratorResult<Record<string, unknown>>) => void> = [];
      let closed = false;
      let unsubscribe: (() => Promise<void>) | null = null;

      const push = (chunk: Record<string, unknown>) => {
        if (closed) return;
        const waiter = waiters.shift();
        if (waiter) {
          waiter({ value: chunk, done: false });
          return;
        }
        queue.push(chunk);
      };

      const finish = () => {
        if (closed) return;
        closed = true;
        if (unsubscribe) {
          const u = unsubscribe;
          unsubscribe = null;
          void u();
        }
        while (waiters.length > 0) {
          const waiter = waiters.shift();
          waiter?.({ value: undefined, done: true });
        }
      };

      const fail = (message: string) => {
        push({ type: "RUN_ERROR", message });
        finish();
      };

      signal.addEventListener(
        "abort",
        () => {
          finish();
        },
        { once: true },
      );

      void (async () => {
        let runId: string;
        let replyMessageId = "reply-pending";

        try {
          const accepted = await apiClient.ironclaw.threads.sendMessage({
            id: threadId,
            content,
            clientActionId,
            attachments: attachments?.map((a) => ({
              mimeType: a.mimeType,
              filename: a.filename,
              dataBase64: a.dataBase64,
            })),
          });
          runId = accepted.runId ?? accepted.activeRunId ?? crypto.randomUUID();
          replyMessageId = `reply-${runId}`;
          onRunStarted(runId);
          onRunStateChange({ phase: "running", runId });
          push({ type: "RUN_STARTED", threadId, runId });

          const afterCursor = accepted.eventCursor != null ? String(accepted.eventCursor) : undefined;

          unsubscribe = consumeEventIterator(
            apiClient.ironclaw.threads.streamEvents({ id: threadId, afterCursor }),
            {
              onEvent: (event: any) => {
                if (closed) return;

                switch (event.type) {
                  case "accepted": {
                    onRunStateChange({
                      phase: "running",
                      runId: event.ack?.runId ?? runId,
                      message: undefined,
                    });
                    break;
                  }
                  case "running": {
                    onRunStateChange({ phase: "running", runId: event.runState?.runId ?? runId });
                    break;
                  }
                  case "gate": {
                    const g = event.prompt ?? {};
                    onRunStateChange({
                      phase: "awaiting_approval",
                      gateRef: g.gateRef,
                      gateHeadline: g.headline,
                      gateBody: g.body,
                    });
                    push({
                      type: "GATE_REQUIRED",
                      runId,
                      gateRef: g.gateRef,
                      headline: g.headline,
                      body: g.body,
                    });
                    break;
                  }
                  case "auth_required": {
                    const a = event.authPrompt ?? {};
                    onRunStateChange({
                      phase: "auth_required",
                      authRequestRef: a.authRequestRef,
                      authHeadline: a.headline,
                      authBody: a.body,
                      authUrl: a.authorizationUrl,
                    });
                    push({
                      type: "AUTH_REQUIRED",
                      runId,
                      authRequestRef: a.authRequestRef,
                      headline: a.headline,
                      body: a.body,
                      authorizationUrl: a.authorizationUrl,
                    });
                    break;
                  }
                  case "final_reply": {
                    const reply = event.reply ?? {};
                    if (reply.text) {
                      push({
                        type: "TEXT_MESSAGE_START",
                        messageId: replyMessageId,
                        role: "assistant",
                      });
                      push({
                        type: "TEXT_MESSAGE_CONTENT",
                        messageId: replyMessageId,
                        delta: reply.text,
                        content: reply.text,
                      });
                      push({ type: "TEXT_MESSAGE_END", messageId: replyMessageId });
                    }
                    break;
                  }
                  case "failed": {
                    const failMsg =
                      event.response?.status ?? event.runState?.failure ?? "Run failed";
                    const msg = typeof failMsg === "string" ? failMsg : JSON.stringify(failMsg);
                    onRunStateChange({ phase: "failed", message: msg });
                    push({ type: "RUN_FAILED", runId, message: msg });
                    finish();
                    break;
                  }
                  case "cancelled": {
                    onRunStateChange({ phase: "cancelled", message: "Run was cancelled" });
                    push({ type: "RUN_CANCELLED", runId });
                    finish();
                    break;
                  }
                  case "capability_progress":
                  case "capability_activity":
                  case "capability_display_preview": {
                    push({ type: "CAPABILITY_UPDATED", runId, event });
                    break;
                  }
                  case "projection_snapshot":
                  case "projection_update":
                    break;
                  case "keep_alive":
                    break;
                  default:
                    break;
                }
              },
              onError: (err: any) => {
                const msg = err instanceof Error ? err.message : String(err);
                onRunStateChange({ phase: "disconnected", message: msg });
                fail(msg);
              },
              onFinish: (_state: any) => {
                push({ type: "RUN_FINISHED", runId, finishReason: "stop" });
                finish();
              },
            },
          );
        } catch (error) {
          const msg = error instanceof Error ? error.message : String(error);
          onRunStateChange({ phase: "failed", message: msg });
          fail(msg);
        }
      })();

      return {
        next(): Promise<IteratorResult<StreamChunk>> {
          if (queue.length > 0) {
            const value = queue.shift()!;
            return Promise.resolve({ value: value as StreamChunk, done: false });
          }
          if (closed) {
            return Promise.resolve({ value: undefined, done: true });
          }
          return new Promise<IteratorResult<StreamChunk>>((resolve) => {
            waiters.push(resolve as (result: IteratorResult<Record<string, unknown>>) => void);
          });
        },
        return(): Promise<IteratorResult<StreamChunk>> {
          finish();
          return Promise.resolve({ value: undefined, done: true });
        },
        [Symbol.asyncIterator]() {
          return this;
        },
      };
    },
  };
}

export function useIronclawChat(
  threadId: string,
  apiClient: ApiClient,
  initialMessages: Array<UIMessage>,
) {
  const activeRunIdRef = useRef<string | null>(null);
  const pendingAttachmentsRef = useRef<StagedAttachment[]>([]);
  const [runState, setRunState] = useState<RunState>({ phase: "idle" });

  const initialMessagesRef = useRef(initialMessages);

  const chat = useChat({
    threadId,
    initialMessages,
    fetcher: async ({ messages }, { signal }) => {
      const lastUser = [...messages].reverse().find((message) => message.role === "user");
      const content = lastUser ? messageTextContent(lastUser) : "";
      const attachments = pendingAttachmentsRef.current;
      pendingAttachmentsRef.current = [];

      setRunState({ phase: "submitted", message: undefined });

      return createIronclawStream({
        apiClient,
        threadId,
        content,
        clientActionId: `ui-${crypto.randomUUID()}`,
        attachments,
        signal,
        onRunStarted: (runId) => {
          activeRunIdRef.current = runId;
        },
        onRunStateChange: (update) => {
          setRunState((prev) => ({ ...prev, ...update }));
        },
      });
    },
    onError: (error) => {
      console.error("[useIronclawChat]", error);
      setRunState({
        phase: "failed",
        message: error instanceof Error ? error.message : String(error),
      });
    },
  });

  useEffect(() => {
    if (initialMessages.length > 0 && initialMessages !== initialMessagesRef.current) {
      initialMessagesRef.current = initialMessages;
      chat.setMessages(initialMessages);
    }
  }, [initialMessages, chat]);

  useEffect(() => {
    if (
      !chat.isLoading &&
      runState.phase !== "idle" &&
      runState.phase !== "failed" &&
      runState.phase !== "cancelled"
    ) {
      setRunState({ phase: "idle" });
    }
  }, [chat.isLoading, runState.phase]);

  const sendMessageWithAttachments = useCallback(
    (content: string, attachments?: StagedAttachment[]) => {
      pendingAttachmentsRef.current = attachments ?? [];
      chat.sendMessage(content);
    },
    [chat.sendMessage],
  );

  const resolveGate = useCallback(
    async (gateRef: string, approved: boolean) => {
      chat.addToolApprovalResponse({ id: gateRef, approved });
      const runId = activeRunIdRef.current;
      if (!runId) {
        throw new Error("Missing run ID for gate resolution");
      }
      await apiClient.ironclaw.threads.resolveGate({
        id: threadId,
        runId,
        gateRef,
        resolution: approved ? "approved" : "denied",
      });
    },
    [apiClient, chat, threadId],
  );

  return {
    messages: chat.messages,
    sendMessage: sendMessageWithAttachments,
    isLoading: chat.isLoading,
    status: chat.status,
    error: chat.error,
    stop: chat.stop,
    setMessages: chat.setMessages,
    resolveGate,
    runState,
  };
}
