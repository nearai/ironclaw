// @vitest-environment jsdom

import assert from "node:assert/strict";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, test, vi } from "vitest";

import { INSPECTOR_HEALTH } from "./inspector-state";
import { useInspector } from "./useInspector";

const eventStreams = vi.hoisted(() => [] as any[]);

vi.mock("event-source-plus", () => ({
  EventSourcePlus: class EventSourcePlus {
    url: string;
    options: Record<string, unknown>;
    hooks: Record<string, Function> = {};
    controller: { abort: ReturnType<typeof vi.fn>; reconnect: ReturnType<typeof vi.fn> };

    constructor(url: string, options: Record<string, unknown>) {
      this.url = url;
      this.options = options;
      this.controller = {
        abort: vi.fn(),
        reconnect: vi.fn(() => this.hooks.onRequest?.({})),
      };
      eventStreams.push(this);
    }

    listen(hooks: Record<string, Function>) {
      this.hooks = hooks;
      hooks.onRequest?.({});
      return this.controller;
    }

    respond(status = 200, contentType = "text/event-stream") {
      const response = new Response("", {
        status,
        headers: { "content-type": contentType },
      });
      if (status >= 200 && status < 300) this.hooks.onResponse?.({ response });
      else this.hooks.onResponseError?.({ response });
    }

    message(event: string, id: string, payload: Record<string, unknown>) {
      this.hooks.onMessage?.({ event, id, data: JSON.stringify(payload) });
    }
  },
}));

let latestState: ReturnType<typeof useInspector> | null = null;
let root: ReturnType<typeof createRoot> | null = null;

function Probe() {
  latestState = useInspector({ enabled: true, threadId: "thread-a", runId: "run-a" });
  return <div data-health={latestState.health} />;
}

beforeEach(() => {
  eventStreams.length = 0;
  latestState = null;
  sessionStorage.setItem("ironclaw_token", "operator-token");
  vi.stubGlobal(
    "fetch",
    vi.fn(async () =>
      new Response(JSON.stringify({ snapshot: null }), {
        status: 200,
        headers: { "content-type": "application/json" },
      }),
    ),
  );
  Object.defineProperty(document, "visibilityState", {
    configurable: true,
    value: "visible",
  });
  root = createRoot(document.body.appendChild(document.createElement("div")));
});

afterEach(async () => {
  await act(async () => root?.unmount());
  document.body.replaceChildren();
  vi.unstubAllGlobals();
});

test("loads a scoped snapshot and configures bounded authenticated reconnects", async () => {
  await act(async () => root?.render(<Probe />));

  const fetchMock = vi.mocked(fetch);
  assert.equal(fetchMock.mock.calls.length, 1);
  assert.match(
    String(fetchMock.mock.calls[0][0]),
    /operator\/inspector\/threads\/thread-a\/runs\/run-a$/,
  );
  const stream = eventStreams[0];
  assert.equal(stream.options.maxRetryInterval, 30_000);
  assert.equal(stream.options.retryStrategy, "always");
  assert.deepEqual(stream.options.headers(), {
    Authorization: "Bearer operator-token",
  });
  assert.equal(new URL(stream.url).searchParams.has("token"), false);

  await act(async () => stream.respond());
  assert.equal(latestState?.health, INSPECTOR_HEALTH.CONNECTED);

  await act(async () => stream.hooks.onRequestError?.({}));
  assert.equal(latestState?.health, INSPECTOR_HEALTH.RECONNECTING);
  assert.equal((latestState?.updates[0].update as any)?.data?.kind, "stream_disconnected");

  await act(async () => stream.respond());
  assert.equal((latestState?.updates[1].update as any)?.data?.kind, "stream_resumed");
});

test("deduplicates cursors, rebases snapshots, and stops on forbidden", async () => {
  await act(async () => root?.render(<Probe />));
  const stream = eventStreams[0];
  await act(async () => stream.respond());

  const streamId = "550e8400-e29b-41d4-a716-446655440000";
  await act(async () => {
    stream.message("diagnostic_update", `${streamId}:1`, { sequence: 1 });
    stream.message("diagnostic_update", `${streamId}:1`, { sequence: 1 });
    stream.message("diagnostic_update", `${streamId}:2`, { sequence: 2 });
  });
  assert.equal(latestState?.updates.length, 2);
  assert.equal(latestState?.lastCursor, `${streamId}:2`);

  await act(async () => {
    stream.message("diagnostic_update", `${streamId}:3`, {
      update: { type: "prompt_updated" },
    });
  });
  assert.equal(vi.mocked(fetch).mock.calls.length, 2);

  await act(async () => {
    stream.message("diagnostic_update", `${streamId}:4`, {
      update: { type: "model_call", data: {} },
    });
  });
  assert.equal(vi.mocked(fetch).mock.calls.length, 3);

  await act(async () => {
    stream.message("diagnostic_rebase", `${streamId}:5`, {
      latest_cursor: { stream_id: streamId, sequence: 5 },
    });
  });
  assert.equal(latestState?.updates.length, 0);
  assert.equal(vi.mocked(fetch).mock.calls.length, 4);

  await act(async () => stream.respond(403, "application/json"));
  assert.equal(latestState?.health, INSPECTOR_HEALTH.FORBIDDEN);
  assert.equal(stream.controller.abort.mock.calls.length, 1);
});

test("hidden tabs release the stream and reconnect when visible", async () => {
  await act(async () => root?.render(<Probe />));
  const stream = eventStreams[0];
  await act(async () => stream.respond());

  Object.defineProperty(document, "visibilityState", {
    configurable: true,
    value: "hidden",
  });
  await act(async () => document.dispatchEvent(new Event("visibilitychange")));
  assert.equal(stream.controller.abort.mock.calls.at(-1)?.[0], "inspector hidden");

  Object.defineProperty(document, "visibilityState", {
    configurable: true,
    value: "visible",
  });
  await act(async () => document.dispatchEvent(new Event("visibilitychange")));
  assert.equal(stream.controller.reconnect.mock.calls.length, 1);
});
