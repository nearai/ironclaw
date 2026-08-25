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
  return `${lines.join("\n")}\nglobalThis.__testExports = { useNotifications };`;
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
  mutationsPending = false,
  archiveError = null,
} = {}) {
  let queryOptions;
  const readCalls = [];
  const allReadCalls = [];
  const inboxCalls = [];
  const archiveCalls = [];
  const optimisticWrites = [];
  const queryKeys = [];
  const refetchCalls = [];
  const mutationFailures = [];
  const refetch = () => {
    refetchCalls.push(true);
  };
  const react = createReactStub();
  /* A real query cache composes: the second optimistic write starts from the
   * first one's result. Reading the pristine fixture back every time would let
   * a second update silently clobber the first and still pass. */
  let cached = data;
  const queryClient = {
    cancelQueries: async () => {},
    getQueryData: () => cached,
    setQueryData: (_key, updater) => {
      cached = typeof updater === "function" ? updater(cached) : updater;
      optimisticWrites.push(cached);
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
      queryKeys.push(options.queryKey);
      return {
        data: cached,
        isLoading: false,
        error: null,
        refetch,
      };
    },
    useMutation: ({ mutationFn, onMutate, onError, onSuccess, onSettled }) => {
      mutationIndex += 1;
      return {
        /* The whole lifecycle, not just the happy half: `onMutate` writes the
         * optimistic value, and a rejecting `mutationFn` has to reach `onError`
         * so the rollback is exercised. Stopping at `onMutate` left a failed
         * archive looking permanently applied with every test still green. */
        mutate: (value) => {
          const context = onMutate?.(value);
          // Still synchronous, so callers can assert the request without
          // flushing; only the outcome handling is deferred.
          const outcome = mutationFn(value);
          return Promise.resolve(context).then((resolvedContext) =>
            Promise.resolve(outcome)
              .then(
                (result) => {
                  onSuccess?.(result, value, resolvedContext);
                  return result;
                },
                (error) => {
                  onError?.(error, value, resolvedContext);
                  mutationFailures.push(error);
                },
              )
              .finally(() => onSettled?.()),
          );
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
    markNotificationRead: async (id) => readCalls.push(id),
    markAllNotificationsRead: async () => allReadCalls.push(true),
    archiveNotification: async (id) => {
      archiveCalls.push(id);
      if (archiveError) throw archiveError;
    },
    notificationMessages: (notifications) => (notifications || []).map((notification) => ({
      id: notification.id,
      type: notification.kind,
      href: `/chat/${notification.action.thread_id}`,
      threadId: notification.thread_id || notification.action.thread_id,
      turnRunId: notification.turn_run_id || null,
      timestamp: notification.timestamp || 2,
      read: Boolean(notification.read_at),
    })),
    globalThis: {},
  };
  vm.runInNewContext(sourceForTest(), context);
  /* Settle the hook the way React would, then hand back converged state. If it
   * never settles, say so: returning the last mid-flight attempt would make a
   * render loop in `useNotifications` read as green, with every assertion after
   * it measuring unconverged state. */
  const MAX_RENDER_ATTEMPTS = 5;
  const render = () => {
    let hook;
    for (let attempt = 0; attempt < MAX_RENDER_ATTEMPTS; attempt += 1) {
      react.beginRender();
      mutationIndex = 0;
      hook = context.globalThis.__testExports.useNotifications({ profile, activeThreadId });
      if (!react.didScheduleUpdate()) return hook;
    }
    throw new Error(
      `useNotifications still scheduled an update after ${MAX_RENDER_ATTEMPTS} renders`,
    );
  };
  return {
    hook: render(),
    render,
    get queryOptions() { return queryOptions; },
    readCalls,
    allReadCalls,
    inboxCalls,
    queryKeys,
    refetchCalls,
    mutationFailures,
    archiveCalls,
    optimisticWrites,
    setProfile(next) { profile = next; },
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

test("reads only the durable inbox for an authenticated recipient", async () => {
  const harness = instantiate({
    data: {
      inbox: { notifications: [notification()], unread_count: 1 },
    },
  });
  assert.equal(harness.queryOptions.enabled, true);
  const result = await harness.queryOptions.queryFn({ signal: new AbortController().signal });
  assert.deepEqual(JSON.parse(JSON.stringify(harness.inboxCalls)), [
    { limit: 30, signal: {} },
  ], "every page carries the query's abort signal");
  assert.equal(result.inbox.notifications[0].id, "notification-1");
  assert.deepEqual(Object.keys(result), ["inbox"]);

  const pending = instantiate({ data: {}, profile: null });
  assert.equal(pending.queryOptions.enabled, false);
});

test("surfaces inbox failures instead of presenting a false empty state", async () => {
  const inboxError = Object.assign(new Error("inbox unavailable"), { status: 503 });
  const harness = instantiate({ inboxError });
  await assert.rejects(harness.queryOptions.queryFn({ signal: new AbortController().signal }), inboxError);
  const unsupported = Object.assign(new Error("inbox route missing"), { status: 404 });
  const unsupportedHarness = instantiate({ inboxError: unsupported });
  await assert.rejects(
    unsupportedHarness.queryOptions.queryFn({ signal: new AbortController().signal }),
    unsupported,
  );
});

test("marks durable notifications through the inbox API", () => {
  const harness = instantiate({
    data: {
      inbox: { notifications: [notification()], unread_count: 1 },
    },
  });
  harness.hook.dismissMessage("notification-1");
  assert.deepEqual(harness.readCalls, ["notification-1"]);
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
  await harness.queryOptions.queryFn({ signal: new AbortController().signal });

  assert.deepEqual(
    harness.inboxCalls.map((call) => call.cursor),
    [undefined],
    "the first pass asks for the head with no cursor",
  );
  assert.equal(harness.hook.canLoadMore, true);

  harness.hook.loadMore();
  harness.render();
  harness.inboxCalls.length = 0;
  const merged = await harness.queryOptions.queryFn({ signal: new AbortController().signal });

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
  const merged = await harness.queryOptions.queryFn({ signal: new AbortController().signal });

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

test("mark all read settles the durable inbox", async () => {
  const harness = instantiate({
    data: {
      inbox: { notifications: [notification()], unread_count: 1 },
    },
  });
  harness.hook.markAllRead();
  await flushAsyncWork();
  assert.deepEqual(harness.allReadCalls, [true]);
});

test("an abort mid-walk stops paging instead of draining the inbox", async () => {
  const harness = instantiate({ data: PAGED_INBOX });
  harness.hook.loadMore();
  harness.hook.loadMore();
  harness.render();
  harness.inboxCalls.length = 0;

  // Abort as soon as the head has been read, which is what an unmount or a
  // superseding refetch does to a query that is still walking its cursor.
  const controller = new AbortController();
  const paged = harness.queryOptions.queryFn({ signal: controller.signal });
  controller.abort();

  await assert.rejects(paged);
  assert.deepEqual(
    harness.inboxCalls.map((call) => call.cursor),
    [undefined],
    "the walk stops at the head rather than following cursor-2 and cursor-3",
  );
});

test("archiving twice composes instead of resurrecting the first record", async () => {
  const harness = instantiate({
    data: {
      inbox: {
        notifications: [
          notification("notification-1"),
          notification("notification-2"),
          notification("notification-3"),
        ],
        unread_count: 3,
      },
      approvalThreads: { threads: [] },
    },
  });

  harness.hook.archiveMessage("notification-1");
  await flushAsyncWork();
  harness.render();
  harness.hook.archiveMessage("notification-2");
  await flushAsyncWork();

  assert.deepEqual(harness.archiveCalls, ["notification-1", "notification-2"]);
  assert.deepEqual(
    JSON.parse(
      JSON.stringify(
        harness.optimisticWrites.at(-1).inbox.notifications.map((record) => record.id),
      ),
    ),
    ["notification-3"],
    "the second archive builds on the first rather than restoring it",
  );
  assert.equal(harness.optimisticWrites.at(-1).inbox.unread_count, 1);
});

test("loading more keeps one cache entry so the open panel never blanks", async () => {
  const harness = instantiate({ data: PAGED_INBOX });
  const firstKey = harness.queryKeys.at(-1);

  harness.hook.loadMore();
  harness.render();

  assert.deepEqual(
    JSON.parse(JSON.stringify(harness.queryKeys.at(-1))),
    JSON.parse(JSON.stringify(firstKey)),
    "the page count is a request parameter, so it must not split the cache",
  );
  assert.ok(
    harness.refetchCalls.length > 0,
    "a stable key means the wider read has to be asked for explicitly",
  );
  assert.deepEqual(
    JSON.parse(JSON.stringify(harness.hook.messages.map((message) => message.id))),
    ["page1-a", "page1-b"],
    "the rows already on screen survive the transition",
  );
  assert.equal(harness.hook.unreadCount, 5, "and so does the badge");

  harness.setProfile({ tenant_id: "other-tenant", user_id: "other-user" });
  harness.render();
  assert.notDeepEqual(
    JSON.parse(JSON.stringify(harness.queryKeys.at(-1))),
    JSON.parse(JSON.stringify(firstKey)),
    "changing recipient scope selects a different cache entry",
  );
});

test("closing the panel collapses paging back to the head", async () => {
  const harness = instantiate({ data: PAGED_INBOX });
  harness.hook.loadMore();
  harness.hook.loadMore();
  harness.render();
  harness.inboxCalls.length = 0;
  await harness.queryOptions.queryFn({ signal: new AbortController().signal });
  assert.deepEqual(
    harness.inboxCalls.map((call) => call.cursor),
    [undefined, "cursor-2", "cursor-3"],
    "a reader who paged twice has the poll walking three pages",
  );

  harness.hook.collapsePages();
  harness.render();
  harness.inboxCalls.length = 0;
  await harness.queryOptions.queryFn({ signal: new AbortController().signal });
  assert.deepEqual(
    harness.inboxCalls.map((call) => call.cursor),
    [undefined],
    "with the panel shut the poll is back to one request for the badge",
  );
});

test("a failed archive puts the row back and reports the failure", async () => {
  const archiveError = Object.assign(new Error("archive rejected"), { status: 500 });
  const harness = instantiate({
    archiveError,
    data: {
      inbox: {
        notifications: [notification("notification-1"), notification("notification-2")],
        unread_count: 2,
      },
      approvalThreads: { threads: [] },
    },
  });

  harness.hook.archiveMessage("notification-1");
  await flushAsyncWork();

  assert.deepEqual(harness.archiveCalls, ["notification-1"]);
  assert.deepEqual(
    JSON.parse(JSON.stringify(harness.mutationFailures.map((error) => error.message))),
    ["archive rejected"],
    "the rejection reaches onError rather than being swallowed",
  );
  assert.deepEqual(
    JSON.parse(
      JSON.stringify(
        harness.optimisticWrites.at(-1).inbox.notifications.map((record) => record.id),
      ),
    ),
    ["notification-1", "notification-2"],
    "the row the server refused to archive is restored, not left missing",
  );
  assert.equal(harness.optimisticWrites.at(-1).inbox.unread_count, 2);
});
