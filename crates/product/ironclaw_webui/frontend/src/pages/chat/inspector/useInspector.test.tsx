// @vitest-environment jsdom

import assert from "node:assert/strict";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, test, vi } from "vitest";

import { INSPECTOR_HEALTH } from "./inspector-state";
import { resetInspectorSessionStats } from "./inspector-session-stats";
import { useInspector } from "./useInspector";

const eventStreams = vi.hoisted(() => [] as any[]);

vi.mock("event-source-plus", () => ({
  EventSourcePlus: class EventSourcePlus {
    url: string;
    options: Record<string, unknown>;
    requestOptions: Record<string, any> = {};
    hooks: Record<string, Function> = {};
    listenCalls = 0;
    controller: { abort: ReturnType<typeof vi.fn>; reconnect: ReturnType<typeof vi.fn> };

    constructor(url: string, options: Record<string, unknown>) {
      this.url = url;
      this.options = options;
      this.controller = {
        abort: vi.fn(),
        reconnect: vi.fn(() => this.hooks.onRequest?.({ options: this.requestOptions })),
      };
      eventStreams.push(this);
    }

    listen(hooks: Record<string, Function>) {
      this.listenCalls += 1;
      this.hooks = hooks;
      hooks.onRequest?.({ options: this.requestOptions });
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

function Probe({ enabled = true }: { enabled?: boolean }) {
  latestState = useInspector({ enabled, threadId: "thread-a", runId: "run-a" });
  return <div data-health={latestState.health} />;
}

beforeEach(() => {
  resetInspectorSessionStats();
  eventStreams.length = 0;
  latestState = null;
  sessionStorage.clear();
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

test("records authoritative run snapshots in page-session statistics", async () => {
  Object.defineProperty(document, "visibilityState", {
    configurable: true,
    value: "hidden",
  });
  vi.stubGlobal(
    "fetch",
    vi.fn(async () =>
      new Response(JSON.stringify({
        snapshot: {
          stats: {
            total_model_calls: 2,
            total_tool_calls: 1,
            successful_tool_calls: 1,
            failed_tool_calls: 0,
            calls_per_model: [{
              model: { content: "model-a", original_bytes: 7, truncated: false },
              calls: 2,
            }],
            calls_per_model_truncated: false,
            input_tokens: { known_total: 20, unavailable_samples: 0 },
            output_tokens: { known_total: 5, unavailable_samples: 0 },
            cache_read_input_tokens: { known_total: 0, unavailable_samples: 2 },
            cache_creation_input_tokens: { known_total: 0, unavailable_samples: 2 },
            total_latency_ms: { known_total: 200, unavailable_samples: 0 },
          },
        },
      }), { status: 200, headers: { "content-type": "application/json" } }),
    ),
  );

  await act(async () => root?.render(<Probe />));

  assert.equal(latestState?.sessionStats?.total_model_calls, 2);
  assert.equal(latestState?.sessionStats?.total_tool_calls, 1);
  assert.equal(latestState?.sessionStats?.input_tokens.known_total, 20);
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
  assert.match(stream.requestOptions.query.connection_id, /^[A-Za-z0-9_-]{1,64}$/);
  assert.equal(stream.requestOptions.query.connection_generation, 1);

  await act(async () => stream.message("diagnostic_connected", "", {}));
  assert.equal(latestState?.health, INSPECTOR_HEALTH.CONNECTED);

  await act(async () => stream.respond());
  assert.equal(latestState?.health, INSPECTOR_HEALTH.CONNECTED);

  await act(async () => stream.hooks.onRequestError?.({}));
  assert.equal(latestState?.health, INSPECTOR_HEALTH.RECONNECTING);
  assert.equal((latestState?.updates[0].update as any)?.data?.kind, "stream_disconnected");

  await act(async () => stream.respond());
  assert.equal((latestState?.updates[1].update as any)?.data?.kind, "stream_resumed");

  await act(async () => stream.controller.reconnect());
  assert.equal(stream.requestOptions.query.connection_generation, 2);
  assert.equal(latestState?.reconnectCount, 1);
});

test("recovers a transient snapshot failure after the diagnostics stream connects", async () => {
  vi.useFakeTimers();
  const recoveredSnapshot = { prompt: { system: "Recovered prompt" } };
  vi.stubGlobal(
    "fetch",
    vi.fn()
      .mockResolvedValueOnce(new Response(JSON.stringify({ error: "temporarily_unavailable" }), {
        status: 503,
        headers: { "content-type": "application/json" },
      }))
      .mockResolvedValueOnce(new Response(JSON.stringify({ snapshot: recoveredSnapshot }), {
        status: 200,
        headers: { "content-type": "application/json" },
      })),
  );

  try {
    await act(async () => root?.render(<Probe />));
    assert.equal(vi.mocked(fetch).mock.calls.length, 1);
    assert.equal(latestState?.snapshot, null);

    const stream = eventStreams[0];
    await act(async () => stream.respond());
    assert.equal(latestState?.health, INSPECTOR_HEALTH.CONNECTED);

    await act(async () => vi.advanceTimersByTimeAsync(500));
    assert.equal(vi.mocked(fetch).mock.calls.length, 2);
    assert.deepEqual(latestState?.snapshot, recoveredSnapshot);
    assert.equal(latestState?.health, INSPECTOR_HEALTH.CONNECTED);
    assert.equal(latestState?.error, null);
  } finally {
    vi.useRealTimers();
  }
});

test("a live update's snapshot refresh does not downgrade the connected stream", async () => {
  vi.useFakeTimers();
  try {
    await act(async () => root?.render(<Probe />));
    const stream = eventStreams[0];
    await act(async () => stream.respond());
    assert.equal(latestState?.health, INSPECTOR_HEALTH.CONNECTED);

    const streamId = "550e8400-e29b-41d4-a716-446655440000";
    await act(async () => {
      stream.message("diagnostic_update", `${streamId}:1`, {
        update: { type: "stats" },
      });
    });
    // The settling `stats` update is the last one a completed run emits. Its
    // debounced snapshot refresh must stay a background read: reporting
    // LOADING/CONNECTING here left an open, healthy stream displayed as
    // "Connecting" indefinitely.
    await act(async () => vi.advanceTimersByTimeAsync(300));
    assert.equal(vi.mocked(fetch).mock.calls.length, 2);
    assert.equal(latestState?.health, INSPECTOR_HEALTH.CONNECTED);
  } finally {
    vi.useRealTimers();
  }
});

test("incomplete snapshot statistics stay unavailable instead of reading as zero", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn(async () =>
      new Response(JSON.stringify({ snapshot: { stats: {} } }), {
        status: 200,
        headers: { "content-type": "application/json" },
      }),
    ),
  );

  await act(async () => root?.render(<Probe />));

  assert.equal(latestState?.sessionStats, null);
});

test("bounds transient snapshot retries", async () => {
  vi.useFakeTimers();
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => new Response(JSON.stringify({ error: "temporarily_unavailable" }), {
      status: 503,
      headers: { "content-type": "application/json" },
    })),
  );

  try {
    await act(async () => root?.render(<Probe />));
    await act(async () => vi.advanceTimersByTimeAsync(10_000));
    assert.equal(vi.mocked(fetch).mock.calls.length, 3);
    assert.equal(latestState?.health, INSPECTOR_HEALTH.RECONNECTING);
  } finally {
    vi.useRealTimers();
  }
});

test("deduplicates cursors, bounds refresh bursts, rebases, and stops on forbidden", async () => {
  vi.useFakeTimers();
  try {
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
    assert.equal(latestState?.receivedUpdateCount, 2);
    assert.match(latestState?.lastUpdateAt || "", /^\d{4}-\d{2}-\d{2}T/);

    await act(async () => {
      stream.message("diagnostic_update", `${streamId}:3`, {
        update: { type: "prompt_updated" },
      });
      stream.message("diagnostic_update", `${streamId}:4`, {
        update: { type: "model_call", data: {} },
      });
      stream.message("diagnostic_update", `${streamId}:5`, {
        update: { type: "tool_execution_updated" },
      });
      stream.message("diagnostic_update", `${streamId}:6`, {
        update: { type: "stats" },
      });
    });
    assert.equal(vi.mocked(fetch).mock.calls.length, 1);
    await act(async () => vi.advanceTimersByTimeAsync(49));
    assert.equal(vi.mocked(fetch).mock.calls.length, 1);
    await act(async () => vi.advanceTimersByTimeAsync(1));
    assert.equal(vi.mocked(fetch).mock.calls.length, 2);
    assert.deepEqual(stream.options.headers(), {
      Authorization: "Bearer operator-token",
      "Last-Event-ID": `${streamId}:6`,
    });

    for (let sequence = 7; sequence <= 12; sequence += 1) {
      await act(async () => {
        stream.message("diagnostic_update", `${streamId}:${sequence}`, {
          update: { type: "stats" },
        });
        await vi.advanceTimersByTimeAsync(40);
      });
    }
    await act(async () => {
      stream.message("diagnostic_update", `${streamId}:13`, {
        update: { type: "stats" },
      });
    });
    assert.equal(vi.mocked(fetch).mock.calls.length, 2);
    await act(async () => vi.advanceTimersByTimeAsync(9));
    assert.equal(vi.mocked(fetch).mock.calls.length, 2);
    await act(async () => vi.advanceTimersByTimeAsync(1));
    assert.equal(vi.mocked(fetch).mock.calls.length, 3);
    assert.deepEqual(stream.options.headers(), {
      Authorization: "Bearer operator-token",
      "Last-Event-ID": `${streamId}:13`,
    });

    await act(async () => stream.hooks.onRequestError?.({}));
    await act(async () => stream.respond());
    const retainedLocalIds = latestState?.updates
      .filter((update) => update.local_id)
      .map((update) => update.local_id);
    assert.deepEqual(retainedLocalIds, ["transport-1", "transport-2"]);
    const receivedBeforeRebase = latestState?.receivedUpdateCount;

    await act(async () => {
      stream.message("diagnostic_rebase", `${streamId}:14`, {
        latest_cursor: { stream_id: streamId, sequence: 14 },
      });
    });
    assert.deepEqual(
      latestState?.updates.map((update) => update.local_id),
      retainedLocalIds,
    );
    assert.equal(latestState?.receivedUpdateCount, receivedBeforeRebase);
    assert.equal(vi.mocked(fetch).mock.calls.length, 3);
    await act(async () => vi.advanceTimersByTimeAsync(50));
    assert.equal(vi.mocked(fetch).mock.calls.length, 4);

    await act(async () => stream.respond(403, "application/json"));
    assert.equal(latestState?.health, INSPECTOR_HEALTH.FORBIDDEN);
    assert.equal(stream.controller.abort.mock.calls.length, 1);
  } finally {
    vi.useRealTimers();
  }
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

test("starts the diagnostics stream when an initially hidden tab becomes visible", async () => {
  Object.defineProperty(document, "visibilityState", {
    configurable: true,
    value: "hidden",
  });
  await act(async () => root?.render(<Probe />));

  const stream = eventStreams[0];
  assert.equal(stream.listenCalls, 0);

  Object.defineProperty(document, "visibilityState", {
    configurable: true,
    value: "visible",
  });
  await act(async () => document.dispatchEvent(new Event("visibilitychange")));
  assert.equal(stream.listenCalls, 1);
  assert.equal(stream.controller.reconnect.mock.calls.length, 0);

  await act(async () => stream.respond());
  assert.equal(latestState?.health, INSPECTOR_HEALTH.CONNECTED);
});

test("preserves forbidden terminal state across visibility changes", async () => {
  await act(async () => root?.render(<Probe />));
  const stream = eventStreams[0];
  await act(async () => stream.respond(403, "application/json"));
  assert.equal(latestState?.health, INSPECTOR_HEALTH.FORBIDDEN);

  Object.defineProperty(document, "visibilityState", {
    configurable: true,
    value: "hidden",
  });
  await act(async () => document.dispatchEvent(new Event("visibilitychange")));
  assert.equal(latestState?.health, INSPECTOR_HEALTH.FORBIDDEN);
  assert.equal(stream.controller.abort.mock.calls.length, 1);

  Object.defineProperty(document, "visibilityState", {
    configurable: true,
    value: "visible",
  });
  await act(async () => document.dispatchEvent(new Event("visibilitychange")));
  assert.equal(stream.listenCalls, 1);
  assert.equal(stream.controller.reconnect.mock.calls.length, 0);
  assert.equal(latestState?.health, INSPECTOR_HEALTH.FORBIDDEN);
});

test("disabling releases the diagnostics stream and reenabling starts a fresh one", async () => {
  await act(async () => root?.render(<Probe />));
  const firstStream = eventStreams[0];
  await act(async () => firstStream.respond());

  await act(async () => root?.render(<Probe enabled={false} />));
  assert.equal(firstStream.controller.abort.mock.calls.at(-1)?.[0], "inspector disposed");
  assert.equal(latestState?.health, INSPECTOR_HEALTH.IDLE);

  await act(async () => root?.render(<Probe />));
  assert.equal(eventStreams.length, 2);
  const secondStream = eventStreams[1];
  await act(async () => secondStream.respond());
  assert.equal(latestState?.health, INSPECTOR_HEALTH.CONNECTED);
});
