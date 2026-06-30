// Run with:
//   node --test crates/ironclaw_webui_v2_static/static/js/hooks/useNotifications.test.mjs

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function sourceForTest() {
  const source = readFileSync(new URL("./useNotifications.js", import.meta.url), "utf8");
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
    lines.push(line.replace(/^export function /, "function "));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { useNotifications };`;
}

function depsEqual(left, right) {
  if (!Array.isArray(left) || !Array.isArray(right) || left.length !== right.length) {
    return false;
  }
  return left.every((item, index) => Object.is(item, right[index]));
}

function createReactStub() {
  const slots = [];
  let cursor = 0;
  let pendingRender = false;
  return {
    beginRender: () => {
      cursor = 0;
      pendingRender = false;
    },
    didScheduleUpdate: () => pendingRender,
    useCallback: (fn, deps) => {
      const index = cursor++;
      const slot = slots[index];
      if (slot && depsEqual(slot.deps, deps)) return slot.value;
      slots[index] = { deps, value: fn };
      return fn;
    },
    useEffect: (fn, deps) => {
      const index = cursor++;
      const slot = slots[index];
      if (slot && depsEqual(slot.deps, deps)) return;
      slots[index] = { deps };
      fn();
    },
    useMemo: (fn, deps) => {
      const index = cursor++;
      const slot = slots[index];
      if (slot && depsEqual(slot.deps, deps)) return slot.value;
      const value = fn();
      slots[index] = { deps, value };
      return value;
    },
    useState: (initial) => {
      const index = cursor++;
      if (!slots[index]) {
        slots[index] = {
          value: typeof initial === "function" ? initial() : initial,
        };
      }
      return [
        slots[index].value,
        (next) => {
          const value =
            typeof next === "function" ? next(slots[index].value) : next;
          if (!Object.is(value, slots[index].value)) {
            slots[index].value = value;
            pendingRender = true;
          }
        },
      ];
    },
  };
}

function instantiate(queryState, options = {}) {
  const baselineCalls = [];
  const getStateScopes = [];
  const markSeenCalls = [];
  const subscribeCalls = [];
  const react = createReactStub();
  const translate = (key) => key;
  const messages = options.messages || [{ id: "message-1" }];
  const context = {
    AUTOMATIONS_BASE_REFETCH_MS: 30_000,
    React: react,
    useI18n: () => ({ t: translate, lang: "en" }),
    useQuery: () => queryState,
    listAutomations: async () => ({}),
    normalizeAutomations: () => [],
    automationRunNotifications: () => messages,
    ensureNotificationBaseline: (ids, scope) => {
      baselineCalls.push({ ids, scope });
      return { initialized: true, seenIds: new Set(ids) };
    },
    getNotificationState: (scope) => {
      getStateScopes.push(scope);
      return { initialized: false, seenIds: new Set() };
    },
    markNotificationIdsSeen: (ids, scope) => {
      markSeenCalls.push({ ids, scope });
      return { initialized: true, seenIds: new Set(ids) };
    },
    subscribeNotifications: (listener) => {
      subscribeCalls.push(listener);
      return () => {};
    },
    globalThis: {},
  };
  vm.runInNewContext(sourceForTest(), context);
  const render = () => {
    let hook;
    for (let attempt = 0; attempt < 5; attempt += 1) {
      react.beginRender();
      hook = context.globalThis.__testExports.useNotifications({
        profile: { tenant_id: "tenant", user_id: "user" },
      });
      if (!react.didScheduleUpdate()) break;
    }
    return hook;
  };
  const hook = render();
  return { hook, render, baselineCalls, getStateScopes, markSeenCalls, subscribeCalls };
}

test("does not baseline notifications before the automations query succeeds", () => {
  const { baselineCalls } = instantiate({
    data: undefined,
    isLoading: true,
    isSuccess: false,
    error: null,
    refetch: () => {},
  });

  assert.deepEqual(baselineCalls, []);
});

test("baselines the current notification ids after the first successful query", () => {
  const { baselineCalls, hook } = instantiate({
    data: { automations: [] },
    isLoading: false,
    isSuccess: true,
    error: null,
    refetch: () => {},
  });

  assert.deepEqual(baselineCalls, [
    { ids: ["message-1"], scope: "tenant:user" },
  ]);
  assert.equal(hook.unreadCount, 0);
});

test("uses the profile scope for notification persistence", () => {
  const { baselineCalls, getStateScopes, hook, markSeenCalls } = instantiate({
    data: { automations: [] },
    isLoading: false,
    isSuccess: true,
    error: null,
    refetch: () => {},
  });

  assert(getStateScopes.includes("tenant:user"));
  assert.deepEqual(baselineCalls, [
    { ids: ["message-1"], scope: "tenant:user" },
  ]);

  hook.markAllRead();
  assert.deepEqual(markSeenCalls, [
    { ids: ["message-1"], scope: "tenant:user" },
  ]);
});
