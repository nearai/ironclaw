// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

import { messagesFromTimeline } from "./history-messages";
import { toRenderAttachment, toWireAttachment } from "./attachments";
import {
  addPending,
  recordAcceptedMessageRef,
  removePending,
  timelineMessageIdFromAcceptedRef,
} from "./pending-messages";
import {
  createToolActivityState,
  failGateToolActivity,
  resetToolActivityState,
} from "./tool-activity-state";
import {
  CONNECTION_LOST_RUN_FAILURE_MESSAGE,
  rewriteConnectionLostRunFailures,
  upsertConnectionLostRunFailure,
} from "./failureMessages";
import {
  CONNECTION_STATUS,
  isConnectionLostStatus,
} from "./connection-status";
import {
  CHAT_MESSAGE_ROLES,
  createErrorChatMessage,
  createRequestFailureChatMessage,
  isRequestFailureForMessage,
  requestFailureIdForMessage,
} from "./message-types";
import {
  channelConnectionContinuationMessage,
  connectionEventMatchesOnboarding,
  forgetChannelConnectionWaiter,
  normalizeConnectionChannel,
  rememberChannelConnectionWaiter,
  subscribeChannelConnected,
} from "../../../lib/channel-connection-events";
import { productAuthOAuthEventsSource } from "../../../lib/product-auth-oauth-events.vm-inline";
import { moduleSourceForVm } from "../../../lib/vm-inline-source";

const STATE_SLOT = Object.freeze({
  cooldownUntil: 0,
  now: 1,
  activeRun: 2,
  isProcessing: 3,
  pendingGate: 4,
  pendingOnboarding: 5,
  busyGateNotice: 6,
  stateThreadId: 7,
});

function stateUpdatesFor(updates, slot) {
  return updates.filter((update) => update.index === slot);
}

function useChatSourceForTest() {
  const source = readFileSync(
    new URL("../hooks/useChat.ts", import.meta.url),
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
  // Inline the shared OAuth-events source and the extracted `useChannelOnboarding`
  // hook so their window-dependent primitives (e.g. openAuthPopup) compile inside
  // the vm and resolve the per-test `window`, and so `useChat` drives the real
  // onboarding state machine (not a stub) through the caller. The onboarding
  // source is inlined after OAuth-events (which it imports from) and before the
  // hook body that calls it.
  const channelOnboardingSource = moduleSourceForVm(
    new URL("../hooks/useChannelOnboarding.ts", import.meta.url),
  );
  return `${productAuthOAuthEventsSource()}\n${channelOnboardingSource}\n${lines.join("\n")}\nglobalThis.__testExports = { useChat };`;
}

function runUseChatSource(context) {
  Object.assign(context, {
    createToolActivityState,
    failGateToolActivity,
    resetToolActivityState,
    timelineMessageIdFromAcceptedRef,
    rewriteConnectionLostRunFailures,
    upsertConnectionLostRunFailure,
    CONNECTION_STATUS,
    isConnectionLostStatus,
    channelConnectionContinuationMessage,
    CHAT_MESSAGE_ROLES,
    connectionEventMatchesOnboarding,
    createErrorChatMessage,
    createRequestFailureChatMessage,
    forgetChannelConnectionWaiter,
    isRequestFailureForMessage,
    normalizeConnectionChannel,
    rememberChannelConnectionWaiter,
    requestFailureIdForMessage,
  });
  if (!context.subscribeChannelConnected) {
    context.subscribeChannelConnected = subscribeChannelConnected;
  }
  if (!("failureMessageForRequestError" in context)) {
    context.failureMessageForRequestError = (error) =>
      typeof error?.message === "string" && error.message.trim()
        ? error.message.trim()
        : "request failed";
  }
  if (!context.notifyChannelConnected) context.notifyChannelConnected = async () => {};
  if (!context.redeemPairingCode) {
    context.redeemPairingCode = async () => ({ success: true });
  }
  if (!context.fetchExtensionSetup) {
    context.fetchExtensionSetup = async () => ({ secrets: [] });
  }
  if (!context.startExtensionOauth) {
    context.startExtensionOauth = async () => ({ success: false });
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

test("useChat: disconnected SSE rewrites an active driver_unavailable error", () => {
  const threadId = "thread-1";
  const setCalls = [];
  let renderedMessages = [
    {
      id: "err-run-1",
      role: "error",
      content:
        "The run failed because the execution driver was temporarily unavailable.",
      failureStatus: "failed",
      failureCategory: "driver_unavailable",
      failureSummary:
        "The run failed because the execution driver was temporarily unavailable.",
    },
  ];
  const initialByIndex = new Map([
    [STATE_SLOT.activeRun, { runId: "run-1", threadId, status: "running" }],
    [STATE_SLOT.isProcessing, true],
  ]);
  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({ initialByIndex, setCalls, runEffects: true }),
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
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    resolveGateRequest: async () => {},
    sendMessage: async () => {
      throw new Error("send should not run");
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
      loadError: null,
      loadHistory: () => {},
      seedThreadMessages: () => {},
      setMessages: (updater) => {
        renderedMessages =
          typeof updater === "function" ? updater(renderedMessages) : updater;
      },
    }),
    useSSE: () => ({ status: CONNECTION_STATUS.DISCONNECTED }),
  };

  runUseChatSource(context);
  const chat = context.globalThis.__testExports.useChat(threadId);

  assert.equal(chat.sseStatus, CONNECTION_STATUS.DISCONNECTED);
  assert.equal(renderedMessages.length, 1);
  assert.equal(renderedMessages[0].content, CONNECTION_LOST_RUN_FAILURE_MESSAGE);
  assert.equal(
    stateUpdatesFor(setCalls, STATE_SLOT.isProcessing).at(-1)?.value,
    false,
  );
  assert.equal(
    stateUpdatesFor(setCalls, STATE_SLOT.activeRun).at(-1)?.value,
    null,
  );
});

test("useChat: disconnected SSE surfaces connection error before run id is known", () => {
  const threadId = "thread-1";
  const setCalls = [];
  const historicalFailure =
    "The run failed because the execution driver was temporarily unavailable.";
  let renderedMessages = [
    {
      id: "err-old-run",
      role: "error",
      content: historicalFailure,
      failureStatus: "failed",
      failureCategory: "driver_unavailable",
      failureSummary: historicalFailure,
    },
  ];
  const initialByIndex = new Map([[STATE_SLOT.isProcessing, true]]);
  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({ initialByIndex, setCalls, runEffects: true }),
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
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    resolveGateRequest: async () => {},
    sendMessage: async () => {
      throw new Error("send should not run");
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
      loadError: null,
      loadHistory: () => {},
      seedThreadMessages: () => {},
      setMessages: (updater) => {
        renderedMessages =
          typeof updater === "function" ? updater(renderedMessages) : updater;
      },
    }),
    useSSE: () => ({ status: CONNECTION_STATUS.DISCONNECTED }),
  };

  runUseChatSource(context);
  const chat = context.globalThis.__testExports.useChat(threadId);

  assert.equal(chat.sseStatus, CONNECTION_STATUS.DISCONNECTED);
  assert.equal(renderedMessages.length, 2);
  assert.equal(renderedMessages[0].content, historicalFailure);
  assert.equal(renderedMessages[1].id, "err-connection-lost");
  assert.equal(renderedMessages[1].content, CONNECTION_LOST_RUN_FAILURE_MESSAGE);
  assert.equal(
    stateUpdatesFor(setCalls, STATE_SLOT.isProcessing).at(-1)?.value,
    false,
  );
  assert.equal(
    stateUpdatesFor(setCalls, STATE_SLOT.activeRun).at(-1)?.value,
    null,
  );
});

test("useChat: disconnected SSE ignores a stale active run after processing ended", () => {
  const threadId = "thread-1";
  const setCalls = [];
  let renderedMessages = [];
  const initialByIndex = new Map([
    [STATE_SLOT.activeRun, { runId: "run-1", threadId, status: "running" }],
    [STATE_SLOT.isProcessing, false],
  ]);
  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({ initialByIndex, setCalls, runEffects: true }),
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
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    resolveGateRequest: async () => {},
    sendMessage: async () => {
      throw new Error("send should not run");
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
      loadError: null,
      loadHistory: () => {},
      seedThreadMessages: () => {},
      setMessages: (updater) => {
        renderedMessages =
          typeof updater === "function" ? updater(renderedMessages) : updater;
      },
    }),
    useSSE: () => ({ status: CONNECTION_STATUS.DISCONNECTED }),
  };

  runUseChatSource(context);
  const chat = context.globalThis.__testExports.useChat(threadId);

  assert.equal(chat.sseStatus, CONNECTION_STATUS.DISCONNECTED);
  assert.deepEqual(renderedMessages, []);
  assert.equal(stateUpdatesFor(setCalls, STATE_SLOT.isProcessing).length, 0);
  assert.equal(stateUpdatesFor(setCalls, STATE_SLOT.activeRun).length, 0);
});

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
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
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
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
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
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
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
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
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
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
  };

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat(currentThreadId);
  await assert.rejects(
    chat.send("send while network is down", { threadId: targetThreadId }),
    /network unavailable/,
  );

  assert.deepEqual(currentMessages, []);
  const targetMessages = seededByThread.get(targetThreadId);
  assert.equal(targetMessages.length, 2);
  assert.equal(targetMessages[0].role, "user");
  assert.equal(targetMessages[0].isOptimistic, false);
  assert.equal(targetMessages[0].status, "error");
  assert.equal(targetMessages[0].error, "network unavailable");
  assert.equal(targetMessages[1].role, "error");
  assert.equal(targetMessages[1].content, "network unavailable");
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
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
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

test("useChat.send: request failure appends inline error in the active thread", async () => {
  const threadId = "thread-1";
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
    failureMessageForRequestError: (error) =>
      `inline:${error?.message || "unknown"}`,
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
      throw new Error("AI provider account is out of credits");
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
  await assert.rejects(chat.send("please answer"), /out of credits/);

  assert.equal(renderedMessages.length, 2);
  assert.equal(renderedMessages[0].role, "user");
  assert.equal(renderedMessages[0].status, "error");
  assert.equal(
    renderedMessages[0].error,
    "inline:AI provider account is out of credits",
  );
  assert.equal(renderedMessages[1].role, "error");
  assert.equal(renderedMessages[1].requestForMessageId, renderedMessages[0].id);
  assert.equal(
    renderedMessages[1].content,
    "inline:AI provider account is out of credits",
  );
});

test("useChat.send: create-thread failure appends inline error on new chat", async () => {
  let renderedMessages = [];
  let createThreadCalls = 0;
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
      createThreadCalls += 1;
      throw new Error("Thread service unavailable");
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

  const chat = context.globalThis.__testExports.useChat(null);
  await assert.rejects(
    chat.send("start a new chat"),
    /Thread service unavailable/,
  );

  assert.equal(createThreadCalls, 1);
  assert.equal(sendCalls, 0);
  assert.equal(renderedMessages.length, 1);
  assert.equal(renderedMessages[0].role, "error");
  assert.equal(renderedMessages[0].content, "Thread service unavailable");
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
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
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

test("useChat.retryMessage: retry removes the prior request error bubble", async () => {
  const threadId = "thread-1";
  const failedMessage = {
    id: "failed-1",
    role: "user",
    content: "retry me",
    retryContent: "retry me",
    status: "error",
  };
  const failedRequestError = {
    id: "err-request-legacy-or-renamed-id",
    role: "error",
    content: "Network unavailable",
    requestForMessageId: "failed-1",
  };
  let renderedMessages = [failedMessage, failedRequestError];
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
    timelineMessageIdFromAcceptedRef,
    resolveGateRequest: async () => {},
    sendMessage: async () => {
      sendCalls += 1;
      return {
        accepted_message_ref: "msg:retry-success",
        run_id: "run-retry-success",
        status: "queued",
        thread_id: threadId,
      };
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
  await chat.retryMessage(failedMessage);

  assert.equal(sendCalls, 1);
  assert.equal(
    renderedMessages.some((message) => message.id === failedMessage.id),
    false,
  );
  assert.equal(
    renderedMessages.some((message) => message.id === failedRequestError.id),
    false,
  );
  assert.equal(renderedMessages.at(-1)?.role, "user");
  assert.equal(renderedMessages.at(-1)?.content, "retry me");
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
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
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
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
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
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
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
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
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
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
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
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
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
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
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
      // useChat state call order: cooldownUntil, now, activeRun,
      // isProcessing, pendingGate, pendingOnboarding, busyGateNotice,
      // stateThreadId.
      initialByIndex: new Map([
        [2, { runId: "run-old", threadId: "thread-old", status: "awaiting_gate" }],
        [3, true],
        [4, { runId: "run-old", gateRef: "gate-old" }],
        [
          5,
          {
            extensionName: "telegram",
            state: "pairing_required",
            threadId: "thread-old",
          },
        ],
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
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
  };

  runUseChatSource(context);
  const chat = context.globalThis.__testExports.useChat("thread-new");

  assert.deepEqual(stateUpdates.slice(0, 6), [
    { index: 7, value: "thread-new" },
    { index: 3, value: false },
    { index: 4, value: null },
    { index: 5, value: null },
    { index: 6, value: null },
    { index: 2, value: null },
  ]);
  assert.equal(
    chat.pendingOnboarding,
    null,
    "onboarding owned by the previous thread must never render in the new thread",
  );
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
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
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
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
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
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
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
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
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
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
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
          strategy: "oauth",
          command_aliases: ["slack", "slack account"],
          action: {
            title: "Slack account connection",
            instructions:
              "Connect Slack with OAuth from the extension configuration, then message the Slack bot directly.",
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
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
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
          strategy: "oauth",
          command_aliases: ["slack", "slack account"],
          action: {
            title: "Slack account connection",
            instructions:
              "Connect Slack with OAuth from the extension configuration, then message the Slack bot directly.",
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
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
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
// challenge whose gate also carries a `connection` requirement. `gates.ts`
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

test("useChat.submitChannelConnectionPairing: Slack gate redeems through the generic channel endpoint", async () => {
  const threadId = "thread-slack-pairing-gate";
  const stateUpdates = [];
  const pairingCalls = [];
  let resolveGateCalls = 0;

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({
      initialByIndex: new Map([[STATE_SLOT.pendingGate, pairingGate("slack")]]),
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
    redeemPairingCode: async (channel, code, options) => {
      pairingCalls.push({ channel, code, options });
      return { success: true };
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
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
  };

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat(threadId);
  const response = await chat.submitChannelConnectionPairing(" A1B2C3 ");

  assert.equal(response.success, true);
  assert.deepEqual(JSON.parse(JSON.stringify(pairingCalls)), [
    { channel: "slack", code: "A1B2C3", options: { threadId } },
  ]);
  assert.equal(
    resolveGateCalls,
    0,
    "redeeming the channel connection resumes the parked turn server-side",
  );
  assert.deepEqual(
    stateUpdates
      .filter((update) => update.index === STATE_SLOT.pendingGate)
      .map((update) => update.value),
    [null],
    "successful redemption clears the local pending gate",
  );
  assert.deepEqual(
    stateUpdates
      .filter((update) => update.index === STATE_SLOT.isProcessing)
      .map((update) => update.value),
    [true],
    "the resumed turn is shown as processing while SSE catches up",
  );
});

test("useChat.submitChannelConnectionPairing: generic channel gate redeems via redeemPairingCode", async () => {
  const threadId = "thread-telegram-pairing-gate";
  const pairingCalls = [];

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({
      initialByIndex: new Map([[STATE_SLOT.pendingGate, pairingGate("telegram")]]),
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
    redeemPairingCode: async (channel, code, options) => {
      pairingCalls.push({ channel, code, options });
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
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
  };

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat(threadId);
  await chat.submitChannelConnectionPairing("PAIR42");

  assert.deepEqual(JSON.parse(JSON.stringify(pairingCalls)), [
    { channel: "telegram", code: "PAIR42", options: { threadId } },
  ]);
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
      initialByIndex: new Map([[STATE_SLOT.pendingGate, pairingGate("slack")]]),
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
    redeemPairingCode: async () => {
      throw new Error("Invalid or expired pairing code.");
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
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
  };

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat(threadId);
  await assert.rejects(
    () => chat.submitChannelConnectionPairing("STALE123"),
    /Invalid or expired pairing code\./,
  );

  assert.deepEqual(
    stateUpdates
      .filter((update) => update.index === STATE_SLOT.pendingGate)
      .map((update) => update.value),
    [],
    "a failed redeem keeps the pairing gate open so the user can retry",
  );
});

test("useChat: a channel-connected event refreshes the connection caches without touching the gate", async () => {
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
        initialByIndex: new Map([[STATE_SLOT.pendingGate, pairingGate("slack")]]),
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
      useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
    };

    runUseChatSource(context);
    context.globalThis.__testExports.useChat(threadId);

    const emitter = new globalThis.window.BroadcastChannel(
      "ironclaw-channel-connection",
    );
    emitter.postMessage({
      type: "ironclaw:channel-connection:connected",
      channel: "slack",
    });
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
      stateUpdates
        .filter((update) => update.index === STATE_SLOT.pendingGate)
        .map((update) => update.value),
      [],
      "the channel-connected event must not clear the pending gate itself",
    );
  } finally {
    globalThis.window = originalWindow;
  }
});

test("useChat.submitOnboardingPairing: generic redemption resumes chat without leaking code", async () => {
  const threadId = "thread-telegram-pairing";
  const stateUpdates = [];
  const pairingCalls = [];
  const sendBodies = [];
  let renderedMessages = [];

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({
      initialByIndex: new Map([
        [
          5,
          {
            state: "pairing_required",
            extensionName: "telegram",
            threadId,
            requestId: null,
          },
        ],
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
    queryClient: {
      getQueryData: () => ({
        threads: [{ thread_id: threadId, title: "Telegram pairing thread" }],
      }),
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    redeemPairingCode: async (channel, code, options) => {
      pairingCalls.push({ channel, code, options });
      return { success: true };
    },
    removePending,
    resolveGateRequest: async () => {},
    sendMessage: async (body) => {
      sendBodies.push(body);
      return {
        accepted_message_ref: "msg:message-continue",
        run_id: "run-continue",
        status: "queued",
        thread_id: body.threadId,
      };
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
  const response = await chat.submitOnboardingPairing(" A1B2C3 ");

  assert.equal(pairingCalls.length, 1);
  assert.equal(pairingCalls[0].channel, "telegram");
  assert.equal(pairingCalls[0].code, "A1B2C3");
  assert.equal(pairingCalls[0].options.threadId, threadId);
  assert.equal(pairingCalls[0].options.requestId, null);
  assert.equal(sendBodies.length, 1);
  assert.equal(sendBodies[0].threadId, threadId);
  assert.equal(
    sendBodies[0].content,
    "Telegram is connected. Continue the previous request.",
  );
  assert.doesNotMatch(JSON.stringify(sendBodies), /A1B2C3/);
  assert.equal(response.success, true);
  assert.ok(
    stateUpdates.some((update) => update.index === 5 && update.value === null),
    "the pairing panel should clear after the continuation send succeeds",
  );
});

test("useChat.submitOnboardingPairing: failed local resume keeps pairing panel retryable", async () => {
  const threadId = "thread-telegram-pairing-retry";
  const sourceMessageId = "tool-telegram-activation";
  const stateUpdates = [];
  const storageValues = new Map();

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({
      initialByIndex: new Map([
        [
          5,
          {
            state: "pairing_required",
            extensionName: "telegram",
            threadId,
            requestId: null,
            sourceMessageId,
          },
        ],
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
    globalThis: {
      localStorage: {
        getItem: (key) => (storageValues.has(key) ? storageValues.get(key) : null),
        setItem: (key, value) => storageValues.set(key, String(value)),
      },
    },
    queryClient: {
      getQueryData: () => ({
        threads: [{ thread_id: threadId, title: "Telegram pairing thread" }],
      }),
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    resolveGateRequest: async () => {},
    sendMessage: async () => {
      throw new Error("transient continuation failure");
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
    () => chat.submitOnboardingPairing("A1B2C3"),
    /transient continuation failure/,
  );

  assert.equal(
    stateUpdates.some((update) => update.index === 5 && update.value === null),
    false,
    "failed continuation must not clear the pairing panel",
  );
  assert.equal(
    storageValues.has(`ironclaw.chat.dismissedOnboarding.v1:${threadId}`),
    false,
    "failed continuation must not persist a durable dismissal",
  );
});

test("useChat.submitOnboardingPairing: admission-blocked resume keeps the pairing panel and waiter", async () => {
  const threadId = "thread-telegram-pairing-busy";
  const sourceMessageId = "tool-telegram-activation";
  const stateUpdates = [];
  const storageValues = new Map();
  const sentContents = [];

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({
      initialByIndex: new Map([
        [
          5,
          {
            state: "pairing_required",
            extensionName: "telegram",
            threadId,
            requestId: null,
            sourceMessageId,
          },
        ],
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
    globalThis: {
      localStorage: {
        getItem: (key) => (storageValues.has(key) ? storageValues.get(key) : null),
        setItem: (key, value) => storageValues.set(key, String(value)),
      },
    },
    queryClient: {
      getQueryData: () => ({
        threads: [{ thread_id: threadId, title: "Telegram pairing thread" }],
      }),
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    resolveGateRequest: async () => {},
    sendMessage: ({ content }) => {
      sentContents.push(content);
      // Never settles: keeps the submit re-entrancy guard held so the
      // pairing continuation hits the admission block, not the network.
      return new Promise(() => {});
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
  // Occupy the submit guard with a send to another thread whose POST never
  // settles, so the pairing continuation is admission-blocked (send → null).
  chat.send("occupy the submit guard", { threadId: "thread-other" });
  await Promise.resolve();
  assert.deepEqual(sentContents, ["occupy the submit guard"]);

  const response = await chat.submitOnboardingPairing("A1B2C3");
  assert.equal(response?.success, true, "the redemption itself succeeded");

  assert.deepEqual(
    sentContents,
    ["occupy the submit guard"],
    "the blocked continuation must not reach the network",
  );
  assert.equal(
    stateUpdates.some((update) => update.index === 5 && update.value === null),
    false,
    "an admission-blocked continuation must not clear the pairing panel",
  );
  assert.equal(
    storageValues.has(`ironclaw.chat.dismissedOnboarding.v1:${threadId}`),
    false,
    "an admission-blocked continuation must not persist a durable dismissal",
  );
});

test("useChat.submitOnboardingPairing: stale generic code stays local and does not resume chat", async () => {
  const threadId = "thread-stale-telegram-pairing";
  const stateUpdates = [];
  const pairingCalls = [];
  const sendBodies = [];

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({
      initialByIndex: new Map([
        [
          5,
          {
            state: "pairing_required",
            extensionName: "telegram",
            threadId,
            requestId: null,
          },
        ],
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
    queryClient: {
      getQueryData: () => ({
        threads: [{ thread_id: threadId, title: "Telegram pairing thread" }],
      }),
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    redeemPairingCode: async (channel, code, options) => {
      pairingCalls.push({ channel, code, options });
      throw new Error("Invalid or expired pairing code.");
    },
    removePending,
    resolveGateRequest: async () => {},
    sendMessage: async (body) => {
      sendBodies.push(body);
      throw new Error("stale pairing code must not resume chat");
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
    () => chat.submitOnboardingPairing(" STALE123 "),
    /Invalid or expired pairing code\./,
  );

  assert.equal(pairingCalls.length, 1);
  assert.equal(pairingCalls[0].channel, "telegram");
  assert.equal(pairingCalls[0].code, "STALE123");
  assert.deepEqual(JSON.parse(JSON.stringify(pairingCalls[0].options)), {
    threadId,
    requestId: null,
  });
  assert.equal(sendBodies.length, 0);
  assert.ok(
    !stateUpdates.some((update) => update.index === 5 && update.value === null),
    "failed redemption must keep the pairing panel open",
  );
  assert.doesNotMatch(JSON.stringify(sendBodies), /STALE123/);
});

test("useChat.submitOnboardingPairing: resumes the generic pairing panel's thread, not another open chat", async () => {
  const viewedThreadId = "thread-viewed";
  const pairingThreadId = "thread-needs-telegram";
  const sourceMessageId = "tool-telegram-activation-thread-needs-telegram";
  const stateUpdates = [];
  const sendBodies = [];
  const storageValues = new Map();

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({
      initialByIndex: new Map([
        [
          5,
          {
            state: "pairing_required",
            extensionName: "telegram",
            threadId: pairingThreadId,
            requestId: null,
            sourceMessageId,
          },
        ],
      ]),
      setCalls: stateUpdates,
    }),
    addPending,
    toRenderAttachment,
    toWireAttachment,
    approvePairingCode: async () => {},
    cancelRunRequest: async () => {},
    clearTimeout,
    createThreadRequest: async () => {
      throw new Error("thread should already exist");
    },
    globalThis: {
      localStorage: {
        getItem: (key) => (storageValues.has(key) ? storageValues.get(key) : null),
        setItem: (key, value) => storageValues.set(key, String(value)),
      },
    },
    queryClient: {
      getQueryData: () => ({
        threads: [
          { thread_id: viewedThreadId, title: "Viewed chat" },
          { thread_id: pairingThreadId, title: "Telegram-needed chat" },
        ],
      }),
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    redeemPairingCode: async (channel) => {
      assert.equal(channel, "telegram");
      return { success: true };
    },
    removePending,
    resolveGateRequest: async () => {},
    sendMessage: async (body) => {
      sendBodies.push(body);
      return {
        accepted_message_ref: "msg:message-continue",
        run_id: "run-continue",
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
      seedThreadMessages: () => {},
      setMessages: () => {},
    }),
    useSSE: () => ({ status: "idle" }),
  };

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat(viewedThreadId);
  await chat.submitOnboardingPairing("BTHREAD1");

  assert.equal(sendBodies.length, 1);
  assert.equal(sendBodies[0].threadId, pairingThreadId);
  assert.equal(
    sendBodies[0].content,
    "Telegram is connected. Continue the previous request.",
  );
  assert.equal(
    storageValues.has(`ironclaw.chat.dismissedOnboarding.v1:${viewedThreadId}`),
    false,
    "submitting one thread's panel must not dismiss the viewed thread",
  );
  assert.deepEqual(
    JSON.parse(
      storageValues.get(`ironclaw.chat.dismissedOnboarding.v1:${pairingThreadId}`),
    ),
    [sourceMessageId],
  );
});

test("useChat: channel-connected event from extensions clears a mounted waiting generic channel chat", async () => {
  const threadId = "thread-waiting-for-telegram";
  const sourceMessageId = "tool-telegram-activation";
  const stateUpdates = [];
  const sendBodies = [];
  const storageValues = new Map();
  let connectionHandler = null;

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    Set,
    React: createReactStub({
      initialByIndex: new Map([
        [
          5,
          {
            state: "pairing_required",
            extensionName: "telegram",
            threadId,
            requestId: null,
            sourceMessageId,
          },
        ],
      ]),
      runEffects: true,
      setCalls: stateUpdates,
    }),
    addPending,
    toRenderAttachment,
    toWireAttachment,
    approvePairingCode: async () => {},
    cancelRunRequest: async () => {},
    clearInterval,
    clearTimeout,
    createThreadRequest: async () => {
      throw new Error("thread should already exist");
    },
    globalThis: {
      localStorage: {
        getItem: (key) => (storageValues.has(key) ? storageValues.get(key) : null),
        setItem: (key, value) => storageValues.set(key, String(value)),
      },
    },
    queryClient: {
      getQueryData: () => ({
        threads: [{ thread_id: threadId, title: "Telegram waiting chat" }],
      }),
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    resolveGateRequest: async () => {},
    sendMessage: async (body) => {
      sendBodies.push(body);
      return {
        accepted_message_ref: "msg:message-continue",
        run_id: "run-continue",
        status: "queued",
        thread_id: body.threadId,
      };
    },
    setInterval,
    setTimeout,
    submitManualToken: async () => {},
    subscribeChannelConnected: (handler) => {
      connectionHandler = handler;
      return () => {};
    },
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

  assert.equal(typeof connectionHandler, "function");
  await connectionHandler({ channel: "telegram", source: "extensions" });

  assert.equal(
    sendBodies.length,
    0,
    "persisted waiting-thread registry owns continuation sends, not each mounted chat",
  );
  assert.ok(
    stateUpdates.some((update) => update.index === 5 && update.value === null),
    "waiting channel panel should clear before the continuation send",
  );
  // The broadcast must NOT durably dismiss the card or forget the waiter: the
  // continuation send happens best-effort in the NOTIFYING tab, and on its
  // failure the re-persisted waiter is the only path left to resume the
  // parked request. Re-derive suppression is owned by the per-user connection
  // snapshot (connected) and the durable timeline continuation, not by a
  // dismissal stamped on someone else's success signal.
  assert.equal(
    storageValues.get(`ironclaw.chat.dismissedOnboarding.v1:${threadId}`),
    undefined,
    "cross-tab connect must not persist a dismissal",
  );
});

test("useChat: channel-connected event from same chat does not duplicate the continuation", async () => {
  const threadId = "thread-submitted-code";
  const sendBodies = [];
  const stateUpdates = [];
  let connectionHandler = null;

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    Set,
    React: createReactStub({
      initialByIndex: new Map([
        [
          5,
          {
            state: "pairing_required",
            extensionName: "slack",
            threadId,
            sourceMessageId: "tool-slack-submit",
            requestId: null,
          },
        ],
      ]),
      runEffects: true,
      setCalls: stateUpdates,
    }),
    addPending,
    toRenderAttachment,
    toWireAttachment,
    approvePairingCode: async () => {},
    cancelRunRequest: async () => {},
    clearInterval,
    clearTimeout,
    createThreadRequest: async () => {
      throw new Error("thread should already exist");
    },
    globalThis: {},
    queryClient: {
      getQueryData: () => ({
        threads: [{ thread_id: threadId, title: "Submitting chat" }],
      }),
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    resolveGateRequest: async () => {},
    sendMessage: async (body) => {
      sendBodies.push(body);
      return {
        accepted_message_ref: "msg:message-continue",
        run_id: "run-continue",
        status: "queued",
        thread_id: body.threadId,
      };
    },
    setInterval,
    setTimeout,
    submitManualToken: async () => {},
    subscribeChannelConnected: (handler) => {
      connectionHandler = handler;
      return () => {};
    },
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

  assert.equal(typeof connectionHandler, "function");
  await connectionHandler({
    channel: "slack",
    sourceThreadId: threadId,
    source: "webui",
  });

  assert.equal(sendBodies.length, 0);
  assert.ok(
    stateUpdates.some((update) => update.index === 5 && update.value === null),
    "the originating chat still clears its blocked pairing panel",
  );
});

test("useChat: timeline Slack OAuth activation guidance does not open a connection panel from prose", async () => {
  const threadId = "thread-slack-oauth-activation";
  const stateUpdates = [];
  const renderedMessages = [
    {
      id: "tool-extension-activate",
      role: "tool_activity",
      capabilityId: "builtin.extension_activate",
      toolStatus: "success",
      toolResultPreview: JSON.stringify({
        message:
          "Slack is installed as an inbound channel. Configure Slack OAuth from the extension settings, then message the Slack bot directly. If the user's Slack account is already connected, continue the user's original request. Do not claim Slack message-reading tools are available unless a separate Slack read capability is installed.",
        package_ref: { id: "slack", kind: "extension" },
        payload: {
          activated: true,
          kind: "extension_activate",
          visible_capability_ids: [],
        },
        phase: "active",
      }),
    },
  ];

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    React: createReactStub({ runEffects: true, setCalls: stateUpdates }),
    addPending,
    toRenderAttachment,
    toWireAttachment,
    approvePairingCode: async () => {},
    cancelRunRequest: async () => {},
    clearInterval,
    clearTimeout,
    createThreadRequest: async () => {
      throw new Error("thread should already exist");
    },
    globalThis: {},
    queryClient: {
      getQueryData: () => ({
        threads: [{ thread_id: threadId, title: "Slack OAuth activation thread" }],
      }),
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    resolveGateRequest: async () => {},
    sendMessage: async () => {
      throw new Error("send should not run");
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
      setMessages: () => {},
    }),
    useSSE: () => ({ status: "idle" }),
  };

  runUseChatSource(context);
  context.globalThis.__testExports.useChat(threadId);

  assert.equal(
    stateUpdates.some((update) => update.value?.state === "pairing_required"),
    false,
  );
});

test("useChat: Slack-read package activation guidance does not render a connection panel from prose", async () => {
  const threadId = "thread-slack-read-no-tool";
  const stateUpdates = [];
  const renderedMessages = [
    {
      id: "msg-user",
      role: "user",
      content: "any new slack messages?",
    },
    {
      id: "tool-extension-activate",
      role: "tool_activity",
      capabilityId: "builtin.extension_activate",
      toolStatus: "success",
      toolResultPreview: JSON.stringify({
        message:
          "Slack is installed as an inbound channel. Configure Slack OAuth from the extension settings, then message the Slack bot directly. If the user's Slack account is already connected, continue the user's original request. Do not claim Slack message-reading tools are available unless a separate Slack read capability is installed.",
        package_ref: { id: "slack", kind: "extension" },
        payload: {
          activated: true,
          kind: "extension_activate",
          visible_capability_ids: [],
        },
        phase: "active",
      }),
    },
    {
      id: "msg-assistant",
      role: "assistant",
      content:
        "I can’t check Slack messages from here because no Slack message-reading capability is currently available/enabled.",
    },
  ];

  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    Set,
    React: createReactStub({ runEffects: true, setCalls: stateUpdates }),
    addPending,
    toRenderAttachment,
    toWireAttachment,
    approvePairingCode: async () => {},
    cancelRunRequest: async () => {},
    clearInterval,
    clearTimeout,
    createThreadRequest: async () => {
      throw new Error("thread should already exist");
    },
    globalThis: {},
    queryClient: {
      getQueryData: () => ({
        threads: [{ thread_id: threadId, title: "Slack read no tool" }],
      }),
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    resolveGateRequest: async () => {},
    sendMessage: async () => {
      throw new Error("send should not run");
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
      setMessages: () => {},
    }),
    useSSE: () => ({ status: "idle" }),
  };

  runUseChatSource(context);
  const chat = context.globalThis.__testExports.useChat(threadId);
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.equal(chat.pendingOnboarding, null);
  assert.equal(
    stateUpdates.some((update) => update.value?.state === "pairing_required"),
    false,
    "package-level Slack activation guidance must not create the connection CTA",
  );
});

test("useChat: blank unconnected Slack chat does NOT auto-open a connection panel (no startup poll)", async () => {
  // The in-chat connection panel is driven only by a structured per-thread signal
  // (a channel-connection-required capability preview), never by a global poll
  // over "is Slack connected". A brand-new empty chat that has nothing to do with
  // Slack must therefore never auto-open the panel and must not even fetch the
  // extensions / connectable-channels lists to decide — that global poll was the
  // source of the over-eager CTA, the warm-cache flicker, and the waiter spam.
  const threadId = "thread-blank-slack-connection";
  const stateUpdates = [];
  let extensionsFetched = false;
  let connectableFetched = false;
  const context = {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    Set,
    React: createReactStub({ runEffects: true, setCalls: stateUpdates }),
    addPending,
    toRenderAttachment,
    toWireAttachment,
    approvePairingCode: async () => {},
    cancelRunRequest: async () => {},
    clearInterval,
    clearTimeout,
    createThreadRequest: async () => {
      throw new Error("thread should already exist");
    },
    fetchExtensions: async () => {
      extensionsFetched = true;
      return {
        extensions: [
          {
            package_ref: { id: "slack", kind: "extension" },
            display_name: "Slack",
            kind: "channel",
            activation_status: "active",
            onboarding_state: "setup_required",
          },
        ],
      };
    },
    globalThis: {},
    listConnectableChannels: async () => {
      connectableFetched = true;
      return {
        channels: [
          {
            channel: "slack",
            display_name: "Slack",
            strategy: "oauth",
            action: {
              title: "Slack account connection",
              instructions: "Connect Slack with OAuth from the extension configuration, then message the Slack bot directly.",
              input_placeholder: "",
              submit_label: "Connect Slack",
            },
          },
        ],
      };
    },
    queryClient: {
      fetchQuery: async ({ queryFn }) => queryFn(),
      getQueryData: () => ({
        threads: [{ thread_id: threadId, title: "Blank Slack connection thread" }],
      }),
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    resolveGateRequest: async () => {},
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
  context.globalThis.__testExports.useChat(threadId);
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.equal(
    stateUpdates.some((update) => update.value?.state === "pairing_required"),
    false,
    "a blank chat must not auto-open the Slack connection panel",
  );
  assert.equal(
    extensionsFetched,
    false,
    "no startup poll: the chat must not fetch the extensions list to decide on a panel",
  );
  assert.equal(
    connectableFetched,
    false,
    "no startup poll: the chat must not fetch connectable channels to decide on a panel",
  );
});

// A durable tool-result card carrying the backend's structured
// channel-connection-required signal (`output_kind`/`toolResultPreview`). The
// same card shape arrives live (capability_display_preview) and on reload
// (timeline), so the in-chat panel derives from it in both cases.
function channelConnectionRequiredCard(overrides = {}) {
  return {
    id: "tool-activate-1",
    role: "tool_activity",
    capabilityId: "builtin.extension_activate",
    outputKind: "channel_connection_required",
    toolStatus: "success",
    toolResultPreview: JSON.stringify({
      channel: "slack",
      strategy: "oauth",
      instructions:
        "Connect Slack with OAuth from the extension configuration, then message the Slack bot directly.",
      input_placeholder: "",
      submit_label: "Connect Slack",
      error_message: "Slack OAuth connection failed. Try configuring Slack again.",
    }),
    ...overrides,
  };
}

function channelConnectionContext({
  threadId,
  messages,
  slackExtension,
  stateUpdates,
  storage,
  messagesThreadId,
  fetchExtensions,
  fetchExtensionSetup,
  fetchOauthFlowStatus,
  initialByIndex = new Map(),
  sendMessage,
  startExtensionOauth,
  windowObject,
}) {
  return {
    AbortController,
    Date,
    Error,
    Map,
    Math,
    Set,
    React: createReactStub({ runEffects: true, setCalls: stateUpdates, initialByIndex }),
    URL,
    addPending,
    toRenderAttachment,
    toWireAttachment,
    approvePairingCode: async () => {},
    cancelRunRequest: async () => {},
    clearInterval,
    clearTimeout,
    createThreadRequest: async () => {
      throw new Error("thread should already exist");
    },
    fetchExtensions:
      fetchExtensions ||
      (async () => ({ extensions: slackExtension ? [slackExtension] : [] })),
    fetchExtensionSetup: fetchExtensionSetup || (async () => ({ secrets: [] })),
    fetchOauthFlowStatus: fetchOauthFlowStatus || (async () => null),
    globalThis: storage ? { localStorage: storage } : {},
    queryClient: {
      fetchQuery: async ({ queryFn }) => queryFn(),
      getQueryData: () => ({ threads: [{ thread_id: threadId, title: "Slack chat" }] }),
      invalidateQueries: () => {},
    },
    recordAcceptedMessageRef,
    removePending,
    resolveGateRequest: async () => {},
    sendMessage: sendMessage || (async () => ({ run_id: "run-continue" })),
    setInterval,
    setTimeout,
    startExtensionOauth:
      startExtensionOauth || (async () => ({ success: false, message: "not configured" })),
    submitManualToken: async () => {},
    useChatEvents: () => () => {},
    useHistory: () => ({
      messages,
      // The thread the loaded `messages` belong to. Real useHistory swaps this in
      // step with `messages` (see useHistory.ts); tests default it to the active
      // thread, and override it to model the post-navigation render where
      // `threadId` has advanced but `messages` is still the previous thread's.
      messagesThreadId: messagesThreadId ?? threadId,
      hasMore: false,
      nextCursor: null,
      isLoading: false,
      loadHistory: () => {},
      seedThreadMessages: () => {},
      setMessages: () => {},
    }),
    useSSE: () => ({ status: "idle" }),
    window: windowObject,
  };
}

test("useChat: a channel-connection-required tool card opens the Slack OAuth panel when Slack is unconnected", async () => {
  const threadId = "thread-activate-slack";
  const stateUpdates = [];
  const context = channelConnectionContext({
    threadId,
    messages: [channelConnectionRequiredCard()],
    slackExtension: {
      package_ref: { id: "slack", kind: "extension" },
      kind: "channel",
      authenticated: false,
      needs_setup: true,
      onboarding_state: "setup_required",
    },
    stateUpdates,
  });

  runUseChatSource(context);
  context.globalThis.__testExports.useChat(threadId);
  await new Promise((resolve) => setTimeout(resolve, 0));

  const onboardingUpdate = stateUpdates.find(
    (update) => update.value?.state === "pairing_required",
  );
  assert.equal(onboardingUpdate?.value?.extensionName, "slack");
  assert.equal(onboardingUpdate?.value?.threadId, threadId);
  assert.equal(onboardingUpdate?.value?.sourceMessageId, "tool-activate-1");
  assert.equal(onboardingUpdate?.value?.strategy, "oauth");
  assert.match(onboardingUpdate?.value?.instructions, /Connect Slack with OAuth/);
  assert.equal(onboardingUpdate?.value?.inputPlaceholder, "");
});

test("useChat: a channel-connection-required tool card opens the pairing panel when connection lookup fails", async () => {
  const threadId = "thread-activate-slack-lookup-fails";
  const stateUpdates = [];
  const context = channelConnectionContext({
    threadId,
    messages: [channelConnectionRequiredCard()],
    stateUpdates,
    fetchExtensions: async () => {
      throw new Error("extensions unavailable");
    },
  });

  runUseChatSource(context);
  context.globalThis.__testExports.useChat(threadId);
  await new Promise((resolve) => setTimeout(resolve, 0));

  const onboardingUpdate = stateUpdates.find(
    (update) => update.value?.state === "pairing_required",
  );
  assert.equal(onboardingUpdate?.value?.extensionName, "slack");
  assert.equal(onboardingUpdate?.value?.threadId, threadId);
  assert.equal(onboardingUpdate?.value?.sourceMessageId, "tool-activate-1");
});

test("useChat: a channel-connection-required tool card does NOT open the panel when Slack is already connected", async () => {
  // The card lives forever in the durable timeline; after the user connects, a
  // reload must not re-show "Connect Slack". The panel is gated on the live,
  // per-user connection state, not just the presence of the card.
  const threadId = "thread-activate-connected";
  const stateUpdates = [];
  const context = channelConnectionContext({
    threadId,
    messages: [channelConnectionRequiredCard()],
    slackExtension: {
      package_ref: { id: "slack", kind: "extension" },
      kind: "channel",
      authenticated: true,
      needs_setup: false,
      onboarding_state: "active",
    },
    stateUpdates,
  });

  runUseChatSource(context);
  context.globalThis.__testExports.useChat(threadId);
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.equal(
    stateUpdates.some((update) => update.value?.state === "pairing_required"),
    false,
    "an already-connected Slack account must not re-open the connection panel",
  );
});

test("useChat: a card already resumed by the connected continuation stays closed, even after the extension is removed", async () => {
  // Regression: pairing redeemed in another chat/tab resumes this thread via
  // the durable continuation message, but records no browser-local dismissal
  // for this thread's card. Once the Slack extension is later removed, the
  // live-connection gate can no longer suppress the durable card, and the
  // panel re-opened on exactly the threads whose pairing had been redeemed
  // elsewhere — inconsistently, since sibling threads redeemed in-panel stayed
  // closed. The continuation in the durable timeline is the cross-browser
  // proof this thread's flow completed; a fresh requirement on such a thread
  // arrives as a new backend card on the next send.
  const threadId = "thread-activate-resumed-then-removed";
  const stateUpdates = [];
  const context = channelConnectionContext({
    threadId,
    messages: [
      channelConnectionRequiredCard(),
      {
        id: "user-continuation-1",
        role: "user",
        content: channelConnectionContinuationMessage("slack"),
      },
      { id: "assistant-after-1", role: "assistant", content: "Continuing." },
    ],
    // No slackExtension: the extension was removed, so the connection
    // snapshot cannot satisfy the gate.
    stateUpdates,
  });

  runUseChatSource(context);
  context.globalThis.__testExports.useChat(threadId);
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.equal(
    stateUpdates.some((update) => update.value?.state === "pairing_required"),
    false,
    "a thread already resumed by the connected continuation must not re-open the panel",
  );
});

test("useChat: a dismissed channel-connection-required tool card stays closed", async () => {
  const threadId = "thread-activate-dismissed";
  const stateUpdates = [];
  const dismissedKey = `ironclaw.chat.dismissedOnboarding.v1:${threadId}`;
  const context = channelConnectionContext({
    threadId,
    messages: [channelConnectionRequiredCard()],
    slackExtension: {
      package_ref: { id: "slack", kind: "extension" },
      kind: "channel",
      authenticated: false,
      needs_setup: true,
      onboarding_state: "setup_required",
    },
    stateUpdates,
    storage: {
      getItem: (key) => (key === dismissedKey ? JSON.stringify(["tool-activate-1"]) : null),
      setItem: () => {},
    },
  });

  runUseChatSource(context);
  context.globalThis.__testExports.useChat(threadId);
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.equal(
    stateUpdates.some((update) => update.value?.state === "pairing_required"),
    false,
    "a panel the user already dismissed must not re-derive from the durable card",
  );
});

test("useChat: a connection-required card from another thread's still-loaded timeline must not open the panel here", async () => {
  // Regression for the cross-thread "panel crosses over" bug. Repro: refresh on a
  // chat that is requesting Slack, then navigate to a chat that does not need it.
  // On the navigation render `threadId` advances to the new chat one render before
  // useHistory swaps `messages` to the new thread's timeline, so the derive effect
  // briefly sees the PREVIOUS thread's durable connection card. It must not stamp
  // that card onto — and open the pairing panel for — the newly-viewed chat.
  const threadId = "thread-no-slack";
  const stateUpdates = [];
  const context = channelConnectionContext({
    threadId,
    // A Slack card + an unconnected Slack account, so the ONLY thing that can keep
    // the panel closed is the cross-thread guard — not the already-connected gate.
    messages: [channelConnectionRequiredCard()],
    messagesThreadId: "thread-needs-slack",
    slackExtension: {
      package_ref: { id: "slack", kind: "extension" },
      kind: "channel",
      authenticated: false,
      needs_setup: true,
      onboarding_state: "setup_required",
    },
    stateUpdates,
  });

  runUseChatSource(context);
  context.globalThis.__testExports.useChat(threadId);
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.equal(
    stateUpdates.some((update) => update.value?.state === "pairing_required"),
    false,
    "a connection card belonging to another thread must not open the pairing panel on this chat",
  );
});

test("useChat: a channel-connected event from elsewhere clears the panel and refreshes the connection cache", async () => {
  // The auto-resume contract for requirement (d): connecting Slack anywhere — the
  // extensions page, another tab, or another chat — must close this chat's
  // pairing panel and refresh the per-user connection snapshot, not leave a stale
  // "Connect Slack" behind.
  const threadId = "thread-cross-source-resume";
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
        initialByIndex: new Map([
          [5, { state: "connection_required", extensionName: "slack", threadId, sourceMessageId: "tool-x" }],
        ]),
      }),
      addPending,
      toRenderAttachment,
      toWireAttachment,
      approvePairingCode: async () => {},
      cancelRunRequest: async () => {},
      clearInterval,
      clearTimeout,
      createThreadRequest: async () => {
        throw new Error("thread should already exist");
      },
      fetchExtensions: async () => ({ extensions: [] }),
      globalThis: {},
      queryClient: {
        fetchQuery: async ({ queryFn }) => queryFn(),
        getQueryData: () => ({ threads: [{ thread_id: threadId, title: "Slack chat" }] }),
        invalidateQueries: (arg) => invalidated.push(arg?.queryKey?.[0]),
      },
      recordAcceptedMessageRef,
      removePending,
      resolveGateRequest: async () => {},
      sendMessage: async () => ({ run_id: "run-continue" }),
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
      "the per-user extensions snapshot is refreshed on connect",
    );
    assert.ok(
      stateUpdates.some((update) => update.index === 5 && update.value === null),
      "the pairing panel clears when the channel connects elsewhere",
    );
  } finally {
    globalThis.window = originalWindow;
  }
});

test("useChat: Slack OAuth completion consumes the in-chat connection card", async () => {
  const threadId = "thread-chat-oauth-complete";
  const sourceMessageId = "tool-slack-oauth-complete";
  const stateUpdates = [];
  const sendBodies = [];
  const store = new Map();
  const intervalCallbacks = [];
  const storage = {
    getItem: (key) => store.get(key) || null,
    setItem: (key, value) => store.set(key, value),
  };
  const popup = { closed: false, location: { href: "about:blank" }, opener: "test" };
  const windowObject = {
    open: () => popup,
    localStorage: storage,
    addEventListener: () => {},
    removeEventListener: () => {},
    setInterval: (callback) => {
      intervalCallbacks.push(callback);
      return intervalCallbacks.length;
    },
    clearInterval: () => {},
  };
  const context = channelConnectionContext({
    threadId,
    messages: [channelConnectionRequiredCard({ id: sourceMessageId })],
    stateUpdates,
    storage,
    initialByIndex: new Map([
      [
        STATE_SLOT.pendingOnboarding,
        {
          extensionName: "slack",
          state: "pairing_required",
          threadId,
          sourceMessageId,
          strategy: "oauth",
        },
      ],
    ]),
    fetchExtensionSetup: async () => ({
      secrets: [
        {
          provider: "slack_personal",
          setup: { kind: "oauth", invocation_id: "invocation-slack" },
        },
      ],
    }),
    startExtensionOauth: async () => ({
      flow_id: "flow-slack-chat",
      authorization_url: "https://slack.com/oauth/v2/authorize?client_id=client",
    }),
    sendMessage: async (body) => {
      sendBodies.push(body);
      return { run_id: "run-continue", status: "queued", thread_id: body.threadId };
    },
    windowObject,
  });

  runUseChatSource(context);
  const chat = context.globalThis.__testExports.useChat(threadId);
  await chat.startOnboardingOAuth();
  store.set(
    "ironclaw:product-auth:oauth-complete",
    JSON.stringify({
      type: "ironclaw:product-auth:oauth-complete",
      flowId: "flow-slack-chat",
      status: "completed",
    }),
  );
  for (const callback of intervalCallbacks) callback();
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.ok(
    stateUpdates.some(
      (update) => update.index === STATE_SLOT.pendingOnboarding && update.value === null,
    ),
    "OAuth completion must clear the mounted Slack connection card",
  );
  const dismissed = JSON.parse(
    store.get(`ironclaw.chat.dismissedOnboarding.v1:${threadId}`) || "[]",
  );
  assert.deepEqual(dismissed, [sourceMessageId]);
  assert.equal(sendBodies.length, 1);
  assert.equal(sendBodies[0].threadId, threadId);
  assert.equal(sendBodies[0].content, channelConnectionContinuationMessage("slack"));
});

test("useChat: a late flow A status cannot complete or fail newer flow B", async () => {
  const threadId = "thread-chat-oauth-generation-fence";
  const sourceMessageId = "tool-slack-oauth-generation-fence";
  const stateUpdates = [];
  const sendBodies = [];
  const intervalCallbacks = [];
  const storage = {
    getItem: () => null,
    setItem: () => {},
  };
  let startCount = 0;
  let resolveFlowAStatus;
  const flowAStatus = new Promise((resolve) => {
    resolveFlowAStatus = resolve;
  });
  const popups = [
    { closed: false, close() { this.closed = true; }, location: { href: "about:blank" } },
    { closed: false, close() { this.closed = true; }, location: { href: "about:blank" } },
  ];
  const windowObject = {
    open: () => popups[startCount],
    localStorage: storage,
    addEventListener: () => {},
    removeEventListener: () => {},
    setInterval: (callback) => {
      intervalCallbacks.push(callback);
      return intervalCallbacks.length;
    },
    clearInterval: () => {},
  };
  const context = channelConnectionContext({
    threadId,
    messages: [channelConnectionRequiredCard({ id: sourceMessageId })],
    stateUpdates,
    storage,
    initialByIndex: new Map([
      [
        STATE_SLOT.pendingOnboarding,
        {
          extensionName: "slack",
          state: "pairing_required",
          threadId,
          sourceMessageId,
          strategy: "oauth",
        },
      ],
    ]),
    fetchExtensionSetup: async () => ({
      secrets: [
        {
          provider: "slack_personal",
          setup: { kind: "oauth", invocation_id: "invocation-slack" },
        },
      ],
    }),
    startExtensionOauth: async () => {
      startCount += 1;
      return {
        flow_id: startCount === 1 ? "flow-a" : "flow-b",
        callback_scope: { invocation_id: `invocation-${startCount}` },
        authorization_url: "https://slack.com/oauth/v2/authorize?client_id=client",
      };
    },
    fetchOauthFlowStatus: async (flowId) =>
      flowId === "flow-a" ? flowAStatus : { status: "completed" },
    sendMessage: async (body) => {
      sendBodies.push(body);
      return { run_id: "run-continue", status: "queued", thread_id: body.threadId };
    },
    windowObject,
  });

  runUseChatSource(context);
  const chat = context.globalThis.__testExports.useChat(threadId);
  await chat.startOnboardingOAuth();
  for (const callback of intervalCallbacks) callback();
  await chat.startOnboardingOAuth();

  resolveFlowAStatus({ status: "failed" });
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.ok(
    !stateUpdates.some(
      (update) =>
        update.index === STATE_SLOT.pendingOnboarding &&
        typeof update.value?.oauthError === "string",
    ),
    "flow A must not stamp an error onto flow B",
  );
  assert.equal(sendBodies.length, 0, "flow A must not resume the chat");

  for (const callback of intervalCallbacks) callback();
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(sendBodies.length, 1, "flow B can still complete normally");
});

test("useChat: a failed Slack OAuth signal surfaces a retryable error on the connection card", async () => {
  const threadId = "thread-chat-oauth-failed";
  const sourceMessageId = "tool-slack-oauth-failed";
  const stateUpdates = [];
  const sendBodies = [];
  const flowStatusCalls = [];
  let flowStatus = null;
  const store = new Map();
  const intervalCallbacks = [];
  const storage = {
    getItem: (key) => store.get(key) || null,
    setItem: (key, value) => store.set(key, value),
  };
  const popup = { closed: false, location: { href: "about:blank" }, opener: "test" };
  const windowObject = {
    open: () => popup,
    localStorage: storage,
    addEventListener: () => {},
    removeEventListener: () => {},
    setInterval: (callback) => {
      intervalCallbacks.push(callback);
      return intervalCallbacks.length;
    },
    clearInterval: () => {},
  };
  const context = channelConnectionContext({
    threadId,
    messages: [channelConnectionRequiredCard({ id: sourceMessageId })],
    stateUpdates,
    storage,
    initialByIndex: new Map([
      [
        STATE_SLOT.pendingOnboarding,
        {
          extensionName: "slack",
          state: "pairing_required",
          threadId,
          sourceMessageId,
          strategy: "oauth",
        },
      ],
    ]),
    fetchExtensionSetup: async () => ({
      secrets: [
        {
          provider: "slack_personal",
          setup: { kind: "oauth", invocation_id: "invocation-slack" },
        },
      ],
    }),
    startExtensionOauth: async () => ({
      flow_id: "flow-slack-failed",
      callback_scope: { invocation_id: "invocation-slack-failed" },
      authorization_url: "https://slack.com/oauth/v2/authorize?client_id=client",
    }),
    fetchOauthFlowStatus: async (flowId, invocationId) => {
      flowStatusCalls.push({ flowId, invocationId });
      return flowStatus;
    },
    sendMessage: async (body) => {
      sendBodies.push(body);
      return { run_id: "run-continue", status: "queued", thread_id: body.threadId };
    },
    windowObject,
  });

  runUseChatSource(context);
  const chat = context.globalThis.__testExports.useChat(threadId);
  await chat.startOnboardingOAuth();
  // The callback popup broadcast a FAILURE for this exact flow (provider
  // denial / exchange failure) and closed itself.
  store.set(
    "ironclaw:product-auth:oauth-complete",
    JSON.stringify({
      type: "ironclaw:product-auth:oauth-complete",
      flowId: "flow-slack-failed",
      status: "failed",
    }),
  );
  for (const callback of intervalCallbacks) callback();
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.ok(
    !stateUpdates.some(
      (update) =>
        update.index === STATE_SLOT.pendingOnboarding &&
        typeof update.value?.oauthError === "string",
    ),
    "a callback failure must wait for durable compensation to settle",
  );
  flowStatus = { status: "failed" };
  for (const callback of intervalCallbacks) callback();
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.ok(
    stateUpdates.some(
      (update) =>
        update.index === STATE_SLOT.pendingOnboarding &&
        typeof update.value?.oauthError === "string" &&
        /authorization failed/i.test(update.value.oauthError),
    ),
    "a flow-matched failure must surface a retryable error on the card",
  );
  assert.ok(
    !stateUpdates.some(
      (update) => update.index === STATE_SLOT.pendingOnboarding && update.value === null,
    ),
    "the card must stay mounted so the user can retry",
  );
  assert.equal(sendBodies.length, 0, "a failed flow must not send the continuation");
  assert.deepEqual(flowStatusCalls.at(-1), {
    flowId: "flow-slack-failed",
    invocationId: "invocation-slack-failed",
  });

  // Retry after the failure: a fresh start must clear the stale card error and
  // track a new flow instead of leaving the dead one's message up.
  await chat.startOnboardingOAuth();
  assert.ok(
    stateUpdates.some(
      (update) =>
        update.index === STATE_SLOT.pendingOnboarding &&
        update.value &&
        update.value.oauthError === null,
    ),
    "a retry must clear the stale card error",
  );
});

test("useChat: an abandoned Slack OAuth flow times out instead of polling forever", async () => {
  const threadId = "thread-chat-oauth-timeout";
  const sourceMessageId = "tool-slack-oauth-timeout";
  const stateUpdates = [];
  const store = new Map();
  const intervalCallbacks = [];
  let fetchExtensionsCalls = 0;
  let nowValue = 1_000_000;
  class FakeDate extends Date {
    static now() {
      return nowValue;
    }
  }
  const storage = {
    getItem: (key) => store.get(key) || null,
    setItem: (key, value) => store.set(key, value),
  };
  const popup = { closed: true, location: { href: "about:blank" }, opener: "test" };
  const windowObject = {
    open: () => popup,
    localStorage: storage,
    addEventListener: () => {},
    removeEventListener: () => {},
    setInterval: (callback) => {
      intervalCallbacks.push(callback);
      return intervalCallbacks.length;
    },
    clearInterval: () => {},
  };
  const context = channelConnectionContext({
    threadId,
    messages: [channelConnectionRequiredCard({ id: sourceMessageId })],
    stateUpdates,
    storage,
    initialByIndex: new Map([
      [
        STATE_SLOT.pendingOnboarding,
        {
          extensionName: "slack",
          state: "pairing_required",
          threadId,
          sourceMessageId,
          strategy: "oauth",
        },
      ],
    ]),
    fetchExtensions: async () => {
      fetchExtensionsCalls += 1;
      return { extensions: [] };
    },
    fetchExtensionSetup: async () => ({
      secrets: [
        {
          provider: "slack_personal",
          setup: { kind: "oauth", invocation_id: "invocation-slack" },
        },
      ],
    }),
    startExtensionOauth: async () => ({
      flow_id: "flow-slack-timeout",
      authorization_url: "https://slack.com/oauth/v2/authorize?client_id=client",
    }),
    windowObject,
  });
  context.Date = FakeDate;

  runUseChatSource(context);
  const chat = context.globalThis.__testExports.useChat(threadId);
  await chat.startOnboardingOAuth();

  // Within the timeout budget the watcher polls the per-user extension state.
  for (const callback of intervalCallbacks) callback();
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.ok(fetchExtensionsCalls >= 1, "the watcher polls server state before the timeout");

  // The user closed/abandoned the popup and never authorized: past the budget
  // the watcher must stop polling and surface a retryable timeout error.
  nowValue += 11 * 60 * 1000;
  for (const callback of intervalCallbacks) callback();
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.ok(
    stateUpdates.some(
      (update) =>
        update.index === STATE_SLOT.pendingOnboarding &&
        typeof update.value?.oauthError === "string" &&
        /timed out/i.test(update.value.oauthError),
    ),
    "an expired flow must surface a retryable timeout error",
  );
  const callsAtTimeout = fetchExtensionsCalls;
  for (const callback of intervalCallbacks) callback();
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(
    fetchExtensionsCalls,
    callsAtTimeout,
    "polling must stop once the flow timed out",
  );
});

test("useChat: dismissing the connection card stops the pending OAuth flow's polling", async () => {
  const threadId = "thread-chat-oauth-dismiss";
  const sourceMessageId = "tool-slack-oauth-dismiss";
  const stateUpdates = [];
  const store = new Map();
  const intervalCallbacks = [];
  let fetchExtensionsCalls = 0;
  const storage = {
    getItem: (key) => store.get(key) || null,
    setItem: (key, value) => store.set(key, value),
  };
  const popup = { closed: false, location: { href: "about:blank" }, opener: "test" };
  const windowObject = {
    open: () => popup,
    localStorage: storage,
    addEventListener: () => {},
    removeEventListener: () => {},
    setInterval: (callback) => {
      intervalCallbacks.push(callback);
      return intervalCallbacks.length;
    },
    clearInterval: () => {},
  };
  const context = channelConnectionContext({
    threadId,
    messages: [channelConnectionRequiredCard({ id: sourceMessageId })],
    stateUpdates,
    storage,
    initialByIndex: new Map([
      [
        STATE_SLOT.pendingOnboarding,
        {
          extensionName: "slack",
          state: "pairing_required",
          threadId,
          sourceMessageId,
          strategy: "oauth",
        },
      ],
    ]),
    fetchExtensions: async () => {
      fetchExtensionsCalls += 1;
      return { extensions: [] };
    },
    fetchExtensionSetup: async () => ({
      secrets: [
        {
          provider: "slack_personal",
          setup: { kind: "oauth", invocation_id: "invocation-slack" },
        },
      ],
    }),
    startExtensionOauth: async () => ({
      flow_id: "flow-slack-dismiss",
      authorization_url: "https://slack.com/oauth/v2/authorize?client_id=client",
    }),
    windowObject,
  });

  runUseChatSource(context);
  const chat = context.globalThis.__testExports.useChat(threadId);
  await chat.startOnboardingOAuth();
  chat.dismissOnboardingPairing();

  for (const callback of intervalCallbacks) callback();
  for (const callback of intervalCallbacks) callback();
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.equal(
    fetchExtensionsCalls,
    0,
    "dismissing the card must clear the pending flow so nothing polls the server",
  );
});

test("useChat: Slack OAuth completion polls per-user extension state when callback storage is unavailable", async () => {
  const threadId = "thread-chat-oauth-polling-complete";
  const sourceMessageId = "tool-slack-oauth-polling-complete";
  const stateUpdates = [];
  const sendBodies = [];
  const intervalCallbacks = [];
  let extensionState = {
    package_ref: { id: "slack", kind: "extension" },
    kind: "channel",
    authenticated: false,
    needs_setup: true,
    onboarding_state: "setup_required",
  };
  const popup = { closed: true, location: { href: "about:blank" }, opener: "test" };
  const windowObject = {
    open: () => popup,
    localStorage: { getItem: () => null, setItem: () => {} },
    addEventListener: () => {},
    removeEventListener: () => {},
    setInterval: (callback) => {
      intervalCallbacks.push(callback);
      return intervalCallbacks.length;
    },
    clearInterval: () => {},
  };
  const context = channelConnectionContext({
    threadId,
    messages: [channelConnectionRequiredCard({ id: sourceMessageId })],
    stateUpdates,
    initialByIndex: new Map([
      [
        STATE_SLOT.pendingOnboarding,
        {
          extensionName: "slack",
          state: "pairing_required",
          threadId,
          sourceMessageId,
          strategy: "oauth",
        },
      ],
    ]),
    fetchExtensions: async () => ({ extensions: [extensionState] }),
    fetchExtensionSetup: async () => ({
      secrets: [
        {
          provider: "slack_personal",
          setup: { kind: "oauth", invocation_id: "invocation-slack" },
        },
      ],
    }),
    startExtensionOauth: async () => ({
      flow_id: "flow-slack-chat-polling",
      authorization_url: "https://slack.com/oauth/v2/authorize?client_id=client",
    }),
    sendMessage: async (body) => {
      sendBodies.push(body);
      return { run_id: "run-continue", status: "queued", thread_id: body.threadId };
    },
    windowObject,
  });

  runUseChatSource(context);
  const chat = context.globalThis.__testExports.useChat(threadId);
  await chat.startOnboardingOAuth();

  for (const callback of intervalCallbacks) callback();
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(sendBodies.length, 0);

  extensionState = {
    package_ref: { id: "slack", kind: "extension" },
    kind: "channel",
    authenticated: true,
    needs_setup: false,
    onboarding_state: "active",
  };
  for (const callback of intervalCallbacks) callback();
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.ok(
    stateUpdates.some(
      (update) => update.index === STATE_SLOT.pendingOnboarding && update.value === null,
    ),
    "server-state OAuth completion must clear the mounted Slack connection card",
  );
  assert.equal(sendBodies.length, 1);
  assert.equal(sendBodies[0].threadId, threadId);
  assert.equal(sendBodies[0].content, channelConnectionContinuationMessage("slack"));
});

test("useChat.cancelRun: clears the pairing panel, forgets the waiter, and persists the dismissal", async () => {
  // Cancelling a run with an open pairing panel must (1) close the panel, (2)
  // forget the channel-connection waiter so a later connect can't blast "Continue
  // the previous request" into a chat the user explicitly cancelled, and (3)
  // persist the dismissal so the durable activation card can't re-derive the panel.
  const threadId = "thread-cancel-pairing";
  const sourceMessageId = "tool-slack-cancel";
  const stateUpdates = [];
  const cancelCalls = [];
  const store = new Map();
  const localStorage = {
    getItem: (key) => (store.has(key) ? store.get(key) : null),
    setItem: (key, value) => store.set(key, String(value)),
    removeItem: (key) => store.delete(key),
  };

  const originalWindow = globalThis.window;
  // The waiter registry reads window.localStorage; the dismissal store reads the
  // vm context's globalThis.localStorage. Back both with the same map.
  globalThis.window = {
    localStorage,
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
        initialByIndex: new Map([
          [2, { runId: "run-cancel", threadId, status: "running" }],
          [
            5,
            { state: "connection_required", extensionName: "slack", threadId, sourceMessageId },
          ],
        ]),
      }),
      addPending,
      toRenderAttachment,
      toWireAttachment,
      cancelRunRequest: async (body) => {
        cancelCalls.push(body);
        return {};
      },
      clearInterval,
      clearTimeout,
      createThreadRequest: async () => {
        throw new Error("thread should already exist");
      },
      fetchExtensions: async () => ({ extensions: [] }),
      globalThis: { localStorage },
      queryClient: {
        fetchQuery: async ({ queryFn }) => queryFn(),
        getQueryData: () => ({ threads: [{ thread_id: threadId, title: "Slack chat" }] }),
        invalidateQueries: () => {},
      },
      recordAcceptedMessageRef,
      removePending,
      resolveGateRequest: async () => {},
      sendMessage: async () => ({ run_id: "run-continue" }),
      setInterval,
      setTimeout,
      submitManualToken: async () => {},
      useChatEvents: () => () => {},
      useHistory: () => ({
        messages: [],
        messagesThreadId: threadId,
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

    // Showing the panel registered a waiter for this thread.
    const beforeCancel = JSON.parse(
      store.get("ironclaw:channel-connection:waiting:v1") || "[]",
    );
    assert.ok(
      beforeCancel.some((w) => w.channel === "slack" && w.threadId === threadId),
      "an open pairing panel registers a connection waiter",
    );

    await chat.cancelRun("user_requested");

    assert.equal(cancelCalls.length, 1);
    assert.equal(cancelCalls[0].runId, "run-cancel");
    assert.ok(
      stateUpdates.some((update) => update.index === 5 && update.value === null),
      "cancel closes the pairing panel",
    );
    assert.deepEqual(
      JSON.parse(store.get(`ironclaw.chat.dismissedOnboarding.v1:${threadId}`)),
      [sourceMessageId],
      "cancel persists the dismissal so the durable card can't re-open the panel",
    );
    const afterCancel = JSON.parse(
      store.get("ironclaw:channel-connection:waiting:v1") || "[]",
    );
    assert.ok(
      !afterCancel.some((w) => w.channel === "slack" && w.threadId === threadId),
      "cancel forgets the waiter so a later connect won't resume the cancelled chat",
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
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
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
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
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
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
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
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
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
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
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
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
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
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
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

test("useChat.send: stream error clears same-thread local admission", async () => {
  const threadId = "thread-stream-error";
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
    timelineMessageIdFromAcceptedRef,
    resolveGateRequest: async () => {},
    sendMessage: async ({ content, threadId }) => {
      sendCalls += 1;
      return {
        accepted_message_ref: `msg:stream-error-${sendCalls}`,
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
      seedThreadMessages: () => {},
      setMessages: (updater) => {
        renderedMessages =
          typeof updater === "function" ? updater(renderedMessages) : updater;
      },
    }),
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
  };

  runUseChatSource(context);

  const chat = context.globalThis.__testExports.useChat(threadId);
  const first = await chat.send("first request");
  assert.equal(first.run_id, "run-1");

  context.chatEventsArgs.setPendingGate(null);
  context.chatEventsArgs.setIsProcessing(false);
  context.chatEventsArgs.setActiveRun(null);
  context.chatEventsArgs.onStreamError({
    error: "unavailable",
    kind: "service_unavailable",
    retryable: true,
  });

  const second = await chat.send("retry after stream error");

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
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
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
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
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
    useSSE: () => ({ status: CONNECTION_STATUS.IDLE }),
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
