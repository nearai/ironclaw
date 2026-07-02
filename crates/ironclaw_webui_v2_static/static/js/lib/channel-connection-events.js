const CHANNEL_CONNECTION_BROADCAST = "ironclaw-channel-connection";
const CHANNEL_CONNECTION_STORAGE_KEY = "ironclaw:channel-connection:connected";
const CHANNEL_CONNECTION_MESSAGE_TYPE = "ironclaw:channel-connection:connected";

export function normalizeConnectionChannel(channel) {
  return String(channel || "")
    .trim()
    .toLowerCase()
    .replace(/[-_\s]+/g, "-");
}

export function channelConnectionDisplayName(channel) {
  const normalized = normalizeConnectionChannel(channel);
  if (normalized === "slack") return "Slack";
  const raw = String(channel || "the channel").trim();
  if (!raw) return "the channel";
  return raw
    .replace(/[-_]+/g, " ")
    .replace(/\b\w/g, (char) => char.toUpperCase());
}

// Broadcast that a connectable channel just connected (pairing redeemed here,
// in another tab, or on the extensions page). The parked turn is resumed
// backend-side on redeem; this notification exists only so other open surfaces
// (the extensions/connectable-channels caches, other chat tabs) can invalidate
// their stale "needs setup" snapshots. It never resumes chats itself.
export async function notifyChannelConnected({
  channel,
  provider = null,
  providerUserId = null,
  sourceThreadId = null,
  source = "webui",
} = {}) {
  const normalized = normalizeConnectionChannel(channel);
  if (!normalized) return;
  const payload = {
    type: CHANNEL_CONNECTION_MESSAGE_TYPE,
    channel: normalized,
    provider,
    providerUserId,
    sourceThreadId,
    source,
    completedAt: Date.now(),
    nonce: `${Date.now()}-${Math.random().toString(36).slice(2)}`,
  };

  if (typeof window !== "undefined" && typeof window.BroadcastChannel === "function") {
    const broadcast = new window.BroadcastChannel(CHANNEL_CONNECTION_BROADCAST);
    try {
      broadcast.postMessage(payload);
    } finally {
      broadcast.close();
    }
  }

  if (typeof window !== "undefined") {
    try {
      window.localStorage?.setItem?.(
        CHANNEL_CONNECTION_STORAGE_KEY,
        JSON.stringify(payload),
      );
    } catch (_) {
      // Best-effort cross-tab wakeup; pairing success should never fail because
      // the browser cannot persist a notification.
    }
  }
}

export function subscribeChannelConnected(handler) {
  if (typeof handler !== "function" || typeof window === "undefined") {
    return () => {};
  }

  const handlePayload = (payload) => {
    if (payload?.type !== CHANNEL_CONNECTION_MESSAGE_TYPE) return;
    if (!normalizeConnectionChannel(payload.channel)) return;
    handler(payload);
  };

  let broadcast = null;
  if (typeof window.BroadcastChannel === "function") {
    broadcast = new window.BroadcastChannel(CHANNEL_CONNECTION_BROADCAST);
    broadcast.onmessage = (event) => handlePayload(event.data);
  }

  const onStorage = (event) => {
    if (event.key !== CHANNEL_CONNECTION_STORAGE_KEY) return;
    handlePayload(parseStoredConnectionEvent(event.newValue));
  };
  window.addEventListener("storage", onStorage);

  return () => {
    window.removeEventListener("storage", onStorage);
    if (broadcast) broadcast.close();
  };
}

function parseStoredConnectionEvent(value) {
  if (!value) return null;
  try {
    const parsed = JSON.parse(value);
    return parsed && typeof parsed === "object" ? parsed : null;
  } catch (_) {
    return null;
  }
}
