import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function useLogsSourceForTest() {
  const source = readFileSync(new URL("./useLogs.js", import.meta.url), "utf8");
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

function createHookHarness({ search = "" } = {}) {
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
    globalThis: {},
    normalizeOperatorLogsResponse: (response) => ({
      entries: response?.entries || response?.logs?.entries || [],
    }),
    queryOperatorLogs: async (request) => {
      calls.push(request);
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
      return context.globalThis.__testExports.useLogs();
    },
    async runEffects() {
      const effects = pendingEffects.splice(0);
      for (const effect of effects) {
        effect();
      }
      await Promise.resolve();
      await Promise.resolve();
    },
    setSearch(nextSearch) {
      location = { search: nextSearch };
    },
  };
}

test("useLogs reloads scoped logs once when scope changes while paused", async () => {
  const harness = createHookHarness({ search: "?thread_id=thread-a" });

  let result = harness.render();
  await harness.runEffects();
  assert.equal(harness.calls.length, 1);
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
  assert.equal(harness.calls[1].threadId, "thread-b");
  assert.equal(harness.intervals.length, 1);
});
