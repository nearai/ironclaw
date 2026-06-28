/**
 * Single source of truth for retry eligibility.
 *
 * Consumed by both the render guard (message-bubble.js) and the hook
 * (useChat.js) so the rendered control and the handler can never diverge
 * (prevents dead/misleading retry buttons).
 *
 * Attachments are not retryable: failed bubbles only carry render-shape
 * attachment metadata — the original File blobs are gone after the first
 * send and cannot be re-uploaded on retry.
 */
export function isRetryableMessage(message) {
  return (
    !!message &&
    message.role === "user" &&
    message.status === "error" &&
    typeof message.content === "string" &&
    message.content.trim() !== "" &&
    !(Array.isArray(message.attachments) && message.attachments.length > 0)
  );
}
