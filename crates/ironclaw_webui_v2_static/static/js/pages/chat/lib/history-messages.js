// Map v2 `ThreadMessageRecord[]` from RebornTimelineResponse into
// the message shape the UI components render. Kept narrow: the v2
// timeline contract has no attachments, no generated images, no
// turn-grouping metadata — those v1 affordances are out of scope
// for issue #3886.

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
    messages.push({
      id,
      role: roleForRecord(record),
      content: record.content || "",
      timestamp: timestampForRecord(record),
      kind: record.kind,
      status: record.status,
      sequence: record.sequence,
      turnRunId: record.turn_run_id || null,
    });
  }

  for (const pending of pendingMessages) {
    if (seen.has(pending.id)) continue;
    messages.push(pending);
  }

  return messages;
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
