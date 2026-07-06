import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

import { channelConnectionDisplayName } from "../../../lib/channel-connection-events.js";

function chatSourceForTest() {
  const source = readFileSync(new URL("../chat.js", import.meta.url), "utf8");
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
    lines.push(line.replace("export function Chat", "function Chat"));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { Chat };`;
}

function findComponent(node, component) {
  if (!node || typeof node !== "object") return null;
  if (!Array.isArray(node.values)) return null;
  const componentIndex = node.values.indexOf(component);
  if (componentIndex >= 0) {
    return node;
  }
  for (const value of node.values) {
    const found = findComponent(value, component);
    if (found) return found;
  }
  return null;
}

function findNode(node, predicate) {
  if (!node || typeof node !== "object") return null;
  if (Array.isArray(node.strings) && predicate(node)) return node;
  if (!Array.isArray(node.values)) return null;
  for (const value of node.values) {
    const found = findNode(value, predicate);
    if (found) return found;
  }
  return null;
}

function componentProps(node, component) {
  const props = {};
  const start = node.values.indexOf(component);
  for (let index = start + 1; index < node.values.length; index += 1) {
    const name = node.strings[index]?.match(/([A-Za-z][A-Za-z0-9]*)=\s*$/)?.[1];
    if (name) props[name] = node.values[index];
  }
  return props;
}

function renderChat({
  hookState,
  activeThreadId = "thread-1",
  runEffects = false,
  threadStateUpdates = [],
  globalAutoApproveEnabled = false,
}) {
  const components = {
    ApprovalCard() {},
    AuthGenericCard() {},
    AuthOauthCard() {},
    AuthTokenCard() {},
    ChatInput() {},
    ConnectionStatus() {},
    EmptyState() {},
    KeyboardShortcuts() {},
    Link() {},
    MessageList() {},
    OnboardingPairingCard() {},
    RecoveryNotice() {},
    SuggestionChips() {},
    TypingIndicator() {},
  };
  const context = {
    ...components,
    React: {
      useCallback: (fn) => fn,
      useEffect: (effect) => {
        if (runEffects) effect();
      },
      useMemo: (fn) => fn(),
      useRef: (initial) => ({ current: initial }),
      useState: (initial) => [initial, () => {}],
    },
    NEW_DRAFT_KEY: "new",
    THREAD_STATE: { NEEDS_ATTENTION: "needs_attention", RUNNING: "running" },
    buildRuntimeContext: () => ({}),
    buildScopedLogsPath: ({ threadId }) => `/logs?thread_id=${threadId}`,
    clearThreadState: (threadId) =>
      threadStateUpdates.push({ threadId, state: null }),
    globalThis: {},
    html: (strings, ...values) => ({ strings: Array.from(strings), values }),
    channelConnectionDisplayName,
    setThreadState: (threadId, state) =>
      threadStateUpdates.push({ threadId, state }),
    setTimeout: () => 1,
    clearTimeout: () => {},
    window: {
      addEventListener: () => {},
      removeEventListener: () => {},
    },
    useChat: () => hookState,
    useT: () => (key) => key,
  };

  vm.runInNewContext(chatSourceForTest(), context);
  const tree = context.globalThis.__testExports.Chat({
    threads: activeThreadId ? [{ id: activeThreadId }] : [],
    activeThreadId,
    onSelectThread: () => {},
    isCreatingThread: false,
    gatewayStatus: {},
    globalAutoApproveEnabled,
  });
  return { tree, components };
}

test("Chat cancel button routes through active thread run cancellation", async () => {
  const cancelReasons = [];
  const { tree, components } = renderChat({
    hookState: {
      messages: [{ id: "message-1" }],
      isProcessing: true,
      pendingGate: null,
      suggestions: [],
      sseStatus: "open",
      historyLoading: false,
      hasMore: false,
      cooldownSeconds: 0,
      recoveryNotice: null,
      activeRun: { runId: "run-1", threadId: "thread-1", status: "running" },
      send: async () => ({}),
      cancelRun: async (reason) => cancelReasons.push(reason),
      retryMessage: () => {},
      approve: () => {},
      recoverHistory: () => {},
      loadMore: () => {},
      setSuggestions: () => {},
      submitAuthToken: async () => {},
    },
  });

  const chatInput = findComponent(tree, components.ChatInput);
  const props = componentProps(chatInput, components.ChatInput);
  assert.equal(props.canCancel, true);
  await props.onCancel();
  assert.deepEqual(cancelReasons, ["user_requested"]);
});

test("Chat leaves the composer editable while a run is processing", () => {
  const { tree, components } = renderChat({
    hookState: {
      messages: [{ id: "message-1" }],
      isProcessing: true,
      pendingGate: null,
      suggestions: [],
      sseStatus: "open",
      historyLoading: false,
      hasMore: false,
      cooldownSeconds: 0,
      recoveryNotice: null,
      activeRun: { runId: "run-1", threadId: "thread-1", status: "running" },
      send: async () => ({}),
      cancelRun: async () => {},
      retryMessage: () => {},
      approve: () => {},
      recoverHistory: () => {},
      loadMore: () => {},
      setSuggestions: () => {},
      submitAuthToken: async () => {},
    },
  });

  const chatInput = findComponent(tree, components.ChatInput);
  const props = componentProps(chatInput, components.ChatInput);
  assert.equal(props.disabled, false);
  assert.equal(props.sendDisabled, true);
});

test("Chat refuses composer sends while a run is processing", async () => {
  let sendCalls = 0;
  const { tree, components } = renderChat({
    hookState: {
      messages: [{ id: "message-1" }],
      isProcessing: true,
      pendingGate: null,
      suggestions: [],
      sseStatus: "open",
      historyLoading: false,
      hasMore: false,
      cooldownSeconds: 0,
      recoveryNotice: null,
      activeRun: { runId: "run-1", threadId: "thread-1", status: "running" },
      send: async () => {
        sendCalls += 1;
        return {};
      },
      cancelRun: async () => {},
      retryMessage: () => {},
      approve: () => {},
      recoverHistory: () => {},
      loadMore: () => {},
      setSuggestions: () => {},
      submitAuthToken: async () => {},
    },
  });

  const chatInput = findComponent(tree, components.ChatInput);
  const props = componentProps(chatInput, components.ChatInput);
  const response = await props.onSend("draft while busy");

  assert.equal(response, null);
  assert.equal(sendCalls, 0);
});

test("Chat cancel button ignores active runs from another thread", () => {
  const { tree, components } = renderChat({
    hookState: {
      messages: [{ id: "message-1" }],
      isProcessing: true,
      pendingGate: null,
      suggestions: [],
      sseStatus: "open",
      historyLoading: false,
      hasMore: false,
      cooldownSeconds: 0,
      recoveryNotice: null,
      activeRun: { runId: "run-1", threadId: "thread-2", status: "running" },
      send: async () => ({}),
      cancelRun: async () => {},
      retryMessage: () => {},
      approve: () => {},
      recoverHistory: () => {},
      loadMore: () => {},
      setSuggestions: () => {},
      submitAuthToken: async () => {},
    },
  });

  const chatInput = findComponent(tree, components.ChatInput);
  const props = componentProps(chatInput, components.ChatInput);
  assert.equal(props.canCancel, false);
});

test("Chat keeps composer send blocked while a gate owns the run decision", async () => {
  const pendingGate = {
    kind: "gate",
    requestId: "request-1",
    toolName: "tool",
    description: "",
    parameters: "",
  };
  let sendCount = 0;
  const { tree, components } = renderChat({
    hookState: {
      messages: [{ id: "message-1" }],
      isProcessing: false,
      pendingGate,
      suggestions: [],
      sseStatus: "open",
      historyLoading: false,
      hasMore: false,
      cooldownSeconds: 0,
      recoveryNotice: null,
      activeRun: { runId: "run-1", threadId: "thread-1", status: "blocked" },
      send: async () => {
        sendCount += 1;
        return {};
      },
      cancelRun: async () => {},
      retryMessage: () => {},
      approve: () => {},
      recoverHistory: () => {},
      loadMore: () => {},
      setSuggestions: () => {},
      submitAuthToken: async () => {},
    },
  });

  const chatInput = findComponent(tree, components.ChatInput);
  const props = componentProps(chatInput, components.ChatInput);
  assert.equal(props.canCancel, false);
  assert.equal(props.sendDisabled, true);
  assert.equal(
    props.statusText,
    "Resolve the approval request before sending another message.",
  );
  await assert.rejects(
    props.onSend("draft while approval is open"),
    /Resolve the approval request before sending another message/,
  );
  assert.equal(sendCount, 0);
});

test("Chat keeps the new-conversation composer sendable while a prior run is settling", async () => {
  let sentBody = null;
  const { tree, components } = renderChat({
    activeThreadId: null,
    hookState: {
      messages: [],
      isProcessing: true,
      pendingGate: null,
      suggestions: [],
      sseStatus: "open",
      historyLoading: false,
      hasMore: false,
      cooldownSeconds: 0,
      recoveryNotice: null,
      activeRun: { runId: "run-1", threadId: "thread-1", status: "running" },
      send: async (content, options) => {
        sentBody = { content, options };
        return { thread_id: "thread-2" };
      },
      cancelRun: async () => {},
      retryMessage: () => {},
      approve: () => {},
      recoverHistory: () => {},
      loadMore: () => {},
      setSuggestions: () => {},
      submitAuthToken: async () => {},
    },
  });

  const emptyState = findComponent(tree, components.EmptyState);
  const props = componentProps(emptyState, components.EmptyState);
  assert.equal(props.sendDisabled, false);
  assert.equal(props.canCancel, false);

  await props.onSend("hi how are you");

  assert.equal(sentBody.content, "hi how are you");
  assert.equal(sentBody.options.threadId, null);
  assert.equal(sentBody.options.images.length, 0);
  assert.equal(sentBody.options.attachments.length, 0);
});

test("Chat renders the pairing card from a channel-connection gate and blocks composer sends", async () => {
  // A connectable channel that needs connection blocks the turn as a standard
  // auth gate: a `manual_token` challenge that also carries a `connection`
  // requirement. Chat renders the pairing card off that gate — no timeline
  // heuristic — wired to a redeem submit and a run-cancel dismiss.
  const pendingGate = {
    kind: "auth_required",
    challengeKind: "manual_token",
    runId: "run-1",
    gateRef: "gate-1",
    connection: {
      channel: "telegram",
      instructions: "Message the Telegram bot and paste the code here.",
      inputPlaceholder: "Enter code",
      submitLabel: "Connect",
      errorMessage: "Pairing failed.",
    },
  };
  const submissions = [];
  const cancelReasons = [];
  const threadStateUpdates = [];
  let sendCount = 0;
  const { tree, components } = renderChat({
    runEffects: true,
    threadStateUpdates,
    hookState: {
      messages: [{ id: "message-1" }],
      isProcessing: false,
      pendingGate,
      suggestions: [],
      sseStatus: "open",
      historyLoading: false,
      hasMore: false,
      cooldownSeconds: 0,
      recoveryNotice: null,
      activeRun: { runId: "run-1", threadId: "thread-1", status: "awaiting_gate" },
      send: async () => {
        sendCount += 1;
        return {};
      },
      cancelRun: async (reason) => cancelReasons.push(reason),
      retryMessage: () => {},
      approve: () => {},
      recoverHistory: () => {},
      loadMore: () => {},
      setSuggestions: () => {},
      submitAuthToken: async () => {},
      submitChannelConnectionPairing: async (code) => submissions.push(code),
    },
  });

  const pairingCard = findComponent(tree, components.OnboardingPairingCard);
  assert.ok(pairingCard, "pairing card should render off the manual_token+connection gate");
  const pairingProps = componentProps(pairingCard, components.OnboardingPairingCard);
  // The gate's connection context is normalized onto an onboarding-shaped prop.
  assert.equal(pairingProps.onboarding.extensionName, "telegram");
  assert.equal(
    pairingProps.onboarding.instructions,
    "Message the Telegram bot and paste the code here.",
  );
  assert.deepEqual(threadStateUpdates, [
    { threadId: "thread-1", state: "needs_attention" },
  ]);
  // Submit redeems through the pairing handler (no resolveGate here).
  await pairingProps.onSubmit("A1B2C3");
  assert.deepEqual(submissions, ["A1B2C3"]);
  // Cancel abandons the parked turn via the run-cancel endpoint.
  await pairingProps.onCancel();
  assert.deepEqual(cancelReasons, ["user_requested"]);

  const chatInput = findComponent(tree, components.ChatInput);
  const inputProps = componentProps(chatInput, components.ChatInput);
  assert.equal(inputProps.sendDisabled, true);
  assert.equal(
    inputProps.statusText,
    "Finish connecting the channel before sending another message.",
  );
  // The pairing gate blocks the composer exactly like any other pending gate.
  await assert.rejects(
    inputProps.onSend("do not send while pairing"),
    /Finish connecting the channel before sending another message/,
  );
  assert.equal(sendCount, 0);
});

test("Chat renders a timeline load failure as an alert instead of the empty landing", () => {
  const historyLoadError = "Failed to load conversation history.";
  const { tree, components } = renderChat({
    hookState: {
      messages: [],
      isProcessing: false,
      pendingGate: null,
      suggestions: [],
      sseStatus: "open",
      historyLoading: false,
      historyLoadError,
      hasMore: false,
      cooldownSeconds: 0,
      recoveryNotice: null,
      activeRun: null,
      send: async () => ({}),
      cancelRun: async () => {},
      retryMessage: () => {},
      approve: () => {},
      recoverHistory: () => {},
      loadMore: () => {},
      setSuggestions: () => {},
      submitAuthToken: async () => {},
    },
  });

  const alert = findNode(tree, (node) =>
    node.strings.some((part) => part.includes('role="alert"')),
  );
  assert.ok(alert, "history load failure should render a role=alert banner");
  assert.ok(alert.values.includes(historyLoadError));
  assert.equal(findComponent(tree, components.EmptyState), null);
});

test("Chat does not render a top-level logs header for the active thread run", () => {
  const { tree, components } = renderChat({
    hookState: {
      messages: [{ id: "message-1" }],
      isProcessing: true,
      pendingGate: null,
      suggestions: [],
      sseStatus: "open",
      historyLoading: false,
      hasMore: false,
      cooldownSeconds: 0,
      recoveryNotice: null,
      activeRun: { runId: "run-1", threadId: "thread-1", status: "running" },
      send: async () => ({}),
      cancelRun: async () => {},
      retryMessage: () => {},
      approve: () => {},
      recoverHistory: () => {},
      loadMore: () => {},
      setSuggestions: () => {},
      submitAuthToken: async () => {},
    },
  });

  assert.equal(
    findComponent(tree, components.Link),
    null,
    "active chat should not render an extra run logs router link outside message actions",
  );
  const messageList = findComponent(tree, components.MessageList);
  assert.equal(
    componentProps(messageList, components.MessageList).logsPath,
    "/logs?thread_id=thread-1",
    "chat should pass a prebuilt thread-scoped logs path down to MessageList",
  );
  assert.equal(
    findNode(tree, (node) =>
      node.strings.some((part) =>
        part.includes("justify-end border-b border-[var(--v2-panel-border)]")
      )
    ),
    null,
    "active run logs link should not render as a duplicate top header bar",
  );
});

test("Chat deny gate callback routes through approve compatibility path", () => {
  const approveCalls = [];
  const pendingGate = {
    kind: "gate",
    requestId: "request-1",
    toolName: "tool",
    description: "",
    parameters: "",
  };
  const { tree, components } = renderChat({
    hookState: {
      messages: [{ id: "message-1" }],
      isProcessing: false,
      pendingGate,
      suggestions: [],
      sseStatus: "open",
      historyLoading: false,
      hasMore: false,
      cooldownSeconds: 0,
      recoveryNotice: null,
      activeRun: { runId: "run-1", threadId: "thread-1", status: "blocked" },
      send: async () => ({}),
      cancelRun: async () => {},
      retryMessage: () => {},
      approve: (...args) => approveCalls.push(args),
      recoverHistory: () => {},
      loadMore: () => {},
      setSuggestions: () => {},
      submitAuthToken: async () => {},
    },
  });

  const approvalCard = findComponent(tree, components.ApprovalCard);
  const props = componentProps(approvalCard, components.ApprovalCard);
  assert.equal(props.globalAutoApproveEnabled, false);
  props.onDeny();
  assert.deepEqual(approveCalls, [["request-1", "deny", "gate"]]);
});
