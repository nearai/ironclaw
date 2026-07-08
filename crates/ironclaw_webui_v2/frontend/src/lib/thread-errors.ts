// Map a failed thread-deletion error to a user-facing toast message.
//
// A running/processing thread rejects deletion with an HTTP 409 carrying a
// `busy` kind ({"error":"conflict","kind":"busy"}). The generic humanized API
// message for that body is just "Busy", which reads as no error at all (#4823),
// so callers must translate it into a clear, actionable line. Everything else
// falls back to the API-provided message, then a generic failure string.
//
// Match on `kind === "busy"` specifically rather than any 409: the backend
// also maps generic conflicts to 409, and showing "stop it first" for a
// non-busy conflict would be wrong guidance.
export function isThreadBusyError(error) {
  return error?.status === 409 && error?.payload?.kind === "busy";
}

export function deleteThreadErrorMessage(error, t) {
  if (isThreadBusyError(error)) return t("chat.deleteBusy");
  return error?.message || t("chat.deleteFailed");
}
