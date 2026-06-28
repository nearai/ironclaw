import assert from "node:assert/strict";
import test from "node:test";

import {
  channelConnectionContinuationMessage,
  connectionEventMatchesOnboarding,
  notifyChannelConnected,
  rememberChannelConnectionWaiter,
  subscribeChannelConnected,
} from "./channel-connection-events.js";

test("channel connection continuation text is generic and channel-matchable", () => {
  assert.equal(
    channelConnectionContinuationMessage("slack"),
    "Slack is connected. Continue the previous request.",
  );
  assert.equal(
    channelConnectionContinuationMessage("telegram-bot"),
    "Telegram Bot is connected. Continue the previous request.",
  );
  assert.equal(
    connectionEventMatchesOnboarding(
      { channel: "telegram-bot" },
      { extensionName: "telegram_bot" },
    ),
    true,
  );
});

test("channel connection notifications reach subscribers and storage listeners", () => {
  const originalWindow = globalThis.window;
  const listeners = new Map();
  const storageWrites = [];
  const broadcasts = new Set();

  class FakeBroadcastChannel {
    constructor(name) {
      this.name = name;
      this.onmessage = null;
      this.closed = false;
      broadcasts.add(this);
    }

    postMessage(payload) {
      for (const channel of broadcasts) {
        if (channel === this || channel.closed || channel.name !== this.name) {
          continue;
        }
        channel.onmessage?.({ data: payload });
      }
    }

    close() {
      this.closed = true;
      broadcasts.delete(this);
    }
  }

  globalThis.window = {
    BroadcastChannel: FakeBroadcastChannel,
    localStorage: {
      setItem: (key, value) => storageWrites.push({ key, value }),
    },
    addEventListener: (type, handler) => listeners.set(type, handler),
    removeEventListener: (type, handler) => {
      if (listeners.get(type) === handler) listeners.delete(type);
    },
  };

  try {
    const received = [];
    const unsubscribe = subscribeChannelConnected((event) => {
      received.push(event);
    });

    notifyChannelConnected({
      channel: "telegram",
      source: "extensions",
    });

    assert.equal(received.length, 1);
    assert.equal(received[0].channel, "telegram");
    assert.equal(received[0].source, "extensions");
    assert.equal(storageWrites.length, 1);
    assert.equal(storageWrites[0].key, "ironclaw:channel-connection:connected");

    listeners.get("storage")?.({
      key: storageWrites[0].key,
      newValue: JSON.stringify({
        type: "ironclaw:channel-connection:connected",
        channel: "slack",
      }),
    });

    assert.equal(received.length, 2);
    assert.equal(received[1].channel, "slack");
    unsubscribe();
    assert.equal(listeners.has("storage"), false);
  } finally {
    globalThis.window = originalWindow;
  }
});

test("notifyChannelConnected resumes persisted waiting threads and skips the source thread", async () => {
  const originalWindow = globalThis.window;
  const originalFetch = globalThis.fetch;
  const originalSessionStorage = globalThis.sessionStorage;
  const storage = new Map();
  const fetches = [];

  globalThis.sessionStorage = {
    getItem: () => "token-1",
    setItem: () => {},
    removeItem: () => {},
  };
  globalThis.fetch = async (path, options) => {
    fetches.push({ path, options });
    return new Response(JSON.stringify({ run_id: "run-1" }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };
  globalThis.window = {
    localStorage: {
      getItem: (key) => (storage.has(key) ? storage.get(key) : null),
      setItem: (key, value) => storage.set(key, String(value)),
      removeItem: (key) => storage.delete(key),
    },
    addEventListener: () => {},
    removeEventListener: () => {},
  };

  try {
    rememberChannelConnectionWaiter({
      channel: "slack",
      threadId: "thread-source",
      sourceMessageId: "tool-source",
    });
    rememberChannelConnectionWaiter({
      channel: "slack",
      threadId: "thread-waiting",
      sourceMessageId: "tool-waiting",
    });

    await notifyChannelConnected({
      channel: "slack",
      sourceThreadId: "thread-source",
    });

    assert.equal(fetches.length, 1);
    assert.equal(
      fetches[0].path,
      "/api/webchat/v2/threads/thread-waiting/messages",
    );
    const body = JSON.parse(fetches[0].options.body);
    assert.equal(body.content, "Slack is connected. Continue the previous request.");
    assert.equal(typeof body.client_action_id, "string");
    assert.notEqual(body.client_action_id, "");
    assert.equal(fetches[0].options.headers.get("Authorization"), "Bearer token-1");
    const remaining = JSON.parse(
      storage.get("ironclaw:channel-connection:waiting:v1") || "[]",
    );
    assert.deepEqual(
      remaining.map((waiter) => waiter.threadId),
      ["thread-source"],
      "source waiter is skipped so the submitting chat can do its own continuation cleanup",
    );
  } finally {
    globalThis.window = originalWindow;
    globalThis.fetch = originalFetch;
    globalThis.sessionStorage = originalSessionStorage;
  }
});

test("waiters older than the TTL are purged and never resumed", async () => {
  const originalWindow = globalThis.window;
  const originalFetch = globalThis.fetch;
  const originalSessionStorage = globalThis.sessionStorage;
  const storage = new Map();
  const fetches = [];

  globalThis.sessionStorage = {
    getItem: () => "token-1",
    setItem: () => {},
    removeItem: () => {},
  };
  globalThis.fetch = async (path, options) => {
    fetches.push({ path, options });
    return new Response(JSON.stringify({ run_id: "run-1" }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };
  globalThis.window = {
    localStorage: {
      getItem: (key) => (storage.has(key) ? storage.get(key) : null),
      setItem: (key, value) => storage.set(key, String(value)),
      removeItem: (key) => storage.delete(key),
    },
    addEventListener: () => {},
    removeEventListener: () => {},
  };

  try {
    const dayMs = 24 * 60 * 60 * 1000;
    // A waiter that has been parked far longer than any plausible
    // connect-and-return window is stale: the chat that registered it is no
    // longer meaningfully "waiting", so connecting Slack must NOT blast a
    // continuation into it, and it must be evicted so waiters can't pile up
    // unbounded in localStorage.
    storage.set(
      "ironclaw:channel-connection:waiting:v1",
      JSON.stringify([
        {
          channel: "slack",
          threadId: "thread-stale",
          sourceMessageId: null,
          createdAt: Date.now() - dayMs - 60_000,
        },
        {
          channel: "slack",
          threadId: "thread-fresh",
          sourceMessageId: null,
          createdAt: Date.now(),
        },
      ]),
    );

    await notifyChannelConnected({ channel: "slack", source: "extensions" });

    assert.deepEqual(
      fetches.map((fetchCall) => fetchCall.path),
      ["/api/webchat/v2/threads/thread-fresh/messages"],
      "the stale waiter must not be resumed",
    );
    const remaining = JSON.parse(
      storage.get("ironclaw:channel-connection:waiting:v1") || "[]",
    );
    assert.deepEqual(
      remaining.map((waiter) => waiter.threadId),
      [],
      "the stale waiter is purged and the fresh waiter is consumed",
    );
  } finally {
    globalThis.window = originalWindow;
    globalThis.fetch = originalFetch;
    globalThis.sessionStorage = originalSessionStorage;
  }
});

test("notifyChannelConnected resumes every waiting thread when connection has no source chat", async () => {
  const originalWindow = globalThis.window;
  const originalFetch = globalThis.fetch;
  const originalSessionStorage = globalThis.sessionStorage;
  const storage = new Map();
  const fetches = [];

  globalThis.sessionStorage = {
    getItem: () => "token-1",
    setItem: () => {},
    removeItem: () => {},
  };
  globalThis.fetch = async (path, options) => {
    fetches.push({ path, options });
    return new Response(JSON.stringify({ run_id: `run-${fetches.length}` }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };
  globalThis.window = {
    localStorage: {
      getItem: (key) => (storage.has(key) ? storage.get(key) : null),
      setItem: (key, value) => storage.set(key, String(value)),
      removeItem: (key) => storage.delete(key),
    },
    addEventListener: () => {},
    removeEventListener: () => {},
  };

  try {
    rememberChannelConnectionWaiter({
      channel: "slack",
      threadId: "thread-a",
      sourceMessageId: "tool-a",
    });
    rememberChannelConnectionWaiter({
      channel: "slack",
      threadId: "thread-b",
      sourceMessageId: "tool-b",
    });
    rememberChannelConnectionWaiter({
      channel: "slack",
      threadId: "thread-b",
      sourceMessageId: "tool-b-duplicate",
    });
    rememberChannelConnectionWaiter({
      channel: "telegram",
      threadId: "thread-telegram",
      sourceMessageId: "tool-telegram",
    });

    await notifyChannelConnected({ channel: "slack", source: "extensions" });

    assert.deepEqual(
      fetches.map((fetchCall) => fetchCall.path),
      [
        "/api/webchat/v2/threads/thread-a/messages",
        "/api/webchat/v2/threads/thread-b/messages",
      ],
    );
    assert.deepEqual(
      fetches.map((fetchCall) => JSON.parse(fetchCall.options.body).content),
      [
        "Slack is connected. Continue the previous request.",
        "Slack is connected. Continue the previous request.",
      ],
    );
    const remaining = JSON.parse(
      storage.get("ironclaw:channel-connection:waiting:v1") || "[]",
    );
    assert.deepEqual(remaining, [
      {
        channel: "telegram",
        threadId: "thread-telegram",
        sourceMessageId: "tool-telegram",
        createdAt: remaining[0].createdAt,
      },
    ]);
  } finally {
    globalThis.window = originalWindow;
    globalThis.fetch = originalFetch;
    globalThis.sessionStorage = originalSessionStorage;
  }
});
