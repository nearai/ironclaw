// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

function useLogsSourceForTest() {
  const source = readFileSync(new URL("./useLogs.ts", import.meta.url), "utf8");
  const lines = [];
  for (const line of source.split("\n")) {
    if (line.startsWith("import ")) continue;
    lines.push(
      line
        .replace("export function readLogScopeFromLocation", "function readLogScopeFromLocation")
        .replace("export function useLogs", "function useLogs"),
    );
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { readLogScopeFromLocation, useLogs };`;
}

function depsChanged(previous, next) {
  if (!previous || !next || previous.length !== next.length) return true;
  return next.some((value, index) => !Object.is(value, previous[index]));
}

function createHookHarness({
  search = "",
  globalSearch = "",
  useLogsArgs = {},
  queryLogsImpl,
  queryOperatorLogsImpl,
} = {}) {
  const calls = [];
  const intervals = [];
  let location = { search };
  let hookIndex = 0;
  const hooks = [];
  const pendingEffects = [];

  const React = {
    useCallback(fn, deps) {
      const index = hookIndex++;
      const hook = hooks[index];
      if (!hook || depsChanged(hook.deps, deps)) {
        hooks[index] = { deps, value: fn };
      }
      return hooks[index].value;
    },
    useEffect(fn, deps) {
      const index = hookIndex++;
      const hook = hooks[index];
      if (!hook || depsChanged(hook.deps, deps)) {
        hooks[index] = { deps };
        pendingEffects.push(fn);
      }
    },
    useMemo(fn, deps) {
      const index = hookIndex++;
      const hook = hooks[index];
      if (!hook || depsChanged(hook.deps, deps)) {
        hooks[index] = { deps, value: fn() };
      }
      return hooks[index].value;
    },
    useRef(initial) {
      const index = hookIndex++;
      if (!hooks[index]) {
        hooks[index] = { current: initial };
      }
      return hooks[index];
    },
    useState(initial) {
      const index = hookIndex++;
      if (!hooks[index]) {
        hooks[index] = {
          value: typeof initial === "function" ? initial() : initial,
        };
      }
      const setValue = (next) => {
        hooks[index].value =
          typeof next === "function" ? next(hooks[index].value) : next;
      };
      return [hooks[index].value, setValue];
    },
  };

  const context = {
    React,
    clearInterval: () => {},
    globalThis: { location: { search: globalSearch } },
    normalizeOperatorLogsResponse: (response) => ({
      entries: response?.entries || response?.logs?.entries || [],
    }),
    queryLogs: async (request) => {
      calls.push({ endpoint: "logs", ...request });
      if (queryLogsImpl) return queryLogsImpl(request, calls.length);
      return { entries: [{ id: String(calls.length) }] };
    },
    queryOperatorLogs: async (request) => {
      calls.push({ endpoint: "operator", ...request });
      if (queryOperatorLogsImpl) return queryOperatorLogsImpl(request, calls.length);
      return { entries: [{ id: String(calls.length) }] };
    },
    setInterval: (fn, ms) => {
      intervals.push({ fn, ms });
      return intervals.length;
    },
    useLocation: () => location,
    URLSearchParams,
  };

  vm.runInNewContext(useLogsSourceForTest(), context);

  return {
    calls,
    intervals,
    render() {
      hookIndex = 0;
      pendingEffects.length = 0;
      return context.globalThis.__testExports.useLogs(useLogsArgs);
    },
    async runEffects() {
      const effects = pendingEffects.splice(0);
      for (const effect of effects) {
        effect();
      }
      for (let i = 0; i < 20; i += 1) {
        await Promise.resolve();
      }
    },
    setSearch(nextSearch) {
      location = { search: nextSearch };
    },
  };
}

test("useLogs reads deep-link scope from browser location when router search is initially empty", async () => {
  const harness = createHookHarness({
    search: "",
    globalSearch: "?thread_id=thread-deep-link&run_id=run-deep-link",
    useLogsArgs: { isAdmin: false },
  });

  const result = harness.render();
  await harness.runEffects();

  assert.equal(result.needsThreadScope, false);
  assert.equal(result.scope.threadId, "thread-deep-link");
  assert.equal(result.scope.runId, "run-deep-link");
  assert.equal(harness.calls.length, 1);
  assert.equal(harness.calls[0].endpoint, "logs");
  assert.equal(harness.calls[0].threadId, "thread-deep-link");
  assert.equal(harness.calls[0].runId, "run-deep-link");
});

test("useLogs reloads scoped logs once when scope changes while paused", async () => {
  const harness = createHookHarness({
    search: "?thread_id=thread-a",
    useLogsArgs: { isAdmin: true },
  });

  let result = harness.render();
  await harness.runEffects();
  assert.equal(harness.calls.length, 1);
  assert.equal(harness.calls[0].endpoint, "operator");
  assert.equal(harness.calls[0].threadId, "thread-a");
  assert.equal(harness.intervals.length, 1);

  result.togglePause();
  result = harness.render();
  await harness.runEffects();
  assert.equal(harness.calls.length, 1);

  harness.setSearch("?thread_id=thread-b");
  result = harness.render();
  await harness.runEffects();

  assert.equal(result.paused, true);
  assert.equal(harness.calls.length, 2);
  assert.equal(harness.calls[1].endpoint, "operator");
  assert.equal(harness.calls[1].threadId, "thread-b");
  assert.equal(harness.intervals.length, 1);
});

test("useLogs uses the non-operator endpoint when the caller is not admin", async () => {
  const harness = createHookHarness({
    search: "?thread_id=thread-a",
    useLogsArgs: { isAdmin: false },
  });

  harness.render();
  await harness.runEffects();

  assert.equal(harness.calls.length, 1);
  assert.equal(harness.calls[0].endpoint, "logs");
  assert.equal(harness.calls[0].threadId, "thread-a");
});

test("useLogs falls back to caller-scoped logs when operator logs reject privileges", async () => {
  const harness = createHookHarness({
    search: "?thread_id=thread-a",
    useLogsArgs: { isAdmin: true },
    queryOperatorLogsImpl: async () => {
      const error = new Error("Operator WebUI configuration privileges required");
      error.status = 403;
      throw error;
    },
    queryLogsImpl: async () => ({
      entries: [{ id: "fallback-entry", message: "caller scoped log" }],
    }),
  });

  const result = harness.render();
  await harness.runEffects();

  assert.equal(harness.calls.length, 2);
  assert.equal(harness.calls[0].endpoint, "operator");
  assert.equal(harness.calls[0].threadId, "thread-a");
  assert.equal(harness.calls[1].endpoint, "logs");
  assert.equal(harness.calls[1].threadId, "thread-a");
  assert.equal(result.status, "loading");

  const settled = harness.render();
  assert.equal(settled.error, null);
  assert.equal(settled.entries.length, 1);
  assert.equal(settled.entries[0].id, "fallback-entry");
});

test("useLogs surfaces the fallback error when caller-scoped logs also fail", async () => {
  const harness = createHookHarness({
    search: "?thread_id=thread-a",
    useLogsArgs: { isAdmin: true },
    queryOperatorLogsImpl: async () => {
      const error = new Error("Operator WebUI configuration privileges required");
      error.status = 403;
      throw error;
    },
    queryLogsImpl: async () => {
      const error = new Error("caller logs unavailable");
      error.status = 503;
      throw error;
    },
  });

  harness.render();
  await harness.runEffects();
  const result = harness.render();

  assert.equal(harness.calls.length, 2);
  assert.equal(harness.calls[0].endpoint, "operator");
  assert.equal(harness.calls[1].endpoint, "logs");
  assert.equal(result.status, "error");
  assert.equal(result.error.message, "caller logs unavailable");
});

test("useLogs falls back to the caller's active thread without exposing a clearable scope chip", async () => {
  const harness = createHookHarness({
    useLogsArgs: { isAdmin: false, defaultThreadId: "thread-fallback" },
  });

  const result = harness.render();
  await harness.runEffects();

  assert.equal(result.scope.threadId, "thread-fallback");
  assert.equal(result.scope.active.length, 0);
  assert.equal(harness.calls.length, 1);
  assert.equal(harness.calls[0].endpoint, "logs");
  assert.equal(harness.calls[0].threadId, "thread-fallback");
  assert.equal(result.scope.threadId, "thread-fallback");
  assert.equal(result.scope.active.length, 0);
});

test("useLogs clears stale entries and ignores prior in-flight loads when scope changes", async () => {
  const pending = [];
  const harness = createHookHarness({
    search: "?thread_id=thread-a",
    useLogsArgs: { isAdmin: false },
    queryLogsImpl: async (request) => new Promise((resolve) => {
      pending.push({ request, resolve });
    }),
  });

  harness.render();
  await harness.runEffects();
  assert.equal(pending.length, 1);
  pending[0].resolve({ entries: [{ id: "thread-a-entry" }] });
  await harness.runEffects();
  let result = harness.render();
  assert.equal(result.entries.length, 1);
  assert.equal(result.entries[0].id, "thread-a-entry");

  harness.intervals[0].fn();
  await Promise.resolve();
  assert.equal(pending.length, 2);
  assert.equal(pending[1].request.threadId, "thread-a");

  harness.setSearch("?thread_id=thread-b");
  result = harness.render();
  await harness.runEffects();
  result = harness.render();
  assert.equal(result.entries.length, 0);
  assert.equal(pending.length, 3);
  assert.equal(pending[2].request.threadId, "thread-b");

  pending[1].resolve({ entries: [{ id: "late-thread-a-entry" }] });
  await harness.runEffects();
  result = harness.render();
  assert.equal(result.entries.length, 0);

  pending[2].resolve({ entries: [{ id: "thread-b-entry" }] });
  await harness.runEffects();
  result = harness.render();
  assert.equal(result.entries.length, 1);
  assert.equal(result.entries[0].id, "thread-b-entry");
});

test("useLogs does not poll public logs until a thread scope is available", async () => {
  const harness = createHookHarness({
    useLogsArgs: { isAdmin: false },
  });

  let result = harness.render();
  await harness.runEffects();

  assert.equal(result.needsThreadScope, true);
  assert.equal(result.status, "needs_scope");
  assert.equal(harness.calls.length, 0);
  assert.equal(harness.intervals.length, 0);

  harness.setSearch("?thread_id=thread-a");
  result = harness.render();
  await harness.runEffects();

  assert.equal(result.needsThreadScope, false);
  assert.equal(harness.calls.length, 1);
  assert.equal(harness.calls[0].endpoint, "logs");
  assert.equal(harness.calls[0].threadId, "thread-a");
});

test("useLogs recovers public logs after a stale scoped thread returns 404", async () => {
  const harness = createHookHarness({
    search: "?thread_id=thread-stale",
    useLogsArgs: { isAdmin: false },
    queryLogsImpl: async (request, callCount) => {
      if (request.threadId === "thread-stale") {
        const error = new Error("not found");
        error.status = 404;
        throw error;
      }
      return { entries: [{ id: String(callCount) }] };
    },
  });

  harness.render();
  await harness.runEffects();

  assert.equal(harness.calls.length, 1);
  assert.equal(harness.calls[0].threadId, "thread-stale");

  harness.setSearch("?thread_id=thread-good");
  const result = harness.render();
  await harness.runEffects();

  assert.equal(result.needsThreadScope, false);
  assert.equal(harness.calls.length, 2);
  assert.equal(harness.calls[1].endpoint, "logs");
  assert.equal(harness.calls[1].threadId, "thread-good");
});
