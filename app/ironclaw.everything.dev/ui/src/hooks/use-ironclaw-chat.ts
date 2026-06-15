import type { StreamChunk } from "@tanstack/ai";
import type { UIMessage } from "@tanstack/ai/client";
import { useChat } from "@tanstack/ai-react";
import { useCallback, useRef, useState } from "react";
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
    | "cancelled";
  runId?: string;
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
  debugCustomFlows = false,
) {
  const activeRunIdRef = useRef<string | null>(null);
  const pendingAttachmentsRef = useRef<StagedAttachment[]>([]);
  const lastSentContentRef = useRef<string>("");
  const lastSentAttachmentsRef = useRef<StagedAttachment[]>([]);
  const [runState, setRunState] = useState<RunState>({ phase: "idle" });
  const onRunStateChangeRef = useRef<(update: Partial<RunState>) => void>(() => {});
  const runStateRef = useRef<RunState>({ phase: "idle" });
  const handledCustomEventNamesRef = useRef(
    new Set([
      "ironclaw.accepted",
      "ironclaw.running",
      "ironclaw.capability-progress",
      "ironclaw.capability-display-preview",
      "ironclaw.capability-activity",
      "ironclaw.gate",
      "ironclaw.auth-required",
      "ironclaw.final-reply",
      "ironclaw.failed",
      "ironclaw.cancelled",
    ]),
  );

  onRunStateChangeRef.current = (update) => {
    setRunState((prev) => {
      const next = { ...prev, ...update };
      runStateRef.current = next;
      return next;
    });
  };

  const logCustomFlow = useCallback(
    (kind: "swallowed" | "unhandled", name: string, value: unknown) => {
      if (!debugCustomFlows) return;
      const base = {
        threadId,
        runId: activeRunIdRef.current,
        phase: runStateRef.current.phase,
        name,
        value,
      };
      if (kind === "swallowed") {
        console.debug("[chat.custom.swallowed]", base);
      } else {
        console.warn(`[chat.custom.unhandled] event="${name}"`, base.value);
      }
    },
    [debugCustomFlows, threadId],
  );

  if (debugCustomFlows) {
    console.debug(`[useIronclawChat] seed: ${initialMessages.length} msgs`, initialMessages.map((m) => ({ id: m.id, role: m.role })));
  }

  const chat = useChat({
    threadId,
    initialMessages,
    fetcher: async function* ({ messages }, { signal }) {
      const lastUser = [...messages].reverse().find((m) => m.role === "user");
      const content = lastUser ? messageTextContent(lastUser) : "";
      const attachments = pendingAttachmentsRef.current;
      pendingAttachmentsRef.current = [];
      lastSentContentRef.current = content;
      lastSentAttachmentsRef.current = attachments ?? [];

      onRunStateChangeRef.current({ phase: "submitted", message: undefined, activeToolName: undefined });

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

      const live = await apiClient.conversation.live({
        threadId,
        runId: accepted.runId ?? accepted.activeRunId,
        afterCursor: accepted.eventCursor != null ? String(accepted.eventCursor) : undefined,
      });

      activeRunIdRef.current = accepted.runId ?? accepted.activeRunId ?? null;

      for await (const chunk of live as AsyncIterable<StreamChunk>) {
        if (signal.aborted) return;
        yield chunk;
      }
    },
    onChunk: (chunk: StreamChunk) => {
      if (debugCustomFlows) {
        if (chunk.type === "TEXT_MESSAGE_CONTENT") {
          console.debug(`[chat.chunk] type=${chunk.type} delta=${(chunk as any).delta?.slice(0, 80)}`);
        } else if (chunk.type === "TOOL_CALL_START") {
          console.debug(`[chat.chunk] type=${chunk.type} parentMessageId=${(chunk as any).parentMessageId} toolCallName=${(chunk as any).toolCallName}`);
        } else {
          console.debug(`[chat.chunk] type=${chunk.type}`);
        }
      }
      if (chunk.type === "CUSTOM") {
        if (chunk.name === "approval-requested") {
          logCustomFlow("swallowed", chunk.name, chunk.value);
        } else if (!handledCustomEventNamesRef.current.has(chunk.name)) {
          logCustomFlow("unhandled", chunk.name, chunk.value);
        }
      }

      if (chunk.type === "RUN_STARTED") {
        const { runId } = chunk;
        activeRunIdRef.current = runId ?? activeRunIdRef.current;
        setRunState((prev) => ({
          ...prev,
          phase: "running",
          runId: runId ?? prev.runId,
          message: undefined,
          gateRef: undefined,
          gateHeadline: undefined,
          gateBody: undefined,
          authRequestRef: undefined,
          authHeadline: undefined,
          authBody: undefined,
          authUrl: undefined,
        }));
        return;
      }

      if (chunk.type === "RUN_ERROR") {
        setRunState((prev) => ({
          ...prev,
          phase: "failed",
          message: chunk.message ?? "Run failed",
          activeToolName: undefined,
        }));
        return;
      }

      if (chunk.type === "RUN_FINISHED") {
        setRunState((prev) => {
          if (prev.phase === "failed" || prev.phase === "cancelled") return prev;
          return {
            ...prev,
            phase: "idle",
            activeToolName: undefined,
            gateRef: undefined,
            gateHeadline: undefined,
            gateBody: undefined,
            authRequestRef: undefined,
            authHeadline: undefined,
            authBody: undefined,
            authUrl: undefined,
          };
        });
      }
    },
    onCustomEvent: (eventType, data) => {
      const payload = data as Record<string, unknown> | undefined;

      if (eventType === "approval-requested") {
        setRunState((prev) => ({
          ...prev,
          phase: "awaiting_approval",
          gateRef: typeof payload?.approval === "object" && payload?.approval !== null ? String((payload.approval as Record<string, unknown>).id ?? "") : undefined,
          gateHeadline: typeof payload?.headline === "string" ? payload.headline : prev.gateHeadline,
          gateBody: typeof payload?.body === "string" ? payload.body : prev.gateBody,
          activeToolName: typeof payload?.toolName === "string" ? payload.toolName : prev.activeToolName,
        }));
        return;
      }

      if (eventType === "ironclaw.accepted" || eventType === "ironclaw.running") {
        setRunState((prev) => ({
          ...prev,
          phase: "running",
          runId: typeof payload?.runId === "string" ? payload.runId : prev.runId,
          activeToolName: typeof payload?.toolName === "string" ? payload.toolName : prev.activeToolName,
          message: undefined,
        }));
        return;
      }

      if (eventType === "ironclaw.capability-progress") {
        setRunState((prev) => ({
          ...prev,
          phase: "running",
          activeToolName: typeof payload?.toolName === "string" ? payload.toolName : prev.activeToolName,
        }));
        return;
      }

      if (eventType === "ironclaw.capability-display-preview" || eventType === "ironclaw.capability-activity") {
        setRunState((prev) => ({
          ...prev,
          phase: "running",
          activeToolName:
            typeof payload?.toolName === "string"
              ? payload.toolName
              : typeof payload?.title === "string"
                ? payload.title
                : prev.activeToolName,
        }));
        return;
      }

      if (eventType === "ironclaw.gate") {
        setRunState((prev) => ({
          ...prev,
          phase: "awaiting_approval",
          gateRef: typeof payload?.gateRef === "string" ? payload.gateRef : prev.gateRef,
          gateHeadline: typeof payload?.headline === "string" ? payload.headline : prev.gateHeadline,
          gateBody: typeof payload?.body === "string" ? payload.body : prev.gateBody,
          activeToolName: typeof payload?.toolName === "string" ? payload.toolName : prev.activeToolName,
        }));
        return;
      }

      if (eventType === "ironclaw.auth-required") {
        setRunState((prev) => ({
          ...prev,
          phase: "auth_required",
          authRequestRef:
            typeof payload?.authRequestRef === "string" ? payload.authRequestRef : prev.authRequestRef,
          authHeadline: typeof payload?.headline === "string" ? payload.headline : prev.authHeadline,
          authBody: typeof payload?.body === "string" ? payload.body : prev.authBody,
          authUrl: typeof payload?.authorizationUrl === "string" ? payload.authorizationUrl : prev.authUrl,
        }));
        return;
      }

      if (eventType === "ironclaw.final-reply") {
        setRunState((prev) => ({
          ...prev,
          activeToolName: undefined,
        }));
        return;
      }

      if (eventType === "ironclaw.failed") {
        setRunState((prev) => ({
          ...prev,
          phase: "failed",
          message: typeof payload?.message === "string" ? payload.message : "Run failed",
          activeToolName: undefined,
        }));
        return;
      }

      if (eventType === "ironclaw.cancelled") {
        setRunState((prev) => ({
          ...prev,
          phase: "cancelled",
          message: "Run was cancelled",
          activeToolName: undefined,
        }));
      }
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

  const retry = useCallback(() => {
    const content = lastSentContentRef.current;
    const attachments = lastSentAttachmentsRef.current;
    if (content) {
      pendingAttachmentsRef.current = attachments;
      chat.sendMessage(content);
    }
  }, [chat.sendMessage]);

  const idle = useCallback(() => {
    setRunState({ phase: "idle", activeToolName: undefined });
  }, []);

  return {
    messages: chat.messages,
    sendMessage: sendMessageWithAttachments,
    status: chat.status,
    stop: chat.stop,
    setMessages: chat.setMessages,
    resolveGate,
    retry,
    idle,
    runState,
  };
}
