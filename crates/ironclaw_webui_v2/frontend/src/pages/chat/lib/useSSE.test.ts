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

function createHarness({ online = true, visibilityState = "visible" } = {}) {
  const statuses = [];
  const streams = [];
  const timers = [];
  const documentListeners = new Map();
  const windowListeners = new Map();
  let cleanup = null;

  function EventSource() {}
  EventSource.CONNECTING = 0;
  EventSource.OPEN = 1;
  EventSource.CLOSED = 2;

  const context = {
    CONNECTION_STATUS,
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
      useRef: (initial) => ({ current: initial }),
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
  const result = context.globalThis.__testExports.useSSE({
    threadId: "thread-1",
    enabled: true,
    onEvent: () => {},
  });

  return {
    cleanup,
    context,
    documentListeners,
    result,
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

test("useSSE lets EventSource handle transient reconnects", () => {
  const { context, statuses, streams, timers } = createHarness();

  assert.deepEqual(statuses, ["connecting"]);
  const stream = streams[0];
  stream.readyState = context.EventSource.CONNECTING;
  stream.onerror();

  assert.deepEqual(statuses, ["connecting", "reconnecting"]);
  assert.equal(stream.closeCalls, 0);
  assert.equal(timers.length, 1);

  stream.readyState = context.EventSource.OPEN;
  stream.onopen();
  assert.deepEqual(statuses, ["connecting", "reconnecting", "connected"]);
  assert.equal(timers[0].cleared, true);
  assert.equal(stream.closeCalls, 0);
  assert.equal(streams.length, 1);
});

test("useSSE replaces a native reconnect that stays connecting", () => {
  const { context, statuses, streams, timers } = createHarness();

  const stream = streams[0];
  stream.readyState = context.EventSource.CONNECTING;
  stream.onerror();

  assert.deepEqual(statuses, ["connecting", "reconnecting"]);
  assert.equal(timers.length, 1);
  assert.equal(timers[0].delay, 10000);

  timers[0].handler();

  assert.equal(stream.closeCalls, 1);
  assert.deepEqual(statuses, ["connecting", "reconnecting", "disconnected"]);
  assert.equal(timers.length, 2);
  assert.equal(timers[1].delay, 2000);

  timers[1].handler();

  assert.equal(streams.length, 2);
  assert.deepEqual(statuses, [
    "connecting",
    "reconnecting",
    "disconnected",
    "reconnecting",
  ]);
});

test("useSSE keeps the first watchdog deadline across repeated native errors", () => {
  const { context, statuses, streams, timers } = createHarness();

  const stream = streams[0];
  stream.readyState = context.EventSource.CONNECTING;
  stream.onerror();
  stream.onerror();

  assert.equal(timers.length, 1);
  assert.equal(timers[0].delay, 10000);
  assert.deepEqual(statuses, ["connecting", "reconnecting", "reconnecting"]);

  timers[0].handler();

  assert.equal(stream.closeCalls, 1);
  assert.equal(timers.length, 2);
  assert.equal(timers[1].delay, 2000);
});

test("useSSE clears reconnecting when the native stream has reopened", () => {
  const { context, statuses, streams, timers } = createHarness();

  const stream = streams[0];
  stream.readyState = context.EventSource.CONNECTING;
  stream.onerror();
  stream.readyState = context.EventSource.OPEN;

  timers[0].handler();

  assert.equal(stream.closeCalls, 0);
  assert.equal(streams.length, 1);
  assert.equal(timers.length, 1);
  assert.deepEqual(statuses, ["connecting", "reconnecting", "connected"]);
});

test("useSSE clears native reconnect watchdog on cleanup", () => {
  const { cleanup, context, streams, timers } = createHarness();

  const stream = streams[0];
  stream.readyState = context.EventSource.CONNECTING;
  stream.onerror();

  assert.equal(timers.length, 1);

  cleanup();

  assert.equal(timers[0].cleared, true);
  assert.equal(stream.closeCalls, 1);
});

test("useSSE falls back to app reconnect timer for closed streams", () => {
  const { context, statuses, streams, timers } = createHarness();

  const stream = streams[0];
  stream.readyState = context.EventSource.CLOSED;
  stream.onerror();

  assert.deepEqual(statuses, ["connecting", "disconnected"]);
  assert.equal(stream.closeCalls, 1);
  assert.equal(timers.length, 1);
  assert.equal(timers[0].delay, 2000);

  timers[0].handler();

  assert.equal(streams.length, 2);
  assert.deepEqual(statuses, ["connecting", "disconnected", "reconnecting"]);
});
