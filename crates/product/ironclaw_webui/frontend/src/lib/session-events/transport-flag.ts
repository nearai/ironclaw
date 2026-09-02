// Deployment-advertised session-event transport flag.
//
// `GET /api/webchat/v2/session` advertises `features.session_events` only
// when the deployment wired a single-use socket ticket store (fail closed).
// The auth session hook records the flag here; route hooks consult it when
// choosing between the session socket and the compatibility SSE transport.
// Deliberately tiny and dependency-free: it is imported by the eager /chat
// closure, while the socket client itself loads lazily.

let sessionEventsAdvertised = false;

export function setSessionEventsAdvertised(enabled: boolean): void {
  sessionEventsAdvertised = Boolean(enabled);
}

export function isSessionEventsAdvertised(): boolean {
  return sessionEventsAdvertised;
}
