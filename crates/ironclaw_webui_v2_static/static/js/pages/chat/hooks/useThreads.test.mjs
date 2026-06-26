import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

// Load useThreads.js into a fresh VM context with its imports stripped, the
// same harness pattern useHistory.test.mjs uses. The hook's collaborators
// (React, react-query, the api.js requests, the query client) are injected as
// context globals so the test can drive `handleCreateThread` directly.
function useThreadsSourceForTest() {
  const source = readFileSync(new URL("./useThreads.js", import.meta.url), "utf8");
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

function createReactStub() {
  return {
    useCallback: (fn) => fn,
    useMemo: (fn) => fn(),
    useEffect: (fn) => {
      fn();
    },
    useRef: (value) => ({ current: value }),
    useState: (initial) => {
      let value = typeof initial === "function" ? initial() : initial;
      return [value, (next) => {
        value = typeof next === "function" ? next(value) : next;
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

function instantiate(createThreadRequest) {
  const context = {
    console,
    useQuery: () => ({ data: { threads: [] }, isLoading: false }),
    React: createReactStub(),
    createThreadRequest,
    deleteThreadRequest: async () => {},
    listThreads: async () => ({ threads: [] }),
    queryClient: { invalidateQueries: () => {} },
    globalThis: {},
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
