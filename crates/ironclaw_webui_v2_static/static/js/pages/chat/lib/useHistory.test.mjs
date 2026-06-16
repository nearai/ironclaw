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
    lines.push(
      line
        .replace("export function clearHistoryCache", "function clearHistoryCache")
        .replace("export function useHistory", "function useHistory"),
    );
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { clearHistoryCache, useHistory };`;
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

test("useHistory full refresh rebases live gate activity after timeline tools", async () => {
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
      activityOrder: 1,
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
      ["tool-gate-web-search", 4],
    ],
  );
});
