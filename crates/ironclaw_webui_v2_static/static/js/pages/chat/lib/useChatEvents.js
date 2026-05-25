import { React } from "../../../lib/html.js";
import { gateFromEvent } from "./gates.js";

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
// one of `{ run_status, text, gate }` as a sub-object.
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
  onRunCompleted,
}) {
  // Track which runIds we've already announced completion for so that
  // SSE replays (reconnect with `last-event-id`, repeated snapshots)
  // don't trigger duplicate timeline refetches.
  const completedRunsRef = React.useRef(new Set());

  return React.useCallback(
    (envelope) => {
      const { type, frame } = envelope || {};
      if (!type || !frame) return;

      switch (type) {
        case "accepted": {
          const ack = frame.ack || {};
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
            setActiveRun?.((current) =>
              current && current.runId === progress.turn_run_id
                ? current
                : { runId: progress.turn_run_id, threadId, status: "running" },
            );
          }
          setIsProcessing(true);
          return;
        }

        case "gate":
        case "auth_required": {
          const pending = gateFromEvent(type, frame.prompt);
          if (pending) {
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
            },
          ]);
          setPendingGate(null);
          setIsProcessing(false);
          return;
        }

        case "cancelled":
        case "failed": {
          setPendingGate(null);
          setIsProcessing(false);
          setActiveRun?.(null);
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
            onRunCompleted,
            completedRunsRef,
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
      onRunCompleted,
    ],
  );
}

const TERMINAL_RUN_STATUSES = new Set([
  "completed",
  "succeeded",
  "failed",
  "cancelled",
  "recovery_required",
]);

const SUCCESS_RUN_STATUSES = new Set(["completed", "succeeded"]);

function applyProjectionItems({
  items,
  threadId,
  setMessages,
  setIsProcessing,
  setPendingGate,
  setActiveRun,
  onRunCompleted,
  completedRunsRef,
}) {
  for (const item of items) {
    if (item.run_status) {
      const { run_id: runId, status } = item.run_status;
      if (runId) {
        setActiveRun?.((current) =>
          current && current.runId === runId
            ? { ...current, status }
            : { runId, threadId, status },
        );
      }
      if (TERMINAL_RUN_STATUSES.has(status)) {
        setIsProcessing(false);
        if (
          SUCCESS_RUN_STATUSES.has(status) &&
          onRunCompleted &&
          runId &&
          !completedRunsRef?.current.has(runId)
        ) {
          // Reborn's projection bridge does not currently emit `Text`
          // items for assistant replies — the reply lives only in the
          // thread timeline. Trigger a timeline refetch on terminal
          // success so the assistant message becomes visible. Dedup
          // by runId because SSE replays the same projection on every
          // reconnect.
          completedRunsRef.current.add(runId);
          onRunCompleted(runId);
        }
        if (status === "failed" || status === "recovery_required") {
          // Dedup by `err-<runId>` so replays of the same projection
          // (SSE reconnect with `last-event-id`, or repeated updates
          // carrying the same terminal status) collapse to one
          // bubble instead of stacking.
          const messageId = `err-${runId || "unknown"}`;
          setMessages((prev) => {
            if (prev.some((m) => m.id === messageId)) return prev;
            return [
              ...prev,
              {
                id: messageId,
                role: "error",
                content:
                  status === "recovery_required"
                    ? "The run is awaiting recovery — backend reported `recovery_required`."
                    : "The run failed before producing a reply.",
                timestamp: new Date().toISOString(),
              },
            ];
          });
        }
      } else {
        setIsProcessing(true);
      }
    }

    if (item.text) {
      // ProductProjectionItem::Text { id, body } — the body is the
      // assistant-visible reply text accumulated through projection.
      // Dedup by item id so repeated snapshots don't duplicate the
      // same bubble.
      const messageId = `text-${item.text.id}`;
      setMessages((prev) => {
        const existing = prev.findIndex((m) => m.id === messageId);
        const next = {
          id: messageId,
          role: "assistant",
          content: item.text.body || "",
          timestamp: new Date().toISOString(),
        };
        if (existing >= 0) {
          const copy = [...prev];
          copy[existing] = next;
          return copy;
        }
        return [...prev, next];
      });
      setIsProcessing(false);
      setPendingGate(null);
    }

    if (item.gate) {
      // ProductProjectionItem::Gate { gate_ref, headline } — projection
      // only carries a coarse summary; we surface it as a pending gate
      // even though the run_id needs to come from the active run state.
      setPendingGate((current) => current || {
        kind: "gate",
        gateRef: item.gate.gate_ref,
        headline: item.gate.headline,
        body: "",
      });
      setIsProcessing(false);
    }
  }
}
