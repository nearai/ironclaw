// @ts-nocheck
// Run with:
//   pnpm test -- hooks/useNotifications.test.ts

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

function sourceForTest() {
  const source = readFileSync(new URL("./useNotifications.ts", import.meta.url), "utf8");
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
  const getStateScopes = [];
  const markSeenCalls = [];
  const listThreadCalls = [];
  const subscribeCalls = [];
  const notificationInputs = [];
  let queryOptions = null;
  const react = createReactStub();
  const translate = (key) => key;
  let storedState = options.initialState || { initialized: true, seenIds: new Set() };
  const states = options.threadStates || new Map();
  const context = {
    React: react,
    useI18n: () => ({ t: translate, lang: "en" }),
    useQuery: (options) => {
      queryOptions = options;
      return queryState;
    },
    listThreads: async (request) => {
      listThreadCalls.push(request);
      return {};
    },
    THREAD_STATE: { NEEDS_ATTENTION: "needs_attention" },
    useThreadStates: () => states,
    approvalThreadNotifications: (threads, threadStates, t) => {
      notificationInputs.push({ threads, threadStates, t });
      return typeof options.approvalThreadNotifications === "function"
        ? options.approvalThreadNotifications(threads, threadStates, t)
        : threads.map((thread) => {
            const threadId = thread.id || thread.thread_id;
            return {
              id: `approval:${threadId}`,
              href: `/chat/${threadId}`,
            };
          });
    },
    getNotificationState: (scope) => {
      getStateScopes.push(scope);
      return storedState;
    },
    markNotificationIdsSeen: (ids, scope) => {
      markSeenCalls.push({ ids, scope });
      storedState = {
        initialized: true,
        seenIds: new Set([...storedState.seenIds, ...ids]),
      };
      return storedState;
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
        profile:
          options.profile === undefined
            ? { tenant_id: "tenant", user_id: "user" }
            : options.profile,
        activeThreadId: options.activeThreadId || null,
      });
      if (!react.didScheduleUpdate()) break;
    }
    return hook;
  };
  const hook = render();
  return {
    hook,
    render,
    get queryOptions() {
      return queryOptions;
    },
    getStateScopes,
    markSeenCalls,
    listThreadCalls,
    notificationInputs,
    subscribeCalls,
  };
}

function plainCalls(calls) {
  return calls.map((call) => ({ ids: [...call.ids], scope: call.scope }));
}

function plainThreadRequests(calls) {
  return calls.map((call) => ({
    limit: call.limit,
    needsApproval: call.needsApproval,
    candidateThreadId: call.candidateThreadId,
  }));
}

test("does not baseline approval notifications on first load", () => {
  const harness = instantiate({
    data: undefined,
    isLoading: true,
    isSuccess: false,
    error: null,
    refetch: () => {},
  });

  assert.deepEqual(harness.markSeenCalls, []);
  assert.equal(harness.hook.unreadCount, 0);
  assert.deepEqual(harness.listThreadCalls, []);
});

test("queries only threads that need approval", async () => {
  const harness = instantiate({
    data: { threads: [] },
    isLoading: false,
    isSuccess: true,
    error: null,
    refetch: () => {},
  });

  await harness.queryOptions.queryFn();
  assert.deepEqual(plainThreadRequests(harness.listThreadCalls), [
    { limit: 20, needsApproval: true, candidateThreadId: undefined },
  ]);
});

test("does not use active thread as an approval query candidate", async () => {
  const harness = instantiate(
    {
      data: { threads: [] },
      isLoading: false,
      isSuccess: true,
      error: null,
      refetch: () => {},
    },
    { activeThreadId: "thread-active" },
  );

  await harness.queryOptions.queryFn();
  assert.deepEqual(plainThreadRequests(harness.listThreadCalls), [
    { limit: 20, needsApproval: true, candidateThreadId: undefined },
  ]);
});

test("does not poll or persist notification state before profile scope hydrates", () => {
  const harness = instantiate(
    {
      data: { threads: [{ id: "thread-1", state: "needs_attention" }] },
      isLoading: false,
      isSuccess: true,
      error: null,
      refetch: () => {},
    },
    { activeThreadId: "thread-1", profile: null },
  );

  assert.equal(harness.queryOptions.enabled, false);
  assert.deepEqual(harness.getStateScopes, []);
  assert.deepEqual(harness.markSeenCalls, []);
  assert.equal(harness.hook.unreadCount, 0);
});

test("does not include locally known non-automation approval threads", () => {
  const harness = instantiate(
    {
      data: { threads: [] },
      isLoading: false,
      isSuccess: true,
      error: null,
      refetch: () => {},
    },
    {
      threadStates: new Map([["thread-local", "needs_attention"]]),
      approvalThreadNotifications: (threads) =>
        threads.map((thread) => ({
          id: `approval:${thread.id}`,
          href: `/chat/${thread.id}`,
        })),
    },
  );

  assert.deepEqual(
    JSON.parse(JSON.stringify(
      harness.notificationInputs.at(-1).threads.map((thread) => ({
        id: thread.id,
        state: thread.state,
        title: thread.title,
      })),
    )),
    [],
  );
  assert.equal(harness.hook.messages.length, 0);
});

test("passes local thread state to approval notification presenter for backend records", () => {
  const threadStates = new Map([["thread-1", "needs_attention"]]);
  const harness = instantiate(
    {
      data: { threads: [{ id: "thread-1", state: "idle" }] },
      isLoading: false,
      isSuccess: true,
      error: null,
      refetch: () => {},
    },
    {
      threadStates,
      approvalThreadNotifications: (threads, states) =>
        threads
          .filter((thread) => states.get(thread.id) === "needs_attention")
          .map((thread) => ({
            id: `approval:${thread.id}`,
            href: `/chat/${thread.id}`,
          })),
    },
  );

  assert.equal(harness.notificationInputs.at(-1).threadStates, threadStates);
  assert.equal(harness.hook.unreadCount, 1);
});

test("keeps pending approval messages after they are marked seen", () => {
  const { hook, markSeenCalls, render } = instantiate({
    data: { threads: [{ id: "thread-1", state: "needs_attention" }] },
    isLoading: false,
    isSuccess: true,
    error: null,
    refetch: () => {},
  });

  assert.equal(hook.unreadCount, 1);
  hook.dismissMessage("approval:thread-1");
  assert.deepEqual(plainCalls(markSeenCalls), [
    { ids: ["approval:thread-1"], scope: "tenant:user" },
  ]);
  const nextHook = render();
  assert.equal(nextHook.messages.length, 1);
  assert.equal(nextHook.unreadCount, 0);
  assert.equal(nextHook.unreadIds.has("approval:thread-1"), false);
});

test("uses the profile scope for notification dismissal", () => {
  const { getStateScopes, hook, markSeenCalls } = instantiate({
    data: { threads: [{ id: "thread-1", state: "needs_attention" }] },
    isLoading: false,
    isSuccess: true,
    error: null,
    refetch: () => {},
  });

  assert(getStateScopes.includes("tenant:user"));

  hook.dismissMessage("approval:thread-1");
  assert.deepEqual(plainCalls(markSeenCalls), [
    { ids: ["approval:thread-1"], scope: "tenant:user" },
  ]);
});

test("marks an approval notification seen after the thread has been opened", () => {
  const { hook, markSeenCalls } = instantiate(
    {
      data: { threads: [{ id: "thread-1", state: "needs_attention" }] },
      isLoading: false,
      isSuccess: true,
      error: null,
      refetch: () => {},
    },
    { activeThreadId: "thread-1" },
  );

  assert.deepEqual(plainCalls(markSeenCalls), [
    { ids: ["approval:thread-1"], scope: "tenant:user" },
  ]);
  assert.equal(hook.messages.length, 1);
  assert.equal(hook.unreadCount, 0);
  assert.equal(hook.unreadIds.has("approval:thread-1"), false);
});
