import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

import { messagesFromTimeline } from "./history-messages.js";
import { toRenderAttachment, toWireAttachment } from "./attachments.js";
import {
  looksLikeChannelConnectCommand,
  resolveChannelConnectCommand,
} from "../../../lib/channel-connect.js";
import {
  addPending,
  recordAcceptedMessageRef,
  removePending,
  timelineMessageIdFromAcceptedRef,
} from "./pending-messages.js";
import {
  createToolActivityState,
  failGateToolActivity,
  resetToolActivityState,
} from "./tool-activity-state.js";

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

function runUseChatSource(context) {
  Object.assign(context, {
    createToolActivityState,
    failGateToolActivity,
    resetToolActivityState,
    timelineMessageIdFromAcceptedRef,
  });
  vm.runInNewContext(useChatSourceForTest(), context);
}

function createReactStub({
  initialByIndex = new Map(),
  setCalls = [],
  stateSlots = new Map(),
  refs = [],
} = {}) {
  let stateIndex = 0;
  return {
    useCallback: (fn) => fn,
    useEffect: () => {},
    useRef: (value) => {
      const ref = { current: value };
      refs.push(ref);
      return ref;
    },
    useState: (initial) => {
      const index = stateIndex++;
      const slot = stateSlots.get(index) || {
        value: initialByIndex.has(index)
          ? initialByIndex.get(index)
          : typeof initial === "function"
            ? initial()
            : initial,
      };
      stateSlots.set(index, slot);
      return [
        slot.value,
        (next) => {
          slot.value = typeof next === "function" ? next(slot.value) : next;
          setCalls.push({ index, value: slot.value });
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
    toRenderAttachment,
    toWireAttachment,
    cancelRunRequest: async () => {},
    clearTimeout,
    createThreadRequest: async () => {
      throw new Error("thread should already exist");
    },
    globalThis: {},
    listConnectableChannels: async () => {
      throw new Error("ordinary prompts should not fetch connectable channels");
    },
    looksLikeChannelConnectCommand,
    queryClient: {
      fetchQuery: async () => {
        throw new Error("ordinary prompts should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    timelineMessageIdFromAcceptedRef,
    resolveChannelConnectCommand,
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

  runUseChatSource(context);

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

function createSendCaptureContext() {
  let sentBody = null;
  let renderedMessages = [];
  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub(),
    addPending,
    toRenderAttachment,
    toWireAttachment,
    cancelRunRequest: async () => {},
    clearTimeout,
    createThreadRequest: async () => {
      throw new Error("thread should already exist");
    },
    globalThis: {},
    listConnectableChannels: async () => {
      throw new Error("attachment sends should not fetch connectable channels");
    },
    looksLikeChannelConnectCommand,
    queryClient: {
      fetchQuery: async () => {
        throw new Error("attachment sends should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef: () => null,
    removePending,
    timelineMessageIdFromAcceptedRef,
    resolveChannelConnectCommand,
    resolveGateRequest: async () => {},
    sendMessage: async (body) => {
      sentBody = body;
      return {
        accepted_message_ref: "msg:message-1",
        run_id: "run-1",
        status: "queued",
        thread_id: body.threadId,
      };
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
      setMessages: (updater) => {
        renderedMessages =
          typeof updater === "function" ? updater(renderedMessages) : updater;
      },
    }),
    useSSE: () => ({ status: "idle" }),
  };
  return {
    context,
    sentBody: () => sentBody,
    renderedMessages: () => renderedMessages,
  };
}

test("useChat.send: forwards staged attachments to sendMessage in wire shape", async () => {
  const threadId = "thread-1";
  const { context, sentBody } = createSendCaptureContext();

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat(threadId);
  await chat.send("please review", {
    attachments: [
      {
        id: "staged-0",
        filename: "notes.txt",
        mimeType: "text/plain",
        kind: "document",
        sizeBytes: 4,
        sizeLabel: "4 B",
        dataBase64: "bm90ZQ==",
        previewUrl: null,
      },
    ],
  });

  const body = sentBody();
  assert.equal(body.content, "please review");
  assert.equal(body.threadId, threadId);
  // The wire shape the v2 ingress (`WebUiInboundAttachment`) expects —
  // never the staged camelCase object, never `[non_text_content]`.
  assert.deepEqual(body.attachments, [
    { mime_type: "text/plain", filename: "notes.txt", data_base64: "bm90ZQ==" },
  ]);
});

test("useChat.send: stamps render attachments on the optimistic message", async () => {
  const threadId = "thread-1";
  const { context, renderedMessages } = createSendCaptureContext();

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat(threadId);
  await chat.send("look at this", {
    attachments: [
      {
        id: "staged-7",
        filename: "shot.png",
        mimeType: "image/png",
        kind: "image",
        sizeBytes: 11,
        sizeLabel: "11 B",
        dataBase64: "cG5n",
        previewUrl: "data:image/png;base64,cG5n",
      },
    ],
  });

  // The optimistic bubble carries the render shape so the card/thumbnail
  // shows immediately, before the timeline projection returns.
  const optimistic = renderedMessages().find((m) => m.isOptimistic);
  assert.ok(optimistic, "an optimistic user message is rendered");
  assert.deepEqual(optimistic.attachments, [
    {
      id: "staged-7",
      filename: "shot.png",
      mime_type: "image/png",
      kind: "image",
      size_label: "11 B",
      preview_url: "data:image/png;base64,cG5n",
    },
  ]);
});

test("useChat.send: target-thread send does not append into active thread", async () => {
  const currentThreadId = "thread-current";
  const targetThreadId = "thread-target";
  let currentMessages = [];
  const seededByThread = new Map();
  const stateUpdates = [];

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({ setCalls: stateUpdates }),
    addPending,
    toRenderAttachment,
    toWireAttachment,
    cancelRunRequest: async () => {},
    clearTimeout,
    createThreadRequest: async () => {
      throw new Error("target thread should already exist");
    },
    globalThis: {},
    listConnectableChannels: async () => {
      throw new Error("ordinary prompts should not fetch connectable channels");
    },
    looksLikeChannelConnectCommand,
    queryClient: {
      fetchQuery: async () => {
        throw new Error("ordinary prompts should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    resolveChannelConnectCommand,
    resolveGateRequest: async () => {},
    sendMessage: async ({ threadId }) => ({
      accepted_message_ref: "msg:target-message-1",
      run_id: "run-target",
      status: "queued",
      thread_id: threadId,
    }),
    setInterval,
    setTimeout,
    submitManualToken: async () => {},
    useChatEvents: () => () => {},
    useHistory: () => ({
      messages: currentMessages,
      hasMore: false,
      nextCursor: null,
      isLoading: false,
      loadHistory: () => {},
      seedThreadMessages: (threadId, updater) => {
        const previous = seededByThread.get(threadId) || [];
        const next = typeof updater === "function" ? updater(previous) : updater;
        seededByThread.set(threadId, next);
      },
      setMessages: (updater) => {
        currentMessages =
          typeof updater === "function" ? updater(currentMessages) : updater;
      },
    }),
    useSSE: () => ({ status: "idle" }),
  };

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat(currentThreadId);
  await chat.send("send to another thread", { threadId: targetThreadId });

  assert.deepEqual(currentMessages, []);
  assert.equal(seededByThread.get(targetThreadId).length, 1);
  assert.equal(seededByThread.get(targetThreadId)[0].role, "user");
  assert.equal(
    seededByThread.get(targetThreadId)[0].timelineMessageId,
    "target-message-1",
  );
  assert.deepEqual(stateUpdates.filter((update) => update.index === 2), []);
  assert.deepEqual(stateUpdates.filter((update) => update.index === 4), []);
});

test("useChat.send: target-thread rejected_busy updates seeded cache", async () => {
  const currentThreadId = "thread-current";
  const targetThreadId = "thread-target";
  let currentMessages = [];
  const seededByThread = new Map();
  const stateUpdates = [];

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({ setCalls: stateUpdates }),
    addPending,
    toRenderAttachment,
    toWireAttachment,
    cancelRunRequest: async () => {},
    clearTimeout,
    createThreadRequest: async () => {
      throw new Error("target thread should already exist");
    },
    globalThis: {},
    listConnectableChannels: async () => {
      throw new Error("ordinary prompts should not fetch connectable channels");
    },
    looksLikeChannelConnectCommand,
    queryClient: {
      fetchQuery: async () => {
        throw new Error("ordinary prompts should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    timelineMessageIdFromAcceptedRef,
    resolveChannelConnectCommand,
    resolveGateRequest: async () => {},
    sendMessage: async () => ({
      outcome: "rejected_busy",
      notice: "Thread is busy, please try again.",
    }),
    setInterval,
    setTimeout,
    submitManualToken: async () => {},
    useChatEvents: () => () => {},
    useHistory: () => ({
      messages: currentMessages,
      hasMore: false,
      nextCursor: null,
      isLoading: false,
      loadHistory: () => {},
      seedThreadMessages: (threadId, updater) => {
        const previous = seededByThread.get(threadId) || [];
        const next = typeof updater === "function" ? updater(previous) : updater;
        seededByThread.set(threadId, next);
      },
      setMessages: (updater) => {
        currentMessages =
          typeof updater === "function" ? updater(currentMessages) : updater;
      },
    }),
    useSSE: () => ({ status: "idle" }),
  };

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat(currentThreadId);
  await chat.send("send while target busy", { threadId: targetThreadId });

  assert.deepEqual(currentMessages, []);
  const targetMessages = seededByThread.get(targetThreadId);
  assert.equal(targetMessages.length, 2);
  assert.equal(targetMessages[0].role, "user");
  assert.equal(targetMessages[0].isOptimistic, false);
  assert.equal(targetMessages[0].status, "error");
  assert.equal(targetMessages[1].role, "system");
  assert.equal(targetMessages[1].content, "Thread is busy, please try again.");
  assert.deepEqual(stateUpdates.filter((update) => update.index === 2), []);
  assert.deepEqual(stateUpdates.filter((update) => update.index === 4), []);
});

test("useChat.send: target-thread thrown errors update seeded cache", async () => {
  const currentThreadId = "thread-current";
  const targetThreadId = "thread-target";
  let currentMessages = [];
  const seededByThread = new Map();
  const stateUpdates = [];

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({ setCalls: stateUpdates }),
    addPending,
    toRenderAttachment,
    toWireAttachment,
    cancelRunRequest: async () => {},
    clearTimeout,
    createThreadRequest: async () => {
      throw new Error("target thread should already exist");
    },
    globalThis: {},
    listConnectableChannels: async () => {
      throw new Error("ordinary prompts should not fetch connectable channels");
    },
    looksLikeChannelConnectCommand,
    queryClient: {
      fetchQuery: async () => {
        throw new Error("ordinary prompts should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    timelineMessageIdFromAcceptedRef,
    resolveChannelConnectCommand,
    resolveGateRequest: async () => {},
    sendMessage: async () => {
      throw new Error("network unavailable");
    },
    setInterval,
    setTimeout,
    submitManualToken: async () => {},
    useChatEvents: () => () => {},
    useHistory: () => ({
      messages: currentMessages,
      hasMore: false,
      nextCursor: null,
      isLoading: false,
      loadHistory: () => {},
      seedThreadMessages: (threadId, updater) => {
        const previous = seededByThread.get(threadId) || [];
        const next = typeof updater === "function" ? updater(previous) : updater;
        seededByThread.set(threadId, next);
      },
      setMessages: (updater) => {
        currentMessages =
          typeof updater === "function" ? updater(currentMessages) : updater;
      },
    }),
    useSSE: () => ({ status: "idle" }),
  };

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat(currentThreadId);
  await assert.rejects(
    chat.send("send while network is down", { threadId: targetThreadId }),
    /network unavailable/,
  );

  assert.deepEqual(currentMessages, []);
  const targetMessages = seededByThread.get(targetThreadId);
  assert.equal(targetMessages.length, 1);
  assert.equal(targetMessages[0].role, "user");
  assert.equal(targetMessages[0].isOptimistic, false);
  assert.equal(targetMessages[0].status, "error");
  assert.equal(targetMessages[0].error, "network unavailable");
  assert.deepEqual(stateUpdates.filter((update) => update.index === 2), []);
  assert.deepEqual(stateUpdates.filter((update) => update.index === 4), []);
});

test("useChat.send: pending approval blocks before sendMessage", async () => {
  const threadId = "thread-1";
  const pendingGate = {
    runId: "run-gated",
    gateRef: "gate-shell",
    kind: "gate",
    toolName: "builtin.shell",
  };
  const stateUpdates = [];
  let renderedMessages = [];
  let sendCalls = 0;

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({
      initialByIndex: new Map([
        [4, false],
        [5, pendingGate],
      ]),
      setCalls: stateUpdates,
    }),
    addPending,
    toRenderAttachment,
    toWireAttachment,
    cancelRunRequest: async () => {},
    clearTimeout,
    createThreadRequest: async () => {
      throw new Error("thread should already exist");
    },
    globalThis: {},
    listConnectableChannels: async () => {
      throw new Error("ordinary prompts should not fetch connectable channels");
    },
    looksLikeChannelConnectCommand,
    queryClient: {
      fetchQuery: async () => {
        throw new Error("ordinary prompts should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    resolveChannelConnectCommand,
    resolveGateRequest: async () => {},
    sendMessage: async () => {
      sendCalls += 1;
      throw new Error("sendMessage should not be called");
    },
    setInterval,
    setTimeout,
    submitManualToken: async () => {},
    useChatEvents: () => () => {},
    useHistory: () => ({
      messages: renderedMessages,
      hasMore: false,
      nextCursor: null,
      isLoading: false,
      loadHistory: () => {},
      seedThreadMessages: () => {},
      setMessages: (updater) => {
        renderedMessages =
          typeof updater === "function" ? updater(renderedMessages) : updater;
      },
    }),
    useSSE: () => ({ status: "idle" }),
  };

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat(threadId);
  await assert.rejects(
    chat.send("another request"),
    (error) =>
      error?.safeErrorCode === "approval_gate_pending_send_blocked" &&
      /Resolve the approval request/.test(error.message),
  );

  assert.deepEqual(stateUpdates, []);
  assert.equal(renderedMessages.length, 0);
  assert.equal(sendCalls, 0);
});

test("useChat.send: accepted send does not clear a gate received while in flight", async () => {
  const threadId = "thread-1";
  const replacementGate = {
    runId: "run-replacement",
    gateRef: "gate-replacement",
    kind: "gate",
    toolName: "nearai.web_search",
  };
  const stateUpdates = [];
  const stateSlots = new Map();
  let renderedMessages = [];
  let setPendingGateFromEvents = null;
  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({
      initialByIndex: new Map([
        [4, false],
        [5, null],
      ]),
      setCalls: stateUpdates,
      stateSlots,
    }),
    addPending,
    toRenderAttachment,
    toWireAttachment,
    cancelRunRequest: async () => {},
    clearTimeout,
    createThreadRequest: async () => {
      throw new Error("thread should already exist");
    },
    globalThis: {},
    listConnectableChannels: async () => {
      throw new Error("ordinary prompts should not fetch connectable channels");
    },
    looksLikeChannelConnectCommand,
    queryClient: {
      fetchQuery: async () => {
        throw new Error("ordinary prompts should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    timelineMessageIdFromAcceptedRef,
    resolveChannelConnectCommand,
    resolveGateRequest: async () => {},
    sendMessage: async () => {
      setPendingGateFromEvents(replacementGate);
      return {
        accepted_message_ref: "msg:message-accepted",
        run_id: "run-accepted",
        status: "queued",
        thread_id: threadId,
      };
    },
    setInterval,
    setTimeout,
    submitManualToken: async () => {},
    useChatEvents: ({ setPendingGate }) => {
      setPendingGateFromEvents = setPendingGate;
      return () => {};
    },
    useHistory: () => ({
      messages: renderedMessages,
      hasMore: false,
      nextCursor: null,
      isLoading: false,
      loadHistory: () => {},
      seedThreadMessages: () => {},
      setMessages: (updater) => {
        renderedMessages =
          typeof updater === "function" ? updater(renderedMessages) : updater;
      },
    }),
    useSSE: () => ({ status: "idle" }),
  };

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat(threadId);
  await chat.send("accepted after gate changed");

  assert.deepEqual(
    stateUpdates.filter((call) => call.index === 5).map((call) => call.value),
    [replacementGate],
    "non-busy success must not clear a gate received while send was in flight",
  );
  assert.equal(stateSlots.get(5).value, replacementGate);
});

test("useChat.send: rejected busy attaches notice to a gate received while in flight", async () => {
  const threadId = "thread-1";
  const replacementGate = {
    runId: "run-replacement",
    gateRef: "gate-replacement",
    kind: "gate",
    toolName: "nearai.web_search",
  };
  const stateUpdates = [];
  const stateSlots = new Map();
  let renderedMessages = [];
  let setPendingGateFromEvents = null;

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({
      initialByIndex: new Map([
        [4, false],
        [5, null],
      ]),
      setCalls: stateUpdates,
      stateSlots,
    }),
    addPending,
    toRenderAttachment,
    toWireAttachment,
    cancelRunRequest: async () => {},
    clearTimeout,
    createThreadRequest: async () => {
      throw new Error("thread should already exist");
    },
    globalThis: {},
    listConnectableChannels: async () => {
      throw new Error("ordinary prompts should not fetch connectable channels");
    },
    looksLikeChannelConnectCommand,
    queryClient: {
      fetchQuery: async () => {
        throw new Error("ordinary prompts should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    timelineMessageIdFromAcceptedRef,
    resolveChannelConnectCommand,
    resolveGateRequest: async () => {},
    sendMessage: async () => {
      setPendingGateFromEvents(replacementGate);
      return {
        outcome: "rejected_busy",
        accepted_message_ref: "msg:busy-message-1",
        notice: "Thread is busy, please try again.",
        thread_id: threadId,
      };
    },
    setInterval,
    setTimeout,
    submitManualToken: async () => {},
    useChatEvents: ({ setPendingGate }) => {
      setPendingGateFromEvents = setPendingGate;
      return () => {};
    },
    useHistory: () => ({
      messages: renderedMessages,
      hasMore: false,
      nextCursor: null,
      isLoading: false,
      loadHistory: () => {},
      seedThreadMessages: () => {},
      setMessages: (updater) => {
        renderedMessages =
          typeof updater === "function" ? updater(renderedMessages) : updater;
      },
    }),
    useSSE: () => ({ status: "idle" }),
  };

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat(threadId);
  await chat.send("busy after gate changed");

  assert.deepEqual(
    stateUpdates.filter((call) => call.index === 5).map((call) => call.value),
    [replacementGate],
    "busy rejection must leave a concurrently received gate untouched",
  );
  assert.equal(stateSlots.get(5).value, replacementGate);
  const busyNoticeUpdates = stateUpdates
    .filter((call) => call.index === 6)
    .map((call) => call.value);
  assert.equal(busyNoticeUpdates.length, 1);
  assert.equal(busyNoticeUpdates[0].content, "Thread is busy, please try again.");
  assert.match(busyNoticeUpdates[0].gateKey, /run-replacement\ngate-replacement$/);
});

test("useChat.send: rejected busy seeds notice when active thread changed in flight", async () => {
  const threadId = "thread-1";
  const nextThreadId = "thread-2";
  const pendingGate = {
    runId: "run-gated",
    gateRef: "gate-shell",
    kind: "gate",
    toolName: "builtin.shell",
  };
  const stateUpdates = [];
  const stateSlots = new Map();
  const refs = [];
  const seededByThread = new Map();
  let renderedMessages = [];
  let setPendingGateFromEvents = null;

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({
      initialByIndex: new Map([
        [4, false],
        [5, null],
      ]),
      setCalls: stateUpdates,
      stateSlots,
      refs,
    }),
    addPending,
    toRenderAttachment,
    toWireAttachment,
    cancelRunRequest: async () => {},
    clearTimeout,
    createThreadRequest: async () => {
      throw new Error("thread should already exist");
    },
    globalThis: {},
    listConnectableChannels: async () => {
      throw new Error("ordinary prompts should not fetch connectable channels");
    },
    looksLikeChannelConnectCommand,
    queryClient: {
      fetchQuery: async () => {
        throw new Error("ordinary prompts should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    timelineMessageIdFromAcceptedRef,
    resolveChannelConnectCommand,
    resolveGateRequest: async () => {},
    sendMessage: async () => {
      refs[0].current = nextThreadId;
      setPendingGateFromEvents(pendingGate);
      return {
        outcome: "rejected_busy",
        accepted_message_ref: "msg:busy-message-1",
        notice: "Thread is busy, please try again.",
        thread_id: threadId,
      };
    },
    setInterval,
    setTimeout,
    submitManualToken: async () => {},
    useChatEvents: ({ setPendingGate }) => {
      setPendingGateFromEvents = setPendingGate;
      return () => {};
    },
    useHistory: () => ({
      messages: renderedMessages,
      hasMore: false,
      nextCursor: null,
      isLoading: false,
      loadHistory: () => {},
      seedThreadMessages: (seedThreadId, updater) => {
        const previous = seededByThread.get(seedThreadId) || [];
        const next = typeof updater === "function" ? updater(previous) : updater;
        seededByThread.set(seedThreadId, next);
      },
      setMessages: (updater) => {
        renderedMessages =
          typeof updater === "function" ? updater(renderedMessages) : updater;
      },
    }),
    useSSE: () => ({ status: "idle" }),
  };

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat(threadId);
  await chat.send("busy after thread switch");

  assert.deepEqual(
    stateUpdates.filter((call) => call.index === 6).map((call) => call.value),
    [],
    "a busy gate notice must not be written into a thread that became active later",
  );
  assert.equal(renderedMessages.at(-1)?.role, "user");
  assert.equal(renderedMessages.at(-1)?.status, "error");

  const seededMessages = seededByThread.get(threadId);
  assert.equal(seededMessages.length, 1);
  assert.equal(seededMessages[0].role, "system");
  assert.equal(seededMessages[0].content, "Thread is busy, please try again.");
});

test("useChat.send: rejected busy appends system notice after gate resolves in flight", async () => {
  const threadId = "thread-1";
  const stateUpdates = [];
  const stateSlots = new Map();
  let renderedMessages = [];
  let setPendingGateFromEvents = null;

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({
      initialByIndex: new Map([
        [4, false],
        [5, null],
      ]),
      setCalls: stateUpdates,
      stateSlots,
    }),
    addPending,
    toRenderAttachment,
    toWireAttachment,
    cancelRunRequest: async () => {},
    clearTimeout,
    createThreadRequest: async () => {
      throw new Error("thread should already exist");
    },
    globalThis: {},
    listConnectableChannels: async () => {
      throw new Error("ordinary prompts should not fetch connectable channels");
    },
    looksLikeChannelConnectCommand,
    queryClient: {
      fetchQuery: async () => {
        throw new Error("ordinary prompts should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    timelineMessageIdFromAcceptedRef,
    resolveChannelConnectCommand,
    resolveGateRequest: async () => {},
    sendMessage: async () => {
      setPendingGateFromEvents(null);
      return {
        outcome: "rejected_busy",
        accepted_message_ref: "msg:busy-message-1",
        notice: "Thread is busy, please try again.",
        thread_id: threadId,
      };
    },
    setInterval,
    setTimeout,
    submitManualToken: async () => {},
    useChatEvents: ({ setPendingGate }) => {
      setPendingGateFromEvents = setPendingGate;
      return () => {};
    },
    useHistory: () => ({
      messages: renderedMessages,
      hasMore: false,
      nextCursor: null,
      isLoading: false,
      loadHistory: () => {},
      seedThreadMessages: () => {},
      setMessages: (updater) => {
        renderedMessages =
          typeof updater === "function" ? updater(renderedMessages) : updater;
      },
    }),
    useSSE: () => ({ status: "idle" }),
  };

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat(threadId);
  await chat.send("busy after gate resolved");

  assert.equal(stateSlots.get(5).value, null);
  assert.equal(renderedMessages.length, 2);
  assert.equal(renderedMessages[0].status, "error");
  assert.equal(renderedMessages[1].role, "system");
  assert.equal(renderedMessages[1].content, "Thread is busy, please try again.");
  assert.deepEqual(
    stateUpdates.filter((call) => call.index === 6).map((call) => call.value),
    [],
    "a resolved gate should not get a lingering card-level busy notice",
  );
});

test("useChat.send: gate received after callback creation blocks before send", async () => {
  const threadId = "thread-1";
  const pendingGate = {
    runId: "run-gated",
    gateRef: "gate-shell",
    kind: "gate",
    toolName: "builtin.shell",
  };
  const stateUpdates = [];
  const stateSlots = new Map();
  let renderedMessages = [];
  let setPendingGateFromEvents = null;

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({
      initialByIndex: new Map([
        [4, false],
        [5, null],
      ]),
      setCalls: stateUpdates,
      stateSlots,
    }),
    addPending,
    toRenderAttachment,
    toWireAttachment,
    cancelRunRequest: async () => {},
    clearTimeout,
    createThreadRequest: async () => {
      throw new Error("thread should already exist");
    },
    globalThis: {},
    listConnectableChannels: async () => {
      throw new Error("ordinary prompts should not fetch connectable channels");
    },
    looksLikeChannelConnectCommand,
    queryClient: {
      fetchQuery: async () => {
        throw new Error("ordinary prompts should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    timelineMessageIdFromAcceptedRef,
    resolveChannelConnectCommand,
    resolveGateRequest: async () => {},
    sendMessage: async () => {
      throw new Error("sendMessage should not run");
    },
    setInterval,
    setTimeout,
    submitManualToken: async () => {},
    useChatEvents: ({ setPendingGate }) => {
      setPendingGateFromEvents = setPendingGate;
      return () => {};
    },
    useHistory: () => ({
      messages: renderedMessages,
      hasMore: false,
      nextCursor: null,
      isLoading: false,
      loadHistory: () => {},
      seedThreadMessages: () => {},
      setMessages: (updater) => {
        renderedMessages =
          typeof updater === "function" ? updater(renderedMessages) : updater;
      },
    }),
    useSSE: () => ({ status: "idle" }),
  };

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat(threadId);
  setPendingGateFromEvents(pendingGate);
  await assert.rejects(
    chat.send("busy after gate arrived"),
    (error) =>
      error?.safeErrorCode === "approval_gate_pending_send_blocked" &&
      /Resolve the approval request/.test(error.message),
  );

  assert.deepEqual(
    stateUpdates.filter((call) => call.index === 5).map((call) => call.value),
    [pendingGate],
    "send must read the latest gate from SSE instead of a stale null callback closure",
  );
  assert.equal(stateSlots.get(5).value, pendingGate);
  assert.equal(renderedMessages.length, 0);
  assert.deepEqual(
    stateUpdates.filter((call) => call.index === 6).map((call) => call.value?.content),
    [],
  );
});

test("useChat.send: repeated sends under the same pending gate stay blocked locally", async () => {
  const threadId = "thread-1";
  const pendingGate = {
    runId: "run-gated",
    gateRef: "gate-shell",
    kind: "gate",
    toolName: "builtin.shell",
  };
  const stateUpdates = [];
  let renderedMessages = [];
  let sendCalls = 0;

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({
      initialByIndex: new Map([
        [4, false],
        [5, pendingGate],
      ]),
      setCalls: stateUpdates,
    }),
    addPending,
    toRenderAttachment,
    toWireAttachment,
    cancelRunRequest: async () => {},
    clearTimeout,
    createThreadRequest: async () => {
      throw new Error("thread should already exist");
    },
    globalThis: {},
    listConnectableChannels: async () => {
      throw new Error("ordinary prompts should not fetch connectable channels");
    },
    looksLikeChannelConnectCommand,
    queryClient: {
      fetchQuery: async () => {
        throw new Error("ordinary prompts should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    resolveChannelConnectCommand,
    resolveGateRequest: async () => {},
    sendMessage: async () => {
      sendCalls += 1;
      throw new Error("sendMessage should not run");
    },
    setInterval,
    setTimeout,
    submitManualToken: async () => {},
    useChatEvents: () => () => {},
    useHistory: () => ({
      messages: renderedMessages,
      hasMore: false,
      nextCursor: null,
      isLoading: false,
      loadHistory: () => {},
      seedThreadMessages: () => {},
      setMessages: (updater) => {
        renderedMessages =
          typeof updater === "function" ? updater(renderedMessages) : updater;
      },
    }),
    useSSE: () => ({ status: "idle" }),
  };

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat(threadId);
  await assert.rejects(
    chat.send("first blocked send"),
    /Resolve the approval request/,
  );
  await assert.rejects(
    chat.send("second blocked send"),
    /Resolve the approval request/,
  );

  assert.equal(renderedMessages.length, 0);
  assert.deepEqual(stateUpdates, []);
  assert.equal(sendCalls, 0);
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
      // channelConnectAction, isProcessing, pendingGate.
      initialByIndex: new Map([
        [2, { runId: "run-1", threadId, status: "running" }],
        [4, true],
        [5, { runId: "run-1", gateRef: "gate-1" }],
      ]),
      setCalls: stateUpdates,
    }),
    addPending,
    toRenderAttachment,
    toWireAttachment,
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
    listConnectableChannels: async () => ({
      channels: [],
    }),
    looksLikeChannelConnectCommand,
    queryClient: {
      fetchQuery: async () => ({ channels: [] }),
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    resolveChannelConnectCommand,
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
      seedThreadMessages: () => {},
      setMessages: () => {},
    }),
    useSSE: () => ({ status: "idle" }),
  };

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat(threadId);
  const cancelPromise = chat.cancelRun("user_requested");

  assert.equal(cancelRequest.threadId, threadId);
  assert.equal(cancelRequest.runId, "run-1");
  assert.equal(cancelRequest.reason, "user_requested");
  assert.deepEqual(stateUpdates.slice(0, 3), [
    { index: 5, value: null },
    { index: 4, value: false },
    { index: 2, value: null },
  ]);

  resolveCancelRequest({});
  await cancelPromise;
});

test("useChat clears transient run and gate state during thread switch render", () => {
  const stateUpdates = [];
  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({
      // useChat state call order: cooldownUntil, now, activeRun,
      // channelConnectAction, isProcessing, pendingGate, busyGateNotice,
      // stateThreadId.
      initialByIndex: new Map([
        [2, { runId: "run-old", threadId: "thread-old", status: "awaiting_gate" }],
        [3, { channel: "slack" }],
        [4, true],
        [5, { runId: "run-old", gateRef: "gate-old" }],
        [6, { gateKey: "thread-old\nrun-old\ngate-old", content: "busy" }],
        [7, "thread-old"],
      ]),
      setCalls: stateUpdates,
    }),
    addPending,
    toRenderAttachment,
    toWireAttachment,
    cancelRunRequest: async () => {},
    clearTimeout,
    createThreadRequest: async () => {
      throw new Error("createThread should not run");
    },
    globalThis: {},
    listConnectableChannels: async () => ({ channels: [] }),
    looksLikeChannelConnectCommand,
    queryClient: {
      fetchQuery: async () => ({ channels: [] }),
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    resolveChannelConnectCommand,
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
      seedThreadMessages: () => {},
      setMessages: () => {},
    }),
    useSSE: () => ({ status: "idle" }),
  };

  runUseChatSource(context);
  context.globalThis.__testExports.useChat("thread-new");

  assert.deepEqual(stateUpdates.slice(0, 6), [
    { index: 7, value: "thread-new" },
    { index: 4, value: false },
    { index: 5, value: null },
    { index: 6, value: null },
    { index: 2, value: null },
    { index: 3, value: null },
  ]);
});

test("useChat.approve deny marks the current gated tool declined before resume", async () => {
  const threadId = "thread-1";
  const runId = "run-1";
  const gateRef = "gate-1";
  const stateUpdates = [];
  let renderedMessages = [
    {
      id: "tool-invocation-1",
      role: "tool_activity",
      turnRunId: runId,
      toolStatus: "running",
      toolName: "builtin.shell",
    },
  ];
  let resolveRequest = null;

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({
      initialByIndex: new Map([
        [2, { runId, threadId, status: "awaiting_gate" }],
        [4, false],
        [5, {
          runId,
          gateRef,
          kind: "gate",
          invocationId: "invocation-1",
          toolName: "builtin.shell",
        }],
      ]),
      setCalls: stateUpdates,
    }),
    addPending,
    cancelRunRequest: async () => {},
    clearTimeout,
    createThreadRequest: async () => {
      throw new Error("createThread should not run");
    },
    createToolActivityState,
    failGateToolActivity,
    globalThis: {},
    listConnectableChannels: async () => ({ channels: [] }),
    looksLikeChannelConnectCommand,
    queryClient: {
      fetchQuery: async () => ({ channels: [] }),
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    resolveChannelConnectCommand,
    resolveGateRequest: async (request) => {
      resolveRequest = request;
      return { outcome: "resumed", run_id: runId, status: "queued" };
    },
    resetToolActivityState,
    sendMessage: async () => {
      throw new Error("sendMessage should not run");
    },
    setInterval,
    setTimeout,
    submitManualToken: async () => {},
    useChatEvents: () => () => {},
    useHistory: () => ({
      messages: renderedMessages,
      hasMore: false,
      nextCursor: null,
      isLoading: false,
      loadHistory: () => {},
      setMessages: (updater) => {
        renderedMessages =
          typeof updater === "function" ? updater(renderedMessages) : updater;
      },
    }),
    useSSE: () => ({ status: "idle" }),
  };

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat(threadId);
  await chat.approve(null, "deny", "gate");

  assert.deepEqual(JSON.parse(JSON.stringify(resolveRequest)), {
    threadId,
    runId,
    gateRef,
    resolution: "denied",
    always: false,
  });
  assert.equal(renderedMessages.length, 1);
  assert.equal(renderedMessages[0].toolStatus, "declined");
  assert.equal(renderedMessages[0].toolError, "gate_declined");
  assert.equal(renderedMessages[0].toolErrorKind, "gate_declined");
  assert.equal(renderedMessages[0].gateRef, gateRef);
  assert.deepEqual(JSON.parse(JSON.stringify(stateUpdates.slice(-3))), [
    { index: 5, value: null },
    { index: 4, value: true },
    { index: 2, value: { runId, threadId, status: "queued" } },
  ]);
});

test("useChat.approve deny treats queued response without outcome as resumed", async () => {
  const threadId = "thread-1";
  const runId = "run-queued-response";
  const gateRef = "gate-queued-response";
  const stateUpdates = [];

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({
      initialByIndex: new Map([
        [2, { runId, threadId, status: "awaiting_gate" }],
        [4, false],
        [5, {
          runId,
          gateRef,
          kind: "gate",
          invocationId: "invocation-queued-response",
          toolName: "nearai.web_search",
        }],
      ]),
      setCalls: stateUpdates,
    }),
    addPending,
    cancelRunRequest: async () => {},
    clearTimeout,
    createThreadRequest: async () => {
      throw new Error("createThread should not run");
    },
    createToolActivityState,
    failGateToolActivity,
    globalThis: {},
    listConnectableChannels: async () => ({ channels: [] }),
    looksLikeChannelConnectCommand,
    queryClient: {
      fetchQuery: async () => ({ channels: [] }),
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    resolveChannelConnectCommand,
    resolveGateRequest: async () => ({ run_id: runId, status: "queued" }),
    resetToolActivityState,
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
      seedThreadMessages: () => {},
      setMessages: () => {},
    }),
    useSSE: () => ({ status: "idle" }),
  };

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat(threadId);
  await chat.approve(null, "deny", "gate");

  assert.deepEqual(JSON.parse(JSON.stringify(stateUpdates.slice(-3))), [
    { index: 5, value: null },
    { index: 4, value: true },
    { index: 2, value: { runId, threadId, status: "queued" } },
  ]);
});

test("useChat.approve treats already_terminal false as resumed", async () => {
  const threadId = "thread-1";
  const runId = "run-already-terminal-false";
  const gateRef = "gate-already-terminal-false";
  const stateUpdates = [];

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({
      initialByIndex: new Map([
        [2, { runId, threadId, status: "awaiting_gate" }],
        [4, false],
        [5, {
          runId,
          gateRef,
          kind: "gate",
          invocationId: "invocation-terminal-false",
          toolName: "nearai.web_search",
        }],
      ]),
      setCalls: stateUpdates,
    }),
    addPending,
    cancelRunRequest: async () => {},
    clearTimeout,
    createThreadRequest: async () => {
      throw new Error("createThread should not run");
    },
    createToolActivityState,
    failGateToolActivity,
    globalThis: {},
    listConnectableChannels: async () => ({ channels: [] }),
    looksLikeChannelConnectCommand,
    queryClient: {
      fetchQuery: async () => ({ channels: [] }),
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    resolveChannelConnectCommand,
    resolveGateRequest: async () => ({ run_id: runId, already_terminal: false }),
    resetToolActivityState,
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
      seedThreadMessages: () => {},
      setMessages: () => {},
    }),
    useSSE: () => ({ status: "idle" }),
  };

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat(threadId);
  await chat.approve(null, "deny", "gate");

  assert.deepEqual(JSON.parse(JSON.stringify(stateUpdates.slice(-3))), [
    { index: 5, value: null },
    { index: 4, value: true },
    { index: 2, value: { runId, threadId, status: "queued" } },
  ]);
});

test("useChat.approve deny with already_terminal true does not synthesize failed activity", async () => {
  const threadId = "thread-1";
  const runId = "run-already-terminal-true";
  const gateRef = "gate-already-terminal-true";
  const stateUpdates = [];
  let renderedMessages = [
    {
      id: "tool-existing-terminal",
      role: "tool_activity",
      turnRunId: runId,
      gateRef,
      toolStatus: "ok",
      toolName: "search",
    },
  ];

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({
      initialByIndex: new Map([
        [2, { runId, threadId, status: "awaiting_gate" }],
        [4, false],
        [5, {
          runId,
          gateRef,
          kind: "gate",
          invocationId: "invocation-terminal-true",
          toolName: "nearai.web_search",
        }],
      ]),
      setCalls: stateUpdates,
    }),
    addPending,
    cancelRunRequest: async () => {},
    clearTimeout,
    createThreadRequest: async () => {
      throw new Error("createThread should not run");
    },
    createToolActivityState,
    failGateToolActivity,
    globalThis: {},
    listConnectableChannels: async () => ({ channels: [] }),
    looksLikeChannelConnectCommand,
    queryClient: {
      fetchQuery: async () => ({ channels: [] }),
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    resolveChannelConnectCommand,
    resolveGateRequest: async () => ({ run_id: runId, already_terminal: true }),
    resetToolActivityState,
    sendMessage: async () => {
      throw new Error("sendMessage should not run");
    },
    setInterval,
    setTimeout,
    submitManualToken: async () => {},
    useChatEvents: () => () => {},
    useHistory: () => ({
      messages: renderedMessages,
      hasMore: false,
      nextCursor: null,
      isLoading: false,
      loadHistory: () => {},
      setMessages: (updater) => {
        renderedMessages =
          typeof updater === "function" ? updater(renderedMessages) : updater;
      },
    }),
    useSSE: () => ({ status: "idle" }),
  };

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat(threadId);
  await chat.approve(null, "deny", "gate");

  assert.equal(renderedMessages.length, 1);
  assert.equal(renderedMessages[0].toolStatus, "ok");
  assert.equal(renderedMessages[0].toolError, undefined);
  assert.deepEqual(JSON.parse(JSON.stringify(stateUpdates.slice(-3))), [
    { index: 5, value: null },
    { index: 4, value: false },
    { index: 2, value: null },
  ]);
  assert.equal(
    stateUpdates.some((update) => update.index === 4 && update.value === true),
    false,
    "already_terminal gate resolution must not turn processing back on",
  );
});

test("useChat.cancelRun completion does not clear a newer run", async () => {
  const threadId = "thread-1";
  const stateUpdates = [];
  let resolveCancelRequest;

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({
      initialByIndex: new Map([
        [2, { runId: "run-1", threadId, status: "running" }],
        [4, true],
      ]),
      setCalls: stateUpdates,
    }),
    addPending,
    toRenderAttachment,
    toWireAttachment,
    cancelRunRequest: async () =>
      new Promise((resolve) => {
        resolveCancelRequest = resolve;
      }),
    clearTimeout,
    createThreadRequest: async () => {
      throw new Error("createThread should not run");
    },
    globalThis: {},
    listConnectableChannels: async () => {
      throw new Error("ordinary prompts should not fetch connectable channels");
    },
    looksLikeChannelConnectCommand,
    queryClient: {
      fetchQuery: async () => {
        throw new Error("ordinary prompts should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    resolveChannelConnectCommand,
    resolveGateRequest: async () => {},
    sendMessage: async () => ({
      accepted_message_ref: "msg:message-2",
      run_id: "run-2",
      status: "queued",
      thread_id: threadId,
    }),
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
      seedThreadMessages: () => {},
      setMessages: () => {},
    }),
    useSSE: () => ({ status: "idle" }),
  };

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat(threadId);
  const cancelPromise = chat.cancelRun("user_requested");
  await chat.send("next request");

  const newerRunUpdate = stateUpdates.find(
    (update) => update.index === 2 && update.value?.runId === "run-2",
  );
  assert.equal(newerRunUpdate?.value.threadId, threadId);
  assert.equal(newerRunUpdate?.value.status, "queued");
  assert.equal(newerRunUpdate?.value.source, "local");

  const updatesBeforeCancelResolution = stateUpdates.length;
  resolveCancelRequest({});
  await cancelPromise;

  assert.deepEqual(stateUpdates.slice(updatesBeforeCancelResolution), []);
});

test("useChat.send: channel connect requests return an action without submitting a prompt", async () => {
  let createThreadCalled = false;
  let sendMessageCalled = false;

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub(),
    addPending,
    toRenderAttachment,
    toWireAttachment,
    cancelRunRequest: async () => {},
    clearTimeout,
    createThreadRequest: async () => {
      createThreadCalled = true;
      throw new Error("connect action should not create a thread");
    },
    globalThis: {},
    listConnectableChannels: async () => ({
      channels: [
        {
          channel: "slack",
          display_name: "Slack",
          strategy: "inbound_proof_code",
          command_aliases: ["slack", "slack account"],
          action: {
            title: "Slack account connection",
            instructions: "Message the Slack app, then enter the code here.",
          },
        },
      ],
    }),
    looksLikeChannelConnectCommand,
    queryClient: {
      fetchQuery: async ({ queryFn }) => queryFn(),
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    resolveChannelConnectCommand,
    resolveGateRequest: async () => {},
    sendMessage: async () => {
      sendMessageCalled = true;
      throw new Error("connect action should not submit a model prompt");
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
      seedThreadMessages: () => {},
      setMessages: () => {},
    }),
    useSSE: () => ({ status: "idle" }),
  };

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat(null);
  const response = await chat.send("connect my Slack account");

  assert.equal(createThreadCalled, false);
  assert.equal(sendMessageCalled, false);
  assert.equal(response.channel_connect_action.channel, "slack");
  assert.equal(response.channel_connect_action.strategy, "inbound_proof_code");
});

test("useChat.send: unmatched channel connect requests submit the prompt", async () => {
  let createThreadCalled = false;
  let sentContent = null;

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub(),
    addPending,
    toRenderAttachment,
    toWireAttachment,
    cancelRunRequest: async () => {},
    clearTimeout,
    createThreadRequest: async () => {
      createThreadCalled = true;
      return { thread: { thread_id: "thread-created" } };
    },
    globalThis: {},
    listConnectableChannels: async () => ({
      channels: [
        {
          channel: "slack",
          display_name: "Slack",
          strategy: "inbound_proof_code",
          command_aliases: ["slack", "slack account"],
          action: {
            title: "Slack account connection",
            instructions: "Message the Slack app, then enter the code here.",
          },
        },
      ],
    }),
    looksLikeChannelConnectCommand,
    queryClient: {
      fetchQuery: async ({ queryFn }) => queryFn(),
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    resolveChannelConnectCommand,
    resolveGateRequest: async () => {},
    sendMessage: async ({ content, threadId }) => {
      sentContent = content;
      return {
        accepted_message_ref: "msg:message-2",
        run_id: "run-2",
        status: "queued",
        thread_id: threadId,
      };
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
      seedThreadMessages: () => {},
      setMessages: () => {},
    }),
    useSSE: () => ({ status: "idle" }),
  };

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat(null);
  const response = await chat.send("connect telegram account");

  assert.equal(createThreadCalled, true);
  assert.equal(sentContent, "connect telegram account");
  assert.equal(response.channel_connect_action, undefined);
  assert.equal(response.thread_id, "thread-created");
});

test("useChat.send: rejected_busy appends system notice, marks optimistic failed, clears isProcessing", async () => {
  const threadId = "thread-busy";
  let renderedMessages = [];
  const stateUpdates = [];

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({ setCalls: stateUpdates }),
    addPending,
    toRenderAttachment,
    toWireAttachment,
    cancelRunRequest: async () => {},
    clearTimeout,
    createThreadRequest: async () => {
      throw new Error("thread should already exist");
    },
    globalThis: {},
    listConnectableChannels: async () => {
      throw new Error("ordinary prompts should not fetch connectable channels");
    },
    looksLikeChannelConnectCommand,
    queryClient: {
      fetchQuery: async () => {
        throw new Error("ordinary prompts should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    resolveChannelConnectCommand,
    resolveGateRequest: async () => {},
    sendMessage: async () => ({
      outcome: "rejected_busy",
      notice: "Thread is busy, please try again.",
    }),
    setInterval,
    setTimeout,
    submitManualToken: async () => {},
    useChatEvents: () => () => {},
    useHistory: (_threadId, options) => ({
      messages: renderedMessages,
      hasMore: false,
      nextCursor: null,
      isLoading: false,
      loadHistory: () => {},
      setMessages: (updater) => {
        renderedMessages =
          typeof updater === "function" ? updater(renderedMessages) : updater;
      },
    }),
    useSSE: () => ({ status: "idle" }),
  };

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat(threadId);
  await chat.send("hello while busy");

  // (a) a system message with the notice text is appended
  const systemMessages = renderedMessages.filter((m) => m.role === "system");
  assert.equal(systemMessages.length, 1);
  assert.equal(systemMessages[0].content, "Thread is busy, please try again.");
  assert.match(systemMessages[0].id, /^system-rejected-/);

  // (b) the optimistic user message is marked failed (not shown as sent)
  const userMessages = renderedMessages.filter((m) => m.role === "user");
  assert.equal(userMessages.length, 1);
  assert.equal(userMessages[0].isOptimistic, false);
  assert.equal(userMessages[0].status, "error");

  // (c) isProcessing is cleared (index 4 set to false)
  const isProcessingUpdates = stateUpdates.filter((u) => u.index === 4);
  const lastIsProcessing = isProcessingUpdates[isProcessingUpdates.length - 1];
  assert.equal(lastIsProcessing?.value, false);
});

test("useChat.send: rejected_busy without notice still clears isProcessing", async () => {
  const threadId = "thread-busy-no-notice";
  let renderedMessages = [];
  const stateUpdates = [];

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({ setCalls: stateUpdates }),
    addPending,
    toRenderAttachment,
    toWireAttachment,
    cancelRunRequest: async () => {},
    clearTimeout,
    createThreadRequest: async () => {
      throw new Error("thread should already exist");
    },
    globalThis: {},
    listConnectableChannels: async () => {
      throw new Error("ordinary prompts should not fetch connectable channels");
    },
    looksLikeChannelConnectCommand,
    queryClient: {
      fetchQuery: async () => {
        throw new Error("ordinary prompts should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    resolveChannelConnectCommand,
    resolveGateRequest: async () => {},
    sendMessage: async () => ({
      outcome: "rejected_busy",
    }),
    setInterval,
    setTimeout,
    submitManualToken: async () => {},
    useChatEvents: () => () => {},
    useHistory: (_threadId, options) => ({
      messages: renderedMessages,
      hasMore: false,
      nextCursor: null,
      isLoading: false,
      loadHistory: () => {},
      setMessages: (updater) => {
        renderedMessages =
          typeof updater === "function" ? updater(renderedMessages) : updater;
      },
    }),
    useSSE: () => ({ status: "idle" }),
  };

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat(threadId);
  await chat.send("hello while busy");

  // no system notice appended when notice is absent
  const systemMessages = renderedMessages.filter((m) => m.role === "system");
  assert.equal(systemMessages.length, 0);

  // optimistic user message still marked failed
  const userMessages = renderedMessages.filter((m) => m.role === "user");
  assert.equal(userMessages.length, 1);
  assert.equal(userMessages[0].status, "error");

  // isProcessing is cleared (index 4 set to false)
  const isProcessingUpdates = stateUpdates.filter((u) => u.index === 4);
  const lastIsProcessing = isProcessingUpdates[isProcessingUpdates.length - 1];
  assert.equal(lastIsProcessing?.value, false);
});

test("useChat.send: connectable channel fetch failures submit the prompt", async () => {
  let createThreadCalled = false;
  let sentContent = null;
  const loggedErrors = [];

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub(),
    addPending,
    toRenderAttachment,
    toWireAttachment,
    cancelRunRequest: async () => {},
    clearTimeout,
    console: {
      error: (...args) => loggedErrors.push(args),
    },
    createThreadRequest: async () => {
      createThreadCalled = true;
      return { thread: { thread_id: "thread-created" } };
    },
    globalThis: {},
    listConnectableChannels: async () => {
      throw new Error("connectable channel service unavailable");
    },
    looksLikeChannelConnectCommand,
    queryClient: {
      fetchQuery: async ({ queryFn }) => queryFn(),
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    resolveChannelConnectCommand,
    resolveGateRequest: async () => {},
    sendMessage: async ({ content, threadId }) => {
      sentContent = content;
      return {
        accepted_message_ref: "msg:message-3",
        run_id: "run-3",
        status: "queued",
        thread_id: threadId,
      };
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
      seedThreadMessages: () => {},
      setMessages: () => {},
    }),
    useSSE: () => ({ status: "idle" }),
  };

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat(null);
  const response = await chat.send("connect my Slack account");

  assert.equal(createThreadCalled, true);
  assert.equal(sentContent, "connect my Slack account");
  assert.equal(response.channel_connect_action, undefined);
  assert.equal(response.thread_id, "thread-created");
  assert.equal(loggedErrors[0][0], "Failed to resolve connectable channels:");
});

function createResolveGateContext({
  stateUpdates = [],
  resolveGateResponse = {
    outcome: "resumed",
    run_id: "run-1",
    thread_id: "thread-1",
    status: "queued",
  },
} = {}) {
  // useChat state call order: cooldownUntil(0), now(1), activeRun(2),
  // channelConnectAction(3), isProcessing(4), pendingGate(5).
  const pendingGate = {
    runId: "run-1",
    gateRef: "gate-1",
    kind: "gate",
    invocationId: "invocation-1",
    toolName: "web-access.search",
  };
  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({
      initialByIndex: new Map([
        [2, { runId: "run-1", threadId: "thread-1", status: "running" }],
        [4, true],
        [5, pendingGate],
      ]),
      setCalls: stateUpdates,
    }),
    addPending,
    toRenderAttachment,
    toWireAttachment,
    cancelRunRequest: async () => {},
    clearTimeout,
    createThreadRequest: async () => {
      throw new Error("createThread should not run");
    },
    globalThis: {},
    listConnectableChannels: async () => ({ channels: [] }),
    looksLikeChannelConnectCommand,
    queryClient: {
      fetchQuery: async () => ({ channels: [] }),
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    resolveChannelConnectCommand,
    resolveGateRequest: async () => resolveGateResponse,
    sendMessage: async () => {
      throw new Error("sendMessage should not run");
    },
    setInterval,
    setTimeout,
    submitManualToken: async () => {},
    useChatEvents: (args) => {
      context.chatEventsArgs = args;
      return () => {};
    },
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
  return context;
}

test("useChat.resolveGate: denied keeps isProcessing true and does not clear activeRun", async () => {
  const stateUpdates = [];
  const context = createResolveGateContext({ stateUpdates });

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat("thread-1");
  await chat.resolveGate("denied");

  // pendingGate (index 5) is cleared
  const pendingGateUpdates = stateUpdates.filter((u) => u.index === 5);
  assert.equal(pendingGateUpdates.length, 1);
  assert.equal(pendingGateUpdates[0].value, null);

  // isProcessing (index 4) is set to true — run continues
  const isProcessingUpdates = stateUpdates.filter((u) => u.index === 4);
  assert.ok(isProcessingUpdates.length > 0, "isProcessing should be updated");
  const lastIsProcessing = isProcessingUpdates[isProcessingUpdates.length - 1];
  assert.equal(lastIsProcessing.value, true);

  // activeRun (index 2) is NOT cleared by resolveGate
  const activeRunClears = stateUpdates.filter(
    (u) => u.index === 2 && u.value === null,
  );
  assert.equal(activeRunClears.length, 0, "resolveGate must not clear activeRun");
  assert.deepEqual(
    JSON.parse(JSON.stringify(
      context.chatEventsArgs.locallyResolvedGatesRef.current.get("run-1\ngate-1"),
    )),
    { resolution: "denied", outcome: "resumed" },
  );
});

test("useChat.resolveGate: resumed cancelled auth keeps processing until follow-up run settles", async () => {
  const stateUpdates = [];
  const context = createResolveGateContext({ stateUpdates });

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat("thread-1");
  await chat.resolveGate("cancelled");

  // isProcessing (index 4) is set to true — run continues
  const isProcessingUpdates = stateUpdates.filter((u) => u.index === 4);
  assert.ok(isProcessingUpdates.length > 0, "isProcessing should be updated");
  const lastIsProcessing = isProcessingUpdates[isProcessingUpdates.length - 1];
  assert.equal(lastIsProcessing.value, true);

  // activeRun (index 2) is NOT cleared
  const activeRunClears = stateUpdates.filter(
    (u) => u.index === 2 && u.value === null,
  );
  assert.equal(activeRunClears.length, 0, "resolveGate must not clear activeRun");
  assert.deepEqual(
    JSON.parse(JSON.stringify(
      context.chatEventsArgs.locallyResolvedGatesRef.current.get("run-1\ngate-1"),
    )),
    { resolution: "cancelled", outcome: "resumed" },
  );
});

test("useChat.resolveGate: terminal cancelled clears processing and activeRun", async () => {
  const stateUpdates = [];
  const context = createResolveGateContext({
    stateUpdates,
    resolveGateResponse: {
      outcome: "cancelled",
      run_id: "run-1",
      thread_id: "thread-1",
      status: "cancelled",
    },
  });

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat("thread-1");
  await chat.resolveGate("cancelled");

  const isProcessingUpdates = stateUpdates.filter((u) => u.index === 4);
  assert.ok(isProcessingUpdates.length > 0, "isProcessing should be updated");
  assert.equal(isProcessingUpdates[isProcessingUpdates.length - 1].value, false);

  const pendingGateUpdates = stateUpdates.filter((u) => u.index === 5);
  assert.equal(pendingGateUpdates[pendingGateUpdates.length - 1].value, null);

  const activeRunUpdates = stateUpdates.filter((u) => u.index === 2);
  assert.equal(activeRunUpdates[activeRunUpdates.length - 1].value, null);
  assert.deepEqual(
    JSON.parse(JSON.stringify(
      context.chatEventsArgs.locallyResolvedGatesRef.current.get("run-1\ngate-1"),
    )),
    { resolution: "cancelled", outcome: "cancelled" },
  );
});

test("useChat.resolveGate: approved also keeps isProcessing true", async () => {
  const stateUpdates = [];
  const context = createResolveGateContext({ stateUpdates });

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat("thread-1");
  await chat.resolveGate("approved");

  const isProcessingUpdates = stateUpdates.filter((u) => u.index === 4);
  assert.ok(isProcessingUpdates.length > 0);
  const lastIsProcessing = isProcessingUpdates[isProcessingUpdates.length - 1];
  assert.equal(lastIsProcessing.value, true);
});
