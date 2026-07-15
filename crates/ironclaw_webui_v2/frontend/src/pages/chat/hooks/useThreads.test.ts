// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

// Load useThreads.ts into a fresh VM context with its imports stripped, the
// same harness pattern useHistory.test.ts uses. The hook's collaborators
// (React, react-query, the api.ts requests, the query client) are injected as
// context globals so the test can drive `handleCreateThread` directly.
function useThreadsSourceForTest() {
  const source = readFileSync(new URL("./useThreads.ts", import.meta.url), "utf8");
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
  return `${lines.join("\n")}\nglobalThis.__testExports = { useThreads };`;
}

function createReactStub({ initialStateByIndex = {}, onStateChange } = {}) {
  let stateIndex = 0;
  return {
    useCallback: (fn) => fn,
    useMemo: (fn) => fn(),
    useEffect: (fn) => {
      fn();
    },
    useRef: (value) => ({ current: value }),
    useState: (initial) => {
      const index = stateIndex;
      stateIndex += 1;
      let value = Object.prototype.hasOwnProperty.call(initialStateByIndex, index)
        ? initialStateByIndex[index]
        : typeof initial === "function" ? initial() : initial;
      return [value, (next) => {
        value = typeof next === "function" ? next(value) : next;
        onStateChange?.(index, value);
      }];
    },
  };
}

// Each `createThreadRequest` call resolves to a thread id derived from the
// project scope, but only once `release()` is invoked — so the test can hold
// several creates in flight simultaneously and inspect the dedup behaviour.
function makeDeferredCreate() {
  const calls = [];
  const releasers = [];
  const createThreadRequest = (arg) => {
    calls.push(arg);
    const scope = arg && arg.projectId ? arg.projectId : "global";
    return new Promise((resolve) => {
      releasers.push(() => resolve({ thread: { thread_id: `thread-${scope}` } }));
    });
  };
  return {
    calls,
    createThreadRequest,
    releaseAll: () => releasers.forEach((release) => release()),
  };
}

function instantiate(createThreadRequest, overrides = {}) {
  const context = {
    console,
    useQuery: () => ({ data: { threads: [] }, isLoading: false }),
    React: createReactStub(),
    createThreadRequest,
    deleteThreadRequest: async () => {},
    listThreads: async () => ({ threads: [] }),
    normalizeSidebarTitle: (title, threadId) => {
      const trimmed = String(title || "").trim();
      return trimmed && trimmed !== threadId ? trimmed : null;
    },
    removeThreadList: (data, threadId) => ({
      ...data,
      threads: data.threads.filter((thread) => (thread.thread_id || thread.id) !== threadId),
    }),
    queryClient: { setQueryData: () => {}, invalidateQueries: () => {} },
    upsertThreadInCache: () => {},
    globalThis: {},
    ...overrides,
  };
  vm.runInNewContext(useThreadsSourceForTest(), context);
  return context.globalThis.__testExports.useThreads();
}

test("handleCreateThread scopes the in-flight dedup by project", async () => {
  const { calls, createThreadRequest, releaseAll } = makeDeferredCreate();
  const hook = instantiate(createThreadRequest);

  // Two concurrent creates for *different* projects. A single shared in-flight
  // ref would hand project-b the pending project-a promise, firing one request
  // and mis-routing project-b to project-a's new thread.
  const pendingA = hook.createThread("project-a");
  const pendingB = hook.createThread("project-b");

  assert.equal(calls.length, 2, "each project scope issues its own create request");
  assert.deepEqual(
    calls.map((arg) => arg.projectId),
    ["project-a", "project-b"],
  );

  releaseAll();
  assert.equal(await pendingA, "thread-project-a");
  assert.equal(await pendingB, "thread-project-b");
});

test("handleCreateThread still collapses concurrent creates within one project", async () => {
  const { calls, createThreadRequest, releaseAll } = makeDeferredCreate();
  const hook = instantiate(createThreadRequest);

  const first = hook.createThread("project-a");
  const second = hook.createThread("project-a");

  assert.equal(calls.length, 1, "a true double-submit within one scope dedups to one request");

  releaseAll();
  // Both callers observe the single in-flight create's result.
  assert.equal(await first, "thread-project-a");
  assert.equal(await second, "thread-project-a");
});

test("handleCreateThread upserts the created thread without refetching the list", async () => {
  const upserted = [];
  const hook = instantiate(
    async () => ({ thread: { thread_id: "thread-created", title: "Created" } }),
    {
      queryClient: {
        invalidateQueries: () => {
          throw new Error("create should not refetch the full thread list");
        },
      },
      upsertThreadInCache: (thread) => upserted.push(thread),
    },
  );

  assert.equal(await hook.createThread(), "thread-created");
  assert.deepEqual(upserted, [{ thread_id: "thread-created", title: "Created" }]);
});

test("handleDeleteThread removes the requested thread from cache before refreshing", async () => {
  const deleted = [];
  const invalidations = [];
  const cacheWrites = [];
  const stateChanges = [];
  let cachedData = {
    threads: [
      { thread_id: "thread-old" },
      { thread_id: "thread-keep" },
    ],
    next_cursor: "cursor-1",
  };
  const hook = instantiate(
    async () => ({ thread: { thread_id: "unused" } }),
    {
      React: createReactStub({
        initialStateByIndex: { 0: "thread-old" },
        onStateChange: (index, value) => stateChanges.push({ index, value }),
      }),
      deleteThreadRequest: async ({ threadId }) => {
        deleted.push(threadId);
      },
      queryClient: {
        setQueryData: (queryKey, update) => {
          cacheWrites.push([...queryKey]);
          cachedData = update(cachedData);
        },
        invalidateQueries: (request) => invalidations.push([...request.queryKey]),
      },
    },
  );

  await hook.deleteThread("thread-old");

  assert.deepEqual(deleted, ["thread-old"]);
  assert.deepEqual(stateChanges, [{ index: 0, value: null }]);
  assert.deepEqual(cacheWrites, [["threads"]]);
  assert.deepEqual(cachedData, {
    threads: [{ thread_id: "thread-keep" }],
    next_cursor: "cursor-1",
  });
  assert.deepEqual(invalidations, [["threads"]]);
});

test("handleDeleteThread preserves active thread state when deletion fails", async () => {
  const invalidations = [];
  const stateChanges = [];
  const hook = instantiate(
    async () => ({ thread: { thread_id: "unused" } }),
    {
      React: createReactStub({
        initialStateByIndex: { 0: "thread-old" },
        onStateChange: (index, value) => stateChanges.push({ index, value }),
      }),
      deleteThreadRequest: async () => {
        throw new Error("delete failed");
      },
      queryClient: {
        setQueryData: (queryKey) => invalidations.push(["cache-write", ...queryKey]),
        invalidateQueries: (request) => invalidations.push([...request.queryKey]),
      },
    },
  );

  await assert.rejects(hook.deleteThread("thread-old"), /delete failed/);

  assert.deepEqual(stateChanges, []);
  assert.deepEqual(invalidations, []);
});

test("normalizes raw thread id titles out of sidebar records", () => {
  const hook = instantiate(async () => ({ thread: { thread_id: "unused" } }), {
    useQuery: () => ({
      data: {
        threads: [
          { thread_id: "thread_raw123456", title: "thread_raw123456" },
          { thread_id: "thread-named", title: "Named conversation" },
          { thread_id: "thread-real", title: "thread-safety" },
        ],
      },
      isLoading: false,
    }),
    normalizeSidebarTitle: (title, threadId) => {
      const trimmed = String(title || "").trim();
      if (!trimmed || trimmed === threadId) {
        return null;
      }
      return trimmed;
    },
  });

  assert.deepEqual(
    hook.threads.map((thread) => ({ id: thread.id, title: thread.title })),
    [
      { id: "thread_raw123456", title: null },
      { id: "thread-named", title: "Named conversation" },
      { id: "thread-real", title: "thread-safety" },
    ],
  );
});
