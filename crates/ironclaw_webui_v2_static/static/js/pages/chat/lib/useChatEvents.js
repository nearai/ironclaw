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
  // Last `run_status.run_id` we've observed, persisted across event
  // frames. Used by `applyProjectionItems` to correlate an `item.gate`
  // (which doesn't carry `run_id`) with the active run so resolveGate
  // can build its `/runs/{run_id}/gates/{gate_ref}/resolve` URL.
  const latestRunIdRef = React.useRef(null);

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
            latestRunIdRef,
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
  latestRunIdRef,
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
      const { run_id: runId, status } = item.run_status;
      if (runId) {
        activeRunId = runId;
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
      // carries gate_ref but not run_id, so we correlate to the
      // active run (snapshotted above). Without a run_id the
      // pendingGate is unusable (`resolveGate` would 400 at the path
      // construction in `api.js`), so skip emitting the gate entirely
      // if no run is active yet — a later projection_update will
      // re-surface it once a run_status arrives.
      if (activeRunId) {
        setPendingGate((current) => current || {
          kind: "gate",
          runId: activeRunId,
          gateRef: item.gate.gate_ref,
          headline: item.gate.headline,
          body: "",
        });
        setIsProcessing(false);
      }
    }
  }
  if (latestRunIdRef && activeRunId) {
    latestRunIdRef.current = activeRunId;
  }
}
