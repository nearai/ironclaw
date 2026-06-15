// Map v2 `ThreadMessageRecord[]` from RebornTimelineResponse into
// the message shape the UI components render. Turn grouping consumes the
// normalized `turnRunId` carried by records and previews. Records carry
// `attachments: AttachmentRef[]`; we project them into the render shape
// `MessageBubble` expects so attachment cards survive a page refresh and a
// thread switch (the timeline is the source of truth — the bytes stay
// behind the project mount, the cards render from the refs).

import { attachmentKindFromMime, formatBytes } from "./attachments.js";

// Project a stored `AttachmentRef` (snake_case wire shape) into the
// render shape `MessageBubble` consumes. The timeline never carries bytes,
// so `preview_url` is null here; inline image thumbnails only appear on the
// just-sent optimistic message (which has the local data URL).
function attachmentsFromRecord(record) {
  const refs = record.attachments;
  if (!Array.isArray(refs) || refs.length === 0) return undefined;
  return refs.map((ref) => ({
    id: ref.id,
    filename: ref.filename || "attachment",
    mime_type: ref.mime_type || "",
    kind: ref.kind || attachmentKindFromMime(ref.mime_type),
    size_label: Number.isFinite(ref.size_bytes) ? formatBytes(ref.size_bytes) : "",
    preview_url: null,
  }));
}

export function messagesFromTimeline(records, pendingMessages = []) {
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
        turnRunId: record.turn_run_id || null,
      });
      continue;
    }

    const id = `msg-${record.message_id}`;
    if (seen.has(id)) continue;
    seen.add(id);
    const role = roleForRecord(record);
    messages.push({
      id,
      role,
      content: record.content || "",
      attachments: attachmentsFromRecord(record),
      timestamp: timestampForRecord(record),
      kind: record.kind,
      status: record.status,
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
  return {
    invocationId: preview.invocation_id,
    callId: preview.invocation_id,
    toolName: preview.title || preview.capability_id || "tool",
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
  };
}

// Map a `CapabilityActivityView` (SSE lifecycle frame) into the same
// card shape. Activity frames carry only metadata — no title, no
// parameters, no output — so the resulting card is intentionally
// sparse and is meant to be enriched by the next preview frame.
export function toolCardFromActivity(activity) {
  return {
    invocationId: activity.invocation_id,
    callId: activity.invocation_id,
    toolName: activity.capability_id || "tool",
    toolStatus: toolStatusFromActivityStatus(activity.status),
    toolDetail: null,
    toolParameters: null,
    toolResultPreview: null,
    toolError: activity.error_kind || null,
    toolDurationMs: null,
    updatedAt: activity.updated_at || null,
    resultRef: null,
    truncated: false,
    outputBytes: activity.output_bytes ?? null,
    outputKind: null,
    turnRunId: activity.turn_run_id || null,
  };
}

export function isTerminalToolStatus(status) {
  return status === "success" || status === "error";
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
