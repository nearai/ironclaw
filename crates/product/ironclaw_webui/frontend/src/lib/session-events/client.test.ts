// Protocol behavior of the session event client against a scripted
// `text/event-stream` opener: per-subscription cursors, stale-generation
// filtering, independent failure/rebase, reconnect-with-resume on the
// lifetime hint, and capped backoff without any fallback transport.

import assert from "node:assert/strict";
import { test } from "vitest";

import { SessionEventClient, type StreamOpener, type StreamResponse } from "./client";
import { SESSION_EVENT_SCHEMA } from "./protocol";

class ScriptedStream {
  readonly body: ReadableStream<Uint8Array>;
  readonly request: { url: string; headers: Record<string, string>; body: string };
  private controller: ReadableStreamDefaultController<Uint8Array> | null = null;
  private closed = false;

  constructor(request: ScriptedStream["request"]) {
    this.request = request;
    this.body = new ReadableStream<Uint8Array>({
      start: (controller) => {
        this.controller = controller;
      },
    });
  }

  get subscriptions(): Array<{ subscription_id: string; after_cursor: string | null }> {
    return JSON.parse(this.request.body).subscriptions;
  }

  frame(frame: Record<string, unknown>) {
    const text = `event: ${String(frame.type)}\ndata: ${JSON.stringify({
      schema: SESSION_EVENT_SCHEMA,
      ...frame,
    })}\n\n`;
    this.controller?.enqueue(new TextEncoder().encode(text));
  }

  comment() {
    this.controller?.enqueue(new TextEncoder().encode(": keep-alive\n\n"));
  }

  end() {
    if (this.closed) return;
    this.closed = true;
    this.controller?.close();
  }
}

(globalThis as Record<string, unknown>).window = {
  addEventListener: () => {},
  removeEventListener: () => {},
};

function harness(options: { status?: number; failOpen?: boolean } = {}) {
  const streams: ScriptedStream[] = [];
  let opens = 0;
  const openStream: StreamOpener = async (input) => {
    opens += 1;
    if (options.failOpen) throw new Error("connect refused");
    const stream = new ScriptedStream(input);
    streams.push(stream);
    input.signal.addEventListener("abort", () => stream.end());
    const response: StreamResponse = {
      status: options.status ?? 200,
      body: stream.body,
    };
    return response;
  };
  const client = new SessionEventClient(
    openStream,
    () => ({
      url: "/api/webchat/v2/session/events",
      headers: () => ({ Authorization: "Bearer test-token" }),
    }),
    0,
    0,
  );
  return { client, streams, openCount: () => opens };
}

async function settle() {
  // Debounce timers are zero-length in the harness; a few macrotask turns
  // let connect, read, and dispatch complete.
  for (let i = 0; i < 6; i += 1) {
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
}

test("opens one header-authenticated stream naming every selector with its own cursor", async () => {
  const { client, streams } = harness();
  const events: Array<{ id: string; cursor: string | null }> = [];
  client.subscribe(
    { kind: "thread", thread_id: "thread-a" },
    { onEvent: (event) => events.push({ id: "a", cursor: event.cursor }) },
    { fromCursor: '"cursor-a"', idPrefix: "chat" },
  );
  client.subscribe(
    { kind: "run_completions" },
    { onEvent: (event) => events.push({ id: "rc", cursor: event.cursor }) },
    { idPrefix: "rc" },
  );
  await settle();
  assert.equal(streams.length, 1, "both subscriptions coalesce into one stream");
  const stream = streams[0];
  assert.equal(stream.request.headers.Authorization, "Bearer test-token");
  assert.deepEqual(
    stream.subscriptions.map((entry) => entry.after_cursor),
    ['"cursor-a"', null],
  );
  const [chatId, rcId] = stream.subscriptions.map((entry) => entry.subscription_id);
  stream.frame({ type: "subscribed", subscription_id: chatId, generation: 1, cursor: '"cursor-a"' });
  stream.frame({ type: "subscribed", subscription_id: rcId, generation: 2 });
  stream.frame({
    type: "event",
    subscription_id: chatId,
    generation: 1,
    cursor: '"cursor-b"',
    event: { type: "message" },
  });
  stream.frame({
    type: "event",
    subscription_id: rcId,
    generation: 2,
    cursor: '"rc:7"',
    event: { type: "run_completion" },
  });
  await settle();
  assert.deepEqual(events, [
    { id: "a", cursor: '"cursor-b"' },
    { id: "rc", cursor: '"rc:7"' },
  ]);
  client.dispose();
});

test("frames from a stale generation never deliver", async () => {
  const { client, streams } = harness();
  const delivered: string[] = [];
  client.subscribe(
    { kind: "thread", thread_id: "thread-a" },
    { onEvent: (event) => delivered.push(String(event.body.type)) },
  );
  await settle();
  const stream = streams[0];
  const [id] = stream.subscriptions.map((entry) => entry.subscription_id);
  stream.frame({ type: "subscribed", subscription_id: id, generation: 4 });
  stream.frame({ type: "event", subscription_id: id, generation: 3, event: { type: "stale" } });
  stream.frame({ type: "event", subscription_id: id, generation: 4, event: { type: "fresh" } });
  await settle();
  assert.deepEqual(delivered, ["fresh"]);
  client.dispose();
});

test("a retryable subscription error reconnects from the last safe cursor without touching siblings", async () => {
  const { client, streams } = harness();
  const errors: string[] = [];
  const siblingEvents: string[] = [];
  client.subscribe(
    { kind: "thread", thread_id: "thread-a" },
    { onEvent: () => {}, onError: (error) => errors.push(error.error) },
    { fromCursor: '"c1"' },
  );
  client.subscribe(
    { kind: "run_completions" },
    { onEvent: (event) => siblingEvents.push(String(event.cursor)) },
  );
  await settle();
  const first = streams[0];
  const [chatId, rcId] = first.subscriptions.map((entry) => entry.subscription_id);
  first.frame({ type: "subscribed", subscription_id: chatId, generation: 1, cursor: '"c1"' });
  first.frame({ type: "subscribed", subscription_id: rcId, generation: 2 });
  first.frame({ type: "event", subscription_id: chatId, generation: 1, cursor: '"c2"', event: { type: "m" } });
  first.frame({
    type: "subscription_error",
    subscription_id: chatId,
    generation: 1,
    error: "unavailable",
    kind: "service_unavailable",
    retryable: true,
    last_cursor: '"c2"',
  });
  await settle();
  assert.deepEqual(errors, ["unavailable"]);
  assert.equal(streams.length, 2, "the client reconnects to resubscribe the failed selector");
  const second = streams[1];
  assert.deepEqual(
    second.subscriptions.map((entry) => entry.after_cursor),
    ['"c2"', null],
    "the failed selector resumes from its last safe cursor; the sibling is untouched",
  );
  client.dispose();
});

test("a non-retryable subscription error drops that subscription", async () => {
  const { client, streams } = harness();
  client.subscribe(
    { kind: "thread", thread_id: "thread-foreign" },
    { onEvent: () => {}, onError: () => {} },
  );
  client.subscribe({ kind: "run_completions" }, { onEvent: () => {} });
  await settle();
  const first = streams[0];
  const [chatId] = first.subscriptions.map((entry) => entry.subscription_id);
  first.frame({
    type: "subscription_error",
    subscription_id: chatId,
    generation: 1,
    error: "not_found",
    kind: "not_found",
    retryable: false,
  });
  await settle();
  assert.equal(streams.length, 1, "a dropped subscription causes no reconnect");
  first.frame({ type: "reconnect_hint", reason: "lifetime_expired" });
  first.end();
  await settle();
  assert.equal(streams.length, 2);
  assert.deepEqual(
    streams[1].subscriptions.map((entry) => entry.subscription_id.split("-")[0]),
    ["sub"],
    "only the surviving selector is named on the next connection",
  );
  client.dispose();
});

test("the lifetime reconnect hint resumes every selector from its own cursor", async () => {
  const { client, streams } = harness();
  const statuses: string[] = [];
  client.subscribe(
    { kind: "thread", thread_id: "thread-a" },
    { onEvent: () => {}, onStatus: (status) => statuses.push(status) },
  );
  await settle();
  const first = streams[0];
  const [id] = first.subscriptions.map((entry) => entry.subscription_id);
  first.frame({ type: "subscribed", subscription_id: id, generation: 1 });
  first.frame({ type: "event", subscription_id: id, generation: 1, cursor: '"c9"', event: { type: "m" } });
  first.frame({ type: "reconnect_hint", reason: "lifetime_expired" });
  first.end();
  await settle();
  assert.equal(streams.length, 2);
  assert.deepEqual(streams[1].subscriptions.map((entry) => entry.after_cursor), ['"c9"']);
  assert.ok(statuses.includes("open"));
  client.dispose();
});

test("connect failures back off and keep retrying; there is no fallback transport", async () => {
  const { client, openCount } = harness({ failOpen: true });
  const statuses: string[] = [];
  client.subscribe(
    { kind: "thread", thread_id: "thread-a" },
    { onEvent: () => {}, onStatus: (status) => statuses.push(status) },
  );
  await settle();
  assert.equal(openCount(), 1);
  assert.equal(client.currentStatus(), "reconnecting");
  assert.ok(!statuses.includes("degraded" as never));
  client.dispose();
});

test("changing the subscription set reconnects with the new set", async () => {
  const { client, streams } = harness();
  const first = client.subscribe({ kind: "thread", thread_id: "thread-a" }, { onEvent: () => {} });
  await settle();
  assert.equal(streams.length, 1);
  first.unsubscribe();
  client.subscribe({ kind: "thread", thread_id: "thread-b" }, { onEvent: () => {} });
  await settle();
  assert.equal(streams.length, 2, "unsubscribe + subscribe coalesce into one reconnect");
  assert.deepEqual(
    streams[1].subscriptions.map((entry) => JSON.stringify(entry)),
    [JSON.stringify({ subscription_id: streams[1].subscriptions[0].subscription_id, selector: { kind: "thread", thread_id: "thread-b" }, after_cursor: null })],
  );
  client.dispose();
});
