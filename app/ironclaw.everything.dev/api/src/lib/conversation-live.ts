import { ConversationLiveChunkSchema } from "../contract";
import type { ChatEvent } from "../../../plugins/ironclaw/src/contract";

type LiveChunkType =
  | "RUN_STARTED"
  | "RUN_FINISHED"
  | "RUN_ERROR"
  | "TOOL_CALL_START"
  | "TOOL_CALL_ARGS"
  | "TOOL_CALL_END"
  | "TEXT_MESSAGE_START"
  | "TEXT_MESSAGE_CONTENT"
  | "TEXT_MESSAGE_END"
  | "CUSTOM";

type LiveChunk = {
  type: LiveChunkType;
  threadId: string;
  runId?: string;
  messageId?: string;
  parentMessageId?: string;
  role?: "assistant" | "tool";
  toolCallId?: string;
  toolCallName?: string;
  toolName?: string;
  index?: number;
  delta?: string;
  args?: string;
  input?: unknown;
  result?: string;
  state?: string;
  finishReason?: string | null;
  message?: string;
  name?: string;
  value?: unknown;
};

function normalizeMessage(value: unknown): string {
  if (value instanceof Error) return value.message;
  return typeof value === "string" ? value : String(value);
}

function serializeToolResultEnvelope(envelope: {
  output: string;
  outputKind: string | null;
  truncated: boolean;
  inputSummary: string | null;
  title: string;
}): string {
  return JSON.stringify(envelope);
}

function resolveToolCallId(
  preview: ChatEvent["preview"],
  activity: ChatEvent["activity"],
): string | undefined {
  const activityRecord = activity as unknown as Record<string, string | undefined>;
  return (
    preview?.invocationId ||
    preview?.timelineMessageId ||
    activity?.invocationId ||
    activityRecord.timelineMessageId
  ) ?? undefined;
}

function extractEventRunId(event: ChatEvent): string | undefined {
  return (
    event.ack?.runId ||
    event.ack?.activeRunId ||
    event.response?.runId ||
    event.reply?.turnRunId ||
    event.progress?.turnRunId ||
    event.activity?.turnRunId ||
    event.preview?.turnRunId
  ) || undefined;
}

function projVal(item: Record<string, unknown>, ...keys: string[]): unknown {
  for (const key of keys) {
    const v = item[key];
    if (v !== undefined) return v;
  }
  return undefined;
}

function getProjectionRunStatus(item: Record<string, unknown>): Record<string, unknown> | undefined {
  return projVal(item, "runStatus", "run_status") as Record<string, unknown> | undefined;
}

function getProjectionText(item: Record<string, unknown>): Record<string, unknown> | undefined {
  return projVal(item, "text", "Text") as Record<string, unknown> | undefined;
}

function getProjectionThinking(item: Record<string, unknown>): Record<string, unknown> | undefined {
  return projVal(item, "thinking", "Thinking") as Record<string, unknown> | undefined;
}

function getProjectionCapabilityActivity(item: Record<string, unknown>): Record<string, unknown> | undefined {
  return projVal(item, "capabilityActivity", "capability_activity") as Record<string, unknown> | undefined;
}

function getProjectionGate(item: Record<string, unknown>): Record<string, unknown> | undefined {
  return projVal(item, "gate", "Gate") as Record<string, unknown> | undefined;
}

function findAssistantTextForRun(entries: any[], runId: string): string | undefined {
  for (let i = entries.length - 1; i >= 0; i--) {
    const e = entries[i];
    const eRunId = e.turnRunId ?? e.turn_run_id ?? e.runId ?? e.run_id;
    if (eRunId === runId) {
      const lower = (e.kind ?? "").toLowerCase();
      const role = e.role ?? "";
      if (role === "assistant" || lower === "assistant" || lower === "assistant_message") {
        if (e.content) return e.content as string;
      }
    }
  }
  return undefined;
}

function createChunk(chunk: LiveChunk): LiveChunk {
  const result = ConversationLiveChunkSchema.safeParse(chunk);
  if (!result.success) {
    console.error("[live] schema mismatch:", result.error.format(), JSON.stringify(chunk));
  }
  return chunk;
}

export function createConversationLiveHandler(services: { ironclaw: (ctx: any) => any }) {
  return async function* ({ input, signal, context }: any) {
    const ic = services.ironclaw(context);
    const threadId = input.threadId as string;
    const expectedRunId = (input.runId as string | undefined) ?? undefined;
    const afterCursor = (input.afterCursor as string | undefined) ?? undefined;
    const runIdFallback = expectedRunId ?? crypto.randomUUID();
    const upstream = await ic.threads.streamEvents({ id: threadId, afterCursor });
    const pendingPreviews = new Map<string, ChatEvent["preview"]>();
    const activeToolCalls = new Set<string>();
    let runStarted = false;
    let runMatched = !expectedRunId;
    let boundRunId: string | undefined = expectedRunId || undefined;
    let terminalTextEmitted = false;
    let seenRunIds = new Set<string>();
    let seenTextIds = new Set<string>();

    const emitRunStarted = (runId: string | undefined) => {
      if (runStarted) return;
      runStarted = true;
      return createChunk({ type: "RUN_STARTED", threadId, runId: runId ?? runIdFallback });
    };

    const emitCustom = (name: string, value: unknown, runId?: string): LiveChunk =>
      createChunk({ type: "CUSTOM", threadId, runId, name, value });

    const assistantMessageId = (runId: string): string => `assistant:${runId}`;

    const emitToolStart = (toolCallId: string, toolName: string, runId?: string): LiveChunk =>
      createChunk({
        type: "TOOL_CALL_START",
        threadId,
        runId,
        parentMessageId: runId ? assistantMessageId(runId) : undefined,
        toolCallId,
        toolCallName: toolName,
        toolName,
        index: 0,
      });

    const emitToolArgs = (toolCallId: string, input: string, runId?: string): LiveChunk =>
      createChunk({
        type: "TOOL_CALL_ARGS",
        threadId,
        runId,
        toolCallId,
        delta: input,
        args: input,
      });

    const emitToolEnd = (
      toolCallId: string,
      toolName: string,
      state: "complete" | "error",
      result: string,
      input?: unknown,
      runId?: string,
    ): LiveChunk =>
      createChunk({
        type: "TOOL_CALL_END",
        threadId,
        runId,
        toolCallId,
        toolCallName: toolName,
        toolName,
        state,
        input,
        result,
      });

    try {
      for await (const raw of upstream as AsyncIterable<ChatEvent>) {
        if (signal?.aborted) break;

        const type = raw.type;
        const eventRunId = extractEventRunId(raw) ?? expectedRunId ?? runIdFallback;

        if (!runMatched) {
          if (!eventRunId || eventRunId !== expectedRunId) continue;
          runMatched = true;
          boundRunId = eventRunId;
          console.log(`[live] bound to runId=${boundRunId}`);
        }

        if (type === "accepted" || type === "running") {
          const chunk = emitRunStarted(eventRunId);
          if (chunk) yield chunk;
          yield emitCustom(`ironclaw.${type}`, { runId: eventRunId, ...raw }, eventRunId);
          continue;
        }

        if (type === "capability_progress") {
          const chunk = emitRunStarted(eventRunId);
          if (chunk) yield chunk;
          yield emitCustom("ironclaw.capability-progress", raw.progress ?? raw, eventRunId);
          continue;
        }

        if (type === "capability_display_preview") {
          const preview = raw.preview;
          const invocationId = resolveToolCallId(preview, undefined);
          const capabilityId = preview?.capabilityId;
          const title = preview?.title ?? capabilityId ?? "unknown";

          if (invocationId && preview) {
            pendingPreviews.set(invocationId, preview);
            const chunk = emitRunStarted(eventRunId);
            if (chunk) yield chunk;
            if (!activeToolCalls.has(invocationId)) {
              activeToolCalls.add(invocationId);
              yield emitToolStart(invocationId, title, eventRunId);
              yield emitToolArgs(invocationId, JSON.stringify({ input: preview.inputSummary ?? "" }), eventRunId);
            }
            yield emitCustom(
              "ironclaw.capability-display-preview",
              { ...preview, toolCallId: invocationId, toolName: title },
              eventRunId,
            );
          }
          continue;
        }

        if (type === "capability_activity") {
          const activity = raw.activity;
          const invocationId = resolveToolCallId(undefined, activity);
          const capabilityId = activity?.capabilityId;
          const status = activity?.status;
          const errorKind = activity?.errorKind;

          if (!invocationId || !capabilityId) continue;

          const preview = pendingPreviews.get(invocationId);
          const title = preview?.title ?? capabilityId;
          const chunk = emitRunStarted(eventRunId);
          if (chunk) yield chunk;

          if (status === "started" || status === "running") {
            if (!activeToolCalls.has(invocationId)) {
              activeToolCalls.add(invocationId);
              yield emitToolStart(invocationId, title, eventRunId);
              yield emitToolArgs(
                invocationId,
                JSON.stringify({ input: preview?.inputSummary ?? "" }),
                eventRunId,
              );
            }
            yield emitCustom(
              "ironclaw.capability-activity",
              { ...activity, toolCallId: invocationId, toolName: title },
              eventRunId,
            );
            continue;
          }

          if (status === "completed" || status === "failed" || status === "killed") {
            if (!activeToolCalls.has(invocationId)) {
              activeToolCalls.add(invocationId);
              yield emitToolStart(invocationId, title, eventRunId);
              yield emitToolArgs(
                invocationId,
                JSON.stringify({ input: preview?.inputSummary ?? "" }),
                eventRunId,
              );
            }

            const envelope = serializeToolResultEnvelope({
              output: preview?.outputSummary ?? preview?.outputPreview ?? (errorKind ? `Error: ${errorKind}` : ""),
              outputKind: preview?.outputKind ?? null,
              truncated: Boolean(preview?.truncated),
              inputSummary: preview?.inputSummary ?? null,
              title,
            });
            const toolState = status === "failed" || status === "killed" ? "error" : "complete";
            yield emitToolEnd(invocationId, title, toolState, envelope, preview?.inputSummary ?? "", eventRunId);
            yield emitCustom(
              "ironclaw.capability-activity",
              { ...activity, toolCallId: invocationId, toolName: title },
              eventRunId,
            );
            pendingPreviews.delete(invocationId);
            activeToolCalls.delete(invocationId);
          }
          continue;
        }

        if (type === "gate") {
          const prompt = raw.prompt;
          const approvalContext = prompt?.approvalContext;
          const toolName = approvalContext?.toolName ?? "approval";
          const gateRef = prompt?.gateRef;
          const gateToolCallId = gateRef ?? `gate-${toolName}-${eventRunId}`;
          const chunk = emitRunStarted(eventRunId);
          if (chunk) yield chunk;
          yield emitToolStart(gateToolCallId, toolName, eventRunId);
          yield emitToolArgs(gateToolCallId, JSON.stringify({ input: approvalContext }), eventRunId);
          yield emitToolEnd(gateToolCallId, toolName, "complete", "", approvalContext, eventRunId);
          yield emitCustom(
            "approval-requested",
            { toolCallId: gateToolCallId, toolName, input: approvalContext, approval: { id: gateRef ?? gateToolCallId, needsApproval: true } },
            eventRunId,
          );
          yield emitCustom(
            "ironclaw.gate",
            { ...prompt, toolCallId: gateToolCallId, toolName, input: approvalContext },
            eventRunId,
          );
          continue;
        }

        if (type === "auth_required") {
          const authPrompt = raw.authPrompt;
          const chunk = emitRunStarted(eventRunId);
          if (chunk) yield chunk;
          yield emitCustom("ironclaw.auth-required", authPrompt, eventRunId);
          continue;
        }

        if (type === "final_reply") {
          const reply = raw.reply;
          const text = reply?.text ?? "";
          const chunk = emitRunStarted(eventRunId);
          if (chunk) yield chunk;
          yield emitCustom("ironclaw.final-reply", reply, eventRunId);
          if (text) {
            terminalTextEmitted = true;
            const msgId = eventRunId ? assistantMessageId(eventRunId) : `reply-${eventRunId}`;
            yield createChunk({ type: "TEXT_MESSAGE_START", threadId, runId: eventRunId, messageId: msgId, role: "assistant" });
            yield createChunk({ type: "TEXT_MESSAGE_CONTENT", threadId, runId: eventRunId, messageId: msgId, delta: text });
            yield createChunk({ type: "TEXT_MESSAGE_END", threadId, runId: eventRunId, messageId: msgId });
            console.log(`[live] terminal: runId=${eventRunId} path=final_reply with text`);
          } else {
            console.log(`[live] terminal: runId=${eventRunId} path=final_reply no text`);
          }
          yield createChunk({ type: "RUN_FINISHED", threadId, runId: eventRunId, finishReason: "stop" });
          return;
        }

        if (type === "failed") {
          const runState = raw.runState;
          const failure = runState?.failure;
          const message = normalizeMessage(failure ?? raw.response ?? "Run failed");
          const chunk = emitRunStarted(eventRunId);
          if (chunk) yield chunk;
          yield emitCustom("ironclaw.failed", { runId: eventRunId, message, runState }, eventRunId);
          const isMainRun = boundRunId === undefined || eventRunId === boundRunId;
          if (isMainRun) {
            yield createChunk({ type: "RUN_ERROR", threadId, runId: eventRunId, message });
            return;
          }
          console.log(`[live] sub-run failed: runId=${eventRunId}`);
          continue;
        }

        if (type === "cancelled") {
          const response = raw.response;
          const chunk = emitRunStarted(eventRunId);
          if (chunk) yield chunk;
          yield emitCustom("ironclaw.cancelled", { runId: eventRunId, ...response }, eventRunId);
          const isMainRun = boundRunId === undefined || eventRunId === boundRunId;
          if (isMainRun) {
            yield createChunk({ type: "RUN_FINISHED", threadId, runId: eventRunId, finishReason: null });
            return;
          }
          console.log(`[live] sub-run cancelled: runId=${eventRunId}`);
          continue;
        }

        if (type === "projection_snapshot" || type === "projection_update") {
          const projectionState = raw.state as Record<string, unknown> | undefined;
          const items = projectionState?.items as Array<Record<string, unknown>> | undefined;
          if (items && items.length > 0) {
            const runStatuses: Array<{ runId: string; status: string; raw: Record<string, unknown> }> = [];
            const textItems: Array<{ id: string; body: string; runId: string }> = [];
            const thinkingItems: Array<Record<string, unknown>> = [];
            const capActivities: Array<Record<string, unknown>> = [];
            const gateItems: Array<Record<string, unknown>> = [];

            for (const item of items) {
              const rs = getProjectionRunStatus(item);
              if (rs) {
                const rId = (rs.runId ?? rs.run_id) as string | undefined;
                const st = rs.status as string | undefined;
                if (rId && st) runStatuses.push({ runId: rId, status: st, raw: rs });
                continue;
              }
              const tx = getProjectionText(item);
              if (tx) {
                const txId = tx.id as string | undefined;
                const body = tx.body as string | undefined;
                const txRunId = (tx.runId ?? tx.run_id ?? extractEventRunId(raw) ?? expectedRunId ?? runIdFallback) as string;
                if (txId && body) textItems.push({ id: txId, body, runId: txRunId });
                continue;
              }
              const th = getProjectionThinking(item);
              if (th) {
                const tb = th.body as string | undefined;
                if (tb) thinkingItems.push(th);
                continue;
              }
              const ca = getProjectionCapabilityActivity(item);
              if (ca) {
                capActivities.push(ca);
                continue;
              }
              const g = getProjectionGate(item);
              if (g) {
                gateItems.push(g);
                continue;
              }
            }

            console.log(`[live] projection frame: boundRunId=${boundRunId ?? "none"} runStatuses=${runStatuses.length} text=${textItems.length} thinking=${thinkingItems.length} caps=${capActivities.length} gates=${gateItems.length}`);

            // Determine effective runId from collected items
            const projRunId = runStatuses.length > 0
              ? runStatuses[0]!.runId
              : textItems.length > 0
                ? textItems[0]!.runId
                : extractEventRunId(raw) ?? expectedRunId ?? runIdFallback;

            const ikChunk = emitRunStarted(projRunId);
            if (ikChunk) yield ikChunk;

            // Emit thinking items
            for (const th of thinkingItems) {
              yield emitCustom("ironclaw.thinking", {
                body: th.body,
                id: th.id ?? undefined,
                runId: (th.runId ?? th.run_id) ?? projRunId,
              }, projRunId);
            }

            // Emit capability activities
            for (const ca of capActivities) {
              const invocationId = (ca.invocationId ?? ca.invocation_id) as string | undefined;
              const capId = ((ca.capabilityId ?? ca.capability_id) as string) || "tool";
              const capStatus = (ca.status as string) || "";
              const title = capId;
              if (invocationId) {
                if (capStatus === "started" || capStatus === "running") {
                  if (!activeToolCalls.has(invocationId)) {
                    activeToolCalls.add(invocationId);
                    yield emitToolStart(invocationId, title, projRunId);
                    yield emitToolArgs(invocationId, JSON.stringify({ input: "" }), projRunId);
                  }
                  yield emitCustom("ironclaw.capability-activity", { toolCallId: invocationId, toolName: title, ...ca }, projRunId);
                } else {
                  if (!activeToolCalls.has(invocationId)) {
                    activeToolCalls.add(invocationId);
                    yield emitToolStart(invocationId, title, projRunId);
                    yield emitToolArgs(invocationId, JSON.stringify({ input: "" }), projRunId);
                  }
                  const errorKind = (ca.errorKind ?? ca.error_kind) as string | undefined;
                  const envelope = serializeToolResultEnvelope({
                    output: errorKind ? `Error: ${errorKind}` : "",
                    outputKind: null,
                    truncated: false,
                    inputSummary: null,
                    title,
                  });
                  const toolState = capStatus === "failed" || capStatus === "killed" ? "error" : "complete";
                  yield emitToolEnd(invocationId, title, toolState, envelope, "", projRunId);
                  yield emitCustom("ironclaw.capability-activity", { toolCallId: invocationId, toolName: title, ...ca }, projRunId);
                  activeToolCalls.delete(invocationId);
                }
              }
            }

            // Emit gate items
            for (const g of gateItems) {
              const gateRef = (g.gateRef ?? g.gate_ref) as string | undefined;
              const headline = (g.headline as string) || "Approval required";
              if (gateRef) {
                const gateToolCallId = `gate-${gateRef}`;
                yield emitToolStart(gateToolCallId, "approval", projRunId);
                yield emitToolArgs(gateToolCallId, JSON.stringify({ input: headline }), projRunId);
                yield emitToolEnd(gateToolCallId, "approval", "complete", "", headline, projRunId);
                yield emitCustom("approval-requested", { toolCallId: gateToolCallId, toolName: "approval", input: headline, approval: { id: gateRef, needsApproval: true } }, projRunId);
                yield emitCustom("ironclaw.gate", { gateRef, headline, toolCallId: gateToolCallId, toolName: "approval" }, projRunId);
              }
            }

            // Emit text items (deduped by runId:textId composite key)
            for (const tx of textItems) {
              const dedupeKey = `${tx.runId}:${tx.id}`;
              if (seenTextIds.has(dedupeKey)) continue;
              seenTextIds.add(dedupeKey);
              terminalTextEmitted = true;
              const msgId = assistantMessageId(tx.runId);
              yield createChunk({ type: "TEXT_MESSAGE_START", threadId, runId: tx.runId, messageId: msgId, role: "assistant" });
              yield createChunk({ type: "TEXT_MESSAGE_CONTENT", threadId, runId: tx.runId, messageId: msgId, delta: tx.body });
              yield createChunk({ type: "TEXT_MESSAGE_END", threadId, runId: tx.runId, messageId: msgId });
            }

            // Emit terminal status last (after all other items)
            for (const rs of runStatuses) {
              const { runId, status: st } = rs;
              if (!seenRunIds.has(runId)) {
                seenRunIds.add(runId);
                yield emitCustom("ironclaw.running", { runId, status: st }, runId);
              }
              const isTerminal = ["completed", "succeeded", "failed", "cancelled", "recovery_required"].includes(st);
              const isFailed = ["failed", "recovery_required"].includes(st);
              const isMainRun = boundRunId === undefined || runId === boundRunId;
              if (isTerminal) {
                if (isFailed) {
                  const msg = (rs.raw.failureSummary ?? rs.raw.failure_summary) as string ?? `Run ${st}`;
                  yield emitCustom("ironclaw.failed", { runId, message: msg, runState: rs.raw }, runId);
                  if (isMainRun) {
                    yield createChunk({ type: "RUN_ERROR", threadId, runId, message: msg });
                    console.log(`[live] main run terminal: runId=${runId} status=${st} path=error`);
                    return;
                  }
                  console.log(`[live] sub-run terminal: runId=${runId} status=${st}`);
                  continue;
                }
                if (st === "cancelled") {
                  yield emitCustom("ironclaw.cancelled", { runId }, runId);
                  if (isMainRun) {
                    yield createChunk({ type: "RUN_FINISHED", threadId, runId, finishReason: null });
                    console.log(`[live] main run terminal: runId=${runId} status=cancelled`);
                    return;
                  }
                  console.log(`[live] sub-run terminal: runId=${runId} status=cancelled`);
                  continue;
                }
                if (isMainRun) {
                  if (!terminalTextEmitted) {
                    try {
                      const raw = await ic.threads.getTimeline({ id: threadId, limit: 10 });
                      const entries: any[] = raw.data ?? [];
                      const assistantText = findAssistantTextForRun(entries, runId);
                      if (assistantText) {
                        terminalTextEmitted = true;
                        const msgId = assistantMessageId(runId);
                        yield createChunk({ type: "TEXT_MESSAGE_START", threadId, runId, messageId: msgId, role: "assistant" });
                        yield createChunk({ type: "TEXT_MESSAGE_CONTENT", threadId, runId, messageId: msgId, delta: assistantText });
                        yield createChunk({ type: "TEXT_MESSAGE_END", threadId, runId, messageId: msgId });
                        console.log(`[live] main run terminal: runId=${runId} path=timeline_reconcile`);
                      } else {
                        console.log(`[live] main run terminal: runId=${runId} path=no_text`);
                      }
                    } catch {
                      console.log(`[live] main run terminal: runId=${runId} path=reconcile_error`);
                    }
                  }
                  yield emitCustom("ironclaw.finished", { runId, status: st }, runId);
                  yield createChunk({ type: "RUN_FINISHED", threadId, runId, finishReason: "stop" });
                  console.log(`[live] main run terminal: runId=${runId} status=${st} path=completed`);
                  return;
                }
                // Sub-run completion — don't terminate, just log
                console.log(`[live] sub-run terminal: runId=${runId} status=${st}`);
              }
            }
          }
          continue;
        }

        if (type === "keep_alive") continue;
      }

      if (runStarted) {
        if (!terminalTextEmitted) {
          try {
            const raw = await ic.threads.getTimeline({ id: threadId, limit: 10 });
            const entries: any[] = raw.data ?? [];
            const targetRunId = expectedRunId ?? runIdFallback;
            const assistantText = findAssistantTextForRun(entries, targetRunId);
            if (assistantText) {
              terminalTextEmitted = true;
              const msgId = assistantMessageId(targetRunId);
              yield createChunk({ type: "TEXT_MESSAGE_START", threadId, runId: targetRunId, messageId: msgId, role: "assistant" });
              yield createChunk({ type: "TEXT_MESSAGE_CONTENT", threadId, runId: targetRunId, messageId: msgId, delta: assistantText });
              yield createChunk({ type: "TEXT_MESSAGE_END", threadId, runId: targetRunId, messageId: msgId });
              console.log(`[live] stream-end reconcile: runId=${targetRunId} path=timeline_reconcile`);
            } else {
              console.log(`[live] stream-end reconcile: runId=${targetRunId} path=no_text`);
            }
          } catch {
            console.log(`[live] stream-end reconcile: error`);
          }
        }
        yield createChunk({ type: "RUN_FINISHED", threadId, runId: expectedRunId ?? runIdFallback, finishReason: "stop" });
        console.log(`[live] stream-end reconcile: runId=${expectedRunId ?? runIdFallback} path=finished`);
      }
    } catch (error) {
      if (signal?.aborted) return;
      yield createChunk({ type: "RUN_ERROR", threadId, runId: expectedRunId ?? runIdFallback, message: normalizeMessage(error) });
    } finally {
      if (typeof upstream.return === "function") {
        try {
          await upstream.return(undefined);
        } catch {
          // ignore close failures
        }
      }
    }
  };
}
