// @ts-nocheck
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
  return `${lines.join("\n")}\nglobalThis.__testExports = { useNotifications };`
    .replaceAll(
      'import("../lib/notification-approval-compat")',
      "loadApprovalNotificationCompat()",
    );
}

function depsEqual(left, right) {
  return Array.isArray(left) &&
    Array.isArray(right) &&
    left.length === right.length &&
    left.every((item, index) => Object.is(item, right[index]));
}

function createReactStub() {
  const slots = [];
  let cursor = 0;
  let pendingRender = false;
  return {
    beginRender() {
      cursor = 0;
      pendingRender = false;
    },
    didScheduleUpdate: () => pendingRender,
    useCallback(fn, deps) {
      const index = cursor++;
      const slot = slots[index];
      if (slot && depsEqual(slot.deps, deps)) return slot.value;
      slots[index] = { deps, value: fn };
      return fn;
    },
    useEffect(fn, deps) {
      const index = cursor++;
      const slot = slots[index];
      if (slot && depsEqual(slot.deps, deps)) return;
      slots[index] = { deps };
      fn();
    },
    useMemo(fn, deps) {
      const index = cursor++;
      const slot = slots[index];
      if (slot && depsEqual(slot.deps, deps)) return slot.value;
      const value = fn();
      slots[index] = { deps, value };
      return value;
    },
    useState(initial) {
      const index = cursor++;
      if (!slots[index]) {
        slots[index] = { value: typeof initial === "function" ? initial() : initial };
      }
      return [slots[index].value, (next) => {
        const value = typeof next === "function" ? next(slots[index].value) : next;
        if (!Object.is(value, slots[index].value)) {
          slots[index].value = value;
          pendingRender = true;
        }
      }];
    },
  };
}

function instantiate({
  data,
  profile = { tenant_id: "tenant", user_id: "user" },
  activeThreadId = null,
  inboxError = null,
  approvalError = null,
} = {}) {
  let queryOptions;
  const readCalls = [];
  const allReadCalls = [];
  const inboxCalls = [];
  const threadCalls = [];
  const seenCalls = [];
  let storedState = { initialized: true, seenIds: new Set() };
  const react = createReactStub();
  const queryClient = {
    cancelQueries: async () => {},
    getQueryData: () => data,
    setQueryData: () => {},
    invalidateQueries: () => {},
  };
  let mutationIndex = 0;
  const context = {
    React: react,
    Promise,
    useI18n: () => ({ t: (key) => key }),
    useQueryClient: () => queryClient,
    useThreadStates: () => new Map(),
    THREAD_STATE: { NEEDS_ATTENTION: "needs_attention" },
    useQuery: (options) => {
      queryOptions = options;
      return { data, isLoading: false, error: null, refetch: () => {} };
    },
    useMutation: ({ mutationFn }) => {
      mutationIndex += 1;
      return {
        mutate: (value) => mutationFn(value),
        isPending: false,
        error: null,
        mutationIndex,
      };
    },
    listNotifications: async (request) => {
      inboxCalls.push(request);
      if (inboxError) throw inboxError;
      return data?.inbox || { notifications: [], unread_count: 0 };
    },
    listThreads: async (request) => {
      threadCalls.push(request);
      if (approvalError) throw approvalError;
      return data?.approvalThreads || { threads: [] };
    },
    markNotificationRead: async (id) => readCalls.push(id),
    markAllNotificationsRead: async () => allReadCalls.push(true),
    notificationMessages: (notifications) => (notifications || []).map((notification) => ({
      id: notification.id,
      type: notification.kind,
      href: `/chat/${notification.action.thread_id}`,
      timestamp: notification.timestamp || 2,
      read: Boolean(notification.read_at),
    })),
    loadApprovalNotificationCompat: async () => ({
      approvalThreadNotifications: (threads) => threads.map((thread) => ({
        id: `approval:${thread.id}`,
        type: "approval",
        href: `/chat/${thread.id}`,
        timestamp: 1,
        read: false,
      })),
      getNotificationState: () => storedState,
      markNotificationIdsSeen: (ids, scope) => {
        seenCalls.push({ ids, scope });
        storedState = {
          initialized: true,
          seenIds: new Set([...storedState.seenIds, ...ids]),
        };
        return storedState;
      },
    }),
    globalThis: {},
  };
  vm.runInNewContext(sourceForTest(), context);
  const render = () => {
    let hook;
    for (let attempt = 0; attempt < 5; attempt += 1) {
      react.beginRender();
      mutationIndex = 0;
      hook = context.globalThis.__testExports.useNotifications({ profile, activeThreadId });
      if (!react.didScheduleUpdate()) break;
    }
    return hook;
  };
  return {
    hook: render(),
    render,
    get queryOptions() { return queryOptions; },
    readCalls,
    allReadCalls,
    inboxCalls,
    threadCalls,
    seenCalls,
  };
}

function notification(id = "notification-1", threadId = "thread-1", readAt = null) {
  return {
    id,
    kind: "approval_required",
    action: { kind: "open_thread", thread_id: threadId },
    read_at: readAt,
  };
}

function flushAsyncWork() {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

test("queries only the durable inbox after profile hydration", async () => {
  const harness = instantiate({ data: { inbox: {}, approvalThreads: {} } });
  assert.equal(harness.queryOptions.enabled, true);
  await harness.queryOptions.queryFn();
  assert.deepEqual(JSON.parse(JSON.stringify(harness.inboxCalls)), [{ limit: 30 }]);
  assert.deepEqual(JSON.parse(JSON.stringify(harness.threadCalls)), []);

  const pending = instantiate({ data: {}, profile: null });
  assert.equal(pending.queryOptions.enabled, false);
});

test("uses the compatibility fallback when the server does not support the inbox", async () => {
  const harness = instantiate({
    data: { approvalThreads: { threads: [{ id: "thread-fallback" }] } },
    inboxError: Object.assign(new Error("inbox unavailable"), { status: 404 }),
  });
  const result = await harness.queryOptions.queryFn();
  assert.deepEqual(JSON.parse(JSON.stringify(result.inbox)), {
    notifications: [],
    unread_count: 0,
  });
  assert.equal(result.compatibility[0].id, "approval:thread-fallback");
  assert.equal(result.inboxSupported, false);
  assert.deepEqual(JSON.parse(JSON.stringify(harness.threadCalls)), [
    { limit: 20, needsApproval: true },
  ]);
});

test("surfaces transient inbox failures without activating the compatibility path", async () => {
  const inboxError = Object.assign(new Error("inbox unavailable"), { status: 503 });
  const harness = instantiate({
    data: { approvalThreads: { threads: [{ id: "thread-fallback" }] } },
    inboxError,
  });
  await assert.rejects(harness.queryOptions.queryFn(), inboxError);
  assert.deepEqual(JSON.parse(JSON.stringify(harness.threadCalls)), []);
});

test("deduplicates fallback approvals when the durable inbox has the same thread", () => {
  const harness = instantiate({
    data: {
      inbox: { notifications: [notification()], unread_count: 1 },
      compatibility: [{
        id: "approval:thread-1",
        type: "approval",
        href: "/chat/thread-1",
        timestamp: 1,
        read: false,
      }],
    },
  });
  assert.deepEqual(
    JSON.parse(JSON.stringify(harness.hook.messages.map((message) => message.id))),
    ["notification-1"],
  );
  assert.equal(harness.hook.unreadCount, 1);
});

test("marks durable and compatibility notifications through their owning state", async () => {
  const durable = instantiate({
    data: {
      inbox: { notifications: [notification()], unread_count: 1 },
      approvalThreads: { threads: [] },
    },
  });
  durable.hook.dismissMessage("notification-1");
  assert.deepEqual(durable.readCalls, ["notification-1"]);

  const fallback = instantiate({
    data: {
      inbox: { notifications: [], unread_count: 0 },
      compatibility: [{
        id: "approval:thread-fallback",
        type: "approval",
        href: "/chat/thread-fallback",
        timestamp: 1,
        read: false,
      }],
    },
  });
  fallback.hook.dismissMessage("approval:thread-fallback");
  await flushAsyncWork();
  assert.deepEqual(JSON.parse(JSON.stringify(fallback.seenCalls)), [
    { ids: ["approval:thread-fallback"], scope: "tenant:user" },
  ]);
});

test("marks the active notification and supports mark all across both stores", async () => {
  const harness = instantiate({
    data: {
      inbox: { notifications: [notification()], unread_count: 1 },
      compatibility: [{
        id: "approval:thread-fallback",
        type: "approval",
        href: "/chat/thread-fallback",
        timestamp: 1,
        read: false,
      }],
    },
    activeThreadId: "thread-1",
  });
  assert.deepEqual(harness.readCalls, ["notification-1"]);
  harness.hook.markAllRead();
  await flushAsyncWork();
  assert.deepEqual(harness.allReadCalls, [true]);
  assert.deepEqual(JSON.parse(JSON.stringify(harness.seenCalls)), [
    { ids: ["approval:thread-fallback"], scope: "tenant:user" },
  ]);
});

test("does not call durable mutations when the server lacks the inbox API", async () => {
  const harness = instantiate({
    data: {
      inboxSupported: false,
      inbox: { notifications: [], unread_count: 0 },
      compatibility: [{
        id: "approval:thread-fallback",
        type: "approval",
        href: "/chat/thread-fallback",
        timestamp: 1,
        read: false,
      }],
    },
  });
  harness.hook.markAllRead();
  await flushAsyncWork();
  assert.deepEqual(harness.allReadCalls, []);
  assert.deepEqual(JSON.parse(JSON.stringify(harness.seenCalls)), [
    { ids: ["approval:thread-fallback"], scope: "tenant:user" },
  ]);
});
