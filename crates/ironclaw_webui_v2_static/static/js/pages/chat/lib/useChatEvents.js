import { React } from "../../../lib/html.js";
import { gateFromEvent } from "./gates.js";

// Handler factory for v2 `WebChatV2EventFrame` events. The frame
// vocabulary is defined in `crates/ironclaw_webui_v2/src/schema.rs`:
//   accepted | running | capability_progress | gate | auth_required
//   | final_reply | cancelled | failed
//   | projection_snapshot | projection_update | keep_alive
//
// The UI only needs a subset:
//   - `accepted` → mark a run as in-flight (carries run_id we use later
//     for cancel + gate resolve when the user acts)
//   - `running` / `capability_progress` → set "processing" state
//   - `gate` / `auth_required` → surface pending gate (with run_id +
//     gate_ref so resolveGate can build the v2 path)
//   - `final_reply` → append assistant message, stop processing
//   - `cancelled` / `failed` → stop processing
//   - `keep_alive`, `projection_*` → ignore (M1 UI does not render
//     the projection-state stream)
export function useChatEvents({
  threadId,
  setMessages,
  setIsProcessing,
  setPendingGate,
  setActiveRun,
}) {
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

        case "keep_alive":
        case "projection_snapshot":
        case "projection_update":
        default:
          return;
      }
    },
    [threadId, setMessages, setIsProcessing, setPendingGate, setActiveRun],
  );
}
