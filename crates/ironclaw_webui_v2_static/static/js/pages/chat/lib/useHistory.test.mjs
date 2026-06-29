import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function useHistorySourceForTest() {
  const source = readFileSync(
    new URL("../hooks/useHistory.js", import.meta.url),
    "utf8",
  );
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
  return `${lines.join(
    "\n",
  )}\nglobalThis.__testExports = { clearHistoryCache, useHistory, mergeFullRefresh };`;
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
    "Failed to load conversation history.",
  );
  assert.equal(consoleErrors.length, 1);
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
    { id: "msg-user-1", role: "user", turnRunId: "run-1" },
    {
      id: "tool-abc",
      role: "tool_activity",
      toolParameters: "{}",
      toolResultPreview: "ok",
      turnRunId: "run-1",
    },
    {
      id: "msg-assistant-1",
      role: "assistant",
      isFinalReply: true,
      turnRunId: "run-1",
    },
    { id: "msg-user-2", role: "user", turnRunId: "run-2" },
    {
      id: "msg-assistant-2",
      role: "assistant",
      isFinalReply: true,
      turnRunId: "run-2",
    },
  ];
  const current = [
    { id: "msg-user-1", role: "user", turnRunId: "run-1" },
    {
      id: "tool-abc",
      role: "tool_activity",
      toolParameters: null,
      toolResultPreview: null,
      turnRunId: "run-1",
    },
    {
      id: "err-run-1",
      role: "error",
      content: "run failed",
      turnRunId: "run-1",
    },
  ];

  const merged = mergeFullRefresh(timeline, current, {
    preserveClientOnly: true,
  });

  // Timeline order is authoritative and the rich tool card replaces the
  // sparse live one; the client-only err-* bubble stays anchored to its run.
  assert.equal(
    merged.map((m) => m.id).join(","),
    "msg-user-1,tool-abc,msg-assistant-1,err-run-1,msg-user-2,msg-assistant-2",
  );
  const toolCard = merged.find((m) => m.id === "tool-abc");
  assert.equal(toolCard.toolParameters, "{}");
  assert.equal(toolCard.toolResultPreview, "ok");
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
