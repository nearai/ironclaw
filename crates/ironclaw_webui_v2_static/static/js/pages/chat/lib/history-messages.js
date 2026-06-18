// Map v2 `ThreadMessageRecord[]` from RebornTimelineResponse into
// the message shape the UI components render. Turn grouping consumes the
// normalized `turnRunId` carried by records and previews. Records carry
// `attachments: AttachmentRef[]`; we project them into the render shape
// `MessageBubble` expects so attachment cards survive a page refresh and a
// thread switch (the timeline is the source of truth — the bytes stay
// behind the project mount, the cards render from the refs).

import { attachmentKindFromMime, formatBytes } from "./attachments.js";
import { attachmentUrl } from "../../../lib/api.js";

// Project a stored `AttachmentRef` (snake_case wire shape) into the
// render shape `MessageBubble` consumes. The timeline never carries bytes,
// so `preview_url` is null here; a landed image instead gets a `fetch_url`
// the bubble lazily resolves into a thumbnail (an authenticated byte fetch,
// since `<img>` cannot send a bearer header). The just-sent optimistic
// message keeps its local data URL in `preview_url` and needs no fetch.
function attachmentsFromRecord(record, threadId) {
  const refs = record.attachments;
  if (!Array.isArray(refs) || refs.length === 0) return undefined;
  return refs.map((ref) => {
    const kind = ref.kind || attachmentKindFromMime(ref.mime_type);
    // Any landed attachment can serve its bytes — for an image thumbnail or
    // for click-to-preview of any kind. A ref without a storage_key never
    // landed, so there are no bytes to fetch. Require every addressing part so
    // a malformed record yields a plain card (no fetch) rather than throwing in
    // `attachmentUrl` mid-projection.
    const fetch_url =
      threadId && ref.storage_key && record.message_id && ref.id
        ? attachmentUrl({
            threadId,
            messageId: record.message_id,
            attachmentId: ref.id,
          })
        : null;
    return {
      id: ref.id,
      filename: ref.filename || "attachment",
      mime_type: ref.mime_type || "",
      kind,
      size_label: Number.isFinite(ref.size_bytes) ? formatBytes(ref.size_bytes) : "",
      preview_url: null,
      fetch_url,
    };
  });
}

export function messagesFromTimeline(records, pendingMessages = [], threadId = null) {
  const seen = new Set();
  const messages = [];

  for (const record of records || []) {
    if (record.kind === "tool_result_reference") {
      // LLM-visible transcript artifact (result_ref + safe_summary).
      // Not a UI message — the matching `capability_display_preview`
      // record renders the tool card.
      continue;
    }

    if (record.kind === "capability_display_preview") {
      const card = toolCardFromPreviewRecord(record);
      if (!card) continue;
      const id = `tool-${card.invocationId}`;
      if (seen.has(id)) continue;
      seen.add(id);
      messages.push({
        id,
        role: "tool_activity",
        ...card,
        timestamp: timestampForRecord(record) || card.updatedAt || null,
        sequence: record.sequence,
        activityOrder: card.activityOrder,
        activityOrderSource: card.activityOrderSource,
        turnRunId: record.turn_run_id || null,
      });
      continue;
    }

    const id = `msg-${record.message_id}`;
    if (seen.has(id)) continue;
    seen.add(id);
    const role = roleForRecord(record);
    const isBusyRejected =
      role === "user" &&
      (record.status === "rejected_busy" || record.status === "deferred_busy");
    messages.push({
      id,
      role,
      content: record.content || "",
      attachments: attachmentsFromRecord(record, threadId),
      timestamp: timestampForRecord(record),
      kind: record.kind,
      status: isBusyRejected ? "error" : record.status,
      ...(isBusyRejected && {
        error:
          "This message wasn't sent because Ironclaw was busy. Resend it to try again.",
      }),
      isFinalReply: isFinalAssistantRecord(record),
      sequence: record.sequence,
      turnRunId: record.turn_run_id || null,
    });
  }

  // Pending rows are dropped from the ref by the caller as soon as
  // `sendMessage` returns (server has accepted the message and the
  // confirmed row will arrive via timeline). The id-based guard
  // remains as defense-in-depth in case a caller passes a pending
  // that was already merged into the timeline.
  for (const pending of pendingMessages) {
    if (seen.has(pending.id)) continue;
    const message = pendingMessageForRender(pending);
    if (message.timelineMessageId && seen.has(`msg-${message.timelineMessageId}`)) {
      continue;
    }
    messages.push(message);
  }

  return messages;
}

function pendingMessageForRender(pending) {
  return {
    ...pending,
    role: pending.role || "user",
    isOptimistic: pending.isOptimistic !== false,
  };
}

function isFinalAssistantRecord(record) {
  return (
    (record.kind === "assistant" || record.kind === "assistant_message") &&
    record.status === "finalized"
  );
}

function roleForRecord(record) {
  switch (record.kind) {
    case "user":
    case "user_message":
      return "user";
    case "assistant":
    case "assistant_message":
    case "tool_result":
      return "assistant";
    case "system":
      return "system";
    default:
      return record.actor_id ? "user" : "assistant";
  }
}

function timestampForRecord(record) {
  // ThreadMessageRecord has no top-level timestamp; surfaces use
  // the sequence ordering for now. Browsers render the wall-clock
  // when an event arrives (FinalReplyView.generated_at).
  return record.received_at || record.created_at || null;
}

function toolCardFromPreviewRecord(record) {
  if (!record.content) return null;
  let envelope;
  try {
    envelope = JSON.parse(record.content);
  } catch (err) {
    console.warn("Failed to parse capability_display_preview envelope", err);
    return null;
  }
  if (!envelope || !envelope.invocation_id) return null;
  return toolCardFromPreview(envelope);
}

// Map a `CapabilityDisplayPreviewEnvelope` (timeline) or
// `CapabilityDisplayPreviewView` (SSE) into the field set
// `ToolActivityCard` destructures.
export function toolCardFromPreview(preview) {
  const failed = preview.status === "failed" || preview.status === "killed";
  const activityOrder = numericActivityOrder(preview.activity_order);
  return {
    invocationId: preview.invocation_id,
    callId: preview.invocation_id,
    capabilityId: preview.capability_id || null,
    toolName: toolDisplayName(preview.title || preview.capability_id) || "tool",
    toolStatus: toolStatusFromActivityStatus(preview.status),
    toolDetail: preview.subtitle || null,
    toolParameters: preview.input_summary || null,
    // On failure the output fields carry the error text — surface it
    // only through `toolError` so the card renders it once in red,
    // not twice (once as a teal result preview and once as the error).
    toolResultPreview: failed
      ? null
      : preview.output_preview || preview.output_summary || null,
    toolError: failed
      ? preview.output_summary ||
        preview.output_preview ||
        preview.result_ref ||
        null
      : null,
    toolDurationMs: null,
    updatedAt: preview.updated_at || null,
    resultRef: preview.result_ref || null,
    truncated: Boolean(preview.truncated),
    outputBytes: preview.output_bytes ?? null,
    outputKind: preview.output_kind || null,
    turnRunId: preview.turn_run_id || null,
    activityOrder,
    activityOrderSource: Number.isFinite(activityOrder) ? "projection" : null,
  };
}

// Map a `CapabilityActivityView` (SSE lifecycle frame) into the same
// card shape. While the invocation is still running the backend now
// carries the staged input on the activity frame (`subtitle` =
// inline primary argument, `input_summary` = parameters), so the row
// shows `tool   <arg>` live instead of a bare name. Output fields stay
// empty until the preview frame lands at completion.
export function toolCardFromActivity(activity) {
  const activityOrder = numericActivityOrder(activity.activity_order);
  return {
    invocationId: activity.invocation_id,
    callId: activity.invocation_id,
    capabilityId: activity.capability_id || null,
    toolName: toolDisplayName(activity.capability_id) || "tool",
    toolStatus: toolStatusFromActivityStatus(activity.status),
    toolDetail: activity.subtitle || null,
    toolParameters: activity.input_summary || null,
    toolResultPreview: null,
    toolError: activity.error_kind || null,
    toolDurationMs: null,
    updatedAt: activity.updated_at || null,
    resultRef: null,
    truncated: false,
    outputBytes: activity.output_bytes ?? null,
    outputKind: null,
    turnRunId: activity.turn_run_id || null,
    activityOrder,
    activityOrderSource: Number.isFinite(activityOrder) ? "projection" : null,
  };
}

export function isTerminalToolStatus(status) {
  return status === "success" || status === "error";
}

export function toolDisplayName(name) {
  const value = typeof name === "string" ? name.trim() : "";
  if (!value) return "";
  const parts = value.split(".");
  return parts[parts.length - 1] || value;
}

function toolStatusFromActivityStatus(status) {
  switch (status) {
    case "completed":
      return "success";
    case "failed":
    case "killed":
      return "error";
    case "started":
    case "running":
    default:
      return "running";
  }
}

function numericActivityOrder(value) {
  const number = Number(value);
  return Number.isFinite(number) ? number : null;
}
