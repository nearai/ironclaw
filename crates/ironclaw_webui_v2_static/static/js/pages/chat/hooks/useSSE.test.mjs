import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function useSSESourceForTest() {
  const source = readFileSync(new URL("./useSSE.js", import.meta.url), "utf8");
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

class FakeEventSource {
  constructor() {
    this.closeCalls = 0;
    this.listeners = new Map();
    this.onerror = null;
    this.onmessage = null;
    this.onopen = null;
  }

  addEventListener(name, listener) {
    const listeners = this.listeners.get(name) || [];
    listeners.push(listener);
    this.listeners.set(name, listeners);
  }

  emit(name, frame, lastEventId = "") {
    const event = {
      data: JSON.stringify(frame),
      lastEventId,
    };
    for (const listener of this.listeners.get(name) || []) {
      listener(event);
    }
  }

  close() {
    this.closeCalls += 1;
  }
}

function instantiateUseSSE() {
  const sources = [];
  const events = [];
  const statuses = [];
  const timers = [];
  let cleanup = null;
  const context = {
    clearTimeout: (timer) => {
      timer.cancelled = true;
    },
    document: {
      visibilityState: "visible",
      addEventListener: () => {},
      removeEventListener: () => {},
    },
    globalThis: {},
    openEventStream: () => {
      const source = new FakeEventSource();
      sources.push(source);
      return source;
    },
    React: {
      useEffect: (effect) => {
        cleanup = effect();
      },
      useRef: (value) => ({ current: value }),
      useState: (initial) => {
        let value = typeof initial === "function" ? initial() : initial;
        return [
          value,
          (next) => {
            value = typeof next === "function" ? next(value) : next;
            statuses.push(value);
          },
        ];
      },
    },
    setTimeout: (callback, delay) => {
      const timer = { callback, delay, cancelled: false };
      timers.push(timer);
      return timer;
    },
  };
  vm.runInNewContext(useSSESourceForTest(), context);
  context.globalThis.__testExports.useSSE({
    enabled: true,
    threadId: "thread-missing",
    onEvent: (event) => events.push(event),
  });
  return {
    cleanup: () => cleanup?.(),
    events,
    source: sources[0],
    sources,
    statuses,
    timers,
  };
}

test("useSSE stops reconnecting after a terminal server error event", () => {
  const harness = instantiateUseSSE();

  harness.source.onopen();
  harness.source.emit("error", {
    type: "error",
    error: "not_found",
    kind: "not_found",
    retryable: false,
  });
  harness.source.onerror();

  assert.equal(harness.source.closeCalls, 1, "terminal error closes the stream once");
  assert.equal(harness.timers.length, 0, "terminal error must not schedule reconnect");
  assert.equal(harness.statuses.at(-1), "disconnected");
  assert.deepEqual(harness.events.map((event) => event.type), ["error"]);
});

test("useSSE cancels reconnect if EventSource.onerror runs before the terminal error listener", () => {
  const harness = instantiateUseSSE();

  harness.source.onopen();
  harness.source.onerror();
  harness.source.emit("error", {
    type: "error",
    error: "not_found",
    kind: "not_found",
    retryable: false,
  });

  assert.equal(harness.source.closeCalls, 1, "lifecycle error closes the stream");
  assert.equal(harness.timers.length, 1, "lifecycle error initially schedules reconnect");
  assert.equal(harness.timers[0].cancelled, true, "terminal frame cancels the scheduled reconnect");

  harness.timers[0].callback();
  assert.equal(harness.sources.length, 1, "cancelled terminal reconnect must not open a new stream");
  assert.deepEqual(harness.events.map((event) => event.type), ["error"]);
});

test("useSSE keeps reconnecting after retryable stream errors", () => {
  const harness = instantiateUseSSE();

  harness.source.onopen();
  harness.source.emit("error", {
    type: "error",
    error: "unavailable",
    kind: "replay_unavailable",
    retryable: true,
  });
  harness.source.onerror();

  assert.equal(harness.source.closeCalls, 1, "retryable error closes the failed stream");
  assert.equal(harness.timers.length, 1, "retryable error schedules reconnect");
  assert.equal(harness.statuses.at(-1), "disconnected");
  assert.deepEqual(harness.events.map((event) => event.type), ["error"]);
});
