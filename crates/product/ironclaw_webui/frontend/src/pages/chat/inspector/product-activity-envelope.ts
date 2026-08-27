import { ActivityKind } from "./activity-kind";
import { publishProductInspectorActivity } from "./product-activity";
import {
  AMBIGUOUS_RUN_ID,
  mergeRunIdCandidate,
  type RunIdCandidate,
} from "../lib/run-id-candidate";

interface CapabilityActivityFrame {
  turn_run_id?: unknown;
  invocation_id?: unknown;
  activity_id?: unknown;
  status?: unknown;
}

interface RunStatusFrame {
  run_id?: unknown;
  status?: unknown;
}

interface ProjectionItem {
  run_status?: RunStatusFrame;
  capability_activity?: CapabilityActivityFrame;
}

interface InspectorEnvelope {
  type?: unknown;
  frame?: {
    ack?: { run_id?: unknown };
    progress?: { turn_run_id?: unknown };
    activity?: CapabilityActivityFrame;
    prompt?: { turn_run_id?: unknown; run_id?: unknown; gate_ref?: unknown; request_id?: unknown };
    reply?: { turn_run_id?: unknown };
    state?: { items?: ProjectionItem[] };
  };
}

const ACTIVE_CAPABILITY_STATUSES = new Set(["started", "running"]);
const SUCCESS_CAPABILITY_STATUSES = new Set(["completed", "succeeded"]);
const FAILED_RUN_STATUSES = new Set([
  "failed",
  "cancelled",
  "recovery_required",
  "timed_out",
]);

function publishCapabilityActivity(
  threadId: unknown,
  fallbackRunId: unknown,
  activity: CapabilityActivityFrame | undefined,
): void {
  const runId = activity?.turn_run_id || fallbackRunId;
  const activityId = activity?.invocation_id || activity?.activity_id;
  if (!runId || !activityId) return;
  const status = String(activity.status || "started").toLowerCase();
  const kind = SUCCESS_CAPABILITY_STATUSES.has(status)
    ? ActivityKind.ToolCompleted
    : ACTIVE_CAPABILITY_STATUSES.has(status)
      ? ActivityKind.ToolStarted
      : ActivityKind.ToolFailed;
  publishProductInspectorActivity({
    threadId,
    runId,
    kind,
    activityId,
    summaryKey: kind === ActivityKind.ToolStarted
      ? "inspector.activity.summary.toolStarted"
      : kind === ActivityKind.ToolCompleted
        ? "inspector.activity.summary.toolCompleted"
        : "inspector.activity.summary.toolFailed",
    dedupeKey: `tool:${activityId}:${status}`,
  });
}

function publishRunStatusActivity(threadId: unknown, runStatus: RunStatusFrame): void {
  const runId = runStatus.run_id;
  const status = String(runStatus.status || "").toLowerCase();
  if (!runId || !status) return;
  if (["queued", "running"].includes(status)) {
    publishProductInspectorActivity({
      threadId,
      runId,
      kind: status === "queued" ? ActivityKind.TurnStarted : ActivityKind.Progress,
      summaryKey: status === "queued"
        ? "inspector.activity.summary.turnQueued"
        : "inspector.activity.summary.turnRunning",
      dedupeKey: `run:${status}`,
    });
  } else if (["completed", "succeeded"].includes(status)) {
    publishProductInspectorActivity({
      threadId,
      runId,
      kind: ActivityKind.FinalResponseCompleted,
      summaryKey: "inspector.activity.summary.finalResponseCompleted",
      dedupeKey: "run:completed",
    });
  } else if (FAILED_RUN_STATUSES.has(status)) {
    const summaryKey = status === "cancelled"
      ? "inspector.activity.summary.runCancelled"
      : status === "timed_out"
        ? "inspector.activity.summary.runTimedOut"
        : status === "recovery_required"
          ? "inspector.activity.summary.runRequiresRecovery"
          : "inspector.activity.summary.runFailed";
    publishProductInspectorActivity({
      threadId,
      runId,
      kind: ActivityKind.FinalResponseCompleted,
      summaryKey,
      dedupeKey: "run:terminal-failure",
    });
  } else if (status.startsWith("blocked_") || status === "awaiting_gate") {
    publishProductInspectorActivity({
      threadId,
      runId,
      kind: ActivityKind.GateBlocked,
      summaryKey: "inspector.activity.summary.runBlockedByGate",
      dedupeKey: `run:${status}`,
    });
  }
}

export function publishProductInspectorEnvelope(
  envelope: unknown,
  threadId: unknown,
  fallbackRunId: unknown,
): void {
  const value = envelope as InspectorEnvelope | null;
  if (!threadId || !value?.type || !value.frame) return;
  const { type, frame } = value;
  if (type === "accepted") {
    publishProductInspectorActivity({
      threadId,
      runId: frame.ack?.run_id,
      kind: ActivityKind.TurnStarted,
      summaryKey: "inspector.activity.summary.turnAccepted",
      dedupeKey: "turn:accepted",
    });
    return;
  }
  if (type === "running" || type === "capability_progress") {
    publishProductInspectorActivity({
      threadId,
      runId: frame.progress?.turn_run_id || fallbackRunId,
      kind: ActivityKind.Progress,
      summaryKey: "inspector.activity.summary.progressReceived",
      dedupeKey: `progress:${type}`,
    });
    return;
  }
  if (type === "capability_activity") {
    publishCapabilityActivity(threadId, fallbackRunId, frame.activity);
    return;
  }
  if (type === "gate" || type === "auth_required") {
    const runId = frame.prompt?.turn_run_id || frame.prompt?.run_id || fallbackRunId;
    const gateRef = frame.prompt?.gate_ref || frame.prompt?.request_id || type;
    publishProductInspectorActivity({
      threadId,
      runId,
      kind: ActivityKind.GateBlocked,
      summaryKey: type === "auth_required"
        ? "inspector.activity.summary.runBlockedForAuthorization"
        : "inspector.activity.summary.runBlockedByGate",
      dedupeKey: `gate:${gateRef}`,
    });
    return;
  }
  if (type === "final_reply") {
    publishProductInspectorActivity({
      threadId,
      runId: frame.reply?.turn_run_id || fallbackRunId,
      kind: ActivityKind.FinalResponseCompleted,
      summaryKey: "inspector.activity.summary.finalResponseCompleted",
      dedupeKey: "final:reply",
    });
    return;
  }
  if (type === "projection_snapshot" || type === "projection_update") {
    const items = frame.state?.items || [];
    let batchRunId: RunIdCandidate = null;
    for (const item of items) {
      batchRunId = mergeRunIdCandidate(batchRunId, item.run_status?.run_id);
    }
    const fallbackActivityRunId = batchRunId === AMBIGUOUS_RUN_ID
      ? null
      : batchRunId || fallbackRunId;
    for (const item of items) {
      if (item.run_status) publishRunStatusActivity(threadId, item.run_status);
      if (item.capability_activity) {
        publishCapabilityActivity(threadId, fallbackActivityRunId, item.capability_activity);
      }
    }
  }
}
