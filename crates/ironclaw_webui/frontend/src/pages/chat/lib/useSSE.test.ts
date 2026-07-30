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
  activityExpected = false,
} = {}) {
  const statuses = [];
  const streams = [];
  const timers = [];
  const documentListeners = new Map();
  const windowListeners = new Map();
  let refIndex = 0;
  let effectIndex = 0;
  let currentThreadId = "thread-1";
  let currentActivityExpected = activityExpected;
  const refs = [];
  const effects = [];

  class EventSourcePlus {
    constructor(url, options) {
      this.url = url;
      this.options = options;
      this.lastEventId = undefined;
      this.requestOptions = {};
      streams.push(this);
    }

    listen(hooks) {
      this.hooks = hooks;
      const stream = this;
      this.controller = {
        abortCalls: [],
        reconnectCalls: 0,
        abort(reason) {
          this.abortCalls.push(reason);
        },
        reconnect() {
          this.reconnectCalls += 1;
          stream.request();
        },
      };
      this.request();
      return this.controller;
    }

    request() {
      this.hooks.onRequest?.({ options: this.requestOptions });
    }

    respond(status = 200, contentType = "text/event-stream") {
      const response = {
        ok: status >= 200 && status < 300,
        status,
        headers: new Headers({ "content-type": contentType }),
      };
      if (response.ok) {
        this.hooks.onResponse?.({ response });
        if (!contentType.includes("text/event-stream")) {
          this.hooks.onResponseError?.({ response });
        }
      } else {
        this.hooks.onResponseError?.({ response });
      }
    }

    message(frame, { event = frame.type, id = "" } = {}) {
      if (id) this.lastEventId = id;
      this.hooks.onMessage?.({
        data: JSON.stringify(frame),
        event,
        id,
      });
    }
  }

  const context = {
    CONNECTION_STATUS,
    clientActionId: () => "browser-tab-connection",
    EventSourcePlus,
    eventStreamRequest: ({ threadId, connectionId }) => ({
      url: `http://localhost/events/${threadId}?connection_id=${connectionId}`,
      headers: () => ({ Authorization: "Bearer token-1" }),
    }),
    globalThis: {},
    Headers,
    JSON,
    Math,
    React: {
      useEffect: (effect, dependencies) => {
        const index = effectIndex++;
        const previous = effects[index];
        const changed =
          !previous ||
          !dependencies ||
          !previous.dependencies ||
          dependencies.length !== previous.dependencies.length ||
          dependencies.some(
            (dependency, dependencyIndex) =>
              !Object.is(dependency, previous.dependencies[dependencyIndex]),
          );
        if (!changed) return;
        previous?.cleanup?.();
        const cleanup = effect();
        effects[index] = {
          dependencies: dependencies ? [...dependencies] : null,
          cleanup: typeof cleanup === "function" ? cleanup : null,
        };
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
  function cleanupEffects() {
    for (const effect of effects.splice(0)) effect?.cleanup?.();
  }
  function render(
    threadId = currentThreadId,
    nextActivityExpected = currentActivityExpected,
  ) {
    currentThreadId = threadId;
    currentActivityExpected = nextActivityExpected;
    refIndex = 0;
    effectIndex = 0;
    result = context.globalThis.__testExports.useSSE({
      threadId,
      enabled: true,
      onEvent,
      activityExpected: currentActivityExpected,
    });
    return result;
  }
  function remount(threadId = "thread-1") {
    cleanupEffects();
    refs.length = 0;
    return render(threadId);
  }
  render();

  return {
    cleanup: cleanupEffects,
    context,
    documentListeners,
    get result() {
      return result;
    },
    render,
    renderActivityExpected: (nextActivityExpected) =>
      render(currentThreadId, nextActivityExpected),
    remount,
    statuses,
    streams,
    timers,
    windowListeners,
  };
}

test("useSSE delegates framing, credentials, and retries to EventSourcePlus", () => {
  const { streams, statuses } = createHarness();
  const stream = streams[0];

  assert.equal(
    stream.url,
    "http://localhost/events/thread-1?connection_id=browser-tab-connection",
  );
  assert.deepEqual(stream.options.headers(), { Authorization: "Bearer token-1" });
  assert.equal(new URL(stream.url).searchParams.has("token"), false);
  assert.equal(stream.options.credentials, "same-origin");
  assert.equal(stream.options.retryStrategy, "always");
  assert.equal(stream.options.maxRetryInterval, 30_000);
  assert.equal(typeof stream.requestOptions.query.connection_generation, "number");

  stream.respond();
  assert.deepEqual(statuses, ["connecting", "connecting", "connected"]);

  stream.hooks.onRequestError?.({});
  assert.equal(statuses.at(-1), "reconnecting");
});

test("useSSE dispatches typed frames from the packaged parser", () => {
  const events = [];
  const { streams } = createHarness({ onEvent: (event) => events.push(event) });
  const stream = streams[0];

  stream.message(
    { type: "projection_update", state: { items: [] } },
    { id: "cursor-1" },
  );
  stream.message(
    { state: { items: [] } },
    { event: "projection_snapshot", id: "cursor-2" },
  );

  assert.equal(
    JSON.stringify(events),
    JSON.stringify([
      {
        type: "projection_update",
        frame: { type: "projection_update", state: { items: [] } },
        lastEventId: "cursor-1",
      },
      {
        type: "projection_snapshot",
        frame: { state: { items: [] } },
        lastEventId: "cursor-2",
      },
    ]),
  );
});

test("useSSE reconnects an active run when an open stream stops delivering", () => {
  const { renderActivityExpected, streams, timers } = createHarness();
  const stream = streams[0];
  stream.respond();

  renderActivityExpected(true);
  const watchdog = timers.find(
    (timer) => timer.delay === 30_000 && !timer.cleared,
  );
  assert.ok(watchdog);

  watchdog.handler();
  assert.equal(stream.controller.reconnectCalls, 1);
  assert.equal(typeof stream.requestOptions.query.connection_generation, "number");
});

test("useSSE arms the watchdog when an already-active stream opens", () => {
  const { streams, timers } = createHarness({ activityExpected: true });
  streams[0].respond();

  assert.ok(
    timers.some((timer) => timer.delay === 30_000 && !timer.cleared),
    "an active run must be watched even when no activity transition occurs after connect",
  );
});

test("useSSE rejects a successful response that is not an event stream", () => {
  const { statuses, streams } = createHarness();
  streams[0].respond(200, "text/html");

  assert.notEqual(statuses.at(-1), CONNECTION_STATUS.CONNECTED);
});

test("useSSE pauses while hidden and reconnects when visible", () => {
  const { context, documentListeners, statuses, streams } = createHarness();
  const stream = streams[0];
  stream.respond();

  context.document.visibilityState = "hidden";
  documentListeners.get("visibilitychange")();
  assert.deepEqual(stream.controller.abortCalls, ["document hidden"]);
  assert.equal(statuses.at(-1), "paused");

  context.document.visibilityState = "visible";
  documentListeners.get("visibilitychange")();
  assert.equal(stream.controller.reconnectCalls, 1);
  assert.equal(statuses.at(-2), "connecting");
  assert.equal(statuses.at(-1), "reconnecting");
});

test("useSSE lets retryable responses retry and stops on terminal responses", () => {
  const retryable = createHarness();
  retryable.streams[0].respond(204, "");
  assert.equal(retryable.statuses.at(-1), "reconnecting");
  assert.equal(retryable.streams[0].controller.abortCalls.length, 0);
  retryable.cleanup();

  const terminal = createHarness();
  terminal.streams[0].respond(403, "application/json");
  assert.equal(terminal.statuses.at(-1), "disconnected");
  assert.deepEqual(terminal.streams[0].controller.abortCalls, [
    "non-retryable stream response",
  ]);
  terminal.cleanup();
});

test("useSSE stops after a non-retryable stream event", () => {
  const events = [];
  const { statuses, streams } = createHarness({
    onEvent: (event) => events.push(event),
  });
  const stream = streams[0];

  stream.message({
    type: "stream_error",
    kind: "not_found",
    retryable: false,
  });

  assert.equal(events[0].type, "error");
  assert.equal(statuses.at(-1), "disconnected");
  assert.deepEqual(stream.controller.abortCalls, [
    "non-retryable stream event",
  ]);
});

test("useSSE clears packaged replay state before rebasing from origin", () => {
  const events = [];
  const { statuses, streams } = createHarness({
    onEvent: (event) => events.push(event),
  });
  const stream = streams[0];
  stream.message(
    { type: "projection_update", state: { items: [] } },
    { id: "stale-cursor" },
  );
  assert.equal(stream.lastEventId, "stale-cursor");

  stream.message({
    type: "stream_error",
    kind: "replay_unavailable",
    retryable: true,
  });

  assert.equal(stream.lastEventId, undefined);
  assert.equal(stream.controller.reconnectCalls, 1);
  assert.equal(statuses.at(-1), "reconnecting");
  assert.equal(events.at(-1).type, "error");
});

test("useSSE starts each route from a fresh stream and ignores disposed callbacks", () => {
  const events = [];
  const { cleanup, render, streams } = createHarness({
    onEvent: (event) => events.push(event),
  });
  const first = streams[0];
  first.message(
    { type: "projection_update", state: { items: [] } },
    { id: "thread-1-cursor" },
  );

  render("thread-2");
  const second = streams[1];
  assert.equal(first.controller.abortCalls.at(-1), "component disposed");
  assert.equal(second.lastEventId, undefined);
  assert.match(second.url, /thread-2/);
  assert.equal(
    new URL(first.url).searchParams.get("connection_id"),
    new URL(second.url).searchParams.get("connection_id"),
  );
  assert.ok(
    second.requestOptions.query.connection_generation >
      first.requestOptions.query.connection_generation,
  );

  first.message({ type: "projection_update", state: { items: ["late"] } });
  assert.equal(events.length, 1);
  cleanup();
});
