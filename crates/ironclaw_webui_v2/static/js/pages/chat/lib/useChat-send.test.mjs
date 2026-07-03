import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

import { messagesFromTimeline } from "./history-messages.js";
import { toRenderAttachment, toWireAttachment } from "./attachments.js";
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
import { subscribeChannelConnected } from "../../../lib/channel-connection-events.js";

const STATE_SLOT = Object.freeze({
  cooldownUntil: 0,
  now: 1,
  activeRun: 2,
  isProcessing: 3,
  pendingGate: 4,
  busyGateNotice: 5,
  stateThreadId: 6,
});

function stateUpdatesFor(updates, slot) {
  return updates.filter((update) => update.index === slot);
}

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
  if (!context.subscribeChannelConnected) {
    context.subscribeChannelConnected = subscribeChannelConnected;
  }
  if (!("touchThreadInCache" in context)) context.touchThreadInCache = () => {};
  if (!("upsertThreadInCache" in context)) context.upsertThreadInCache = () => {};
  vm.runInNewContext(useChatSourceForTest(), context);
}

function createReactStub({
  initialByIndex = new Map(),
  setCalls = [],
  stateSlots = new Map(),
  refs = [],
  runEffects = false,
} = {}) {
  let stateIndex = 0;
  let refIndex = 0;
  let effectIndex = 0;
  const refSlots = [];
  const effectSlots = [];
  const depsChanged = (previous, next) => {
    if (!previous || !next || previous.length !== next.length) return true;
    return next.some((value, index) => !Object.is(value, previous[index]));
  };
  const react = {
    __beginRender: () => {
      stateIndex = 0;
      refIndex = 0;
      effectIndex = 0;
    },
    useCallback: (fn) => fn,
    useEffect: (effect, deps) => {
      if (!runEffects) return;
      const index = effectIndex++;
      const slot = effectSlots[index] || { deps: null, cleanup: null };
      if (!depsChanged(slot.deps, deps)) {
        effectSlots[index] = slot;
        return;
      }
      if (typeof slot.cleanup === "function") slot.cleanup();
      slot.deps = deps ? [...deps] : null;
      slot.cleanup = effect() || null;
      effectSlots[index] = slot;
    },
    useRef: (value) => {
      const index = refIndex++;
      const ref = refSlots[index] || { current: value };
      refSlots[index] = ref;
      if (!refs.includes(ref)) refs.push(ref);
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
  return react;
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
    queryClient: {
      fetchQuery: async () => {
        throw new Error("ordinary prompts should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    timelineMessageIdFromAcceptedRef,
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
    queryClient: {
      fetchQuery: async () => {
        throw new Error("attachment sends should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef: () => null,
    removePending,
    timelineMessageIdFromAcceptedRef,
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

test("useChat.send: touches sidebar cache without refetching thread list", async () => {
  const threadId = "thread-1";
  const { context } = createSendCaptureContext();
  let touched = null;

  context.queryClient.invalidateQueries = () => {
    throw new Error("send should not refetch the full thread list");
  };
  context.queryClient.getQueryData = () => ({
    threads: [{ id: threadId, title: "Existing title" }],
  });
  context.touchThreadInCache = (update) => {
    touched = update;
  };

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat(threadId);
  await chat.send("raw wire content", {
    displayContent: "visible sidebar title",
  });

  assert.equal(touched.threadId, threadId);
  assert.equal(touched.messageContent, "visible sidebar title");
  assert.match(touched.updatedAt, /^\d{4}-\d{2}-\d{2}T/);
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
    queryClient: {
      fetchQuery: async () => {
        throw new Error("ordinary prompts should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
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
  assert.deepEqual(stateUpdates.filter((update) => update.index === 3), []);
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
    queryClient: {
      fetchQuery: async () => {
        throw new Error("ordinary prompts should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    timelineMessageIdFromAcceptedRef,
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
  assert.deepEqual(stateUpdates.filter((update) => update.index === 3), []);
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
    queryClient: {
      fetchQuery: async () => {
        throw new Error("ordinary prompts should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    timelineMessageIdFromAcceptedRef,
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
  assert.deepEqual(stateUpdates.filter((update) => update.index === 3), []);
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
        [3, false],
        [4, pendingGate],
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
    queryClient: {
      fetchQuery: async () => {
        throw new Error("ordinary prompts should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
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

test("useChat.retryMessage: pre-admission rejection keeps failed bubble retryable", async () => {
  const threadId = "thread-1";
  const failedMessage = {
    id: "failed-1",
    role: "user",
    content: "retry me",
    retryContent: "retry me",
    status: "error",
  };
  const pendingGate = {
    runId: "run-gated",
    gateRef: "gate-shell",
    kind: "gate",
    toolName: "builtin.shell",
  };
  let renderedMessages = [failedMessage];
  let seededMessages = null;
  let sendCalls = 0;

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({
      initialByIndex: new Map([
        [3, false],
        [4, pendingGate],
      ]),
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
      throw new Error("approval gate should block before channel discovery");
    },
    queryClient: {
      fetchQuery: async () => {
        throw new Error("approval gate should block before channel discovery");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
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
      seedThreadMessages: (_threadId, updater) => {
        seededMessages =
          typeof updater === "function"
            ? updater(seededMessages ?? renderedMessages)
            : updater;
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
  await chat.retryMessage(failedMessage);

  assert.equal(sendCalls, 0);
  assert.equal(renderedMessages.length, 1);
  assert.equal(renderedMessages[0].id, failedMessage.id);
  assert.equal(renderedMessages[0].content, failedMessage.content);
  assert.equal(renderedMessages[0].retryContent, failedMessage.retryContent);
  assert.equal(renderedMessages[0].status, failedMessage.status);
  assert.equal(seededMessages.length, 1);
  assert.equal(seededMessages[0].id, failedMessage.id);
  assert.equal(seededMessages[0].content, failedMessage.content);
  assert.equal(seededMessages[0].retryContent, failedMessage.retryContent);
  assert.equal(seededMessages[0].status, failedMessage.status);
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
        [3, false],
        [4, null],
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
    queryClient: {
      fetchQuery: async () => {
        throw new Error("ordinary prompts should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    timelineMessageIdFromAcceptedRef,
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
    stateUpdates.filter((call) => call.index === 4).map((call) => call.value),
    [replacementGate],
    "non-busy success must not clear a gate received while send was in flight",
  );
  assert.equal(stateSlots.get(4).value, replacementGate);
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
        [3, false],
        [4, null],
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
    queryClient: {
      fetchQuery: async () => {
        throw new Error("ordinary prompts should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    timelineMessageIdFromAcceptedRef,
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
    stateUpdates.filter((call) => call.index === 4).map((call) => call.value),
    [replacementGate],
    "busy rejection must leave a concurrently received gate untouched",
  );
  assert.equal(stateSlots.get(4).value, replacementGate);
  const busyNoticeUpdates = stateUpdates
    .filter((call) => call.index === 5)
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
        [3, false],
        [4, null],
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
    queryClient: {
      fetchQuery: async () => {
        throw new Error("ordinary prompts should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    timelineMessageIdFromAcceptedRef,
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
    stateUpdates.filter((call) => call.index === 5).map((call) => call.value),
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
        [3, false],
        [4, null],
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
    queryClient: {
      fetchQuery: async () => {
        throw new Error("ordinary prompts should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    timelineMessageIdFromAcceptedRef,
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

  assert.equal(stateSlots.get(4).value, null);
  assert.equal(renderedMessages.length, 2);
  assert.equal(renderedMessages[0].status, "error");
  assert.equal(renderedMessages[1].role, "system");
  assert.equal(renderedMessages[1].content, "Thread is busy, please try again.");
  assert.deepEqual(
    stateUpdates.filter((call) => call.index === 5).map((call) => call.value),
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
        [3, false],
        [4, null],
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
    queryClient: {
      fetchQuery: async () => {
        throw new Error("ordinary prompts should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    timelineMessageIdFromAcceptedRef,
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
    stateUpdates.filter((call) => call.index === 4).map((call) => call.value),
    [pendingGate],
    "send must read the latest gate from SSE instead of a stale null callback closure",
  );
  assert.equal(stateSlots.get(4).value, pendingGate);
  assert.equal(renderedMessages.length, 0);
  assert.deepEqual(
    stateUpdates.filter((call) => call.index === 5).map((call) => call.value?.content),
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
        [3, false],
        [4, pendingGate],
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
    queryClient: {
      fetchQuery: async () => {
        throw new Error("ordinary prompts should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
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
      // isProcessing, pendingGate.
      initialByIndex: new Map([
        [2, { runId: "run-1", threadId, status: "running" }],
        [3, true],
        [4, { runId: "run-1", gateRef: "gate-1" }],
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
    queryClient: {
      fetchQuery: async () => ({ channels: [] }),
      invalidateQueries: () => {},
    },
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
    { index: 4, value: null },
    { index: 3, value: false },
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
      // useChat state call order: cooldownUntil(0), now(1), activeRun(2),
      // isProcessing(3), pendingGate(4), busyGateNotice(5), stateThreadId(6).
      initialByIndex: new Map([
        [2, { runId: "run-old", threadId: "thread-old", status: "awaiting_gate" }],
        [3, true],
        [4, { runId: "run-old", gateRef: "gate-old" }],
        [5, { gateKey: "thread-old\nrun-old\ngate-old", content: "busy" }],
        [6, "thread-old"],
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
    queryClient: {
      fetchQuery: async () => ({ channels: [] }),
      invalidateQueries: () => {},
    },
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
      seedThreadMessages: () => {},
      setMessages: () => {},
    }),
    useSSE: () => ({ status: "idle" }),
  };

  runUseChatSource(context);
  context.globalThis.__testExports.useChat("thread-new");

  // The in-render thread-switch reset clears stateThreadId(6), isProcessing(3),
  // pendingGate(4), busyGateNotice(5), and activeRun(2) so no prior-thread
  // gate/run bleeds onto the newly-opened chat.
  assert.deepEqual(stateUpdates.slice(0, 5), [
    { index: 6, value: "thread-new" },
    { index: 3, value: false },
    { index: 4, value: null },
    { index: 5, value: null },
    { index: 2, value: null },
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
        [3, false],
        [4, {
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
    queryClient: {
      fetchQuery: async () => ({ channels: [] }),
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
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
    { index: 4, value: null },
    { index: 3, value: true },
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
        [3, false],
        [4, {
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
    queryClient: {
      fetchQuery: async () => ({ channels: [] }),
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
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
    { index: 4, value: null },
    { index: 3, value: true },
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
        [3, false],
        [4, {
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
    queryClient: {
      fetchQuery: async () => ({ channels: [] }),
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
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
    { index: 4, value: null },
    { index: 3, value: true },
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
        [3, false],
        [4, {
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
    queryClient: {
      fetchQuery: async () => ({ channels: [] }),
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
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
    { index: 4, value: null },
    { index: 3, value: false },
    { index: 2, value: null },
  ]);
  assert.equal(
    stateUpdates.some((update) => update.index === 3 && update.value === true),
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
        [3, true],
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
    queryClient: {
      fetchQuery: async () => {
        throw new Error("ordinary prompts should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
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

test("useChat.send: connect-like prompts submit to the model", async () => {
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
            instructions:
              "Message the IronClaw Reborn app in Slack to get a pairing code, then paste it here. Codes expire in 10 minutes. If a code is invalid or expired, run /pair in Slack for a fresh one.",
          },
        },
      ],
    }),
    queryClient: {
      fetchQuery: async ({ queryFn }) => queryFn(),
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    resolveGateRequest: async () => {},
    sendMessage: async ({ content, threadId }) => {
      sentContent = content;
      return {
        accepted_message_ref: "msg:message-1",
        run_id: "run-1",
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
  assert.equal(response.thread_id, "thread-created");
});

test("useChat.send: routine setup prompts mentioning Slack submit to the model", async () => {
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
            instructions:
              "Message the IronClaw Reborn app in Slack to get a pairing code, then paste it here. Codes expire in 10 minutes. If a code is invalid or expired, run /pair in Slack for a fresh one.",
          },
        },
      ],
    }),
    queryClient: {
      fetchQuery: async ({ queryFn }) => queryFn(),
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
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

  const routinePrompt =
    "Set up a routine to improve daily engineering updates:\n\n" +
    "Every morning, IronClaw should send you a Slack DM with a summary of your GitHub activity from yesterday: PRs, comments, etc.\n\n" +
    "Results: You will get a DM from IronClaw.\n" +
    "The last step is manual: please post the message you got from IronClaw to #x-updates.";

  const chat = context.globalThis.__testExports.useChat(null);
  const response = await chat.send(routinePrompt);

  assert.equal(createThreadCalled, true);
  assert.equal(sentContent, routinePrompt);
  assert.equal(response.thread_id, "thread-created");
});

// A channel-pairing gate rides the standard auth rail: a `manual_token`
// challenge whose gate also carries a `connection` requirement. `gates.js`
// normalizes that into `pendingGate.connection`, and the pairing card submits
// through `submitChannelConnectionPairing`.
function pairingGate(channel = "slack") {
  return {
    runId: "run-pairing",
    gateRef: "gate-auth-pairing",
    kind: "auth_required",
    challengeKind: "manual_token",
    connection: {
      channel,
      strategy: "inbound_proof_code",
      instructions: "Message the app to get a pairing code, then paste it here.",
      inputPlaceholder: "Enter pairing code",
      submitLabel: "Connect",
      errorMessage: "Invalid or expired code.",
    },
  };
}

test("useChat.submitChannelConnectionPairing: Slack pairing gate redeems without resolving the gate", async () => {
  const threadId = "thread-slack-pairing-gate";
  const stateUpdates = [];
  const slackPairingCalls = [];
  let resolveGateCalls = 0;
  let redeemGenericCalls = 0;

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({
      // pendingGate is state slot 4.
      initialByIndex: new Map([[4, pairingGate("slack")]]),
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
    queryClient: {
      getQueryData: () => ({
        threads: [{ thread_id: threadId, title: "Slack pairing thread" }],
      }),
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    redeemSlackPairingCode: async (code, options) => {
      slackPairingCalls.push({ code, options });
      return { success: true };
    },
    redeemPairingCode: async () => {
      redeemGenericCalls += 1;
      throw new Error("Slack pairing must use the Slack redemption endpoint");
    },
    removePending,
    resolveGateRequest: async () => {
      resolveGateCalls += 1;
      throw new Error("a successful pairing redeem must NOT resolve the gate");
    },
    sendMessage: async () => {
      throw new Error("pairing redeem must not post a continuation message");
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
  const response = await chat.submitChannelConnectionPairing(" A1B2C3 ");

  assert.equal(response.success, true);
  assert.equal(slackPairingCalls.length, 1);
  assert.equal(slackPairingCalls[0].code, "A1B2C3");
  assert.equal(slackPairingCalls[0].options.threadId, threadId);
  assert.equal(redeemGenericCalls, 0);
  assert.equal(
    resolveGateCalls,
    0,
    "the backend resumes the parked turn on redeem; the frontend must not resolveGate",
  );
  // The submit leaves the gate untouched — the projection update clears it once
  // the resumed run advances.
  assert.deepEqual(
    stateUpdates.filter((update) => update.index === 4).map((update) => update.value),
    [],
    "a successful redeem must not clear the pending gate locally",
  );
});

test("useChat.submitChannelConnectionPairing: generic channel gate redeems via redeemPairingCode", async () => {
  const threadId = "thread-telegram-pairing-gate";
  const genericCalls = [];

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({
      initialByIndex: new Map([[4, pairingGate("telegram")]]),
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
    queryClient: {
      getQueryData: () => ({ threads: [{ thread_id: threadId }] }),
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    redeemSlackPairingCode: async () => {
      throw new Error("a non-Slack channel must not redeem through Slack");
    },
    redeemPairingCode: async (channel, code, options) => {
      genericCalls.push({ channel, code, options });
      return { success: true };
    },
    removePending,
    resolveGateRequest: async () => {
      throw new Error("pairing redeem must not resolve the gate");
    },
    sendMessage: async () => {
      throw new Error("pairing redeem must not post a continuation message");
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
  await chat.submitChannelConnectionPairing("PAIR42");

  assert.equal(genericCalls.length, 1);
  assert.equal(genericCalls[0].channel, "telegram");
  assert.equal(genericCalls[0].code, "PAIR42");
  assert.equal(genericCalls[0].options.threadId, threadId);
});

test("useChat.submitChannelConnectionPairing: a failed redeem surfaces the error and leaves the gate open", async () => {
  const threadId = "thread-stale-pairing-gate";
  const stateUpdates = [];

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({
      initialByIndex: new Map([[4, pairingGate("slack")]]),
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
    queryClient: {
      getQueryData: () => ({ threads: [{ thread_id: threadId }] }),
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    redeemSlackPairingCode: async () => {
      throw new Error(
        "Invalid or expired Slack pairing code. Run /pair in Slack to get a new one.",
      );
    },
    redeemPairingCode: async () => {
      throw new Error("Slack pairing must use the Slack redemption endpoint");
    },
    removePending,
    resolveGateRequest: async () => {
      throw new Error("a failed pairing redeem must not resolve the gate");
    },
    sendMessage: async () => {
      throw new Error("send should not run");
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
  await assert.rejects(
    () => chat.submitChannelConnectionPairing("STALE123"),
    /Invalid or expired Slack pairing code/,
  );

  assert.deepEqual(
    stateUpdates.filter((update) => update.index === 4).map((update) => update.value),
    [],
    "a failed redeem keeps the pairing gate open so the user can retry",
  );
});

test("useChat: a channel-connected event refreshes the connection caches without touching the gate", async () => {
  // The only cross-surface job left for the channel-connected broadcast is
  // invalidating the extensions / connectable-channels caches so the settings
  // UI in another tab sees "connected". It must NOT resume chats or mutate the
  // pending gate — the parked turn is resumed backend-side and its projection
  // clears the gate.
  const threadId = "thread-cache-refresh";
  const stateUpdates = [];
  const invalidated = [];
  const originalWindow = globalThis.window;
  const broadcasts = new Set();
  class FakeBroadcastChannel {
    constructor(name) {
      this.name = name;
      this.onmessage = null;
      this.closed = false;
      broadcasts.add(this);
    }
    postMessage(payload) {
      for (const channel of broadcasts) {
        if (channel === this || channel.closed || channel.name !== this.name) continue;
        channel.onmessage?.({ data: payload });
      }
    }
    close() {
      this.closed = true;
      broadcasts.delete(this);
    }
  }
  globalThis.window = {
    BroadcastChannel: FakeBroadcastChannel,
    localStorage: { getItem: () => null, setItem: () => {}, removeItem: () => {} },
    addEventListener: () => {},
    removeEventListener: () => {},
  };

  try {
    const context = {
      AbortController,
      Date,
      Error,
      Map,
      Math,
      Set,
      React: createReactStub({
        runEffects: true,
        setCalls: stateUpdates,
        initialByIndex: new Map([[4, pairingGate("slack")]]),
      }),
      addPending,
      toRenderAttachment,
      toWireAttachment,
      cancelRunRequest: async () => {},
      clearInterval,
      clearTimeout,
      createThreadRequest: async () => {
        throw new Error("thread should already exist");
      },
      globalThis: {},
      queryClient: {
        getQueryData: () => ({ threads: [{ thread_id: threadId }] }),
        invalidateQueries: (arg) => invalidated.push(arg?.queryKey?.[0]),
      },
      recordAcceptedMessageRef,
      redeemSlackPairingCode: async () => ({ success: true }),
      redeemPairingCode: async () => ({ success: true }),
      removePending,
      resolveGateRequest: async () => {},
      sendMessage: async () => {
        throw new Error("a channel-connected event must not resume chats");
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
    context.globalThis.__testExports.useChat(threadId);

    const emitter = new globalThis.window.BroadcastChannel("ironclaw-channel-connection");
    emitter.postMessage({ type: "ironclaw:channel-connection:connected", channel: "slack" });
    await new Promise((resolve) => setTimeout(resolve, 0));

    assert.ok(
      invalidated.includes("extensions"),
      "the extensions snapshot is refreshed on connect",
    );
    assert.ok(
      invalidated.includes("connectable-channels"),
      "the connectable-channels snapshot is refreshed on connect",
    );
    assert.deepEqual(
      stateUpdates.filter((update) => update.index === 4).map((update) => update.value),
      [],
      "the channel-connected event must not clear the pending gate itself",
    );
  } finally {
    globalThis.window = originalWindow;
  }
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
    queryClient: {
      fetchQuery: async () => {
        throw new Error("ordinary prompts should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
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

  // (c) isProcessing is cleared (index 3 set to false)
  const isProcessingUpdates = stateUpdates.filter((u) => u.index === 3);
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
    queryClient: {
      fetchQuery: async () => {
        throw new Error("ordinary prompts should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
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

  // isProcessing is cleared (index 3 set to false)
  const isProcessingUpdates = stateUpdates.filter((u) => u.index === 3);
  const lastIsProcessing = isProcessingUpdates[isProcessingUpdates.length - 1];
  assert.equal(lastIsProcessing?.value, false);
});

test("useChat.send: active run refuses duplicate submit before network call", async () => {
  const threadId = "thread-busy-local";
  let renderedMessages = [];
  let sendCalls = 0;

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({ initialByIndex: new Map([[3, true]]) }),
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
      throw new Error("busy prompts should not fetch connectable channels");
    },
    queryClient: {
      fetchQuery: async () => {
        throw new Error("busy prompts should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    resolveGateRequest: async () => {},
    sendMessage: async () => {
      sendCalls += 1;
      throw new Error("busy send should not reach API");
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
  const response = await chat.send("second message while first run is active");

  assert.equal(response, null);
  assert.equal(sendCalls, 0);
  assert.deepEqual(renderedMessages, []);
});

test("useChat.send: accepted run blocks another submit until settlement", async () => {
  const threadId = "thread-1";
  let renderedMessages = [];
  let sendCalls = 0;

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
    queryClient: {
      fetchQuery: async () => {
        throw new Error("ordinary prompts should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    resolveGateRequest: async () => {},
    sendMessage: async ({ content }) => {
      sendCalls += 1;
      return {
        accepted_message_ref: `msg:message-${sendCalls}`,
        run_id: `run-${sendCalls}`,
        status: "queued",
        thread_id: threadId,
        content,
      };
    },
    setInterval,
    setTimeout,
    submitManualToken: async () => {},
    useChatEvents: (args) => {
      context.chatEventsArgs = args;
      return () => {};
    },
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
  const first = await chat.send("first message");
  const second = await chat.send("draft while the reply is still running");

  assert.equal(first.run_id, "run-1");
  assert.equal(second, null);
  assert.equal(sendCalls, 1);
  assert.equal(renderedMessages.length, 1);
  assert.equal(renderedMessages[0].content, "first message");

  context.chatEventsArgs.setIsProcessing(false);
  context.chatEventsArgs.setActiveRun(null);
  context.chatEventsArgs.onRunSettled("run-1", { success: true });

  const third = await chat.send("message after settlement");

  assert.equal(third.run_id, "run-2");
  assert.equal(sendCalls, 2);
});

test("useChat.send: created thread stays blocked until accepted run settles", async () => {
  const createdThreadId = "thread-created";
  let renderedMessages = [];
  let createThreadCalls = 0;
  let sendCalls = 0;
  const seededByThread = new Map();

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
      createThreadCalls += 1;
      return { thread: { thread_id: createdThreadId } };
    },
    globalThis: {},
    listConnectableChannels: async () => {
      throw new Error("ordinary prompts should not fetch connectable channels");
    },
    queryClient: {
      fetchQuery: async () => {
        throw new Error("ordinary prompts should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    timelineMessageIdFromAcceptedRef,
    resolveGateRequest: async () => {},
    sendMessage: async ({ content, threadId }) => {
      sendCalls += 1;
      return {
        accepted_message_ref: `msg:created-${sendCalls}`,
        run_id: `run-${sendCalls}`,
        status: "queued",
        thread_id: threadId,
        content,
      };
    },
    setInterval,
    setTimeout,
    submitManualToken: async () => {},
    useChatEvents: (args) => {
      context.chatEventsArgs = args;
      return () => {};
    },
    useHistory: () => ({
      messages: renderedMessages,
      hasMore: false,
      nextCursor: null,
      isLoading: false,
      loadHistory: () => {},
      seedThreadMessages: (threadId, updater) => {
        const prev = seededByThread.get(threadId) || [];
        seededByThread.set(
          threadId,
          typeof updater === "function" ? updater(prev) : updater,
        );
      },
      setMessages: (updater) => {
        renderedMessages =
          typeof updater === "function" ? updater(renderedMessages) : updater;
      },
    }),
    useSSE: () => ({ status: "idle" }),
  };

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat(null);
  const first = await chat.send("first message creates the thread");
  assert.equal(createThreadCalls, 1);
  assert.equal(first.run_id, "run-1");
  assert.equal(first.thread_id, createdThreadId);

  context.chatEventsArgs.setIsProcessing(false);
  context.chatEventsArgs.setActiveRun(null);

  const second = await chat.send("draft while the reply is still running", {
    threadId: createdThreadId,
  });
  assert.equal(second, null);
  assert.equal(sendCalls, 1);

  context.chatEventsArgs.onRunSettled("run-1", { success: true });

  const third = await chat.send("message after settlement", {
    threadId: createdThreadId,
  });
  assert.equal(third.run_id, "run-2");
  assert.equal(sendCalls, 2);
});

test("useChat.send: clears local busy when run settles before send response", async () => {
  const createdThreadId = "thread-created";
  let renderedMessages = [];
  let sendCalls = 0;
  const seededByThread = new Map();

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
    createThreadRequest: async () => ({
      thread: { thread_id: createdThreadId },
    }),
    globalThis: {},
    listConnectableChannels: async () => {
      throw new Error("ordinary prompts should not fetch connectable channels");
    },
    queryClient: {
      fetchQuery: async () => {
        throw new Error("ordinary prompts should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    timelineMessageIdFromAcceptedRef,
    resolveGateRequest: async () => {},
    sendMessage: async ({ content, threadId }) => {
      sendCalls += 1;
      const runId = `run-${sendCalls}`;
      if (sendCalls === 1) {
        context.chatEventsArgs.setIsProcessing(false);
        context.chatEventsArgs.setActiveRun(null);
        context.chatEventsArgs.onRunSettled(runId, { success: true });
      }
      return {
        accepted_message_ref: `msg:early-settled-${sendCalls}`,
        run_id: runId,
        status: sendCalls === 1 ? "completed" : "queued",
        thread_id: threadId,
        content,
      };
    },
    setInterval,
    setTimeout,
    submitManualToken: async () => {},
    useChatEvents: (args) => {
      context.chatEventsArgs = args;
      return () => {};
    },
    useHistory: () => ({
      messages: renderedMessages,
      hasMore: false,
      nextCursor: null,
      isLoading: false,
      loadHistory: () => {},
      seedThreadMessages: (threadId, updater) => {
        const prev = seededByThread.get(threadId) || [];
        seededByThread.set(
          threadId,
          typeof updater === "function" ? updater(prev) : updater,
        );
      },
      setMessages: (updater) => {
        renderedMessages =
          typeof updater === "function" ? updater(renderedMessages) : updater;
      },
    }),
    useSSE: () => ({ status: "idle" }),
  };

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat(null);
  const first = await chat.send("first message settles before response");
  const second = await chat.send("message after early settlement", {
    threadId: createdThreadId,
  });

  assert.equal(first.run_id, "run-1");
  assert.equal(second.run_id, "run-2");
  assert.equal(sendCalls, 2);
});

test("useChat.send: clears local admission when navigating away before settlement", async () => {
  const threadA = "thread-a";
  const threadB = "thread-b";
  let renderedMessages = [];
  let sendCalls = 0;
  const seededByThread = new Map();
  const ReactStub = createReactStub({ runEffects: true });

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: ReactStub,
    addPending,
    toRenderAttachment,
    toWireAttachment,
    cancelRunRequest: async () => {},
    clearTimeout,
    createThreadRequest: async () => {
      throw new Error("threads already exist in this scenario");
    },
    globalThis: {},
    listConnectableChannels: async () => {
      throw new Error("ordinary prompts should not fetch connectable channels");
    },
    queryClient: {
      fetchQuery: async () => {
        throw new Error("ordinary prompts should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    timelineMessageIdFromAcceptedRef,
    resolveGateRequest: async () => {},
    sendMessage: async ({ content, threadId }) => {
      sendCalls += 1;
      return {
        accepted_message_ref: `msg:navigation-${sendCalls}`,
        run_id: `run-${sendCalls}`,
        status: "queued",
        thread_id: threadId,
        content,
      };
    },
    setInterval,
    setTimeout,
    submitManualToken: async () => {},
    useChatEvents: (args) => {
      context.chatEventsArgs = args;
      return () => {};
    },
    useHistory: () => ({
      messages: renderedMessages,
      hasMore: false,
      nextCursor: null,
      isLoading: false,
      loadHistory: () => {},
      seedThreadMessages: (threadId, updater) => {
        const prev = seededByThread.get(threadId) || [];
        seededByThread.set(
          threadId,
          typeof updater === "function" ? updater(prev) : updater,
        );
      },
      setMessages: (updater) => {
        renderedMessages =
          typeof updater === "function" ? updater(renderedMessages) : updater;
      },
    }),
    useSSE: () => ({ status: "idle" }),
  };

  runUseChatSource(context);
  const renderChat = (threadId) => {
    ReactStub.__beginRender();
    return context.globalThis.__testExports.useChat(threadId);
  };

  let chat = renderChat(threadA);
  const first = await chat.send("first message on thread A");
  assert.equal(first.run_id, "run-1");

  renderChat(threadB);
  renderChat(threadB);

  chat = renderChat(threadA);
  chat = renderChat(threadA);
  context.chatEventsArgs.setIsProcessing(false);
  context.chatEventsArgs.setActiveRun(null);

  const second = await chat.send("resend after returning to thread A");

  assert.equal(second.run_id, "run-2");
  assert.equal(sendCalls, 2);
});

test("useChat.send: a send to another thread is not blocked by an unsettled run (submitBusyRef deadlock #5256)", async () => {
  // The deadlock that hand-rolled unit fixtures missed: `submitBusyRef` is set
  // on send and was only released in `onRunSettled` (delivered over the *open*
  // thread's SSE). When the user starts a chat and then addresses a different
  // thread — the new-chat case — before the first run settles, that settle
  // event may never reach this hook (its SSE is gone), so the guard stays held
  // and every later send is silently dropped. A send whose destination is NOT
  // the running thread must go through; only the per-destination
  // `activeRunBlocksSend` guard may stop a resubmit into the busy thread.
  const viewedThread = "thread-a";
  let sendCalls = 0;
  let renderedMessages = [];
  const seededByThread = new Map();

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
      throw new Error("threads already exist in this scenario");
    },
    globalThis: {},
    listConnectableChannels: async () => {
      throw new Error("ordinary prompts should not fetch connectable channels");
    },
    queryClient: {
      fetchQuery: async () => {
        throw new Error("ordinary prompts should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    timelineMessageIdFromAcceptedRef,
    resolveGateRequest: async () => {},
    sendMessage: async ({ threadId }) => {
      sendCalls += 1;
      return {
        accepted_message_ref: `msg:message-${sendCalls}`,
        run_id: `run-${sendCalls}`,
        status: "queued",
        thread_id: threadId,
      };
    },
    setInterval,
    setTimeout,
    submitManualToken: async () => {},
    // onRunSettled is deliberately NEVER fired: the first run stays in flight
    // exactly as it would after navigating away from the running thread.
    useChatEvents: (args) => {
      context.chatEventsArgs = args;
      return () => {};
    },
    useHistory: () => ({
      messages: renderedMessages,
      hasMore: false,
      nextCursor: null,
      isLoading: false,
      loadHistory: () => {},
      seedThreadMessages: (threadId, updater) => {
        const prev = seededByThread.get(threadId) || [];
        seededByThread.set(threadId, typeof updater === "function" ? updater(prev) : updater);
      },
      setMessages: (updater) => {
        renderedMessages =
          typeof updater === "function" ? updater(renderedMessages) : updater;
      },
    }),
    useSSE: () => ({ status: "idle" }),
  };

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat(viewedThread);
  // 1) Start a run on the viewed thread. This sets submitBusyRef = true.
  const first = await chat.send("kick off a run on thread-a");
  assert.equal(first.run_id, "run-1");
  // 2) Without the run ever settling, send to a DIFFERENT thread (the new-chat
  //    case). With the deadlock, submitBusyRef is still held and this returns
  //    null; with the fix it is released when the POST settled, so it goes
  //    through.
  const second = await chat.send("hi how are you", { threadId: "thread-b" });
  assert.ok(second, "send to another thread must not be blocked by the unsettled run");
  assert.equal(second.run_id, "run-2");
  assert.equal(sendCalls, 2, "sendMessage must be called for the second, different-thread send");
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
  // isProcessing(3), pendingGate(4).
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
        [3, true],
        [4, pendingGate],
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
    queryClient: {
      fetchQuery: async () => ({ channels: [] }),
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
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

  // pendingGate (index 4) is cleared
  const pendingGateUpdates = stateUpdates.filter((u) => u.index === 4);
  assert.equal(pendingGateUpdates.length, 1);
  assert.equal(pendingGateUpdates[0].value, null);

  // isProcessing (index 3) is set to true — run continues
  const isProcessingUpdates = stateUpdates.filter((u) => u.index === 3);
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

  // isProcessing (index 3) is set to true — run continues
  const isProcessingUpdates = stateUpdates.filter((u) => u.index === 3);
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

  const isProcessingUpdates = stateUpdates.filter((u) => u.index === 3);
  assert.ok(isProcessingUpdates.length > 0, "isProcessing should be updated");
  assert.equal(isProcessingUpdates[isProcessingUpdates.length - 1].value, false);

  const pendingGateUpdates = stateUpdates.filter((u) => u.index === 4);
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

  const isProcessingUpdates = stateUpdates.filter((u) => u.index === 3);
  assert.ok(isProcessingUpdates.length > 0);
  const lastIsProcessing = isProcessingUpdates[isProcessingUpdates.length - 1];
  assert.equal(lastIsProcessing.value, true);
});

// ---------------------------------------------------------------------------
// Parallel-thread send admission (regression for #5256).
//
// The send gate must block ONLY a duplicate send into the thread that
// already has a run in flight. A run on some *other* thread — most often the
// one currently on screen — must never stop the user from starting a new
// chat or addressing a different thread in parallel. #5256 keyed the gate on
// `!targetThreadId` and on the *viewed* thread id, which silently dropped
// ("doesn't accept from front end") any send while another thread was busy.
//
// These tests drive the real `send` caller and assert the functional
// outcome — whether the request actually reaches `sendMessage`/`createThread`
// — with no dependence on DOM, markup, or class names.
// ---------------------------------------------------------------------------

function createParallelSendContext({
  threadId,
  activeRun,
  isProcessing,
  createdThreadId = "thread-created",
  stateUpdates = [],
} = {}) {
  let sentBody = null;
  let createThreadCalls = 0;
  let currentMessages = [];
  const seededByThread = new Map();
  const initialByIndex = new Map();
  // State slot order: cooldownUntil(0), now(1), activeRun(2),
  // isProcessing(3), pendingGate(4), busyGateNotice(5), stateThreadId(6).
  if (activeRun !== undefined) {
    initialByIndex.set(2, activeRun);
  }
  // A thread with an in-flight run carries BOTH activeRun and isProcessing.
  // Seeding isProcessing for the viewed-thread cases is what makes these
  // fixtures reproduce the real busy state rather than a half-state the
  // `isProcessing` early-return would never trip.
  if (isProcessing !== undefined) {
    initialByIndex.set(3, isProcessing);
  }

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({ initialByIndex, setCalls: stateUpdates }),
    addPending,
    toRenderAttachment,
    toWireAttachment,
    cancelRunRequest: async () => {},
    clearTimeout,
    createThreadRequest: async () => {
      createThreadCalls += 1;
      return { thread: { thread_id: createdThreadId } };
    },
    globalThis: {},
    listConnectableChannels: async () => {
      throw new Error("ordinary prompts should not fetch connectable channels");
    },
    queryClient: {
      fetchQuery: async () => {
        throw new Error("ordinary prompts should not fetch connectable channels");
      },
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    timelineMessageIdFromAcceptedRef,
    resolveGateRequest: async () => {},
    sendMessage: async (body) => {
      sentBody = body;
      return {
        accepted_message_ref: "msg:parallel-1",
        run_id: "run-parallel",
        status: "queued",
        thread_id: body.threadId,
      };
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
      seedThreadMessages: (seedThreadId, updater) => {
        const previous = seededByThread.get(seedThreadId) || [];
        const next = typeof updater === "function" ? updater(previous) : updater;
        seededByThread.set(seedThreadId, next);
      },
      setMessages: (updater) => {
        currentMessages =
          typeof updater === "function" ? updater(currentMessages) : updater;
      },
    }),
    useSSE: () => ({ status: "idle" }),
  };

  return {
    context,
    sentBody: () => sentBody,
    createThreadCalls: () => createThreadCalls,
  };
}

test("useChat.send: starts a new chat while another thread's run is active", async () => {
  // Landing screen (no open thread), but a run on `thread-busy` is still
  // tracked in activeRun. Starting a brand-new chat must create the thread
  // and send — the active run belongs to a different thread.
  const { context, sentBody, createThreadCalls } = createParallelSendContext({
    threadId: null,
    activeRun: { runId: "run-busy", threadId: "thread-busy", status: "running" },
    createdThreadId: "thread-new",
  });

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat(null);
  const result = await chat.send("start a parallel conversation");

  assert.equal(createThreadCalls(), 1, "a new thread must be created");
  assert.ok(sentBody(), "the message must reach sendMessage, not be dropped");
  assert.equal(sentBody().threadId, "thread-new");
  assert.equal(sentBody().content, "start a parallel conversation");
  assert.ok(result, "send must resolve with a response, not null");
});

test("useChat.send: addresses a second thread in parallel while viewing a running thread", async () => {
  // Viewing thread-a while its run is genuinely in flight — so BOTH activeRun
  // and isProcessing are set, the real busy state. A send explicitly addressed
  // to a different thread-b must still be delivered; neither the viewed
  // thread's active run nor its isProcessing flag may block a parallel thread.
  const { context, sentBody } = createParallelSendContext({
    threadId: "thread-a",
    activeRun: { runId: "run-a", threadId: "thread-a", status: "running" },
    isProcessing: true,
  });

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat("thread-a");
  const result = await chat.send("message for the other thread", {
    threadId: "thread-b",
  });

  assert.ok(sentBody(), "the parallel-thread message must reach sendMessage");
  assert.equal(sentBody().threadId, "thread-b");
  assert.ok(result, "send must resolve with a response, not null");
});

test("useChat.send: still blocks a duplicate send into the already-running thread", async () => {
  // The one case the gate must keep blocking: a second send into the SAME
  // thread that already has a run in flight (both activeRun and isProcessing
  // set — the real busy state).
  const { context, sentBody, createThreadCalls } = createParallelSendContext({
    threadId: "thread-a",
    activeRun: { runId: "run-a", threadId: "thread-a", status: "running" },
    isProcessing: true,
  });

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat("thread-a");
  const result = await chat.send("duplicate into the busy thread", {
    threadId: "thread-a",
  });

  assert.equal(result, null, "duplicate send into the busy thread is rejected");
  assert.equal(sentBody(), null, "sendMessage must not be called for a busy thread");
  assert.equal(createThreadCalls(), 0);
});

test("useChat.send: blocks a send addressed to a busy thread that is NOT the viewed one", async () => {
  // The block must key on the *destination* thread, not the viewed one:
  // viewing thread-a, but the active run is on thread-b, and the send is
  // addressed to thread-b — that destination is busy, so it must be blocked.
  // This complements the parallel-send test (viewed busy, different target →
  // allowed) so the pair pins the block on destination identity alone.
  const { context, sentBody, createThreadCalls } = createParallelSendContext({
    threadId: "thread-a",
    activeRun: { runId: "run-b", threadId: "thread-b", status: "running" },
  });

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat("thread-a");
  const result = await chat.send("into the busy non-viewed thread", {
    threadId: "thread-b",
  });

  assert.equal(result, null, "send into the busy destination thread is rejected");
  assert.equal(sentBody(), null, "sendMessage must not be called for the busy destination");
  assert.equal(createThreadCalls(), 0);
});
