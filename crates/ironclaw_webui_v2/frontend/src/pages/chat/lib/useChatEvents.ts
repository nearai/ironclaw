// @ts-nocheck
import React from "react";
import { gateFromEvent, gateFromProjectionGate } from "./gates";
import {
  isTerminalToolStatus,
  toolCardFromActivity,
  toolCardFromPreview,
} from "./history-messages";
import { failureMessageForRunStatus } from "./failureMessages";
import {
  ensureGateToolActivity,
  upsertToolActivityMessage,
} from "./tool-activity-state";
import {
  isFinalAssistantForRun,
  replaceAssistantReplyForRun,
} from "./stream-order-memory";

const noop = () => {};
const emptyConnectionContext = () => ({});

// Handler factory for v2 `WebChatV2EventFrame` events.
//
// The current local-dev runtime primarily emits `projection_snapshot` and
// `projection_update` over the WebUI stream. Rich `gate` / `auth_required`
// prompt payloads may also arrive for a blocked turn, but the projection gate
// item is the rebuildable source of the pending gate identity. The handler
// therefore drives long-lived UI state off projection items — see
// `ironclaw_product_adapters::outbound::ProductProjectionItem` for
// the item shapes.
//
// Items are externally-tagged enums so each entry carries exactly
// one renderable sub-object such as `{ run_status, thinking, text, gate }`.
//
// Status mapping (from `RunStatus.status`):
//   "queued" | "running"           → processing
//   "completed" | "succeeded"      → stop, no error
//   "failed" | "cancelled"
//   | "recovery_required"          → stop, error / recovery state
//
// The typed branches are still handled for forwards-compat if the
// runtime starts emitting them.
export function useChatEvents({
  threadId,
  setMessages,
  setIsProcessing,
  setPendingGate,
  setActiveRun,
  activeRunRef,
  locallyResolvedGatesRef,
  toolActivityStateRef,
  noteConnectionInterruptedRunId = noop,
  connectionContextForRunFailure = emptyConnectionContext,
  onRunSettled,
}) {
  // Track which runIds we've already settled so that SSE replays
  // (reconnect with `last-event-id`, repeated snapshots) don't trigger
  // duplicate timeline refetches. A run settles on ANY terminal status,
  // not only success — every terminal run reloads the timeline so tool
  // input/output previews are recovered from the durable record even when
  // the run failed, was cancelled, or needs recovery.
  const settledRunsRef = React.useRef(new Set());
  // Last `run_status.run_id` we've observed, persisted across event frames.
  // Used to reject stale terminal statuses after a locally resolved gate
  // resumes a newer active run.
  const latestRunIdRef = React.useRef(null);
  const promptRunIdRef = React.useRef(null);

  return React.useCallback(
    (envelope) => {
      const { type, frame } = envelope || {};
      if (!type || !frame) return;

      switch (type) {
        case "accepted": {
          const ack = frame.ack || {};
          if (ack.run_id) latestRunIdRef.current = ack.run_id;
          noteConnectionInterruptedRunId(ack.run_id);
          setActiveRun?.({
            runId: ack.run_id || null,
            threadId: ack.thread_id || threadId,
            status: ack.status || null,
          });
          setIsProcessing(true);
          return;
        }

        case "running":
        case "capability_progress": {
          const progress = frame.progress || {};
          if (progress.turn_run_id) {
            latestRunIdRef.current = progress.turn_run_id;
            noteConnectionInterruptedRunId(progress.turn_run_id);
            setActiveRun?.((current) =>
              current && current.runId === progress.turn_run_id
                ? { ...current, status: "running" }
                : { runId: progress.turn_run_id, threadId, status: "running" },
            );
            clearPendingNonAuthGateForRun(
              setPendingGate,
              progress.turn_run_id,
              promptRunIdRef,
            );
          }
          setIsProcessing(true);
          return;
        }

        case "capability_activity": {
          // Lifecycle metadata for a capability invocation. Used to
          // render a "running" placeholder card before the richer
          // `capability_display_preview` frame arrives at terminal
          // time. Keyed by invocation_id so the preview frame can
          // upgrade the same bubble in place.
          const activity = frame.activity;
          if (!activity || !activity.invocation_id) return;
          upsertToolActivityMessage(
            setMessages,
            toolCardFromActivity(activity),
            toolActivityStateRef,
          );
          return;
        }

        case "capability_display_preview": {
          // Final sanitized display artifact for a capability
          // invocation (carries title, input/output summaries, and
          // truncated preview). Replaces any prior activity-derived
          // card for the same invocation_id.
          const preview = frame.preview;
          if (!preview || !preview.invocation_id) return;
          const card = toolCardFromPreview(preview);
          upsertToolActivityMessage(setMessages, card, toolActivityStateRef);
          return;
        }

        case "gate":
        case "auth_required": {
          const pending = gateFromEvent(type, frame.prompt);
          if (pending) {
            ensureGateToolActivity(setMessages, pending, toolActivityStateRef);
            setPendingGate(pending);
            setActiveRun?.({
              runId: pending.runId,
              threadId,
              status: "awaiting_gate",
            });
          }
          setIsProcessing(false);
          return;
        }

        case "final_reply": {
          const reply = frame.reply || {};
          const turnRunId = reply.turn_run_id || null;
          const replyMessage = {
            id: `reply-${turnRunId || Date.now()}`,
            role: "assistant",
            content: reply.text || "",
            timestamp: reply.generated_at || new Date().toISOString(),
            turnRunId,
            isFinalReply: true,
          };
          setMessages((prev) =>
            replaceAssistantReplyForRun(prev, replyMessage, turnRunId),
          );
          setPendingGate(null);
          setIsProcessing(false);
          setActiveRun?.(null);
          return;
        }

        case "cancelled": {
          const runId =
            frame.run_state?.run_id || activeRunRef?.current?.runId || null;
          setPendingGate(null);
          setIsProcessing(false);
          setActiveRun?.(null);
          settleRun(settledRunsRef, onRunSettled, runId, false);
          return;
        }

        case "failed": {
          const runState = frame.run_state || {};
          const runId = runState.run_id || activeRunRef?.current?.runId || null;
          setPendingGate(null);
          setIsProcessing(false);
          setActiveRun?.(null);
          appendRunFailureMessage(setMessages, {
            runId,
            status: runState.status || "failed",
            failureCategory: failureCategoryFromRunState(runState),
            failureSummary: null,
            connectionContextForRunFailure,
          });
          settleRun(settledRunsRef, onRunSettled, runId, false);
          return;
        }

        case "projection_snapshot":
        case "projection_update": {
          const items = frame.state?.items || [];
          applyProjectionItems({
            items,
            threadId,
            setMessages,
            setIsProcessing,
            setPendingGate,
            setActiveRun,
            onRunSettled,
            settledRunsRef,
            latestRunIdRef,
            promptRunIdRef,
            activeRunRef,
            locallyResolvedGatesRef,
            toolActivityStateRef,
            noteConnectionInterruptedRunId,
            connectionContextForRunFailure,
          });
          return;
        }

        case "keep_alive":
        default:
          return;
      }
    },
    [
      threadId,
      setMessages,
      setIsProcessing,
      setPendingGate,
      setActiveRun,
      activeRunRef,
      locallyResolvedGatesRef,
      toolActivityStateRef,
      noteConnectionInterruptedRunId,
      connectionContextForRunFailure,
      onRunSettled,
    ],
  );
}

// Fire the settle callback exactly once per runId. A run settles on any
// terminal status; the consumer reloads the timeline so tool input/output
// previews are recovered from the durable record. Deduped because SSE
// replays the same terminal projection on every reconnect.
function settleRun(settledRunsRef, onRunSettled, runId, success) {
  if (!onRunSettled || !runId || !settledRunsRef?.current) return;
  if (settledRunsRef.current.has(runId)) return;
  settledRunsRef.current.add(runId);
  onRunSettled(runId, { success });
}

const TERMINAL_RUN_STATUSES = new Set([
  "completed",
  "succeeded",
  "failed",
  "cancelled",
  "recovery_required",
]);

const SUCCESS_RUN_STATUSES = new Set(["completed", "succeeded"]);
const PROMPT_RUN_STATUSES = new Set([
  "blocked_auth",
  "blocked_approval",
  "blocked_resource",
  "blocked_dependent_run",
]);
const GATE_ACTIVE_RUN_STATUSES = new Set([
  "awaiting_gate",
  "blocked_auth",
  "blocked_approval",
  "blocked_resource",
  "blocked_dependent_run",
]);

function clearPendingGateForRun(setPendingGate, runId, promptRunIdRef) {
  if (!runId) return;
  if (promptRunIdRef?.current === runId) {
    promptRunIdRef.current = null;
  }
  setPendingGate((current) => (current?.runId === runId ? null : current));
}

function clearPendingNonAuthGateForRun(setPendingGate, runId, promptRunIdRef) {
  if (!runId) return;
  setPendingGate((current) => {
    if (current?.runId !== runId || current.kind === "auth_required") {
      return current;
    }
    if (promptRunIdRef?.current === runId) {
      promptRunIdRef.current = null;
    }
    return null;
  });
}

function isObsoleteProjectionGate(
  activeRunRef,
  pendingGate,
  batchRunStatusByRunId,
  latestRunIdRef,
  stalePromptRunIds,
  promptRunIdRef,
) {
  const runId = pendingGate?.runId || null;
  if (!runId) return true;
  if (stalePromptRunIds?.has(runId)) {
    return true;
  }
  const batchStatus = batchRunStatusByRunId?.get(runId);
  if (batchStatus) return !GATE_ACTIVE_RUN_STATUSES.has(batchStatus);
  const activeRun = activeRunRef?.current;
  const activeRunId = activeRun?.runId || latestRunIdRef?.current || null;
  if (activeRunId && runId !== activeRunId) return true;
  const activePromptRunIsCurrent = promptRunIdRef?.current === activeRunId;
  if (
    activeRunId &&
    runId === activeRunId &&
    !activePromptRunIsCurrent &&
    activeRun?.status &&
    !GATE_ACTIVE_RUN_STATUSES.has(activeRun.status)
  ) {
    return true;
  }
  if (!activeRun?.runId) return false;
  if (!activeRun.status) return false;
  return !GATE_ACTIVE_RUN_STATUSES.has(activeRun.status);
}

function applyProjectionItems({
  items,
  threadId,
  setMessages,
  setIsProcessing,
  setPendingGate,
  setActiveRun,
  onRunSettled,
  settledRunsRef,
  latestRunIdRef,
  promptRunIdRef,
  activeRunRef,
  locallyResolvedGatesRef,
  toolActivityStateRef,
  noteConnectionInterruptedRunId,
  connectionContextForRunFailure,
}) {
  // Snapshot the most recent run id so stale terminal run_status frames can
  // be filtered while a locally resolved gate is resuming a newer run.
  const batchRunStatusByRunId = new Map();
  const stalePromptRunIds = new Set();
  const activeRunAtBatchStart = activeRunRef?.current || null;
  const protectedRunId =
    activeRunAtBatchStart?.runId || latestRunIdRef?.current || null;
  for (const item of items) {
    const runStatus = item.run_status;
    if (runStatus?.run_id && runStatus.status) {
      batchRunStatusByRunId.set(runStatus.run_id, runStatus.status);
      if (
        protectedRunId &&
        protectedRunId !== runStatus.run_id &&
        activeRunAtBatchStart?.status &&
        !TERMINAL_RUN_STATUSES.has(activeRunAtBatchStart.status) &&
        PROMPT_RUN_STATUSES.has(runStatus.status)
      ) {
        stalePromptRunIds.add(runStatus.run_id);
      }
    }
  }
  let activeRunId = latestRunIdRef?.current ?? null;
  for (const item of items) {
    if (item.run_status) {
      const {
        run_id: runId,
        status,
        failure_category: failureCategory,
        failure_summary: failureSummary,
      } = item.run_status;
      const isTerminalStatus = TERMINAL_RUN_STATUSES.has(status);
      const locallyPinnedRunId =
        activeRunRef?.current?.source === "local" ? activeRunRef.current.runId : null;
      const isStaleLocalRunStatus = Boolean(
        runId && locallyPinnedRunId && locallyPinnedRunId !== runId,
      );
      const streamActiveRunId = activeRunId ?? latestRunIdRef?.current ?? null;
      const isStaleTerminalStatus = Boolean(
        isTerminalStatus &&
          runId &&
          streamActiveRunId &&
          streamActiveRunId !== runId,
      );
      const locallyResolvedPromptState =
        runId && PROMPT_RUN_STATUSES.has(status)
          ? locallyResolvedStateForRun(locallyResolvedGatesRef, runId)
          : null;
      if (runId && stalePromptRunIds.has(runId)) {
        continue;
      }
      if (isStaleLocalRunStatus) {
        continue;
      }
      if (isStaleTerminalStatus) {
        const activeResolvedPromptState = locallyResolvedStateForRun(
          locallyResolvedGatesRef,
          activeRunRef?.current?.runId,
        );
        if (activeResolvedPromptState?.outcome === "resumed") {
          settleTerminalRunAfterResolvedPrompt({
            runId,
            activePromptRunId: activeRunRef?.current?.runId,
            success: SUCCESS_RUN_STATUSES.has(status),
            status,
            failureCategory,
            failureSummary,
            setMessages,
            setIsProcessing,
            setPendingGate,
            setActiveRun,
            onRunSettled,
            settledRunsRef,
            latestRunIdRef,
            promptRunIdRef,
            locallyResolvedGatesRef,
            connectionContextForRunFailure,
          });
          activeRunId = null;
        }
        continue;
      }
      if (locallyResolvedPromptState) {
        clearPendingGateForRun(setPendingGate, runId, promptRunIdRef);
        if (locallyResolvedPromptState.outcome === "resumed") {
          setIsProcessing(true);
          setActiveRun?.((current) =>
            current && current.runId === runId
              ? {
                  ...current,
                  status: current.status === "awaiting_gate"
                    ? "queued"
                    : current.status || "queued",
                }
              : { runId, threadId, status: "queued" },
          );
          activeRunId = runId;
          if (latestRunIdRef) latestRunIdRef.current = runId;
        } else {
          setIsProcessing(false);
          if (activeRunRef?.current?.runId === runId) {
            setActiveRun?.(null);
          }
          activeRunId = null;
          if (latestRunIdRef?.current === runId) latestRunIdRef.current = null;
        }
        continue;
      }
      if (runId) {
        noteConnectionInterruptedRunId(runId);
        activeRunId = runId;
        if (!isTerminalStatus && latestRunIdRef) {
          latestRunIdRef.current = runId;
        }
        setActiveRun?.((current) =>
          current && current.runId === runId
            ? { ...current, status }
            : { runId, threadId, status },
        );
      }
      if (runId && PROMPT_RUN_STATUSES.has(status)) {
        if (promptRunIdRef) promptRunIdRef.current = runId;
      } else if (runId && promptRunIdRef?.current === runId) {
        promptRunIdRef.current = null;
      }
      if (isTerminalStatus) {
        setIsProcessing(false);
        setPendingGate(null);
        setActiveRun?.(null);
        clearLocallyResolvedRun(locallyResolvedGatesRef, runId);
        activeRunId = null;
        if (latestRunIdRef) latestRunIdRef.current = null;
        if (runId && promptRunIdRef?.current === runId) {
          promptRunIdRef.current = null;
        }
        // Reborn's projection bridge does not currently emit `Text` items
        // for assistant replies, nor `capability_display_preview` items in
        // the projection state — both the assistant reply and the rich tool
        // input/output cards live only in the thread timeline. Reload the
        // timeline on EVERY terminal status (not only success) so a failed,
        // cancelled, or recovery-required run still recovers the tool
        // previews for the tools that completed before it terminated. The
        // reload preserves the client-side `err-*` failure bubble.
        settleRun(
          settledRunsRef,
          onRunSettled,
          runId,
          SUCCESS_RUN_STATUSES.has(status),
        );
        if (status === "failed" || status === "recovery_required") {
          appendRunFailureMessage(setMessages, {
            runId,
            status,
            failureCategory,
            failureSummary,
            connectionContextForRunFailure,
          });
        }
      } else if (!PROMPT_RUN_STATUSES.has(status)) {
        clearPendingGateForRun(setPendingGate, runId, promptRunIdRef);
        clearLocallyResolvedRun(locallyResolvedGatesRef, runId);
        setIsProcessing(true);
      }
    }

    if (item.text) {
      // ProductProjectionItem::Text { id, run_id, body } — the body is the
      // assistant-visible reply text accumulated through projection.
      // Dedup by item id and by the matching durable timeline message id so a
      // late projection cannot duplicate a reply already rendered from
      // history. Text can stream while the run is still active or arrive in the same
      // projection snapshot as a still-blocked gate; run_status remains the source of
      // truth for clearing pendingGate/processing.
      const messageId = `text-${item.text.id}`;
      const textRunId = item.text.run_id || null;
      setMessages((prev) => {
        if (
          textRunId &&
          prev.some((m) => isFinalAssistantForRun(m, textRunId))
        ) {
          return prev;
        }
        const timelineMessageId = item.text.id ? `msg-${item.text.id}` : null;
        const existing = prev.findIndex(
          (m) => m.id === messageId || (timelineMessageId && m.id === timelineMessageId),
        );
        const next = {
          ...(existing >= 0 ? prev[existing] : {}),
          id: messageId,
          role: "assistant",
          content: item.text.body || "",
          timestamp: prev[existing]?.timestamp || new Date().toISOString(),
          turnRunId: prev[existing]?.turnRunId || textRunId,
          isFinalReply: false,
        };
        if (existing >= 0) {
          const copy = [...prev];
          copy[existing] = next;
          return copy;
        }
        return [...prev, next];
      });
    }

    if (item.thinking) {
      const messageId = `thinking-${item.thinking.id}`;
      setMessages((prev) => {
        const existing = prev.findIndex((m) => m.id === messageId);
        const next = {
          id: messageId,
          role: "thinking",
          content: item.thinking.body || "",
          timestamp: new Date().toISOString(),
          turnRunId: item.thinking.run_id || null,
        };
        if (existing >= 0) {
          const copy = [...prev];
          copy[existing] = next;
          return copy;
        }
        return [...prev, next];
      });
    }

    if (item.capability_activity) {
      const activity = item.capability_activity;
      if (activity.invocation_id) {
        upsertToolActivityMessage(
          setMessages,
          toolCardFromActivity(activity),
          toolActivityStateRef,
        );
      }
    }

    if (item.gate) {
      const pendingGate = gateFromProjectionGate(item.gate);
      const runId = pendingGate?.runId || null;
      if (
        runId &&
        !isObsoleteProjectionGate(
          activeRunRef,
          pendingGate,
          batchRunStatusByRunId,
          latestRunIdRef,
          stalePromptRunIds,
          promptRunIdRef,
        ) &&
        !isLocallyResolvedGate(locallyResolvedGatesRef, runId, pendingGate.gateRef)
      ) {
        ensureGateToolActivity(setMessages, pendingGate, toolActivityStateRef);
        setPendingGate((current) => current || pendingGate);
        setActiveRun?.((current) =>
          current && current.runId === runId
            ? {
                ...current,
                status: GATE_ACTIVE_RUN_STATUSES.has(current.status)
                  ? current.status
                  : "awaiting_gate",
              }
            : { runId, threadId, status: "awaiting_gate" },
        );
        if (promptRunIdRef) promptRunIdRef.current = runId;
        setIsProcessing(false);
      }
    }
  }
  if (latestRunIdRef && activeRunId) {
    latestRunIdRef.current = activeRunId;
  }
}

function settleTerminalRunAfterResolvedPrompt({
  runId,
  activePromptRunId,
  success,
  status,
  failureCategory,
  failureSummary,
  setMessages,
  setIsProcessing,
  setPendingGate,
  setActiveRun,
  onRunSettled,
  settledRunsRef,
  latestRunIdRef,
  promptRunIdRef,
  locallyResolvedGatesRef,
  connectionContextForRunFailure,
}) {
  setIsProcessing(false);
  setPendingGate(null);
  setActiveRun?.(null);
  clearLocallyResolvedRun(locallyResolvedGatesRef, activePromptRunId);
  if (latestRunIdRef) latestRunIdRef.current = null;
  if (promptRunIdRef?.current === activePromptRunId) {
    promptRunIdRef.current = null;
  }
  settleRun(settledRunsRef, onRunSettled, runId, success);
  if (status === "failed" || status === "recovery_required") {
    appendRunFailureMessage(setMessages, {
      runId,
      status,
      failureCategory,
      failureSummary,
      connectionContextForRunFailure,
    });
  }
}

function failureCategoryFromRunState(runState) {
  const failure = runState?.failure;
  if (typeof failure === "string" && failure.trim()) return failure.trim();
  if (
    failure &&
    typeof failure === "object" &&
    typeof failure.category === "string" &&
    failure.category.trim()
  ) {
    return failure.category.trim();
  }
  return null;
}

function appendRunFailureMessage(
  setMessages,
  {
    runId,
    status,
    failureCategory,
    failureSummary,
    connectionContextForRunFailure,
  },
) {
  // Dedup by `err-<runId>` so replays of the same projection
  // (SSE reconnect with `last-event-id`, or repeated updates carrying
  // the same terminal status) collapse to one bubble instead of stacking.
  const messageId = `err-${runId || "unknown"}`;
  const connectionContext =
    typeof connectionContextForRunFailure === "function"
      ? connectionContextForRunFailure(runId) || {}
      : {};
  setMessages((prev) => {
    const existing = prev.findIndex((m) => m.id === messageId);
    const content = failureMessageForRunStatus({
      status,
      failureCategory,
      failureSummary,
      ...connectionContext,
    });
    if (existing >= 0) {
      const hasUsefulUpdate = Boolean(failureSummary || failureCategory);
      if (!hasUsefulUpdate || prev[existing].content === content) return prev;
      const next = [...prev];
      next[existing] = {
        ...next[existing],
        content,
        failureStatus: status,
        failureCategory,
        failureSummary,
      };
      return next;
    }
    return [
      ...prev,
      {
        id: messageId,
        role: "error",
        content,
        timestamp: new Date().toISOString(),
        failureStatus: status,
        failureCategory,
        failureSummary,
      },
    ];
  });
}

function locallyResolvedStateForRun(locallyResolvedGatesRef, runId) {
  if (!runId) return null;
  const resolved = locallyResolvedGatesRef?.current;
  if (!resolved) return null;
  for (const [key, value] of resolved.entries()) {
    if (!key.startsWith(`${runId}\n`)) continue;
    return normalizeLocallyResolvedState(value);
  }
  return null;
}

function normalizeLocallyResolvedState(value) {
  if (value && typeof value === "object") {
    return {
      resolution: value.resolution || null,
      outcome: value.outcome || null,
    };
  }
  return { resolution: value || null, outcome: null };
}

function clearLocallyResolvedRun(locallyResolvedGatesRef, runId) {
  if (!runId) return;
  const resolved = locallyResolvedGatesRef?.current;
  if (!resolved) return;
  for (const key of Array.from(resolved.keys())) {
    if (key.startsWith(`${runId}\n`)) resolved.delete(key);
  }
}

function isLocallyResolvedGate(locallyResolvedGatesRef, runId, gateRef) {
  if (!runId || !gateRef) return false;
  return Boolean(locallyResolvedGatesRef?.current?.has(`${runId}\n${gateRef}`));
}

function upsertToolFromActivity(setMessages, invocationId, card) {
  const id = `tool-${invocationId}`;
  setMessages((prev) => {
    const existing = prev.findIndex((m) => m.id === id);
    if (existing >= 0) {
      const current = prev[existing];
      // A late lifecycle frame can carry `running` after the preview
      // already set `success` / `error`. Don't downgrade terminal
      // state — but do let the next terminal state through.
      const nextStatus =
        isTerminalToolStatus(current.toolStatus) && card.toolStatus === "running"
          ? current.toolStatus
          : card.toolStatus;
      const copy = [...prev];
      copy[existing] = {
        ...current,
        toolStatus: nextStatus,
        toolError: card.toolError || current.toolError,
        // Enrich with the live input if a later frame carries it and the
        // current card doesn't yet (e.g. the first frame raced ahead of the
        // input link). Never clobber a populated value with null.
        toolDetail: current.toolDetail || card.toolDetail || null,
        toolParameters: current.toolParameters || card.toolParameters || null,
        updatedAt: card.updatedAt || current.updatedAt,
        turnRunId: card.turnRunId || current.turnRunId || null,
      };
      return copy;
    }
    return [...prev, { id, role: "tool_activity", ...card }];
  });
}
