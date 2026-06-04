import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

import { messagesFromTimeline } from "./history-messages.js";
import {
  looksLikeChannelConnectCommand,
  resolveChannelConnectCommand,
} from "../../../lib/channel-connect.js";
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

function createReactStub() {
  return {
    useCallback: (fn) => fn,
    useEffect: () => {},
    useRef: (value) => ({ current: value }),
    useState: (initial) => {
      let value = typeof initial === "function" ? initial() : initial;
      return [
        value,
        (next) => {
          value = typeof next === "function" ? next(value) : next;
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
      setMessages: () => {},
    }),
    useSSE: () => ({ status: "idle" }),
  };

  vm.runInNewContext(useChatSourceForTest(), context);

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
      setMessages: () => {},
    }),
    useSSE: () => ({ status: "idle" }),
  };

  vm.runInNewContext(useChatSourceForTest(), context);

  const chat = context.globalThis.__testExports.useChat(null);
  const response = await chat.send("connect telegram account");

  assert.equal(createThreadCalled, true);
  assert.equal(sentContent, "connect telegram account");
  assert.equal(response.channel_connect_action, undefined);
  assert.equal(response.thread_id, "thread-created");
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
      setMessages: () => {},
    }),
    useSSE: () => ({ status: "idle" }),
  };

  vm.runInNewContext(useChatSourceForTest(), context);

  const chat = context.globalThis.__testExports.useChat(null);
  const response = await chat.send("connect my Slack account");

  assert.equal(createThreadCalled, true);
  assert.equal(sentContent, "connect my Slack account");
  assert.equal(response.channel_connect_action, undefined);
  assert.equal(response.thread_id, "thread-created");
  assert.equal(loggedErrors[0][0], "Failed to resolve connectable channels:");
});
