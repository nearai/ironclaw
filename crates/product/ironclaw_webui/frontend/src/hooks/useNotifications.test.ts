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
  mutationsPending = false,
} = {}) {
  let queryOptions;
  const readCalls = [];
  const allReadCalls = [];
  const inboxCalls = [];
  const threadCalls = [];
  const seenCalls = [];
  const archiveCalls = [];
  const optimisticWrites = [];
  let storedState = { initialized: true, seenIds: new Set() };
  const react = createReactStub();
  const queryClient = {
    cancelQueries: async () => {},
    getQueryData: () => data,
    setQueryData: (_key, updater) => {
      optimisticWrites.push(typeof updater === "function" ? updater(data) : updater);
    },
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
    useMutation: ({ mutationFn, onMutate }) => {
      mutationIndex += 1;
      return {
        // `onMutate` runs first so the optimistic cache write is exercised,
        // and `mutationFn` still fires synchronously so callers can assert the
        // request without flushing. The optimistic write itself lands a
        // microtask later, after `onMutate` awaits `cancelQueries`.
        mutate: (value) => {
          onMutate?.(value);
          return mutationFn(value);
        },
        isPending: mutationsPending,
        error: null,
        mutationIndex,
      };
    },
    listNotifications: async (request) => {
      inboxCalls.push(request);
      if (inboxError) throw inboxError;
      if (data?.inboxPages) {
        const index = request.cursor
          ? data.inboxPages.findIndex((page) => page.requestCursor === request.cursor)
          : 0;
        return data.inboxPages[index] || { notifications: [], unread_count: 0 };
      }
      return data?.inbox || { notifications: [], unread_count: 0 };
    },
    listThreads: async (request) => {
      threadCalls.push(request);
      if (approvalError) throw approvalError;
      return data?.approvalThreads || { threads: [] };
    },
    markNotificationRead: async (id) => readCalls.push(id),
    markAllNotificationsRead: async () => allReadCalls.push(true),
    archiveNotification: async (id) => archiveCalls.push(id),
    notificationMessages: (notifications) => (notifications || []).map((notification) => ({
      id: notification.id,
      type: notification.kind,
      href: `/chat/${notification.action.thread_id}`,
      threadId: notification.thread_id || notification.action.thread_id,
      turnRunId: notification.turn_run_id || null,
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
    archiveCalls,
    optimisticWrites,
  };
}

function notification(
  id = "notification-1",
  threadId = "thread-1",
  readAt = null,
  kind = "approval_required",
  turnRunId = null,
) {
  return {
    id,
    kind,
    action: { kind: "open_thread", thread_id: threadId },
    thread_id: threadId,
    turn_run_id: turnRunId,
    read_at: readAt,
  };
}

function flushAsyncWork() {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

test("queries the durable inbox and legacy approvals after profile hydration", async () => {
  const harness = instantiate({
    data: {
      inbox: { notifications: [], unread_count: 0 },
      approvalThreads: { threads: [{ id: "thread-transition" }] },
    },
  });
  assert.equal(harness.queryOptions.enabled, true);
  const result = await harness.queryOptions.queryFn();
  assert.deepEqual(JSON.parse(JSON.stringify(harness.inboxCalls)), [{ limit: 30 }]);
  assert.deepEqual(JSON.parse(JSON.stringify(harness.threadCalls)), [
    { limit: 20, needsApproval: true },
  ]);
  assert.equal(result.compatibility[0].id, "approval:thread-transition");
  assert.equal(result.inboxSupported, true);

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
  assert.deepEqual(JSON.parse(JSON.stringify(harness.threadCalls)), [
    { limit: 20, needsApproval: true },
  ]);
});

test("keeps durable notifications when the legacy approval query fails", async () => {
  const harness = instantiate({
    data: {
      inbox: { notifications: [notification()], unread_count: 1 },
    },
    approvalError: new Error("legacy approvals unavailable"),
  });
  const result = await harness.queryOptions.queryFn();
  assert.equal(result.inboxSupported, true);
  assert.equal(result.inbox.notifications[0].id, "notification-1");
  assert.deepEqual(JSON.parse(JSON.stringify(result.compatibility)), []);
});

test("surfaces the legacy approval failure when no durable inbox is available", async () => {
  const approvalError = new Error("legacy approvals unavailable");
  const harness = instantiate({
    inboxError: Object.assign(new Error("inbox unavailable"), { status: 404 }),
    approvalError,
  });
  await assert.rejects(harness.queryOptions.queryFn(), approvalError);
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

test("archives a durable notification and drops it from the cached inbox", async () => {
  const harness = instantiate({
    data: {
      inbox: { notifications: [notification(), notification("notification-2")], unread_count: 2 },
      approvalThreads: { threads: [] },
    },
  });

  harness.hook.archiveMessage("notification-1");
  await flushAsyncWork();

  assert.deepEqual(harness.archiveCalls, ["notification-1"]);
  const optimistic = harness.optimisticWrites.at(-1);
  assert.deepEqual(
    optimistic.inbox.notifications.map((record) => record.id),
    ["notification-2"],
    "the archived record leaves the cached list immediately",
  );
  assert.equal(
    optimistic.inbox.unread_count,
    1,
    "archiving an unread record also drops it from the badge",
  );
});

test("archiving a read record leaves the unread badge alone", async () => {
  const harness = instantiate({
    data: {
      inbox: {
        notifications: [notification("notification-1", "thread-1", "2026-08-19T00:00:00Z")],
        unread_count: 0,
      },
      approvalThreads: { threads: [] },
    },
  });

  harness.hook.archiveMessage("notification-1");
  await flushAsyncWork();

  assert.deepEqual(harness.archiveCalls, ["notification-1"]);
  assert.equal(harness.optimisticWrites.at(-1).inbox.unread_count, 0);
});

test("a compatibility notification is never archived through the server", () => {
  const harness = instantiate({
    data: {
      inbox: { notifications: [], unread_count: 0 },
      approvalThreads: { threads: [{ id: "thread-fallback" }] },
    },
  });

  harness.hook.archiveMessage("approval:thread-fallback");

  assert.deepEqual(
    harness.archiveCalls,
    [],
    "a legacy approval row has no durable record, so archiving it would 404",
  );
});

const PAGED_INBOX = {
  // The harness's `useQuery` hands `data` straight back, so `inbox` stands in
  // for the merged state the hook derives from; `inboxPages` is what the
  // queryFn reads through the cursor.
  inbox: {
    notifications: [notification("page1-a"), notification("page1-b")],
    unread_count: 5,
    next_cursor: "cursor-2",
  },
  inboxPages: [
    {
      notifications: [notification("page1-a"), notification("page1-b")],
      unread_count: 5,
      next_cursor: "cursor-2",
    },
    {
      requestCursor: "cursor-2",
      notifications: [notification("page2-a")],
      unread_count: 5,
      next_cursor: "cursor-3",
    },
    {
      requestCursor: "cursor-3",
      notifications: [notification("page3-a")],
      unread_count: 5,
      next_cursor: null,
    },
  ],
  approvalThreads: { threads: [] },
};

test("paging follows the cursor instead of only widening one request", async () => {
  const harness = instantiate({ data: PAGED_INBOX });
  await harness.queryOptions.queryFn();

  assert.deepEqual(
    harness.inboxCalls.map((call) => call.cursor),
    [undefined],
    "the first pass asks for the head with no cursor",
  );
  assert.equal(harness.hook.canLoadMore, true);

  harness.hook.loadMore();
  harness.render();
  harness.inboxCalls.length = 0;
  const merged = await harness.queryOptions.queryFn();

  assert.deepEqual(
    harness.inboxCalls.map((call) => call.cursor),
    [undefined, "cursor-2"],
    "loading more re-reads the head and follows the reported cursor",
  );
  assert.deepEqual(
    JSON.parse(JSON.stringify(merged.inbox.notifications.map((record) => record.id))),
    ["page1-a", "page1-b", "page2-a"],
    "the loaded pages are one flat list, head first",
  );
  assert.equal(
    merged.inbox.unread_count,
    5,
    "the badge keeps the server total rather than summing pages",
  );
  assert.equal(
    merged.inbox.next_cursor,
    "cursor-3",
    "the cursor to continue from is the last loaded page's",
  );
});

test("paging is not capped by the per-request ceiling", async () => {
  const harness = instantiate({ data: PAGED_INBOX });
  let hook = harness.hook;
  for (let attempt = 0; attempt < 2; attempt += 1) {
    hook.loadMore();
    hook = harness.render();
  }
  harness.inboxCalls.length = 0;
  const merged = await harness.queryOptions.queryFn();

  assert.deepEqual(
    harness.inboxCalls.map((call) => call.cursor),
    [undefined, "cursor-2", "cursor-3"],
    "every reported cursor is followed, so records past one page stay reachable",
  );
  assert.deepEqual(
    JSON.parse(JSON.stringify(merged.inbox.notifications.map((record) => record.id))),
    ["page1-a", "page1-b", "page2-a", "page3-a"],
  );
  assert.equal(
    merged.inbox.next_cursor,
    null,
    "the last page reports no cursor, so there is nothing left to load",
  );
});

test("stops offering more pages once the inbox reports no cursor", () => {
  const harness = instantiate({
    data: {
      inbox: { notifications: [notification()], unread_count: 1, next_cursor: null },
      approvalThreads: { threads: [] },
    },
  });

  assert.equal(harness.hook.canLoadMore, false);
});

test("does not mark a notification merely because its thread route is active", () => {
  const harness = instantiate({
    data: {
      inbox: { notifications: [notification()], unread_count: 1 },
    },
    activeThreadId: "thread-1",
  });
  assert.deepEqual(harness.readCalls, []);
});

test("marks a run completion only after its matching final reply rendered", () => {
  const harness = instantiate({
    data: {
      inbox: {
        notifications: [
          notification(
            "notification-completed",
            "thread-1",
            null,
            "run_completed",
            "run-1",
          ),
        ],
        unread_count: 1,
      },
    },
    activeThreadId: "thread-1",
  });
  harness.hook.prepareMessageOpen(harness.hook.messages[0]);
  const pending = harness.render();
  assert.deepEqual(JSON.parse(JSON.stringify(pending.pendingRenderedNotification)), {
    notificationId: "notification-completed",
    threadId: "thread-1",
    turnRunId: "run-1",
  });
  assert.deepEqual(harness.readCalls, []);

  pending.acknowledgeRenderedNotification({
    threadId: "thread-1",
    turnRunId: "run-1",
  });
  assert.deepEqual(harness.readCalls, ["notification-completed"]);
});

test("acknowledges a rendered completion while an earlier mark-read is in flight", () => {
  // The final reply renders once per run, so an acknowledgement skipped here
  // never gets another trigger and the completion stays unread forever.
  const harness = instantiate({
    data: {
      inbox: {
        notifications: [
          notification(
            "notification-completed",
            "thread-1",
            null,
            "run_completed",
            "run-1",
          ),
        ],
        unread_count: 1,
      },
    },
    activeThreadId: "thread-1",
    mutationsPending: true,
  });
  harness.hook.prepareMessageOpen(harness.hook.messages[0]);
  const pending = harness.render();
  pending.acknowledgeRenderedNotification({
    threadId: "thread-1",
    turnRunId: "run-1",
  });
  assert.deepEqual(harness.readCalls, ["notification-completed"]);
});

test("supports mark all across durable and compatibility stores", async () => {
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
  });
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
