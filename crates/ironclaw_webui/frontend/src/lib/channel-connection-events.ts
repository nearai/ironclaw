// @ts-nocheck
import { sendMessage } from "./api";
const CHANNEL_CONNECTION_BROADCAST = "ironclaw-channel-connection";
const CHANNEL_CONNECTION_STORAGE_KEY = "ironclaw:channel-connection:connected";
const CHANNEL_CONNECTION_MESSAGE_TYPE = "ironclaw:channel-connection:connected";
const CHANNEL_CONNECTION_WAITING_KEY = "ironclaw:channel-connection:waiting:v1";
const CONTINUATION_SUFFIX = " is connected. Continue the previous request.";
const WAITER_TTL_MS = 24 * 60 * 60 * 1000;

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

export function channelConnectionContinuationMessage(channel) {
  return `${channelConnectionDisplayName(channel)}${CONTINUATION_SUFFIX}`;
}

export function connectionEventMatchesOnboarding(event, onboarding) {
  const eventChannel = normalizeConnectionChannel(event?.channel);
  const onboardingChannel = normalizeConnectionChannel(onboarding?.extensionName);
  return Boolean(eventChannel && onboardingChannel && eventChannel === onboardingChannel);
}

// Broadcast that a connectable channel just connected and resume any local
// chat waiters that recorded they were blocked on this channel.
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

  return resumeWaitingChannelConnections(payload).catch((error) => {
    console.error("channel connection waiting-thread resume failed:", error);
    return [];
  });
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

export function rememberChannelConnectionWaiter({
  channel,
  threadId,
  sourceMessageId = null,
} = {}) {
  const normalized = normalizeConnectionChannel(channel);
  const normalizedThreadId = String(threadId || "").trim();
  if (!normalized || !normalizedThreadId) return;
  const waiters = readWaitingChannelConnections().filter(
    (waiter) =>
      !(
        waiter.channel === normalized &&
        waiter.threadId === normalizedThreadId &&
        waiter.sourceMessageId === (sourceMessageId || null)
      ),
  );
  waiters.push({
    channel: normalized,
    threadId: normalizedThreadId,
    sourceMessageId: sourceMessageId || null,
    createdAt: Date.now(),
  });
  writeWaitingChannelConnections(waiters);
}

export function forgetChannelConnectionWaiter({
  channel,
  threadId,
  sourceMessageId = null,
} = {}) {
  const normalized = normalizeConnectionChannel(channel);
  const normalizedThreadId = String(threadId || "").trim();
  if (!normalized || !normalizedThreadId) return;
  writeWaitingChannelConnections(
    readWaitingChannelConnections().filter(
      (waiter) =>
        !(
          waiter.channel === normalized &&
          waiter.threadId === normalizedThreadId &&
          (!sourceMessageId || waiter.sourceMessageId === sourceMessageId)
        ),
    ),
  );
}

export async function resumeWaitingChannelConnections(event = {}) {
  const eventChannel = normalizeConnectionChannel(event.channel);
  if (!eventChannel) return [];
  const sourceThreadId = event.sourceThreadId || null;
  const waiters = readWaitingChannelConnections();
  const matching = [];
  const remaining = [];
  const seenThreads = new Set();
  for (const waiter of waiters) {
    if (waiter.channel !== eventChannel) {
      remaining.push(waiter);
      continue;
    }
    if (sourceThreadId && waiter.threadId === sourceThreadId) {
      remaining.push(waiter);
      continue;
    }
    if (seenThreads.has(waiter.threadId)) continue;
    seenThreads.add(waiter.threadId);
    matching.push(waiter);
  }
  if (matching.length === 0) return [];
  const content = channelConnectionContinuationMessage(eventChannel);
  const results = [];
  const resumedThreadIds = new Set();
  try {
    for (const waiter of matching) {
      results.push(
        await sendMessage({
          threadId: waiter.threadId,
          content,
        }),
      );
      resumedThreadIds.add(waiter.threadId);
    }
  } catch (error) {
    const unresumed = matching.filter(
      (waiter) => !resumedThreadIds.has(waiter.threadId),
    );
    writeWaitingChannelConnections([...remaining, ...unresumed]);
    throw error;
  }
  writeWaitingChannelConnections(remaining);
  return results;
}

function readWaitingChannelConnections() {
  const storage = connectionStorage();
  if (!storage) return [];
  try {
    const parsed = JSON.parse(storage.getItem(CHANNEL_CONNECTION_WAITING_KEY) || "[]");
    if (!Array.isArray(parsed)) return [];
    const now = Date.now();
    return parsed
      .map((item) => ({
        channel: normalizeConnectionChannel(item?.channel),
        threadId: String(item?.threadId || "").trim(),
        sourceMessageId:
          typeof item?.sourceMessageId === "string" ? item.sourceMessageId : null,
        createdAt: Number(item?.createdAt || 0),
      }))
      .filter(
        (item) =>
          item.channel &&
          item.threadId &&
          !isExpiredWaiter(item.createdAt, now),
      );
  } catch (_) {
    return [];
  }
}

function writeWaitingChannelConnections(waiters) {
  const storage = connectionStorage();
  if (!storage) return;
  try {
    storage.setItem(CHANNEL_CONNECTION_WAITING_KEY, JSON.stringify(waiters));
  } catch (_) {
    // Best-effort waiting-thread registry; connection itself has already succeeded.
  }
}

function isExpiredWaiter(createdAt, now) {
  return createdAt > 0 && now - createdAt > WAITER_TTL_MS;
}

function connectionStorage() {
  if (typeof window === "undefined") return null;
  try {
    return window.localStorage || null;
  } catch (_) {
    return null;
  }
}
