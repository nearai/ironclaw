import { useCallback, useRef } from "react";
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

function promptToolName(prompt: Record<string, unknown> | undefined): string {
  const ctx = prompt?.approvalContext as Record<string, unknown> | undefined;
  return (ctx?.toolName as string) ?? (prompt?.headline as string) ?? "approval";
}

function errorMessageFromFailure(failure: unknown): string {
  if (failure && typeof failure === "object" && "message" in failure) {
    const value = (failure as { message?: unknown }).message;
    if (typeof value === "string" && value.length > 0) return value;
  }
  return "Run failed";
}

function createIronclawStream({
  apiClient,
  threadId,
  content,
  clientActionId,
  onRunStarted,
  onGateSeen,
  signal,
}: {
  apiClient: ApiClient;
  threadId: string;
  content: string;
  clientActionId: string;
  onRunStarted: (runId: string) => void;
  onGateSeen: (gateRef: string, runId: string) => void;
  signal: AbortSignal;
}): AsyncIterable<StreamChunk> {
  function getEventCursor(event: Record<string, unknown>): number | undefined {
    const c = (event as any).cursor ?? (event as any).ack?.eventCursor ?? (event as any).runState?.eventCursor;
    return c !== undefined ? Number(c) : undefined;
  }

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
        let sendCursor: number | undefined;
        let runId: string | undefined;
        let replyMessageId = "reply-pending";
        const preSendBuffer: Array<Record<string, unknown>> = [];

        const processEvent = (event: Record<string, unknown>) => {
          if (closed) return;

          if (sendCursor !== undefined) {
            const cursor = getEventCursor(event);
            if (cursor !== undefined && cursor <= sendCursor) return;
          }

          switch (event.type) {
            case "capability_progress": {
              const toolCallId = (event as any).progress?.turnRunId ?? `tool-${crypto.randomUUID()}`;
              push({
                type: "TOOL_CALL_START",
                toolCallId,
                toolCallName: (event as any).progress?.kind ?? "tool",
                toolName: (event as any).progress?.kind ?? "tool",
              });
              return;
            }

            case "capability_activity": {
              if (!(event as any).activity) return;
              push({
                type: "TOOL_CALL_END",
                toolCallId: (event as any).activity.invocationId,
                toolCallName: (event as any).activity.capabilityId,
                toolName: (event as any).activity.capabilityId,
                result: (event as any).activity.status,
                state: (event as any).activity.status === "completed" ? "output-available" : "output-error",
              });
              return;
            }

            case "capability_display_preview": {
              if (!(event as any).preview) return;
              push({
                type: "TOOL_CALL_END",
                toolCallId: (event as any).preview.invocationId,
                toolCallName: (event as any).preview.capabilityId,
                toolName: (event as any).preview.capabilityId,
                result: (event as any).preview.outputSummary ?? (event as any).preview.title,
                state: "output-available",
              });
              return;
            }

            case "gate": {
              const gatePrompt = (event as any).prompt as Record<string, unknown> | undefined;
              const gateRef = gatePrompt?.gateRef as string | undefined;
              if (!gateRef || !runId) return;
              onGateSeen(gateRef, runId);
              const toolName = promptToolName(gatePrompt);
              push({ type: "TOOL_CALL_START", toolCallId: gateRef, toolCallName: toolName, toolName });
              push({
                type: "CUSTOM",
                name: "approval-requested",
                value: {
                  toolCallId: gateRef,
                  toolName,
                  input: gatePrompt?.approvalContext ?? {},
                  approval: { id: gateRef, needsApproval: true },
                  runId,
                },
              });
              return;
            }

            case "auth_required": {
              push({ type: "CUSTOM", name: "auth-required", value: (event as any).authPrompt ?? {} });
              return;
            }

            case "final_reply": {
              const replyText = (event as any).reply?.text ?? "";
              push({ type: "TEXT_MESSAGE_START", messageId: replyMessageId, role: "assistant" });
              if (replyText.length > 0) {
                push({ type: "TEXT_MESSAGE_CONTENT", messageId: replyMessageId, delta: replyText, content: replyText });
              }
              push({ type: "TEXT_MESSAGE_END", messageId: replyMessageId });
              push({ type: "RUN_FINISHED", runId, finishReason: "stop" });
              finish();
              return;
            }

            case "cancelled": {
              push({ type: "RUN_FINISHED", runId, finishReason: "stop" });
              finish();
              return;
            }

            case "failed": {
              push({ type: "RUN_ERROR", message: errorMessageFromFailure((event as any).runState?.failure) });
              finish();
              return;
            }

            default:
              return;
          }
        };

        try {
          sourceHandle = openIronclawEventSource({
            threadId,
            onEvent: ({ event }) => {
              if (closed) return;
              if (sendCursor === undefined) {
                preSendBuffer.push(event as unknown as Record<string, unknown>);
                return;
              }
              processEvent(event as unknown as Record<string, unknown>);
            },
            onError: (status) => {
              if (status === "disconnected") {
                fail("Event stream disconnected");
              }
            },
          });

          const accepted = await apiClient.ironclaw.threads.sendMessage({
            id: threadId,
            content,
            clientActionId,
          });
          runId = accepted.runId ?? accepted.activeRunId ?? crypto.randomUUID();
          sendCursor = accepted.eventCursor != null ? Number(accepted.eventCursor) : 0;
          replyMessageId = `reply-${runId}`;
          onRunStarted(runId);
          push({ type: "RUN_STARTED", threadId, runId });

          for (const buffered of preSendBuffer) {
            processEvent(buffered);
          }
          preSendBuffer.length = 0;
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
  const gateRunIdsRef = useRef(new Map<string, string>());

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
        onGateSeen: (gateRef, runId) => {
          gateRunIdsRef.current.set(gateRef, runId);
        },
      });
    },
    onError: (error) => {
      console.error("[useIronclawChat]", error);
    },
  });

  const resolveGate = useCallback(
    async (gateRef: string, approved: boolean) => {
      chat.addToolApprovalResponse({ id: gateRef, approved });
      const runId = gateRunIdsRef.current.get(gateRef) ?? activeRunIdRef.current;
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
    resolveGate,
  };
}
