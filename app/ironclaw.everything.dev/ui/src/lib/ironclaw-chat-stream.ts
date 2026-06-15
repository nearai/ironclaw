import { EventType, type StreamChunk } from "@tanstack/ai";
import type { ApiClient } from "@/app";
import { serializeIronclawToolResultEnvelope } from "@/lib/ironclaw-message-parts";
import type { RunState } from "@/hooks/use-ironclaw-chat";

type AcceptedRun = {
  runId?: string;
  activeRunId?: string;
  eventCursor?: number;
};

type PendingPreview = {
  title?: string;
  inputSummary?: string;
  output: string;
  outputKind?: string;
  truncated?: boolean;
};

interface CreateIronclawChatStreamOptions {
  threadId: string;
  accepted: AcceptedRun;
  apiClient: ApiClient;
  signal: AbortSignal;
  onRunStarted: (runId: string) => void;
  onRunStateChange: (update: Partial<RunState>) => void;
}

function toolCallChunk(toolCallId: string, toolName: string): StreamChunk {
  return {
    type: EventType.TOOL_CALL_START,
    toolCallId,
    toolCallName: toolName,
    toolName,
    index: 0,
  } as StreamChunk;
}

function toolArgsChunk(toolCallId: string, delta: string): StreamChunk {
  return {
    type: EventType.TOOL_CALL_ARGS,
    toolCallId,
    delta,
    args: delta,
  } as StreamChunk;
}

function toolEndChunk(
  toolCallId: string,
  toolName: string,
  state?: "input-complete" | "approval-responded" | "complete" | "error",
  result?: string,
): StreamChunk {
  return {
    type: EventType.TOOL_CALL_END,
    toolCallId,
    toolCallName: toolName,
    toolName,
    ...(state ? { state } : {}),
    ...(result ? { result } : {}),
  } as StreamChunk;
}

function normalizeMessage(value: unknown): string {
  if (value instanceof Error) return value.message;
  return typeof value === "string" ? value : String(value);
}

export async function* createIronclawChatStream({
  threadId,
  accepted,
  apiClient,
  signal,
  onRunStarted,
  onRunStateChange,
}: CreateIronclawChatStreamOptions): AsyncGenerator<StreamChunk> {
  const runId = accepted.runId ?? accepted.activeRunId ?? crypto.randomUUID();
  const replyMessageId = `reply-${runId}`;
  const afterCursor = accepted.eventCursor != null ? String(accepted.eventCursor) : undefined;
  const activeToolCalls = new Map<string, { toolCallId: string; toolName: string }>();
  const pendingPreviews = new Map<string, PendingPreview>();
  const upstream = await apiClient.conversation.stream({ threadId, afterCursor });

  onRunStarted(runId);
  onRunStateChange({
    phase: "running",
    runId,
    message: undefined,
    replyMessageId: undefined,
    gateRef: undefined,
    gateHeadline: undefined,
    gateBody: undefined,
    authRequestRef: undefined,
    authHeadline: undefined,
    authBody: undefined,
    authUrl: undefined,
  });

  yield { type: EventType.RUN_STARTED, threadId, runId };

  try {
    for await (const event of upstream as AsyncIterable<any>) {
      if (signal.aborted) return;

      const eventAny = event as any;
      const type = eventAny.type as string;

      if (type === "accepted" || type === "running") {
        const currentRunId = eventAny.runState?.runId ?? eventAny.ack?.runId ?? runId;
        onRunStateChange({
          phase: "running",
          runId: currentRunId,
          message: undefined,
          gateRef: undefined,
          gateHeadline: undefined,
          gateBody: undefined,
          authRequestRef: undefined,
          authHeadline: undefined,
          authBody: undefined,
          authUrl: undefined,
        });
        continue;
      }

      if (type === "gate") {
        const prompt = eventAny.prompt ?? {};
        const approvalContext = prompt.approvalContext ?? {};
        const gateToolName = approvalContext.toolName ?? "approval";
        const gateToolCallId = prompt.gateRef ?? `gate-${gateToolName}-${runId}`;

        onRunStateChange({
          phase: "awaiting_approval",
          gateRef: prompt.gateRef,
          gateHeadline: prompt.headline,
          gateBody: prompt.body,
        });

        if (!activeToolCalls.has(gateToolCallId)) {
          activeToolCalls.set(gateToolCallId, { toolCallId: gateToolCallId, toolName: gateToolName });
          yield toolCallChunk(gateToolCallId, gateToolName);
          yield toolArgsChunk(gateToolCallId, JSON.stringify({ input: approvalContext }));
        }

        yield toolEndChunk(gateToolCallId, gateToolName);
        yield {
          type: EventType.CUSTOM,
          name: "approval-requested",
          value: {
            toolCallId: gateToolCallId,
            toolName: gateToolName,
            input: JSON.stringify(approvalContext),
            approval: { id: prompt.gateRef, needsApproval: true },
          },
        };
        continue;
      }

      if (type === "auth_required") {
        const authPrompt = eventAny.authPrompt ?? {};
        onRunStateChange({
          phase: "auth_required",
          authRequestRef: authPrompt.authRequestRef,
          authHeadline: authPrompt.headline,
          authBody: authPrompt.body,
          authUrl: authPrompt.authorizationUrl,
        });
        continue;
      }

      if (type === "capability_display_preview") {
        const preview = eventAny.preview ?? {};
        const invocationId = preview.invocationId as string | undefined;
        if (!invocationId) continue;

        const title = preview.title as string | undefined;
        const capabilityId = preview.capabilityId as string | undefined;
        const displayName = title ?? capabilityId ?? "unknown";

        pendingPreviews.set(invocationId, {
          title,
          inputSummary: preview.inputSummary as string | undefined,
          output: (preview.outputSummary as string) ?? (preview.outputPreview as string) ?? "",
          outputKind: preview.outputKind as string | undefined,
          truncated: Boolean(preview.truncated),
        });

        if (!activeToolCalls.has(invocationId)) {
          activeToolCalls.set(invocationId, { toolCallId: invocationId, toolName: displayName });
          onRunStateChange({ activeToolName: displayName });
          yield toolCallChunk(invocationId, displayName);
          yield toolArgsChunk(invocationId, JSON.stringify({ input: preview.inputSummary ?? "" }));
        }
        continue;
      }

      if (type === "capability_activity") {
        const activity = eventAny.activity ?? {};
        const invocationId = activity.invocationId as string | undefined;
        const capabilityId = activity.capabilityId as string | undefined;
        const activityStatus = activity.status as string | undefined;
        const errorKind = activity.errorKind as string | undefined;
        if (!invocationId || !capabilityId) continue;

        const preview = pendingPreviews.get(invocationId);
        const displayName = preview?.title ?? capabilityId;
        const isTerminal = activityStatus === "completed" || activityStatus === "failed" || activityStatus === "killed";

        if (activityStatus === "started" || activityStatus === "running") {
          if (!activeToolCalls.has(invocationId)) {
            activeToolCalls.set(invocationId, { toolCallId: invocationId, toolName: displayName });
            onRunStateChange({ activeToolName: displayName });
            yield toolCallChunk(invocationId, displayName);
          }
          continue;
        }

        if (isTerminal) {
          if (!activeToolCalls.has(invocationId)) {
            activeToolCalls.set(invocationId, { toolCallId: invocationId, toolName: displayName });
            onRunStateChange({ activeToolName: displayName });
            yield toolCallChunk(invocationId, displayName);
            yield toolArgsChunk(invocationId, JSON.stringify({ input: preview?.inputSummary ?? "" }));
          }

          const state = activityStatus === "failed" || activityStatus === "killed" ? "error" : "complete";
          yield toolEndChunk(
            invocationId,
            displayName,
            state,
            serializeIronclawToolResultEnvelope({
              output: preview?.output ?? (errorKind ? `Error: ${errorKind}` : ""),
              outputKind: preview?.outputKind ?? null,
              truncated: preview?.truncated ?? false,
              inputSummary: preview?.inputSummary ?? null,
              title: displayName,
            }),
          );
          pendingPreviews.delete(invocationId);
          activeToolCalls.delete(invocationId);
          onRunStateChange({ activeToolName: undefined });
        }
      }

      if (type === "final_reply") {
        const reply = eventAny.reply ?? {};
        onRunStateChange({
          phase: reply.text ? "finalizing" : "idle",
          replyMessageId: reply.text ? replyMessageId : undefined,
          activeToolName: undefined,
        });
        if (reply.text) {
          yield { type: EventType.TEXT_MESSAGE_START, messageId: replyMessageId, role: "assistant" };
          yield { type: EventType.TEXT_MESSAGE_CONTENT, messageId: replyMessageId, delta: reply.text };
          yield { type: EventType.TEXT_MESSAGE_END, messageId: replyMessageId };
        }
        yield { type: EventType.RUN_FINISHED, threadId, runId, finishReason: "stop" };
        return;
      }

      if (type === "failed") {
        const failMsg = eventAny.response?.status ?? eventAny.runState?.failure ?? "Run failed";
        const message = normalizeMessage(failMsg);
        onRunStateChange({ phase: "failed", message, activeToolName: undefined });
        yield { type: EventType.RUN_ERROR, threadId, message };
        return;
      }

      if (type === "cancelled") {
        onRunStateChange({ phase: "cancelled", message: "Run was cancelled", activeToolName: undefined });
        yield { type: EventType.RUN_FINISHED, threadId, runId, finishReason: null };
        return;
      }
    }

    yield { type: EventType.RUN_FINISHED, threadId, runId, finishReason: "stop" };
  } catch (error) {
    if (signal.aborted) return;
    const message = normalizeMessage(error);
    onRunStateChange({ phase: "disconnected", message, activeToolName: undefined });
    yield { type: EventType.RUN_ERROR, threadId, message };
  } finally {
    if (typeof upstream.return === "function") {
      try {
        await upstream.return(undefined);
      } catch {
        // ignore close failures
      }
    }
  }
}
