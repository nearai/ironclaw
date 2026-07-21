// Shared product-auth OAuth callback event contract.
//
// A product-auth OAuth flow (extension setup, in-chat channel connect, in-chat
// auth gate) opens a popup to an external provider. The callback tab writes a
// completion payload to localStorage AND posts it on a BroadcastChannel; the
// opener tab listens on both and also polls localStorage. Both
// `pages/extensions/hooks/useExtensions.ts` and `pages/chat/hooks/useChat.ts`
// drive that same event/popup contract, so the channel/storage-key constants,
// the completion parser, the subscribe/read primitives, and the
// HTTPS-auth-URL + popup helpers live here once. Each hook keeps its own state
// machine; only this transport/popup contract is shared.
//
// Mirrors the style of `lib/channel-connection-events.ts`.

export const OAUTH_CALLBACK_CHANNEL = "ironclaw-product-auth";
export const OAUTH_CALLBACK_STORAGE_KEY = "ironclaw:product-auth:oauth-complete";
export const OAUTH_CALLBACK_MESSAGE_TYPE = "ironclaw:product-auth:oauth-complete";

export function isHttpsAuthUrl(url) {
  try {
    return new URL(url).protocol === "https:";
  } catch (_) {
    return false;
  }
}

// Point an authorization popup at `url`. Reuses an already-open popup when one
// is supplied and still open; otherwise opens a fresh `noopener` window. Refuses
// non-HTTPS urls. Returns `{ ok, popup, reason? }`.
export function openAuthPopup(url, popup = null) {
  if (!isHttpsAuthUrl(url)) return { ok: false, popup: null, reason: "insecure_url" };
  if (popup && !popup.closed) {
    popup.location.href = url;
    return { ok: true, popup };
  }
  const opened = window.open(url, "_blank", "noopener,noreferrer");
  // Per spec, `window.open` with "noopener" returns null even when the popup
  // opens, so null here is NOT evidence of a blocked popup (the v1 gateway
  // handles this the same way). Real blocked-popup detection happens at the
  // `about:blank` pre-open sites, whose feature list carries no "noopener".
  return { ok: true, popup: opened || null, reason: null };
}

export function parseProductAuthOAuthCompletion(value) {
  if (!value) return null;
  try {
    return JSON.parse(value);
  } catch {
    return null;
  }
}

// Read + parse the latest completion payload the callback tab persisted to
// localStorage. Callers poll this on their own interval cadence.
export function readLatestProductAuthOAuthCompletion(browserWindow) {
  return parseProductAuthOAuthCompletion(
    browserWindow?.localStorage?.getItem?.(OAUTH_CALLBACK_STORAGE_KEY),
  );
}

// Wire a completion handler to the two cross-tab transports the callback uses: a
// BroadcastChannel post and a localStorage `storage` event. Interval polling of
// the latest stored completion stays in each caller — their poll bodies differ
// (setup refresh, server-state poll, gate clear) and read via
// `readLatestProductAuthOAuthCompletion`. Returns an unsubscribe fn.
export function subscribeProductAuthOAuthCompletion(browserWindow, handler) {
  if (!browserWindow || typeof handler !== "function") return () => {};
  let channel = null;
  if (typeof browserWindow.BroadcastChannel === "function") {
    channel = new browserWindow.BroadcastChannel(OAUTH_CALLBACK_CHANNEL);
    channel.onmessage = (event) => handler(event.data);
  }
  const onStorage = (event) => {
    if (event.key !== OAUTH_CALLBACK_STORAGE_KEY) return;
    handler(parseProductAuthOAuthCompletion(event.newValue));
  };
  browserWindow.addEventListener?.("storage", onStorage);
  return () => {
    if (channel) channel.close();
    browserWindow.removeEventListener?.("storage", onStorage);
  };
}

function isProductAuthOAuthCompletion(payload) {
  return payload?.type === OAUTH_CALLBACK_MESSAGE_TYPE && payload?.status === "completed";
}

// A completion satisfies an extension-setup flow only when it carries a matching
// flow id. A completion carrying no flow id — or one from a DIFFERENT
// extension's flow in another tab — must NOT match: treating a missing flow id
// as a match let a stale cross-tab callback prematurely activate/close the
// modal. When the OAuth response carried no flow id, this fast-path stays
// disabled and the caller's polling path is the sole completion signal. Mirrors
// the stricter in-chat gate, which also keys on the flow id.
export function completionMatchesFlow(payload, flowId) {
  if (!isProductAuthOAuthCompletion(payload)) return false;
  if (!flowId) return false;
  return payload.flowId === flowId || payload.flow_id === flowId;
}

// A FAILURE signal from the callback popup (provider denial, exchange failure,
// route rejection). Failures match only their own flow id, exactly like
// completions: a stale or foreign failure must never flip an unrelated
// surface into an error state.
export function failureMatchesFlow(payload, flowId) {
  if (payload?.type !== OAUTH_CALLBACK_MESSAGE_TYPE) return false;
  if (payload?.status !== "failed") return false;
  if (!flowId) return false;
  return payload.flowId === flowId || payload.flow_id === flowId;
}

// A completion satisfies an in-chat auth gate. A `turn_gate_resume` continuation
// must match the gate's run/gate refs; a completion without one falls back to a
// timestamp check so a callback that fired after we started listening still
// resolves the gate.
export function completionMatchesGate(payload, gate, listeningSince) {
  if (!isProductAuthOAuthCompletion(payload)) return false;
  const continuation = payload?.continuation;
  // The timestamp fallback exists for legacy payloads that carry no
  // continuation at all. A payload with a NON-gate continuation (e.g.
  // `setup_only` from an extension-setup flow completing in another tab) is
  // known to belong to a different flow shape and must never satisfy a gate —
  // treating it as a wildcard cleared pending chat gates whenever any other
  // extension finished OAuth.
  if (!continuation) {
    return Number(payload?.completedAt || 0) >= listeningSince;
  }
  if (continuation.type !== "turn_gate_resume") return false;
  if (continuation.turn_run_ref && continuation.turn_run_ref !== gate?.runId) return false;
  if (continuation.gate_ref && continuation.gate_ref !== gate?.gateRef) return false;
  return true;
}
