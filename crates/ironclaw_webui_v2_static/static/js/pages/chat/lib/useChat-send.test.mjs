import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

import { messagesFromTimeline } from "./history-messages.js";
import {
  addPending,
  recordAcceptedMessageRef,
  removePending,
} from "./pending-messages.js";

function useChatSourceForTest() {
  const source = readFileSync(
    new URL("../hooks/useChat.js", import.meta.url),
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
    lines.push(line.replace("export function useChat", "function useChat"));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { useChat };`;
}

function createReactStub({ initialByIndex = new Map(), setCalls = [] } = {}) {
  let stateIndex = 0;
  return {
    useCallback: (fn) => fn,
    useEffect: () => {},
    useRef: (value) => ({ current: value }),
    useState: (initial) => {
      const index = stateIndex++;
      let value = initialByIndex.has(index)
        ? initialByIndex.get(index)
        : typeof initial === "function"
          ? initial()
          : initial;
      return [
        value,
        (next) => {
          value = typeof next === "function" ? next(value) : next;
          setCalls.push({ index, value });
        },
      ];
    },
  };
}

test("useChat.send: accepted ref reconciles pending message on timeline reload", async () => {
  const threadId = "thread-1";
  let renderedMessages = [];
  let loadHistory;

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub(),
    addPending,
    cancelRunRequest: async () => {},
    clearTimeout,
    createThreadRequest: async () => {
      throw new Error("thread should already exist");
    },
    globalThis: {},
    queryClient: { invalidateQueries: () => {} },
    recordAcceptedMessageRef,
    removePending,
    resolveGateRequest: async () => {},
    sendMessage: async () => ({
      accepted_message_ref: "msg:message-1",
      run_id: "run-1",
      status: "queued",
      thread_id: threadId,
    }),
    setInterval,
    setTimeout,
    submitManualToken: async () => {},
    useChatEvents: () => () => {},
    useHistory: (_threadId, options) => {
      loadHistory = async () => {
        const pendingMessages = options.getPendingMessages();
        renderedMessages = messagesFromTimeline(
          [
            {
              message_id: "message-1",
              kind: "user",
              content: "check my calendar",
              sequence: 1,
              status: "accepted",
            },
          ],
          pendingMessages,
        );
        options.setPendingMessages([]);
      };

      return {
        messages: renderedMessages,
        hasMore: false,
        nextCursor: null,
        isLoading: false,
        loadHistory,
        setMessages: (updater) => {
          renderedMessages =
            typeof updater === "function" ? updater(renderedMessages) : updater;
        },
      };
    },
    useSSE: () => ({ status: "idle" }),
  };

  vm.runInNewContext(useChatSourceForTest(), context);

  const chat = context.globalThis.__testExports.useChat(threadId);
  await chat.send("check my calendar");

  assert.equal(renderedMessages.length, 1);
  assert.equal(renderedMessages[0].id, "pending-1");
  assert.equal(renderedMessages[0].role, "user");
  assert.equal(renderedMessages[0].content, "check my calendar");
  assert.equal(renderedMessages[0].isOptimistic, true);
  assert.equal(renderedMessages[0].timelineMessageId, "message-1");

  await loadHistory();

  assert.deepEqual(
    renderedMessages.map((message) => message.id),
    ["msg-message-1"],
  );
});

test("useChat.cancelRun clears local state before cancel request resolves", async () => {
  const threadId = "thread-1";
  const stateUpdates = [];
  let cancelRequest = null;
  let resolveCancelRequest;

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({
      // useChat state call order: cooldownUntil, now, activeRun,
      // isProcessing, pendingGate.
      initialByIndex: new Map([
        [2, { runId: "run-1", threadId, status: "running" }],
        [3, true],
        [4, { runId: "run-1", gateRef: "gate-1" }],
      ]),
      setCalls: stateUpdates,
    }),
    addPending,
    cancelRunRequest: async (request) => {
      cancelRequest = request;
      return new Promise((resolve) => {
        resolveCancelRequest = resolve;
      });
    },
    clearTimeout,
    createThreadRequest: async () => {
      throw new Error("createThread should not run");
    },
    globalThis: {},
    queryClient: { invalidateQueries: () => {} },
    recordAcceptedMessageRef,
    removePending,
    resolveGateRequest: async () => {},
    sendMessage: async () => {
      throw new Error("sendMessage should not run");
    },
    setInterval,
    setTimeout,
    submitManualToken: async () => {},
    useChatEvents: () => () => {},
    useHistory: () => ({
      messages: [],
      hasMore: false,
      nextCursor: null,
      isLoading: false,
      loadHistory: () => {},
      setMessages: () => {},
    }),
    useSSE: () => ({ status: "idle" }),
  };

  vm.runInNewContext(useChatSourceForTest(), context);

  const chat = context.globalThis.__testExports.useChat(threadId);
  const cancelPromise = chat.cancelRun("user_requested");

  assert.equal(cancelRequest.threadId, threadId);
  assert.equal(cancelRequest.runId, "run-1");
  assert.equal(cancelRequest.reason, "user_requested");
  assert.deepEqual(stateUpdates.slice(0, 3), [
    { index: 4, value: null },
    { index: 3, value: false },
    { index: 2, value: null },
  ]);

  resolveCancelRequest({});
  await cancelPromise;
});
