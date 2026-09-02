// Protocol behavior of the session event client against a scripted
// WebSocket: per-subscription cursors, stale-generation filtering,
// independent failure/rebase, reconnect-with-resume, and fail-closed
// degradation to the SSE fallback.

import { beforeEach, describe, expect, it, vi } from "vitest";
import { SessionEventClient } from "./client";
import { SESSION_EVENT_SCHEMA } from "./protocol";

class FakeSocket {
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSING = 2;
  static CLOSED = 3;
  readyState = FakeSocket.CONNECTING;
  sent: string[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((message: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;

  open() {
    this.readyState = FakeSocket.OPEN;
    this.onopen?.();
  }

  receive(frame: Record<string, unknown>) {
    this.onmessage?.({
      data: JSON.stringify({ schema: SESSION_EVENT_SCHEMA, ...frame }),
    });
  }

  send(text: string) {
    this.sent.push(text);
  }

  close() {
    if (this.readyState === FakeSocket.CLOSED) return;
    this.readyState = FakeSocket.CLOSED;
    this.onclose?.();
  }
}

// The client references the global WebSocket readyState constants.
(globalThis as Record<string, unknown>).WebSocket = FakeSocket;
(globalThis as Record<string, unknown>).window = {
  addEventListener: () => {},
  removeEventListener: () => {},
  location: { protocol: "http:", origin: "http://localhost:3000" },
};

function harness(options: { failMint?: boolean } = {}) {
  const sockets: FakeSocket[] = [];
  let mints = 0;
  const client = new SessionEventClient(
    () => {
      const socket = new FakeSocket();
      sockets.push(socket);
      return socket as unknown as WebSocket;
    },
    async () => {
      mints += 1;
      if (options.failMint) throw new Error("mint refused");
      return {
        ticket: `nonce-${mints}`,
        socket_path: "/api/webchat/v2/session/websocket",
      };
    },
    0,
  );
  return { client, sockets, mintCount: () => mints };
}

async function settle() {
  // Drain the microtask queue so async connect steps complete.
  for (let i = 0; i < 8; i += 1) {
    await Promise.resolve();
  }
}

describe("SessionEventClient", () => {
  beforeEach(() => {
    vi.useRealTimers();
  });

  it("subscribes with each selector's own cursor and delivers events", async () => {
    const { client, sockets } = harness();
    const seenA: unknown[] = [];
    const seenB: unknown[] = [];
    client.subscribe(
      { kind: "thread", thread_id: "thread-a" },
      { onEvent: (event) => seenA.push(event) },
      { fromCursor: '"cursor-a0"' },
    );
    client.subscribe(
      { kind: "thread", thread_id: "thread-b" },
      { onEvent: (event) => seenB.push(event) },
    );
    await settle();
    const socket = sockets[0];
    socket.open();

    const subscribes = socket.sent.map((text) => JSON.parse(text));
    expect(subscribes).toHaveLength(2);
    expect(subscribes[0].selector.thread_id).toBe("thread-a");
    expect(subscribes[0].after_cursor).toBe('"cursor-a0"');
    expect(subscribes[1].selector.thread_id).toBe("thread-b");
    expect(subscribes[1].after_cursor).toBeNull();

    const [subA, subB] = subscribes.map(
      (frame) => frame.subscription_id as string,
    );
    socket.receive({ type: "subscribed", subscription_id: subA, generation: 1 });
    socket.receive({ type: "subscribed", subscription_id: subB, generation: 2 });
    socket.receive({
      type: "event",
      subscription_id: subA,
      generation: 1,
      cursor: '"cursor-a1"',
      event: { type: "projection_update", state: { thread_id: "thread-a" } },
    });
    expect(seenA).toHaveLength(1);
    expect(seenB).toHaveLength(0);
    expect((seenA[0] as { cursor: string }).cursor).toBe('"cursor-a1"');
  });

  it("drops frames from a stale generation after replacement", async () => {
    const { client, sockets } = harness();
    const seen: unknown[] = [];
    client.subscribe(
      { kind: "thread", thread_id: "thread-a" },
      { onEvent: (event) => seen.push(event) },
    );
    await settle();
    const socket = sockets[0];
    socket.open();
    const subscribed = JSON.parse(socket.sent[0]);
    const subId = subscribed.subscription_id as string;

    socket.receive({ type: "subscribed", subscription_id: subId, generation: 5 });
    socket.receive({
      type: "event",
      subscription_id: subId,
      generation: 4,
      cursor: '"stale"',
      event: { type: "projection_update", state: {} },
    });
    expect(seen).toHaveLength(0);
    socket.receive({
      type: "event",
      subscription_id: subId,
      generation: 5,
      cursor: '"fresh"',
      event: { type: "projection_update", state: {} },
    });
    expect(seen).toHaveLength(1);
  });

  it("isolates a subscription error and resubscribes from the last safe cursor", async () => {
    const { client, sockets } = harness();
    const errors: unknown[] = [];
    client.subscribe(
      { kind: "thread", thread_id: "thread-a" },
      { onEvent: () => {}, onError: (error) => errors.push(error) },
    );
    await settle();
    const socket = sockets[0];
    socket.open();
    const subId = JSON.parse(socket.sent[0]).subscription_id as string;
    socket.receive({ type: "subscribed", subscription_id: subId, generation: 1 });
    socket.receive({
      type: "subscription_error",
      subscription_id: subId,
      generation: 1,
      error: "unavailable",
      kind: "replay_unavailable",
      retryable: true,
      last_cursor: '"safe-cursor"',
    });

    expect(errors).toHaveLength(1);
    // The retryable resubscribe is delayed (spin-loop protection); with the
    // test harness's zero delay it lands on the next macrotask.
    await new Promise((resolve) => setTimeout(resolve, 0));
    const resubscribe = JSON.parse(socket.sent[socket.sent.length - 1]);
    expect(resubscribe.type).toBe("subscribe");
    expect(resubscribe.after_cursor).toBe('"safe-cursor"');
  });

  it("drops a subscription the server rejects as non-retryable", async () => {
    const { client, sockets } = harness();
    const errors: Array<{ retryable: boolean }> = [];
    client.subscribe(
      { kind: "thread", thread_id: "thread-a" },
      { onEvent: () => {}, onError: (error) => errors.push(error) },
    );
    await settle();
    const socket = sockets[0];
    socket.open();
    const subId = JSON.parse(socket.sent[0]).subscription_id as string;
    socket.receive({ type: "subscribed", subscription_id: subId, generation: 1 });
    const sentBefore = socket.sent.length;
    socket.receive({
      type: "subscription_error",
      subscription_id: subId,
      generation: 1,
      error: "forbidden",
      kind: "unauthorized",
      retryable: false,
      last_cursor: null,
    });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(errors).toHaveLength(1);
    expect(errors[0].retryable).toBe(false);
    expect(socket.sent.length).toBe(sentBefore);
  });

  it("reconnects on lifetime expiry and resumes each cursor", async () => {
    const { client, sockets, mintCount } = harness();
    client.subscribe(
      { kind: "thread", thread_id: "thread-a" },
      { onEvent: () => {} },
    );
    await settle();
    const first = sockets[0];
    first.open();
    const subId = JSON.parse(first.sent[0]).subscription_id as string;
    first.receive({ type: "subscribed", subscription_id: subId, generation: 1 });
    first.receive({
      type: "event",
      subscription_id: subId,
      generation: 1,
      cursor: '"resume-here"',
      event: { type: "projection_update", state: {} },
    });

    first.receive({ type: "reconnect_hint", reason: "lifetime_expired" });
    await settle();

    expect(mintCount(), "lifetime expiry mints a fresh ticket").toBe(2);
    const second = sockets[1];
    second.open();
    const resumed = JSON.parse(second.sent[0]);
    expect(resumed.after_cursor).toBe('"resume-here"');
  });

  it("degrades after repeated connect failures without ever receiving a frame", async () => {
    vi.useFakeTimers();
    const { client } = harness({ failMint: true });
    const statuses: string[] = [];
    client.subscribe(
      { kind: "thread", thread_id: "thread-a" },
      { onEvent: () => {}, onStatus: (status) => statuses.push(status) },
    );
    for (let attempt = 0; attempt < 4; attempt += 1) {
      await vi.runOnlyPendingTimersAsync();
    }
    expect(client.isDegraded()).toBe(true);
    expect(statuses[statuses.length - 1]).toBe("degraded");
    vi.useRealTimers();
  });
});
