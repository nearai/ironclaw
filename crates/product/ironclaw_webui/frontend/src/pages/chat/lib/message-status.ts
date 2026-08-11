// Single source of truth for chat message status values and the
// wire/record-status -> UI-status mapping.
//
// The same logical message is rendered twice: optimistically by
// `useChat.send` (from a send response `outcome`) and durably by
// `messagesFromTimeline` (from a persisted `ThreadMessageRecord.status`).
// Both paths MUST map the same wire value to the same UI status, or a
// message flips appearance across a reload — e.g. a busy-deferred message
// showing a "queued" badge live but an "error" bubble after refresh
// (the bug this module exists to prevent).

// Wire/record status values produced by the backend (send `outcome` or
// persisted `ThreadMessageRecord.status`).
export const RECORD_STATUS = Object.freeze({
  // Accepted but deferred because the thread was busy: it is queued to run
  // once the active run yields. Renders as queued, never as an error.
  DEFERRED_BUSY: "deferred_busy",
  // Rejected because the thread was busy: it was NOT accepted and must be
  // resent. Renders as an error.
  REJECTED_BUSY: "rejected_busy",
  // Explicitly queued.
  QUEUED: "queued",
  // The run actively processing this message.
  RUNNING: "running",
} as const);

// UI-facing status values consumed by `MessageBubble`.
export const UI_MESSAGE_STATUS = Object.freeze({
  QUEUED: "queued",
  ERROR: "error",
  RUNNING: "running",
} as const);

// Map a wire/record status to the UI status `MessageBubble` renders.
// Unknown statuses pass through unchanged (e.g. "accepted", "finalized",
// "submitted") so this mapper only normalizes the values that diverge.
export function uiStatusFromRecordStatus(recordStatus) {
  switch (recordStatus) {
    case RECORD_STATUS.DEFERRED_BUSY:
    case RECORD_STATUS.QUEUED:
      return UI_MESSAGE_STATUS.QUEUED;
    case RECORD_STATUS.REJECTED_BUSY:
      return UI_MESSAGE_STATUS.ERROR;
    default:
      return recordStatus;
  }
}

// True when a busy outcome means the message was rejected (not accepted),
// so the UI must attach the durable "resend it" copy. A deferred-busy
// message was accepted-and-queued and gets no error copy.
export function isBusyRejectedStatus(recordStatus) {
  return recordStatus === RECORD_STATUS.REJECTED_BUSY;
}
