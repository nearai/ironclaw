import { useCallback, useEffect, useRef } from "react";
import { useChat } from "@tanstack/ai-react";
import type { StreamChunk, UIMessage } from "@tanstack/ai/client";
import type { ApiClient } from "@/app";
import { openIronclawEventSource } from "@/lib/ironclaw-sse";

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
  onRunStarted,
  signal,
}: {
  apiClient: ApiClient;
  threadId: string;
  content: string;
  clientActionId: string;
  onRunStarted: (runId: string) => void;
  signal: AbortSignal;
}): AsyncIterable<StreamChunk> {
  return {
    [Symbol.asyncIterator]() {
      const queue: Array<Record<string, unknown>> = [];
      const waiters: Array<(result: IteratorResult<Record<string, unknown>>) => void> = [];
      let closed = false;
      let sourceHandle: { close: () => void } | null = null;

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
        sourceHandle?.close();
        sourceHandle = null;
        while (waiters.length > 0) {
          const waiter = waiters.shift();
          waiter?.({ value: undefined, done: true });
        }
      };

      const fail = (message: string) => {
        push({ type: "RUN_ERROR", message });
        finish();
      };

      signal.addEventListener("abort", abort, { once: true });

      function abort() {
        finish();
      }

      void (async () => {
        let runId: string;
        let replyMessageId = "reply-pending";

        try {
          const accepted = await (apiClient as any).conversation.sendMessage({
            threadId,
            content,
            clientActionId,
          });
          runId = accepted.runId ?? crypto.randomUUID();
          replyMessageId = `reply-${runId}`;
          onRunStarted(runId);
          push({ type: "RUN_STARTED", threadId, runId });

          sourceHandle = openIronclawEventSource({
            threadId,
            afterCursor: accepted.eventCursor ?? undefined,
            onEvent: (envelope) => {
              const event = envelope.event as Record<string, unknown>;
              if (closed) return;

              if (event.type === "message_added") {
                const msg = (event as any).message;
                if (msg?.role === "assistant" && msg?.text) {
                  push({ type: "TEXT_MESSAGE_START", messageId: replyMessageId, role: "assistant" });
                  push({ type: "TEXT_MESSAGE_CONTENT", messageId: replyMessageId, delta: msg.text, content: msg.text });
                  push({ type: "TEXT_MESSAGE_END", messageId: replyMessageId });
                }
                return;
              }

              if (event.type === "run_finished") {
                push({ type: "RUN_FINISHED", runId, finishReason: "stop" });
                finish();
                return;
              }

              if (event.type === "error") {
                console.error("[createIronclawStream] normalized SSE error — run may still be processing on backend", typeof (event as any).error === "string" ? (event as any).error : String(event));
                fail(typeof (event as any).error === "string" ? (event as any).error : "Run failed");
                return;
              }
            },
            onError: (status) => {
              if (status === "disconnected") {
                fail("Event stream disconnected");
              }
            },
          });
        } catch (error) {
          fail(error instanceof Error ? error.message : String(error));
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

  const initialMessagesRef = useRef(initialMessages);

  const chat = useChat({
    threadId,
    initialMessages,
    fetcher: async ({ messages }, { signal }) => {
      const lastUser = [...messages].reverse().find((message) => message.role === "user");
      const content = lastUser ? messageTextContent(lastUser) : "";

      return createIronclawStream({
        apiClient,
        threadId,
        content,
        clientActionId: `ui-${crypto.randomUUID()}`,
        signal,
        onRunStarted: (runId) => {
          activeRunIdRef.current = runId;
        },
      });
    },
    onError: (error) => {
      console.error("[useIronclawChat]", error);
    },
  });

  useEffect(() => {
    if (
      initialMessages.length > 0 &&
      initialMessages !== initialMessagesRef.current
    ) {
      initialMessagesRef.current = initialMessages;
      chat.setMessages(initialMessages);
    }
  }, [initialMessages, chat]);

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
    sendMessage: chat.sendMessage,
    isLoading: chat.isLoading,
    status: chat.status,
    error: chat.error,
    stop: chat.stop,
    setMessages: chat.setMessages,
    resolveGate,
  };
}
