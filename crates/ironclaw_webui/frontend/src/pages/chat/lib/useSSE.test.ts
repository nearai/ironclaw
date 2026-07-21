// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

import { CONNECTION_STATUS } from "./connection-status";

function useSSESourceForTest() {
  const source = readFileSync(new URL("../hooks/useSSE.ts", import.meta.url), "utf8");
  const lines = [];
  let skippingImport = false;
  for (const line of source.split("\n")) {
    if (!skippingImport && line.startsWith("import ")) {
      skippingImport = !line.trimEnd().endsWith(";");
      continue;
    }
    if (skippingImport) {
      skippingImport = !line.trimEnd().endsWith(";");
      continue;
    }
    lines.push(line.replace("export function useSSE", "function useSSE"));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { useSSE };`;
}

function createHarness({
  online = true,
  visibilityState = "visible",
  onEvent = () => {},
} = {}) {
  const statuses = [];
  const streams = [];
  const timers = [];
  const documentListeners = new Map();
  const windowListeners = new Map();
  let cleanup = null;
  let refIndex = 0;
  const refs = [];

  function EventSource() {}
  EventSource.CONNECTING = 0;
  EventSource.OPEN = 1;
  EventSource.CLOSED = 2;

  const context = {
    CONNECTION_STATUS,
    authScope: () => "tenant:user",
    EventSource,
    JSON,
    Math,
    globalThis: {},
    openEventStream: (args) => {
      const listeners = new Map();
      const stream = {
        args,
        readyState: EventSource.CONNECTING,
        closeCalls: 0,
        onmessage: null,
        onopen: null,
        onerror: null,
        addEventListener: (name, handler) => listeners.set(name, handler),
        close() {
          this.closeCalls += 1;
          this.readyState = EventSource.CLOSED;
        },
        listener(name) {
          return listeners.get(name);
        },
      };
      streams.push(stream);
      return stream;
    },
    React: {
      useEffect: (effect) => {
        cleanup = effect();
      },
      useRef: (initial) => {
        const index = refIndex++;
        refs[index] ||= { current: initial };
        return refs[index];
      },
      useState: (initial) => [initial, (value) => statuses.push(value)],
    },
    document: {
      visibilityState,
      addEventListener: (name, handler) => documentListeners.set(name, handler),
      removeEventListener: (name) => documentListeners.delete(name),
    },
    navigator: { onLine: online },
    window: {
      addEventListener: (name, handler) => windowListeners.set(name, handler),
      removeEventListener: (name) => windowListeners.delete(name),
    },
    setTimeout: (handler, delay) => {
      const timer = { handler, delay };
      timers.push(timer);
      return timer;
    },
    clearTimeout: (timer) => {
      if (timer) timer.cleared = true;
    },
  };

  vm.runInNewContext(useSSESourceForTest(), context);
  let result = null;
  function render(threadId = "thread-1") {
    cleanup?.();
    cleanup = null;
    refIndex = 0;
    result = context.globalThis.__testExports.useSSE({
      threadId,
      enabled: true,
      onEvent,
    });
    return result;
  }
  function remount(threadId = "thread-1") {
    refs.length = 0;
    return render(threadId);
  }
  render();

  return {
    cleanup: () => cleanup?.(),
    context,
    documentListeners,
    get result() {
      return result;
    },
    render,
    remount,
    statuses,
    streams,
    timers,
    windowListeners,
  };
}

test("useSSE reflects browser offline and online events", () => {
  const { cleanup, context, statuses, streams, windowListeners } = createHarness();

  const stream = streams[0];
  stream.readyState = context.EventSource.OPEN;
  stream.onopen();
  windowListeners.get("offline")();
  assert.deepEqual(statuses, ["connecting", "connected", "reconnecting"]);

  windowListeners.get("online")();
  assert.deepEqual(statuses, [
    "connecting",
    "connected",
    "reconnecting",
    "connected",
  ]);

  cleanup();
  assert.equal(windowListeners.has("offline"), false);
  assert.equal(windowListeners.has("online"), false);
});

test("useSSE starts reconnecting when the browser is already offline", () => {
  const { statuses } = createHarness({ online: false });

  assert.deepEqual(statuses, ["reconnecting"]);
});

test("useSSE resumes delivery after returning to a hidden tab", () => {
  const events = [];
  const { context, documentListeners, statuses, streams, timers } = createHarness({
    onEvent: (event) => events.push(event),
  });

  const initial = streams[0];
  initial.readyState = context.EventSource.OPEN;
  initial.onopen();

  context.document.visibilityState = "hidden";
  documentListeners.get("visibilitychange")();
  assert.equal(initial.closeCalls, 1);

  context.document.visibilityState = "visible";
  documentListeners.get("visibilitychange")();
  const resumed = streams[1];
  resumed.readyState = context.EventSource.CONNECTING;
  resumed.onerror({});

  assert.equal(resumed.closeCalls, 0);
  assert.equal(timers.filter((timer) => timer.delay === 10_000).length, 1);

  resumed.readyState = context.EventSource.OPEN;
  resumed.listener("projection_update")({
    data: JSON.stringify({ type: "projection_update", state: { items: [] } }),
    lastEventId: "after-tab-resume",
  });

  assert.equal(streams.length, 2);
  assert.equal(events.length, 1);
  assert.deepEqual(statuses, [
    "connecting",
    "connected",
    "paused",
    "connecting",
    "reconnecting",
    "connected",
  ]);
});

test("useSSE lets EventSource recover transient failures natively", () => {
  const { context, statuses, streams, timers } = createHarness();

  assert.deepEqual(statuses, ["connecting"]);
  const stream = streams[0];
  stream.readyState = context.EventSource.CONNECTING;
  stream.onerror({});

  assert.deepEqual(statuses, ["connecting", "reconnecting"]);
  assert.equal(stream.closeCalls, 0);
  assert.equal(timers.length, 1);
  assert.equal(timers[0].delay, 10_000);

  stream.readyState = context.EventSource.OPEN;
  stream.listener("keep_alive")({
    data: JSON.stringify({ type: "keep_alive" }),
    lastEventId: "resume-cursor",
  });

  assert.equal(timers[0].cleared, true);
  assert.equal(streams.length, 1);
  assert.deepEqual(statuses, ["connecting", "reconnecting", "connected"]);
});

test("useSSE keeps one watchdog across repeated native errors", () => {
  const { context, statuses, streams, timers } = createHarness();

  const stream = streams[0];
  stream.readyState = context.EventSource.CONNECTING;
  stream.onerror({});
  stream.onerror({});

  assert.equal(timers.length, 1);
  assert.equal(timers[0].delay, 10_000);
  assert.equal(stream.closeCalls, 0);
  assert.deepEqual(statuses, ["connecting", "reconnecting", "reconnecting"]);
});

test("useSSE falls back to app reconnect timer for closed streams", () => {
  const { context, statuses, streams, timers } = createHarness();

  const stream = streams[0];
  stream.readyState = context.EventSource.CLOSED;
  stream.onerror({});

  assert.deepEqual(statuses, ["connecting", "disconnected"]);
  assert.equal(stream.closeCalls, 1);
  assert.equal(timers.length, 1);
  assert.equal(timers[0].delay, 2000);

  timers[0].handler();

  assert.equal(streams.length, 2);
  assert.deepEqual(statuses, ["connecting", "disconnected", "reconnecting"]);
});

test("useSSE replaces a reconnect attempt that never finishes opening", () => {
  const { context, statuses, streams, timers } = createHarness();

  const first = streams[0];
  first.readyState = context.EventSource.CONNECTING;
  first.onerror({});

  const nativeWatchdog = timers.find((timer) => timer.delay === 10_000);
  assert.ok(nativeWatchdog);
  nativeWatchdog.handler();

  const reconnectTimer = timers.find((timer) => timer.delay === 2000);
  assert.ok(reconnectTimer);
  reconnectTimer.handler();

  assert.equal(streams.length, 2);
  const stalledReplacement = streams[1];
  const openWatchdog = timers
    .filter((timer) => timer.delay === 10_000 && !timer.cleared)
    .at(-1);
  assert.ok(openWatchdog);

  openWatchdog.handler();

  assert.equal(stalledReplacement.closeCalls, 1);
  assert.equal(
    timers.filter((timer) => timer.delay === 4000 && !timer.cleared).length,
    1,
  );
  assert.deepEqual(statuses, [
    "connecting",
    "reconnecting",
    "reconnecting",
    "reconnecting",
    "reconnecting",
  ]);
});

test("useSSE does not reconnect after a non-retryable server error", () => {
  const events = [];
  const { context, statuses, streams, timers, windowListeners } = createHarness({
    onEvent: (event) => events.push(event),
  });

  const stream = streams[0];
  stream.listener("stream_error")({
    data: JSON.stringify({
      type: "stream_error",
      error: "not_found",
      kind: "not_found",
      retryable: false,
    }),
    lastEventId: "",
  });

  assert.equal(stream.closeCalls, 1);
  assert.equal(stream.readyState, context.EventSource.CLOSED);
  assert.equal(timers.length, 0);
  assert.deepEqual(statuses, ["connecting", "disconnected"]);
  assert.equal(events.length, 1);
  assert.equal(events[0].type, "error");
  assert.equal(events[0].frame.retryable, false);

  // Browsers report the server's subsequent close through `onerror`. It must
  // not override the terminal classification or schedule another connection.
  stream.onerror({});
  windowListeners.get("online")();
  assert.equal(timers.length, 0);
  assert.equal(streams.length, 1);
  assert.deepEqual(statuses, ["connecting", "disconnected"]);
});

test("useSSE resumes each thread from its own cursor after switching", () => {
  const { cleanup, render, streams } = createHarness();

  streams[0].listener("projection_update")({
    data: JSON.stringify({
      type: "projection_update",
      state: { items: [] },
    }),
    lastEventId: "thread-1-cursor",
  });

  render("thread-2");
  assert.equal(streams[1].args.threadId, "thread-2");
  assert.equal(streams[1].args.afterCursor, undefined);

  streams[1].listener("projection_update")({
    data: JSON.stringify({
      type: "projection_update",
      state: { items: [] },
    }),
    lastEventId: "thread-2-cursor",
  });

  render("thread-1");
  assert.equal(streams[2].args.threadId, "thread-1");
  assert.equal(streams[2].args.afterCursor, "thread-1-cursor");

  render("thread-2");
  assert.equal(streams[3].args.threadId, "thread-2");
  assert.equal(streams[3].args.afterCursor, "thread-2-cursor");

  cleanup();
});

test("useSSE keeps a thread cursor when the chat page remounts", () => {
  const { cleanup, remount, streams } = createHarness();

  streams[0].listener("projection_update")({
    data: JSON.stringify({
      type: "projection_update",
      state: { items: [] },
    }),
    lastEventId: "before-navigation",
  });

  remount("thread-1");
  assert.equal(streams[1].args.afterCursor, "before-navigation");

  cleanup();
});

test("useSSE ignores callbacks from a stream disposed by a thread switch", () => {
  const { cleanup, render, statuses, streams, timers } = createHarness();
  const disposedStream = streams[0];

  render("thread-2");
  assert.equal(disposedStream.closeCalls, 1);
  assert.equal(streams.length, 2);

  // A network error can already be queued when React cleans up the old
  // effect. It must not schedule an orphan reconnect for the old thread.
  disposedStream.onerror({});
  disposedStream.listener("stream_error")({
    data: JSON.stringify({
      error: "unavailable",
      kind: "replay_unavailable",
      retryable: true,
    }),
    lastEventId: "",
  });

  assert.equal(timers.length, 0);
  assert.equal(streams.length, 2);
  assert.deepEqual(statuses, ["connecting", "connecting"]);

  cleanup();
});

test("useSSE rebases from origin when the server rejects its replay cursor", () => {
  const events = [];
  const { statuses, streams, timers } = createHarness({
    onEvent: (event) => events.push(event),
  });

  const stream = streams[0];
  stream.listener("projection_update")({
    data: JSON.stringify({
      type: "projection_update",
      state: { items: [] },
    }),
    lastEventId: "stale-cursor",
  });
  stream.listener("stream_error")({
    data: JSON.stringify({
      error: "unavailable",
      kind: "replay_unavailable",
      retryable: true,
    }),
    lastEventId: "",
  });

  assert.equal(stream.closeCalls, 1);
  assert.deepEqual(statuses, ["connecting", "connected", "reconnecting"]);
  assert.equal(timers.length, 1);
  assert.equal(timers[0].delay, 2000);
  assert.equal(events.length, 2);
  assert.equal(events[1].type, "error");

  timers[0].handler();

  assert.equal(streams.length, 2);
  assert.equal(streams[1].args.afterCursor, undefined);
  assert.deepEqual(statuses, [
    "connecting",
    "connected",
    "reconnecting",
    "reconnecting",
  ]);
});

test("useSSE does not treat a legacy application error as a transport error", () => {
  const events = [];
  const { statuses, streams, timers } = createHarness({
    onEvent: (event) => events.push(event),
  });

  streams[0].onerror({
    data: JSON.stringify({
      error: "unavailable",
      kind: "replay_unavailable",
      retryable: true,
    }),
    lastEventId: "",
  });

  assert.equal(events.length, 1);
  assert.equal(events[0].type, "error");
  assert.deepEqual(statuses, ["connecting", "reconnecting"]);
  assert.equal(timers.length, 1);
  assert.equal(timers[0].delay, 2000);
});
