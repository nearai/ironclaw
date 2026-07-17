// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";
import {
  carryFinalAssistantOrderFlags,
  isFinalAssistantMessage,
  isRunActivityMessage,
} from "./stream-order-memory";

function useHistorySourceForTest() {
  const helpers = readFileSync(
    new URL("./stream-order-memory.ts", import.meta.url),
    "utf8",
  );
  const source = readFileSync(
    new URL("../hooks/useHistory.ts", import.meta.url),
    "utf8",
  );
  const helperLines = [];
  for (const line of helpers.split("\n")) {
    if (line.startsWith("export function ")) {
      helperLines.push(line.replace("export function ", "function "));
      continue;
    }
    helperLines.push(line);
  }
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
  return `${helperLines.join("\n")}\n${lines.join(
    "\n",
  )}\nglobalThis.__testExports = { clearHistoryCache, useHistory, mergeFullRefresh, nextCursorAfterFullRefresh, cursorPageCanMerge };`;
}

function createReactStub({ setCalls = [] } = {}) {
  return {
    useCallback: (fn) => fn,
    useEffect: (fn) => {
      fn();
    },
    useRef: (value) => ({ current: value }),
    useState: (initial) => {
      let value = typeof initial === "function" ? initial() : initial;
      return [
        value,
        (next) => {
          value = typeof next === "function" ? next(value) : next;
          setCalls.push(value);
        },
      ];
    },
  };
}

function createPersistentReactStub({ setCalls = [] } = {}) {
  let stateIndex = 0;
  let refIndex = 0;
  const stateSlots = [];
  const refSlots = [];
  return {
    __beginRender: () => {
      stateIndex = 0;
      refIndex = 0;
    },
    useCallback: (fn) => fn,
    useEffect: () => {},
    useRef: (value) => {
      const index = refIndex++;
      const ref = refSlots[index] || { current: value };
      refSlots[index] = ref;
      return ref;
    },
    useState: (initial) => {
      const index = stateIndex++;
      const slot = stateSlots[index] || {
        value: typeof initial === "function" ? initial() : initial,
      };
      stateSlots[index] = slot;
      return [
        slot.value,
        (next) => {
          slot.value = typeof next === "function" ? next(slot.value) : next;
          setCalls.push(slot.value);
        },
      ];
    },
  };
}

async function flushMicrotasks() {
  await Promise.resolve();
  await Promise.resolve();
}

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

test("useHistory records a load error when timeline fetch fails", async () => {
  const setCalls = [];
  const consoleErrors = [];
  const context = {
    console: {
      error: (...args) => consoleErrors.push(args),
    },
    fetchTimeline: async () => {
      throw new Error("timeline unavailable");
    },
    authScope: () => "test-user",
    globalThis: {},
    messagesFromTimeline: () => {
      throw new Error("failed timeline should not be transformed");
    },
    React: createReactStub({ setCalls }),
  };

  vm.runInNewContext(useHistorySourceForTest(), context);
  context.globalThis.__testExports.useHistory("thread-1", {});
  await flushMicrotasks();

  assert.equal(setCalls.at(-1).isLoading, false);
  assert.equal(
    setCalls.at(-1).loadError,
    "chat.history.loadFailed",
  );
  assert.equal(consoleErrors.length, 1);
});

test("useHistory starts an older-page load while latest refresh is in flight", async () => {
  const latestPage = deferred();
  const olderPage = deferred();
  const fetchCalls = [];
  const context = {
    console,
    fetchTimeline: ({ cursor }) => {
      fetchCalls.push(cursor || null);
      return cursor ? olderPage.promise : latestPage.promise;
    },
    globalThis: {},
    messagesFromTimeline: (records) =>
      records.map((record) => ({
        id: record.id,
        role: "user",
        sequence: record.sequence,
      })),
    React: createReactStub(),
    authScope: () => "test-user",
  };

  vm.runInNewContext(useHistorySourceForTest(), context);
  context.globalThis.__testExports.clearHistoryCache();
  const history = context.globalThis.__testExports.useHistory("thread-page", {});

  assert.deepEqual(fetchCalls, [null]);

  const loadOlder = history.loadHistory("cursor-older");
  assert.deepEqual(
    fetchCalls,
    [null, "cursor-older"],
    "a background latest-page refresh must not drop an explicit cursor load",
  );

  latestPage.resolve({
    messages: [{ id: "newer", sequence: 20 }],
    next_cursor: "cursor-older",
  });
  olderPage.resolve({
    messages: [{ id: "older", sequence: 10 }],
    next_cursor: null,
  });
  await loadOlder;
  await flushMicrotasks();
});

test("useHistory discards stale cursor pages after a refresh changes cursor windows", async () => {
  const initialLatest = deferred();
  const staleOlderPage = deferred();
  const refreshedLatest = deferred();
  const fetchCalls = [];
  const setCalls = [];
  let latestCalls = 0;
  const context = {
    console,
    fetchTimeline: ({ cursor }) => {
      fetchCalls.push(cursor || null);
      if (!cursor) {
        latestCalls += 1;
        return latestCalls === 1
          ? initialLatest.promise
          : refreshedLatest.promise;
      }
      if (cursor === "cursor-before-151") return staleOlderPage.promise;
      throw new Error(`unexpected cursor ${cursor}`);
    },
    globalThis: {},
    messagesFromTimeline: (records) =>
      records.map((record) => ({
        id: record.id,
        role: "user",
        sequence: record.sequence,
      })),
    React: createReactStub({ setCalls }),
    authScope: () => "test-user",
  };

  vm.runInNewContext(useHistorySourceForTest(), context);
  context.globalThis.__testExports.clearHistoryCache();
  const history = context.globalThis.__testExports.useHistory(
    "thread-stale-cursor",
    {},
  );

  initialLatest.resolve({
    messages: [
      { id: "msg-151", sequence: 151 },
      { id: "msg-200", sequence: 200 },
    ],
    next_cursor: "cursor-before-151",
  });
  await flushMicrotasks();

  const loadOlder = history.loadHistory("cursor-before-151");
  const refresh = history.loadHistory();
  refreshedLatest.resolve({
    messages: [
      { id: "msg-251", sequence: 251 },
      { id: "msg-300", sequence: 300 },
    ],
    next_cursor: "cursor-before-251",
  });
  await refresh;
  await flushMicrotasks();

  staleOlderPage.resolve({
    messages: [
      { id: "msg-101", sequence: 101 },
      { id: "msg-150", sequence: 150 },
    ],
    next_cursor: null,
  });
  await loadOlder;
  await flushMicrotasks();

  assert.deepEqual(fetchCalls, [
    null,
    "cursor-before-151",
    null,
  ]);
  assert.equal(setCalls.at(-1).nextCursor, "cursor-before-251");
  assert.deepEqual(
    setCalls.at(-1).messages.map((message) => message.id),
    ["msg-251", "msg-300"],
  );
  assert.equal(setCalls.at(-1).isLoading, false);
});

test("useHistory merges cursor pages connected by raw filtered boundary records", async () => {
  const setCalls = [];
  const fetchCalls = [];
  const context = {
    console,
    fetchTimeline: async ({ cursor }) => {
      fetchCalls.push(cursor || null);
      if (!cursor) {
        return {
          messages: [
            {
              id: "hidden-current-boundary",
              kind: "tool_result_reference",
              sequence: 151,
            },
            { id: "current-visible", sequence: 152 },
          ],
          next_cursor: "cursor-before-151-new",
        };
      }
      assert.equal(cursor, "cursor-before-151-old");
      return {
        messages: [
          { id: "older-visible", sequence: 149 },
          {
            id: "hidden-older-boundary",
            kind: "tool_result_reference",
            sequence: 150,
          },
        ],
        next_cursor: "cursor-before-101",
      };
    },
    globalThis: {},
    messagesFromTimeline: (records) =>
      records
        .filter((record) => record.kind !== "tool_result_reference")
        .map((record) => ({
          id: record.id,
          role: "user",
          sequence: record.sequence,
        })),
    React: createReactStub({ setCalls }),
    authScope: () => "test-user",
  };

  vm.runInNewContext(useHistorySourceForTest(), context);
  context.globalThis.__testExports.clearHistoryCache();
  const history = context.globalThis.__testExports.useHistory(
    "thread-filtered-cursor",
    {},
  );
  await flushMicrotasks();

  await history.loadHistory("cursor-before-151-old");
  await flushMicrotasks();

  assert.deepEqual(fetchCalls, [null, "cursor-before-151-old"]);
  assert.equal(setCalls.at(-1).nextCursor, "cursor-before-101");
  assert.deepEqual(
    JSON.parse(
      JSON.stringify(setCalls.at(-1).messages.map((message) => message.id)),
    ),
    ["older-visible", "current-visible"],
  );
});

test("cursor page merge guard requires the current cursor or a connected page", () => {
  const context = { globalThis: {}, React: createReactStub() };
  vm.runInNewContext(useHistorySourceForTest(), context);
  const { cursorPageCanMerge } = context.globalThis.__testExports;

  assert.equal(
    cursorPageCanMerge(
      "cursor-before-151",
      [{ id: "msg-101", sequence: 101 }],
      [{ id: "msg-251", sequence: 251 }],
      "cursor-before-251",
    ),
    false,
  );
  assert.equal(
    cursorPageCanMerge(
      "cursor-before-151",
      [{ id: "msg-101", sequence: 101 }],
      [{ id: "msg-251", sequence: 251 }],
      "cursor-before-151",
    ),
    true,
  );
  assert.equal(
    cursorPageCanMerge(
      "cursor-before-151",
      [
        { id: "msg-101", sequence: 101 },
        { id: "msg-150", sequence: 150 },
      ],
      [{ id: "msg-151", sequence: 151 }],
      "cursor-before-101",
    ),
    true,
  );
  assert.equal(
    cursorPageCanMerge(
      "cursor-before-151-old",
      [{ id: "msg-149", sequence: 149 }],
      [{ id: "msg-152", sequence: 152 }],
      "cursor-before-151-new",
      {
        pageSequenceWindow: { oldest: 101, newest: 150 },
        currentSequenceWindow: { oldest: 151, newest: 200 },
      },
    ),
    true,
  );
});

test("useHistory full refresh preserves older rows and cursor across raw filtered boundaries", async () => {
  const setCalls = [];
  const fetchCalls = [];
  let latestCalls = 0;
  const context = {
    console,
    fetchTimeline: async ({ cursor }) => {
      fetchCalls.push(cursor || null);
      assert.equal(cursor || null, null);
      latestCalls += 1;
      if (latestCalls === 1) {
        return {
          messages: [
            { id: "current-visible", sequence: 99 },
            {
              id: "hidden-current-boundary",
              kind: "tool_result_reference",
              sequence: 100,
            },
          ],
          next_cursor: "cursor-before-99",
        };
      }
      return {
        messages: [
          {
            id: "hidden-fresh-boundary",
            kind: "tool_result_reference",
            sequence: 101,
          },
          { id: "fresh-visible", sequence: 102 },
        ],
        next_cursor: "cursor-before-101",
      };
    },
    globalThis: {},
    messagesFromTimeline: (records) =>
      records
        .filter((record) => record.kind !== "tool_result_reference")
        .map((record) => ({
          id: record.id,
          role: "user",
          sequence: record.sequence,
        })),
    React: createReactStub({ setCalls }),
    authScope: () => "test-user",
  };

  vm.runInNewContext(useHistorySourceForTest(), context);
  context.globalThis.__testExports.clearHistoryCache();
  const history = context.globalThis.__testExports.useHistory(
    "thread-filtered-refresh",
    {},
  );
  await flushMicrotasks();

  await history.loadHistory();
  await flushMicrotasks();

  assert.deepEqual(fetchCalls, [null, null]);
  assert.equal(setCalls.at(-1).nextCursor, "cursor-before-99");
  assert.deepEqual(
    JSON.parse(
      JSON.stringify(setCalls.at(-1).messages.map((message) => message.id)),
    ),
    ["current-visible", "fresh-visible"],
  );
});

test("useHistory tags messages with the thread they belong to (messagesThreadId)", async () => {
  // The cross-thread pairing-panel fix (useChat derive effect) depends on
  // useHistory reporting which thread its `messages` represent, so a consumer can
  // tell when `messages` still holds the previous thread's timeline during a
  // navigation. Pin that the tag is set from the first render and after a load.
  const threadId = "thread-tagged";
  const setCalls = [];
  const context = {
    console,
    fetchTimeline: async () => ({
      messages: [{ message_id: "m1", kind: "user", status: "accepted", content: "hi" }],
      next_cursor: null,
    }),
    globalThis: {},
    messagesFromTimeline: () => [{ id: "msg-m1", role: "user", content: "hi" }],
    React: createReactStub({ setCalls }),
    authScope: () => "test-user",
  };

  vm.runInNewContext(useHistorySourceForTest(), context);
  context.globalThis.__testExports.clearHistoryCache();
  const history = context.globalThis.__testExports.useHistory(threadId, {});

  // Set before any async load resolves: messages are never exposed without the
  // thread they belong to.
  assert.equal(history.messagesThreadId, threadId);

  await flushMicrotasks();

  const loaded = setCalls.at(-1);
  assert.equal(loaded.messagesThreadId, threadId);
  assert.deepEqual(
    loaded.messages.map((message) => message.id),
    ["msg-m1"],
  );
});

test("useHistory full refresh preserves SSE-only activity messages", async () => {
  const threadId = "thread-activity";
  const runId = "run-activity";
  const setCalls = [];
  const context = {
    console,
    fetchTimeline: async () => ({
      messages: [
        {
          message_id: "assistant-1",
          kind: "assistant",
          status: "finalized",
          content: "I could not search.",
          turn_run_id: runId,
        },
      ],
      next_cursor: null,
    }),
    globalThis: {},
    messagesFromTimeline: () => [
      {
        id: "msg-assistant-1",
        role: "assistant",
        content: "I could not search.",
        status: "finalized",
        kind: "assistant",
        isFinalReply: true,
        turnRunId: runId,
      },
    ],
    React: createReactStub({ setCalls }),
    authScope: () => "test-user",
  };

  vm.runInNewContext(useHistorySourceForTest(), context);
  context.globalThis.__testExports.clearHistoryCache();
  const history = context.globalThis.__testExports.useHistory(threadId, {});
  await flushMicrotasks();

  history.setMessages((messages) => [
    ...messages,
    {
      id: "tool-search",
      role: "tool_activity",
      turnRunId: runId,
      toolName: "web-access.search",
      toolStatus: "error",
      toolError: "authorization",
    },
  ]);
  await history.loadHistory();
  await flushMicrotasks();

  assert.deepEqual(
    JSON.parse(JSON.stringify(setCalls.at(-1).messages.map((message) => message.id))),
    ["msg-assistant-1", "tool-search"],
  );
  assert.equal(setCalls.at(-1).messages[1].toolStatus, "error");
});

test("useHistory can seed a newly-created thread before navigation", async () => {
  const setCalls = [];
  const context = {
    console,
    fetchTimeline: async () => ({ messages: [], next_cursor: null }),
    globalThis: {},
    messagesFromTimeline: () => [],
    React: createReactStub({ setCalls }),
    authScope: () => "test-user",
  };

  vm.runInNewContext(useHistorySourceForTest(), context);
  context.globalThis.__testExports.clearHistoryCache();

  const draftHistory = context.globalThis.__testExports.useHistory(null, {});
  draftHistory.seedThreadMessages("thread-new", [
    {
      id: "pending-1",
      role: "user",
      content: "tell me a joke",
      timestamp: "2026-06-25T07:17:00.000Z",
      isOptimistic: true,
    },
  ]);

  const threadHistory = context.globalThis.__testExports.useHistory("thread-new", {});
  await flushMicrotasks();

  assert.deepEqual(JSON.parse(JSON.stringify(threadHistory.messages)), [
    {
      id: "pending-1",
      role: "user",
      content: "tell me a joke",
      timestamp: "2026-06-25T07:17:00.000Z",
      isOptimistic: true,
    },
  ]);
  assert.deepEqual(JSON.parse(JSON.stringify(setCalls.at(-1).messages)), [
    {
      id: "pending-1",
      role: "user",
      content: "tell me a joke",
      timestamp: "2026-06-25T07:17:00.000Z",
      isOptimistic: true,
    },
  ]);
});

test("useHistory clears visible messages immediately when switching to an uncached thread", () => {
  const ReactStub = createPersistentReactStub();
  const context = {
    console,
    fetchTimeline: async () => {
      throw new Error("render reset should not need timeline fetch");
    },
    globalThis: {},
    messagesFromTimeline: () => [],
    React: ReactStub,
    authScope: () => "test-user",
  };

  vm.runInNewContext(useHistorySourceForTest(), context);
  context.globalThis.__testExports.clearHistoryCache();

  const renderHistory = (threadId) => {
    ReactStub.__beginRender();
    return context.globalThis.__testExports.useHistory(threadId, {});
  };

  let history = renderHistory("thread-old");
  history.setMessages([
    {
      id: "pending-old",
      role: "user",
      content: "old thread message",
    },
  ]);
  history = renderHistory("thread-old");
  assert.equal(history.messages.length, 1);

  renderHistory("thread-new");
  history = renderHistory("thread-new");

  assert.deepEqual(JSON.parse(JSON.stringify(history.messages)), []);
  assert.equal(history.isLoading, true);
});

test("useHistory seedThreadMessages updates an accepted first message by timeline id", async () => {
  const context = {
    console,
    fetchTimeline: async () => new Promise(() => {}),
    globalThis: {},
    messagesFromTimeline: () => [],
    React: createReactStub(),
    authScope: () => "test-user",
  };

  vm.runInNewContext(useHistorySourceForTest(), context);
  context.globalThis.__testExports.clearHistoryCache();

  const draftHistory = context.globalThis.__testExports.useHistory(null, {});
  draftHistory.seedThreadMessages("thread-new", [
    {
      id: "pending-1",
      role: "user",
      content: "tell me a joke",
      timestamp: "2026-06-25T07:17:00.000Z",
    },
  ]);
  draftHistory.seedThreadMessages("thread-new", (messages) =>
    messages.map((message) =>
      message.id === "pending-1"
        ? { ...message, timelineMessageId: "message-1" }
        : message,
    ),
  );

  const threadHistory = context.globalThis.__testExports.useHistory("thread-new", {});
  assert.equal(threadHistory.messages[0].timelineMessageId, "message-1");
});

test("useHistory seedThreadMessages updates the mounted target thread", async () => {
  const setCalls = [];
  const context = {
    console,
    fetchTimeline: async () => new Promise(() => {}),
    globalThis: {},
    messagesFromTimeline: () => [],
    React: createReactStub({ setCalls }),
    authScope: () => "test-user",
  };

  vm.runInNewContext(useHistorySourceForTest(), context);
  context.globalThis.__testExports.clearHistoryCache();

  const threadHistory = context.globalThis.__testExports.useHistory("thread-visible", {});
  threadHistory.seedThreadMessages("thread-visible", [
    {
      id: "pending-1",
      role: "user",
      content: "visible update",
      timestamp: "2026-06-25T07:17:00.000Z",
      isOptimistic: true,
    },
  ]);

  assert.deepEqual(JSON.parse(JSON.stringify(setCalls.at(-1).messages)), [
    {
      id: "pending-1",
      role: "user",
      content: "visible update",
      timestamp: "2026-06-25T07:17:00.000Z",
      isOptimistic: true,
    },
  ]);
  assert.equal(setCalls.at(-1).messagesThreadId, "thread-visible");
});

test("useHistory setMessages restamps messages onto the active thread", async () => {
  const setCalls = [];
  const context = {
    console,
    fetchTimeline: async () => new Promise(() => {}),
    globalThis: {},
    messagesFromTimeline: () => [],
    React: createReactStub({ setCalls }),
    authScope: () => "test-user",
  };

  vm.runInNewContext(useHistorySourceForTest(), context);
  context.globalThis.__testExports.clearHistoryCache();

  const threadHistory = context.globalThis.__testExports.useHistory("thread-active", {});
  threadHistory.setMessages([
    {
      id: "pending-1",
      role: "user",
      content: "active update",
    },
  ]);

  assert.equal(setCalls.at(-1).messagesThreadId, "thread-active");
});

test("useHistory stamps messagesThreadId during synchronous thread switches", async () => {
  const setCalls = [];
  const React = createPersistentReactStub({ setCalls });
  const context = {
    console,
    fetchTimeline: async () => new Promise(() => {}),
    globalThis: {},
    messagesFromTimeline: () => [],
    React,
    authScope: () => "test-user",
  };

  vm.runInNewContext(useHistorySourceForTest(), context);
  context.globalThis.__testExports.clearHistoryCache();

  React.__beginRender();
  context.globalThis.__testExports.useHistory("thread-old", {});

  React.__beginRender();
  context.globalThis.__testExports.useHistory("thread-new", {});

  assert.equal(setCalls.at(-1).messagesThreadId, "thread-new");
});

test("useHistory full refresh preserves unnumbered live gate activity after timeline tools", async () => {
  const threadId = "thread-activity-order";
  const runId = "run-activity-order";
  const setCalls = [];
  const timelineMessages = [
    {
      id: "tool-extension-a",
      role: "tool_activity",
      invocationId: "extension-a",
      turnRunId: runId,
      toolName: "extension_search",
      toolStatus: "success",
      activityOrder: 2,
    },
    {
      id: "tool-extension-b",
      role: "tool_activity",
      invocationId: "extension-b",
      turnRunId: runId,
      toolName: "extension_search",
      toolStatus: "success",
      activityOrder: 3,
    },
  ];
  const context = {
    console,
    fetchTimeline: async () => ({
      messages: [],
      next_cursor: null,
    }),
    globalThis: {},
    messagesFromTimeline: () => timelineMessages,
    React: createReactStub({ setCalls }),
    authScope: () => "test-user",
  };

  vm.runInNewContext(useHistorySourceForTest(), context);
  context.globalThis.__testExports.clearHistoryCache();
  const history = context.globalThis.__testExports.useHistory(threadId, {});
  await flushMicrotasks();

  history.setMessages((messages) => [
    {
      id: "tool-gate-web-search",
      role: "tool_activity",
      invocationId: "gate-web-search",
      turnRunId: runId,
      toolName: "search",
      toolStatus: "running",
    },
    ...messages,
  ]);
  await history.loadHistory();
  await flushMicrotasks();

  assert.deepEqual(
    JSON.parse(JSON.stringify(setCalls.at(-1).messages.map((message) => [
      message.id,
      message.activityOrder,
    ]))),
    [
      ["tool-extension-a", 2],
      ["tool-extension-b", 3],
      ["tool-gate-web-search", null],
    ],
  );
});

test("mergeFullRefresh keeps requested client-only bubbles and lets the timeline win otherwise", () => {
  const context = { globalThis: {}, React: createReactStub() };
  vm.runInNewContext(useHistorySourceForTest(), context);
  const { mergeFullRefresh } = context.globalThis.__testExports;

  const timeline = [
    { id: "msg-user-1", role: "user" },
    { id: "tool-abc", role: "tool_activity", toolParameters: "{}", toolResultPreview: "ok" },
    { id: "msg-assistant-1", role: "assistant" },
  ];
  const current = [
    { id: "msg-user-1", role: "user" },
    { id: "tool-abc", role: "tool_activity", toolParameters: null, toolResultPreview: null },
    { id: "err-run-1", role: "error", content: "run failed" },
  ];

  const merged = mergeFullRefresh(timeline, current, {
    preserveClientOnly: true,
  });

  // Timeline order is authoritative and the rich tool card replaces the
  // sparse live one; the client-only err-* bubble is preserved at the end.
  assert.equal(
    merged.map((m) => m.id).join(","),
    "msg-user-1,tool-abc,msg-assistant-1,err-run-1",
  );
  const toolCard = merged.find((m) => m.id === "tool-abc");
  assert.equal(toolCard.toolParameters, "{}");
  assert.equal(toolCard.toolResultPreview, "ok");
});

test("mergeFullRefresh anchors preserved runtime bubbles at their original positions", () => {
  const context = { globalThis: {}, React: createReactStub() };
  vm.runInNewContext(useHistorySourceForTest(), context);
  const { mergeFullRefresh } = context.globalThis.__testExports;

  const merged = mergeFullRefresh(
    [
      { id: "msg-user-1", role: "user" },
      { id: "msg-assistant-1", role: "assistant" },
      { id: "msg-user-2", role: "user" },
    ],
    [
      { id: "msg-user-1", role: "user" },
      { id: "thinking-live", role: "thinking", content: "working" },
      { id: "msg-assistant-1", role: "assistant" },
      { id: "err-run-1", role: "error", content: "run failed" },
      { id: "msg-user-2", role: "user" },
    ],
    {
      preserveClientOnly: true,
    },
  );

  assert.equal(
    merged.map((m) => m.id).join(","),
    "msg-user-1,thinking-live,msg-assistant-1,msg-user-2,err-run-1",
  );
});

test("mergeFullRefresh preserves paginated older timeline messages", () => {
  const context = { globalThis: {}, React: createReactStub() };
  vm.runInNewContext(useHistorySourceForTest(), context);
  const { mergeFullRefresh } = context.globalThis.__testExports;

  const merged = mergeFullRefresh(
    [
      { id: "msg-newer-user", role: "user", sequence: 50 },
      { id: "tool-newer", role: "tool_activity", sequence: 51 },
      { id: "msg-newer-assistant", role: "assistant", sequence: 52 },
    ],
    [
      { id: "msg-older-user", role: "user", sequence: 10 },
      { id: "tool-older", role: "tool_activity", sequence: 11 },
      { id: "msg-newer-user", role: "user", sequence: 50 },
    ],
  );

  assert.equal(
    merged.map((message) => message.id).join(","),
    "msg-older-user,tool-older,msg-newer-user,tool-newer,msg-newer-assistant",
  );
});

test("mergeFullRefresh sorts preserved older timeline messages before pending rows", () => {
  const context = { globalThis: {}, React: createReactStub() };
  vm.runInNewContext(useHistorySourceForTest(), context);
  const { mergeFullRefresh } = context.globalThis.__testExports;

  const merged = mergeFullRefresh(
    [
      { id: "msg-newer-user", role: "user", sequence: 50 },
      { id: "pending-1", role: "user", isOptimistic: true },
    ],
    [
      { id: "msg-older-user", role: "user", sequence: 10 },
      { id: "msg-newer-user", role: "user", sequence: 50 },
      { id: "pending-1", role: "user", isOptimistic: true },
    ],
  );

  assert.equal(
    merged.map((message) => message.id).join(","),
    "msg-older-user,msg-newer-user,pending-1",
  );
});

test("mergeFullRefresh drops older timeline messages when the fresh page skipped ahead", () => {
  const context = { globalThis: {}, React: createReactStub() };
  vm.runInNewContext(useHistorySourceForTest(), context);
  const { mergeFullRefresh } = context.globalThis.__testExports;

  const merged = mergeFullRefresh(
    [
      { id: "msg-fresh-user", role: "user", sequence: 151 },
      { id: "msg-fresh-assistant", role: "assistant", sequence: 200 },
    ],
    [
      { id: "msg-stale-user", role: "user", sequence: 51 },
      { id: "tool-stale", role: "tool_activity", sequence: 75 },
      { id: "msg-stale-assistant", role: "assistant", sequence: 100 },
    ],
  );

  assert.equal(
    merged.map((message) => message.id).join(","),
    "msg-fresh-user,msg-fresh-assistant",
  );
});

test("nextCursorAfterFullRefresh does not rewind past loaded older pages", () => {
  const context = { globalThis: {}, React: createReactStub() };
  vm.runInNewContext(useHistorySourceForTest(), context);
  const { nextCursorAfterFullRefresh } = context.globalThis.__testExports;

  assert.equal(
    nextCursorAfterFullRefresh(
      [{ id: "msg-newer", sequence: 50 }],
      [
        { id: "msg-older", sequence: 10 },
        { id: "msg-newer", sequence: 50 },
      ],
      "cursor-for-page-two",
      "cursor-for-page-three",
    ),
    "cursor-for-page-three",
  );

  assert.equal(
    nextCursorAfterFullRefresh(
      [{ id: "msg-newer", sequence: 50 }],
      [{ id: "msg-newer", sequence: 50 }],
      "cursor-for-page-two",
      null,
    ),
    "cursor-for-page-two",
  );
});

test("nextCursorAfterFullRefresh uses the fresh cursor after a non-overlapping refresh", () => {
  const context = { globalThis: {}, React: createReactStub() };
  vm.runInNewContext(useHistorySourceForTest(), context);
  const { nextCursorAfterFullRefresh } = context.globalThis.__testExports;

  assert.equal(
    nextCursorAfterFullRefresh(
      [
        { id: "msg-fresh-start", sequence: 151 },
        { id: "msg-fresh-end", sequence: 200 },
      ],
      [
        { id: "msg-current-start", sequence: 51 },
        { id: "msg-current-end", sequence: 100 },
      ],
      "cursor-before-151",
      "cursor-before-51",
    ),
    "cursor-before-151",
  );

  assert.equal(
    nextCursorAfterFullRefresh(
      [{ id: "msg-fresh", sequence: 151 }],
      [{ id: "msg-current-adjacent", sequence: 150 }],
      "cursor-before-151",
      "cursor-before-150",
    ),
    "cursor-before-150",
  );
});

test("mergeFullRefresh carries optimistic timestamps onto confirmed messages", () => {
  const context = { globalThis: {}, React: createReactStub() };
  vm.runInNewContext(useHistorySourceForTest(), context);
  const { mergeFullRefresh } = context.globalThis.__testExports;

  const merged = mergeFullRefresh(
    [
      {
        id: "msg-message-1",
        role: "user",
        content: "tell me a joke",
      },
    ],
    [
      {
        id: "pending-1",
        role: "user",
        content: "tell me a joke",
        timestamp: "2026-06-25T07:17:00.000Z",
        timelineMessageId: "message-1",
        isOptimistic: true,
      },
    ],
  );

  assert.equal(merged.length, 1);
  assert.equal(merged[0].id, "msg-message-1");
  assert.equal(merged[0].timestamp, "2026-06-25T07:17:00.000Z");
});

test("mergeFullRefresh carries live assistant timestamps onto confirmed replies", () => {
  const context = { globalThis: {}, React: createReactStub() };
  vm.runInNewContext(useHistorySourceForTest(), context);
  const { mergeFullRefresh } = context.globalThis.__testExports;

  const merged = mergeFullRefresh(
    [
      {
        id: "msg-assistant-1",
        role: "assistant",
        content: "Here's one.",
        isFinalReply: true,
        turnRunId: "run-1",
      },
    ],
    [
      {
        id: "reply-run-1",
        role: "assistant",
        content: "Here's one.",
        timestamp: "2026-06-25T07:18:00.000Z",
        isFinalReply: true,
        turnRunId: "run-1",
      },
    ],
  );

  assert.equal(merged.length, 1);
  assert.equal(merged[0].id, "msg-assistant-1");
  assert.equal(merged[0].timestamp, "2026-06-25T07:18:00.000Z");
});

test("mergeFullRefresh keeps same-run activity before confirmed assistant replies", () => {
  const context = {
    globalThis: {},
    React: createReactStub(),
    carryFinalAssistantOrderFlags,
    isFinalAssistantMessage,
    isRunActivityMessage,
  };
  vm.runInNewContext(useHistorySourceForTest(), context);
  const { mergeFullRefresh } = context.globalThis.__testExports;

  const merged = mergeFullRefresh(
    [
      {
        id: "msg-user-1",
        role: "user",
        turnRunId: "run-1",
      },
      {
        id: "tool-web-search",
        role: "tool_activity",
        toolName: "web_search",
        turnRunId: "run-1",
      },
      {
        id: "msg-assistant-1",
        role: "assistant",
        content: "PRETOOL text and final answer.",
        isFinalReply: true,
        turnRunId: "run-1",
      },
    ],
    [
      {
        id: "msg-user-1",
        role: "user",
        turnRunId: "run-1",
      },
      {
        id: "text-text:run-1",
        role: "assistant",
        content: "PRETOOL text",
        isFinalReply: false,
        turnRunId: "run-1",
      },
      {
        id: "tool-web-search",
        role: "tool_activity",
        toolName: "web_search",
        turnRunId: "run-1",
      },
    ],
  );

  assert.equal(
    merged.map((message) => message.id).join(","),
    "msg-user-1,tool-web-search,msg-assistant-1",
  );
  assert.equal(merged[2].keepFollowingActivityAfter, undefined);
});

test("mergeFullRefresh preserves final assistant activity-order flag by run", () => {
  const context = {
    globalThis: {},
    React: createReactStub(),
    carryFinalAssistantOrderFlags,
    isFinalAssistantMessage,
    isRunActivityMessage,
  };
  vm.runInNewContext(useHistorySourceForTest(), context);
  const { mergeFullRefresh } = context.globalThis.__testExports;

  const merged = mergeFullRefresh(
    [
      {
        id: "msg-user-1",
        role: "user",
        turnRunId: "run-1",
      },
      {
        id: "tool-web-search",
        role: "tool_activity",
        toolName: "web_search",
        turnRunId: "run-1",
      },
      {
        id: "msg-assistant-1",
        role: "assistant",
        content: "confirmed final",
        isFinalReply: true,
        turnRunId: "run-1",
      },
    ],
    [
      {
        id: "reply-run-1",
        role: "assistant",
        content: "live final",
        isFinalReply: true,
        turnRunId: "run-1",
        keepFollowingActivityAfter: true,
      },
    ],
  );

  assert.equal(merged.length, 3);
  assert.equal(merged[2].id, "msg-assistant-1");
  assert.equal(merged[2].keepFollowingActivityAfter, true);
});

test("mergeFullRefresh uses run-settled time for confirmed assistant replies", () => {
  const context = { globalThis: {}, React: createReactStub() };
  vm.runInNewContext(useHistorySourceForTest(), context);
  const { mergeFullRefresh } = context.globalThis.__testExports;

  const merged = mergeFullRefresh(
    [
      {
        id: "msg-assistant-1",
        role: "assistant",
        content: "Here's one.",
        isFinalReply: true,
        turnRunId: "run-1",
      },
    ],
    [],
    {
      finalReplyTimestampByRun: {
        "run-1": "2026-06-25T07:19:00.000Z",
      },
    },
  );

  assert.equal(merged.length, 1);
  assert.equal(merged[0].id, "msg-assistant-1");
  assert.equal(merged[0].timestamp, "2026-06-25T07:19:00.000Z");
});
