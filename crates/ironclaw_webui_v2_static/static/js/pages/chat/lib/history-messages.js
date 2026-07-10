// Map v2 `ThreadMessageRecord[]` from RebornTimelineResponse into
// the message shape the UI components render. Turn grouping consumes the
// normalized `turnRunId` carried by records and previews. Records carry
// `attachments: AttachmentRef[]`; we project them into the render shape
// `MessageBubble` expects so attachment cards survive a page refresh and a
// thread switch (the timeline is the source of truth — the bytes stay
// behind the project mount, the cards render from the refs).

import { attachmentKindFromMime, formatBytes } from "./attachments.js";
import { attachmentUrl } from "../../../lib/api.js";
import {
  isBusyRejectedStatus,
  uiStatusFromRecordStatus,
} from "./message-status.js";

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
    // Normalize busy outcomes through the same mapper `useChat.send` uses on
    // the optimistic path, so a message renders identically live and after a
    // reload. A deferred-busy message was accepted-and-queued (renders
    // queued); only a rejected-busy message was dropped (renders error and
    // carries the durable resend copy).
    const isBusyRejected = role === "user" && isBusyRejectedStatus(record.status);
    messages.push({
      id,
      role,
      content: record.content || "",
      attachments: attachmentsFromRecord(record, threadId),
      timestamp: timestampForRecord(record),
      kind: record.kind,
      status:
        role === "user" ? uiStatusFromRecordStatus(record.status) : record.status,
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

const GATE_DECLINED_ERROR_KIND = "gate_declined";

// Map a `CapabilityDisplayPreviewEnvelope` (timeline) or
// `CapabilityDisplayPreviewView` (SSE) into the field set
// `ToolActivityCard` destructures.
export function toolCardFromPreview(preview) {
  const failed = preview.status === "failed" || preview.status === "killed";
  const errorKind = preview.error_kind || null;
  const activityOrder = numericActivityOrder(preview.activity_order);
  const previewError = failed
    ? previewToolError(preview, errorKind)
    : { text: null, key: null };
  return {
    invocationId: preview.invocation_id,
    callId: preview.invocation_id,
    capabilityId: preview.capability_id || null,
    toolName: toolDisplayName(preview.title || preview.capability_id) || "tool",
    toolStatus: toolStatusFromActivityStatus(preview.status, errorKind),
    toolDetail: preview.subtitle || null,
    toolParameters: preview.input_summary || null,
    // On failure the output fields carry the error text — surface it
    // only through `toolError` so the card renders it once in red,
    // not twice (once as a teal result preview and once as the error).
    toolResultPreview: failed
      ? null
      : preview.output_preview || preview.output_summary || null,
    toolError: previewError.text,
    toolErrorKind: errorKind,
    toolErrorKey: previewError.key,
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

// Resolve a failed preview's error into `{ text, key }`. A sanitized,
// display-safe `output_preview` is surfaced verbatim so a live activity card
// and the reloaded preview card show the same text. Backend failures may
// carry raw/unsafe summary text, so without a preview they fall back to the
// friendly localized message rather than leaking the summary. Other kinds may
// surface their summary / result_ref.
function previewToolError(preview, errorKind) {
  const previewText = trimmedOrNull(preview.output_preview);
  if (previewText) return { text: previewText, key: null };
  const normalizedKind =
    typeof errorKind === "string" ? errorKind.trim().toLowerCase().replaceAll("-", "_") : "";
  if (normalizedKind === "backend") return resolveToolError(errorKind);
  const summary =
    trimmedOrNull(preview.output_summary) || trimmedOrNull(preview.result_ref);
  if (summary) return { text: summary, key: null };
  return resolveToolError(errorKind);
}

function trimmedOrNull(value) {
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  return trimmed || null;
}

// Map a `CapabilityActivityView` (SSE lifecycle frame) into the same
// card shape. While the invocation is still running the backend now
// carries the staged input on the activity frame (`subtitle` =
// inline primary argument, `input_summary` = parameters), so the row
// shows `tool   <arg>` live instead of a bare name. Output fields stay
// empty until the preview frame lands at completion.
export function toolCardFromActivity(activity) {
  const activityOrder = numericActivityOrder(activity.activity_order);
  const errorKind = activity.error_kind || null;
  const errorSummary =
    typeof activity.error_summary === "string" ? activity.error_summary.trim() : "";
  const activityError = resolveToolError(errorKind, errorSummary);
  return {
    invocationId: activity.invocation_id,
    callId: activity.invocation_id,
    capabilityId: activity.capability_id || null,
    toolName: toolDisplayName(activity.capability_id) || "tool",
    toolStatus: toolStatusFromActivityStatus(activity.status, errorKind),
    toolDetail: activity.subtitle || null,
    toolParameters: activity.input_summary || null,
    toolResultPreview: null,
    toolError: activityError.text,
    toolErrorKind: errorKind,
    toolErrorKey: activityError.key,
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

// error_kind -> { i18n key, English fallback }. The fallback string is the
// source of truth for non-localized contexts (and unit tests); the key lets
// the rendering surface localize via `t()` when no concrete summary applies.
const TOOL_ERROR_KIND_I18N = {
  backend: { key: "tool.errorBackend", text: "The tool backend failed." },
  security: { key: "tool.errorSecurity", text: "The tool response was blocked by a security check." },
  security_rejected: { key: "tool.errorSecurity", text: "The tool response was blocked by a security check." },
  security_rejection: { key: "tool.errorSecurity", text: "The tool response was blocked by a security check." },
  cancelled: { key: "tool.errorCancelled", text: "The tool call was cancelled." },
  timeout: { key: "tool.errorTimeout", text: "The tool call timed out." },
  invalid_request: { key: "tool.errorInvalidRequest", text: "The tool request was invalid." },
  auth: { key: "tool.errorAuth", text: "The tool needs authentication." },
  authentication: { key: "tool.errorAuth", text: "The tool needs authentication." },
  authorization: { key: "tool.errorAuth", text: "The tool needs authentication." },
  permission: { key: "tool.errorPermission", text: "The tool call was not allowed." },
  approval_denied: { key: "tool.errorPermission", text: "The tool call was not allowed." },
  [GATE_DECLINED_ERROR_KIND]: { key: "tool.errorGateDeclined", text: "gate declined" },
};

// Resolve a failure into `{ text, key }`:
// - a concrete `errorSummary` wins (raw backend/tool text), with no i18n key
//   (it is not a fixed phrase);
// - a known `errorKind` maps to a localizable fallback (text + key);
// - an unknown `errorKind` is surfaced readably (underscores -> spaces). This
//   is the one explicit, documented fallback — not a silent catch-all: it
//   only fires for kinds the backend adds that the UI has not localized yet.
export function resolveToolError(errorKind, errorSummary = "") {
  const summary = typeof errorSummary === "string" ? errorSummary.trim() : "";
  if (summary) return { text: summary, key: null };
  const value = typeof errorKind === "string" ? errorKind.trim() : "";
  if (!value) return { text: null, key: null };
  const normalized = value.toLowerCase().replaceAll("-", "_");
  const known = TOOL_ERROR_KIND_I18N[normalized];
  if (known) return { text: known.text, key: known.key };
  return { text: value.replaceAll("_", " "), key: null };
}

// Localize a card's stored error: when the builder mapped a known error kind
// to an i18n key, resolve it via `t()`; otherwise the stored `toolError` is a
// concrete summary (or an unknown-kind fallback) shown verbatim.
export function localizedToolError(toolError, toolErrorKey, t) {
  if (toolErrorKey && typeof t === "function") return t(toolErrorKey);
  return toolError;
}

export function isTerminalToolStatus(status) {
  return status === "success" || status === "error" || status === "declined";
}

// Single emptiness predicate for tool display fields. Treats whitespace-only
// strings as empty; everything non-string is "present" when non-null.
export function hasDisplayValue(value) {
  return typeof value === "string" ? value.trim().length > 0 : value != null;
}

// Fill empty display fields on `target` from `source` using `hasDisplayValue`.
// Shared by the live merge (tool-activity-state) and the refresh hydration
// (useHistory) so both apply identical coalescing rules. Returns `target`
// unchanged (same reference) when nothing is filled, copying only on change.
export function coalesceToolFields(target, source, fields) {
  let result = target;
  for (const field of fields) {
    if (!hasDisplayValue(result?.[field]) && hasDisplayValue(source?.[field])) {
      if (result === target) result = { ...target };
      result[field] = source[field];
    }
  }
  return result;
}

export function toolDisplayName(name) {
  const value = typeof name === "string" ? name.trim() : "";
  if (!value) return "";
  const parts = value.split(".");
  return parts[parts.length - 1] || value;
}

function toolStatusFromActivityStatus(status, errorKind = null) {
  if (errorKind === GATE_DECLINED_ERROR_KIND) return "declined";
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
