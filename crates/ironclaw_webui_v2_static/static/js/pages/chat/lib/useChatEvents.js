import { React } from "../../../lib/html.js";
import { gateFromEvent } from "./gates.js";
import {
  toolCardFromActivity,
  toolCardFromPreview,
} from "./history-messages.js";
import { failureMessageForRunStatus } from "./failureMessages.js";
import {
  ensureGateToolActivity,
  upsertToolActivityMessage,
} from "./tool-activity-state.js";

// Handler factory for v2 `WebChatV2EventFrame` events.
//
// The current local-dev runtime ONLY emits `projection_snapshot` and
// `projection_update` over the WebUI stream (the typed `accepted` /
// `running` / `final_reply` / `gate` / `failed` variants are
// scaffolded in the schema but never published by the runtime-owned
// projection bridge today). The handler therefore drives the UI off
// the projection items rather than the typed variants — see
// `ironclaw_product_adapters::outbound::ProductProjectionItem` for
// the item shapes.
//
// Items are externally-tagged enums so each entry carries exactly
// one of `{ run_status, thinking, text, gate }` as a sub-object.
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
  onRunSettled,
}) {
  // Track which runIds we've already settled so that SSE replays
  // (reconnect with `last-event-id`, repeated snapshots) don't trigger
  // duplicate timeline refetches. A run settles on ANY terminal status,
  // not only success — every terminal run reloads the timeline so tool
  // input/output previews are recovered from the durable record even when
  // the run failed, was cancelled, or needs recovery.
  const settledRunsRef = React.useRef(new Set());
  // Last `run_status.run_id` we've observed, persisted across event
  // frames. Used by `applyProjectionItems` to correlate an `item.gate`
  // (which doesn't carry `run_id`) with the active run so resolveGate
  // can build its `/runs/{run_id}/gates/{gate_ref}/resolve` URL.
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
            setActiveRun?.((current) =>
              current && current.runId === progress.turn_run_id
                ? current
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
          setMessages((prev) => [
            ...prev,
            {
              id: `reply-${reply.turn_run_id || Date.now()}`,
              role: "assistant",
              content: reply.text || "",
              timestamp: reply.generated_at || new Date().toISOString(),
              turnRunId: reply.turn_run_id,
              isFinalReply: true,
            },
          ]);
          setPendingGate(null);
          setIsProcessing(false);
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
}) {
  // Snapshot the run_id surfaced by the most recent `run_status` item
  // we've seen — either earlier in this same items batch, or carried
  // over from a prior frame via `latestRunIdRef`. `item.gate` doesn't
  // include a `run_id`, but resolveGate at the v2 endpoint needs both
  // `run_id` + `gate_ref` in the URL, so we have to correlate the
  // gate back to whichever run is currently active. setActiveRun is a
  // React setter and doesn't update synchronously inside this loop;
  // tracking the value locally lets the gate handler that runs later
  // in the same iteration see the run we just learned about.
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
          });
        }
      } else if (!PROMPT_RUN_STATUSES.has(status)) {
        clearPendingGateForRun(setPendingGate, runId, promptRunIdRef);
        clearLocallyResolvedRun(locallyResolvedGatesRef, runId);
        setIsProcessing(true);
      }
    }

    if (item.text) {
      // ProductProjectionItem::Text { id, body } — the body is the
      // assistant-visible reply text accumulated through projection.
      // Dedup by item id so repeated snapshots don't duplicate the
      // same bubble. Text can arrive in the same projection snapshot
      // as a still-blocked gate, so run_status remains the source of
      // truth for clearing pendingGate.
      const messageId = `text-${item.text.id}`;
      setMessages((prev) => {
        const existing = prev.findIndex((m) => m.id === messageId);
        const next = {
          id: messageId,
          role: "assistant",
          content: item.text.body || "",
          timestamp: new Date().toISOString(),
          isFinalReply: true,
        };
        if (existing >= 0) {
          const copy = [...prev];
          copy[existing] = next;
          return copy;
        }
        return [...prev, next];
      });
      setIsProcessing(false);
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
      // ProductProjectionItem::Gate carries gate_ref but not run_id, so we correlate to the
      // active run (snapshotted above). Without a run_id the
      // pendingGate is unusable (`resolveGate` would 400 at the path
      // construction in `api.js`), so skip emitting the gate entirely
      // if no run is active yet — a later projection_update will
      // re-surface it once a run_status arrives.
      if (
        activeRunId &&
        promptRunIdRef?.current === activeRunId &&
        !isLocallyResolvedGate(locallyResolvedGatesRef, activeRunId, item.gate.gate_ref)
      ) {
        setPendingGate((current) => current || {
          kind: "gate",
          runId: activeRunId,
          gateRef: item.gate.gate_ref,
          headline: item.gate.headline,
          body: "",
          allowAlways: item.gate.allow_always === true,
        });
        setIsProcessing(false);
      }
    }

    if (item.skill_activation) {
      const {
        id,
        skill_names: skillNames = [],
        feedback = [],
      } = item.skill_activation;
      if (skillNames.length || feedback.length) {
        const messageId = `skill-${id || skillNames.join("-") || "activation"}`;
        const content = [
          skillNames.length ? `Skill activated: ${skillNames.join(", ")}` : "",
          ...feedback,
        ].filter(Boolean).join("\n");
        setMessages((prev) => {
          if (prev.some((m) => m.id === messageId)) return prev;
          return [
            ...prev,
            {
              id: messageId,
              role: "system",
              content,
              timestamp: new Date().toISOString(),
            },
          ];
        });
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
  { runId, status, failureCategory, failureSummary },
) {
  // Dedup by `err-<runId>` so replays of the same projection
  // (SSE reconnect with `last-event-id`, or repeated updates carrying
  // the same terminal status) collapse to one bubble instead of stacking.
  const messageId = `err-${runId || "unknown"}`;
  setMessages((prev) => {
    const existing = prev.findIndex((m) => m.id === messageId);
    const content = failureMessageForRunStatus({
      status,
      failureCategory,
      failureSummary,
    });
    if (existing >= 0) {
      if (!failureSummary || prev[existing].content === content) return prev;
      const next = [...prev];
      next[existing] = {
        ...next[existing],
        content,
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
