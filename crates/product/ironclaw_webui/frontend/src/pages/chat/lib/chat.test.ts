// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";
import vm from "node:vm";

import { channelConnectionDisplayName } from "../../../lib/channel-connection-events";
import { componentSourceForTest } from "../../../lib/vm-component-harness";
import "../../../test/vm-tsx-setup";
import { channelConnectionFromGate } from "./gates";
import { messageBelongsToActiveRun } from "./message-types";

function chatSourceForTest() {
  return componentSourceForTest(
    new URL("../chat.tsx", import.meta.url),
    "Chat",
  );
}

function findComponent(node, component) {
  if (Array.isArray(node)) {
    for (const item of node) {
      const found = findComponent(item, component);
      if (found) return found;
    }
    return null;
  }
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
  if (Array.isArray(node)) {
    for (const item of node) {
      const found = findNode(item, predicate);
      if (found) return found;
    }
    return null;
  }
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
  assert.ok(node, "expected component node");
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
  toastCalls = [],
  consoleErrors = [],
  globalAutoApproveEnabled = false,
  showChatLogsShortcut = true,
  onSelectThread = () => {},
  contextOverrides = {},
  // Positional ref slots, shared by reference across repeated renderChat()
  // calls that pass the same array -- lets a test simulate the same
  // component "instance" re-rendering (e.g. a navigation-triggered
  // rerender) with useRef state persisted across renders, the way real
  // React would. Left undefined by default: each call gets fresh refs,
  // matching every existing single-render test.
  refs = [],
}) {
  let refSlot = 0;
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
      useRef: (initial) => {
        const slot = refSlot;
        refSlot += 1;
        if (!(slot in refs)) refs[slot] = { current: initial };
        return refs[slot];
      },
      useState: (initial) => [
        typeof initial === "function" ? initial() : initial,
        () => {},
      ],
    },
    NEW_DRAFT_KEY: "new",
    THREAD_STATE: { NEEDS_ATTENTION: "needs_attention", RUNNING: "running" },
    buildRuntimeContext: () => ({}),
    buildScopedLogsPath: ({ threadId }) => `/logs?thread_id=${threadId}`,
    clearThreadState: (threadId) =>
      threadStateUpdates.push({ threadId, state: null }),
    console: {
      error: (...args) => consoleErrors.push(args),
    },
    globalThis: {},
    html: (strings, ...values) => ({ strings: Array.from(strings), values }),
    channelConnectionDisplayName,
    channelConnectionFromGate,
    messageBelongsToActiveRun,
    setThreadState: (threadId, state) =>
      threadStateUpdates.push({ threadId, state }),
    toast: (message, options) => toastCalls.push({ message, options }),
    setTimeout: () => 1,
    clearTimeout: () => {},
    window: {
      addEventListener: () => {},
      removeEventListener: () => {},
    },
    useChat: () => hookState,
    useChatCommands: () => [],
    matchCommand: () => null,
    useInterfacePreferences: () => ({ showChatLogsShortcut }),
    useT: () => (key) => key,
    ...contextOverrides,
  };

  vm.runInNewContext(chatSourceForTest(), context);
  const tree = context.globalThis.__testExports.Chat({
    threads: activeThreadId ? [{ id: activeThreadId }] : [],
    activeThreadId,
    onSelectThread,
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

test("Chat shows a localized error toast when run cancellation fails", async () => {
  const toastCalls = [];
  const consoleErrors = [];
  const cancellationError = Object.assign(
    new Error("sensitive cancellation detail"),
    {
      status: 503,
      body: '{"error":"sensitive cancellation detail"}',
      payload: { error: "sensitive cancellation detail" },
    }
  );
  const { tree, components } = renderChat({
    toastCalls,
    consoleErrors,
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
      cancelRun: async () => {
        throw cancellationError;
      },
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
  await props.onCancel();

  assert.deepEqual(JSON.parse(JSON.stringify(toastCalls)), [
    {
      message: "chat.cancelFailed",
      options: { tone: "error" },
    },
  ]);
  assert.deepEqual(JSON.parse(JSON.stringify(consoleErrors)), [
    [
      "Failed to cancel active run",
      { category: "http_error", status: 503 },
    ],
  ]);
  assert.doesNotMatch(
    JSON.stringify(consoleErrors),
    /sensitive cancellation detail/
  );
});

test("Chat redacts malformed cancellation errors in request diagnostics", async () => {
  const consoleErrors = [];
  const cancellationError = Object.assign(
    new Error("sensitive malformed cancellation detail"),
    {
      status: "503",
      body: '{"error":"sensitive malformed cancellation detail"}',
      payload: { error: "sensitive malformed cancellation detail" },
    }
  );
  const { tree, components } = renderChat({
    consoleErrors,
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
      cancelRun: async () => {
        throw cancellationError;
      },
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
  await props.onCancel();

  assert.deepEqual(JSON.parse(JSON.stringify(consoleErrors)), [
    ["Failed to cancel active run", { category: "request_error" }],
  ]);
  assert.doesNotMatch(
    JSON.stringify(consoleErrors),
    /sensitive malformed cancellation detail/
  );
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
  // Queued-message UX: a processing run no longer disables the composer send —
  // a follow-up is accepted and queued behind the active run.
  assert.equal(props.sendDisabled, false);
});

test("Chat shows typing indicator before assistant text streams", () => {
  const { tree, components } = renderChat({
    hookState: {
      messages: [{ id: "message-1", role: "user", content: "hello" }],
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

  assert.ok(findComponent(tree, components.TypingIndicator));
});

test("Chat keeps typing indicator while the active run streams assistant text", () => {
  const { tree, components } = renderChat({
    hookState: {
      messages: [
        { id: "message-1", role: "user", content: "hello" },
        {
          id: "text-text:run-1",
          role: "assistant",
          content: "H",
          isFinalReply: false,
          turnRunId: "run-1",
        },
      ],
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

  assert.ok(findComponent(tree, components.TypingIndicator));
});

test("Chat keeps typing indicator when streamed text belongs to another run", () => {
  const { tree, components } = renderChat({
    hookState: {
      messages: [
        { id: "message-1", role: "user", content: "hello" },
        {
          id: "text-text:run-0",
          role: "assistant",
          content: "old text",
          isFinalReply: false,
          turnRunId: "run-0",
        },
      ],
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

  assert.ok(findComponent(tree, components.TypingIndicator));
});

test("Chat keeps typing indicator for a historical assistant draft without an active run", () => {
  const { tree, components } = renderChat({
    hookState: {
      messages: [
        { id: "message-1", role: "user", content: "hello" },
        {
          id: "msg-draft",
          role: "assistant",
          content: "historical draft",
          isFinalReply: false,
          turnRunId: "run-old",
        },
      ],
      isProcessing: true,
      pendingGate: null,
      suggestions: [],
      sseStatus: "open",
      historyLoading: false,
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

  assert.ok(findComponent(tree, components.TypingIndicator));
});

test("Chat admits composer sends while a run is processing (queued)", async () => {
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

  // Queued-message UX: the send is admitted (reaches useChat.send, which routes
  // it to the backend queue) rather than being refused locally.
  assert.notEqual(response, null);
  assert.equal(sendCalls, 1);
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
    "chat.resolveApprovalBeforeSend",
  );
  await assert.rejects(
    props.onSend("draft while approval is open"),
    /chat\.resolveApprovalBeforeSend/,
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
  // auth gate. Chat renders the manifest-selected host-issued pairing panel
  // off that gate — no timeline heuristic or pasted-code redeem route.
  const pendingGate = {
    kind: "auth_required",
    challengeKind: "pairing",
    runId: "run-1",
    gateRef: "gate-1",
    connection: {
      channel: "telegram",
      strategy: "web_generated_code",
      instructions: "Open Telegram with the generated link.",
    },
  };
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
    },
  });

  const pairingCard = findComponent(tree, components.OnboardingPairingCard);
  assert.ok(pairingCard, "pairing card should render off the pairing+connection gate");
  const pairingProps = componentProps(pairingCard, components.OnboardingPairingCard);
  // The gate's connection context is normalized onto an onboarding-shaped prop.
  assert.equal(pairingProps.onboarding.extensionName, "telegram");
  assert.equal(
    pairingProps.onboarding.instructions,
    "Open Telegram with the generated link.",
  );
  assert.deepEqual(threadStateUpdates, [
    { threadId: "thread-1", state: "needs_attention" },
  ]);
  assert.equal(pairingProps.onSubmit, undefined);
  // Cancel abandons the parked turn via the run-cancel endpoint.
  await pairingProps.onCancel();
  assert.deepEqual(cancelReasons, ["user_requested"]);

  const chatInput = findComponent(tree, components.ChatInput);
  const inputProps = componentProps(chatInput, components.ChatInput);
  assert.equal(inputProps.sendDisabled, true);
  assert.equal(
    inputProps.statusText,
    "chat.finishPairingBeforeSend",
  );
  // The pairing gate blocks the composer exactly like any other pending gate.
  await assert.rejects(
    inputProps.onSend("do not send while pairing"),
    /chat\.finishPairingBeforeSend/,
  );
  assert.equal(sendCount, 0);
});

test("Chat aligns the composer notice and card for a non-pairing gate carrying connection context", () => {
  // Backend invariant (crates/ironclaw_product_workflow/src/auth_prompt.rs):
  // `connection` rides ONLY on pairing gates. This pins the frontend so that
  // even if a manual_token gate ever carried one, the composer affordance and
  // the rendered card cannot disagree — both key off `channelConnectionFromGate`.
  // Before the fix the composer claimed "finish pairing" while the token-paste
  // card rendered.
  const pendingGate = {
    kind: "auth_required",
    challengeKind: "manual_token",
    requestId: "request-1",
    runId: "run-1",
    gateRef: "gate-1",
    connection: {
      channel: "telegram",
      strategy: "web_generated_code",
      instructions: "stray connection context",
    },
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
      activeRun: { runId: "run-1", threadId: "thread-1", status: "awaiting_gate" },
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

  // The manual_token gate renders the token-paste card, not the pairing panel.
  assert.ok(
    findComponent(tree, components.AuthTokenCard),
    "manual_token gate renders the token card",
  );
  assert.equal(
    findComponent(tree, components.OnboardingPairingCard),
    null,
    "a non-pairing gate must not render the pairing card",
  );
  // ...and the composer shows the generic gate notice, never the pairing one.
  const chatInput = findComponent(tree, components.ChatInput);
  const inputProps = componentProps(chatInput, components.ChatInput);
  assert.equal(inputProps.sendDisabled, true);
  assert.equal(inputProps.statusText, "chat.resolveApprovalBeforeSend");
});

test("Chat renders a timeline load failure as an alert instead of the empty landing", () => {
  const historyLoadError = "chat.history.loadFailed";
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

test("Chat hides the floating thread logs shortcut when the preference is off", () => {
  const { tree, components } = renderChat({
    showChatLogsShortcut: false,
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

  const messageList = findComponent(tree, components.MessageList);
  assert.equal(componentProps(messageList, components.MessageList).logsPath, null);
});

test("Chat threads the server command inventory down to MessageList so a command-result system message can render the dropdown-echoing list", () => {
  // The "available commands" rejection (chat-commands.ts's
  // classifyCommandResponse -> COMMAND_LIST) renders the SAME inventory the
  // composer's dropdown uses (see command-result.tsx), not a re-hardcoded
  // copy — MessageList (and, in turn, MessageBubble) must receive it.
  const commands = [
    { name: "status", title: "Status", description: "d", usage: "/status" },
    { name: "model", title: "Model", description: "d", usage: "/model" },
  ];
  const { tree, components } = renderChat({
    activeThreadId: "thread-1",
    contextOverrides: { useChatCommands: () => commands },
    hookState: {
      messages: [{ id: "message-1" }],
      isProcessing: false,
      pendingGate: null,
      suggestions: [],
      sseStatus: "open",
      historyLoading: false,
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

  const messageList = findComponent(tree, components.MessageList);
  assert.equal(
    componentProps(messageList, components.MessageList).commands,
    commands,
    "MessageList should receive the same command inventory the composer uses",
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

test("Chat intercepts known slash text as a command on an active thread", async () => {
  const sends = [];
  const commandRuns = [];
  const { tree, components } = renderChat({
    activeThreadId: "thread-1",
    contextOverrides: {
      useChatCommands: () => [{ name: "status", usage: "/status" }],
      matchCommand: (text) => (text.startsWith("/status") ? { name: "status" } : null),
    },
    hookState: {
      messages: [{ id: "message-1" }],
      isProcessing: false,
      pendingGate: null,
      suggestions: [],
      sseStatus: "open",
      historyLoading: false,
      hasMore: false,
      cooldownSeconds: 0,
      recoveryNotice: null,
      activeRun: null,
      send: async (content) => {
        sends.push(content);
        return {};
      },
      runCommand: async (text) => {
        commandRuns.push(text);
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

  await props.onSend("/status", {});
  assert.deepEqual(commandRuns, ["/status"], "known slash text executes as a command");
  assert.deepEqual(sends, [], "commands never submit a turn");

  await props.onSend("/status", { images: [{ id: "img-1" }] });
  assert.deepEqual(
    sends,
    ["/status"],
    "slash text with an image submits as a message so the image is not dropped"
  );
  assert.deepEqual(commandRuns, ["/status"], "no second command run");

  await props.onSend("plain text", {});
  assert.deepEqual(sends, ["/status", "plain text"], "ordinary text still submits");
});

test("Chat navigates an active-thread command to a different response thread", async () => {
  const selections = [];
  const { tree, components } = renderChat({
    activeThreadId: "thread-1",
    onSelectThread: (...args) => selections.push(args),
    contextOverrides: {
      useChatCommands: () => [{ name: "new", usage: "/new" }],
      matchCommand: (text) => (text === "/new" ? { name: "new" } : null),
    },
    hookState: {
      messages: [{ id: "message-1" }],
      isProcessing: false,
      pendingGate: null,
      suggestions: [],
      sseStatus: "open",
      historyLoading: false,
      hasMore: false,
      cooldownSeconds: 0,
      recoveryNotice: null,
      activeRun: null,
      send: async () => {
        throw new Error("new command must not submit a turn");
      },
      runCommand: async () => ({ thread_id: "thread-2" }),
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
  await props.onSend("/new", {});

  assert.equal(selections.length, 1);
  assert.equal(selections[0][0], "thread-2");
  assert.equal(selections[0][1].replace, true);
});

test("Chat drops a stale command navigation after the user opens another thread", async () => {
  const selections = [];
  const refs = [];
  const chatProps = (activeThreadId, runCommand) => ({
    activeThreadId,
    onSelectThread: (...args) => selections.push(args),
    refs,
    contextOverrides: {
      useChatCommands: () => [{ name: "new", usage: "/new" }],
      matchCommand: (text) => (text === "/new" ? { name: "new" } : null),
    },
    hookState: {
      messages: [{ id: "message-1" }],
      isProcessing: false,
      pendingGate: null,
      suggestions: [],
      sseStatus: "open",
      historyLoading: false,
      hasMore: false,
      cooldownSeconds: 0,
      recoveryNotice: null,
      activeRun: null,
      send: async () => {
        throw new Error("new command must not submit a turn");
      },
      runCommand,
      cancelRun: async () => {},
      retryMessage: () => {},
      approve: () => {},
      recoverHistory: () => {},
      loadMore: () => {},
      setSuggestions: () => {},
      submitAuthToken: async () => {},
    },
  });
  // The command resolves only after the user has already opened thread-2:
  // mid-flight, re-render the same component instance (shared `refs`) at the
  // newer selection, the way a navigation-triggered rerender would.
  const { tree, components } = renderChat(
    chatProps("thread-1", async () => {
      renderChat(chatProps("thread-2", async () => ({})));
      return { thread_id: "thread-3" };
    })
  );

  const chatInput = findComponent(tree, components.ChatInput);
  const props = componentProps(chatInput, components.ChatInput);
  await props.onSend("/new", {});

  assert.equal(
    selections.length,
    0,
    "a command that resolved after the user navigated elsewhere must not steal the newer selection"
  );
});

test("Chat landing view renders no command menu and submits a known command as an ordinary message", async () => {
  // Homepage commands are intentionally OFF for now (see the interception
  // guard comment in chat.tsx's `handleSend`). A prior change let the
  // landing composer intercept known commands and made `runCommand` create
  // a thread on first contact, but the result notice was appended to the
  // pre-navigation message state and then wiped when the app navigated into
  // the newly created (and still-loading) thread — an empty conversation
  // left behind. Rather than fix that ordering, the landing composer
  // neither shows the command menu nor executes known commands; slash text
  // submits as an ordinary message there, exactly like unknown slash text
  // always has (see the "submits unknown slash text" test below).
  const sends = [];
  const commandRuns = [];
  const commands = [{ name: "status", usage: "/status" }, { name: "model", usage: "/model" }];
  const { tree, components } = renderChat({
    activeThreadId: null,
    contextOverrides: {
      useChatCommands: () => commands,
      matchCommand: (text) => (text.startsWith("/status") ? { name: "status" } : null),
    },
    hookState: {
      messages: [],
      isProcessing: false,
      pendingGate: null,
      suggestions: [],
      sseStatus: "open",
      historyLoading: false,
      hasMore: false,
      cooldownSeconds: 0,
      recoveryNotice: null,
      activeRun: null,
      send: async (content) => {
        sends.push(content);
        return { thread_id: "thread-new" };
      },
      runCommand: async (text) => {
        commandRuns.push(text);
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

  const landing = findComponent(tree, components.EmptyState);
  const props = componentProps(landing, components.EmptyState);
  // Length check rather than `assert.deepEqual(props.commands, [])`: the
  // empty array is a literal evaluated inside the vm-sandboxed chat.tsx
  // source (a different realm), so a direct deepEqual against an
  // outer-realm `[]` fails on prototype identity despite equal values (the
  // same cross-realm gotcha noted on the completion tests in
  // chat-input.test.ts).
  assert.equal(
    props.commands.length,
    0,
    "the landing composer must not render a command menu",
  );

  await props.onSend("/status", {});
  assert.deepEqual(commandRuns, [], "a known command must not execute without an active thread");
  assert.deepEqual(sends, ["/status"], "the known command submits as an ordinary message instead");
});

test("Chat landing view submits unknown slash text as an ordinary message", async () => {
  const sends = [];
  const commandRuns = [];
  const { tree, components } = renderChat({
    activeThreadId: null,
    contextOverrides: {
      useChatCommands: () => [{ name: "status", usage: "/status" }],
      matchCommand: (text) => (text.startsWith("/status") ? { name: "status" } : null),
    },
    hookState: {
      messages: [],
      isProcessing: false,
      pendingGate: null,
      suggestions: [],
      sseStatus: "open",
      historyLoading: false,
      hasMore: false,
      cooldownSeconds: 0,
      recoveryNotice: null,
      activeRun: null,
      send: async (content) => {
        sends.push(content);
        return { thread_id: "thread-new" };
      },
      runCommand: async (text) => {
        commandRuns.push(text);
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

  const landing = findComponent(tree, components.EmptyState);
  const props = componentProps(landing, components.EmptyState);
  await props.onSend("/unknown-text", {});
  assert.deepEqual(commandRuns, [], "unrecognized slash text is not a command");
  assert.deepEqual(sends, ["/unknown-text"], "unknown slash text still sends as an ordinary message");
});


test("Chat does not double-navigate when multiple sends resolve before either can navigate away from the empty-thread view", async () => {
  // Regression test for issue #6581's frontend half: firing several
  // "new chat" sends before the first one's response has navigated the
  // view away from the empty-thread state used to make every one of
  // those sends independently navigate, each tearing down and reopening
  // the single app-wide SSE stream -- exhausting the rate-limit budget
  // through genuinely-accepted reconnects even with the backend fix in
  // place.
  const selections = [];
  const { tree, components } = renderChat({
    activeThreadId: null,
    onSelectThread: (threadId, options) => selections.push({ threadId, options }),
    hookState: {
      messages: [],
      isProcessing: false,
      pendingGate: null,
      suggestions: [],
      sseStatus: "closed",
      historyLoading: false,
      hasMore: false,
      cooldownSeconds: 0,
      recoveryNotice: null,
      activeRun: null,
      send: async (content) => ({ thread_id: `thread-for-${content}` }),
      cancelRun: async () => {},
      retryMessage: () => {},
      approve: () => {},
      recoverHistory: () => {},
      loadMore: () => {},
      setSuggestions: () => {},
      submitAuthToken: async () => {},
    },
  });

  // No messages yet -> the landing composer (EmptyState), not the
  // in-thread ChatInput, is what's rendered and wired to handleSend.
  const emptyState = findComponent(tree, components.EmptyState);
  const { onSend } = componentProps(emptyState, components.EmptyState);

  const [first, second] = await Promise.all([
    onSend("weather in NY"),
    onSend("weather in LA"),
  ]);

  assert.equal(first.thread_id, "thread-for-weather in NY");
  assert.equal(second.thread_id, "thread-for-weather in LA");
  assert.equal(
    selections.length,
    1,
    "only the first concurrent send should navigate the empty-thread view"
  );
  assert.equal(selections[0].threadId, "thread-for-weather in NY");
  // Not deepEqual: the options object is constructed inside the vm-sandboxed
  // chat.tsx source, so it's a cross-realm object relative to this test file
  // and fails Node's strict deep-equality identity checks despite matching
  // structurally.
  assert.equal(selections[0].options.replace, true);
});

test("Chat does not double-navigate when a stale send resolves after the first navigation's rerender", async () => {
  // Regression test for PR #6592 review comment: the previous reset rule
  // ("clear the claim whenever activeThreadId is truthy") fires on the very
  // rerender the first navigation itself causes, re-opening the window for
  // a second, still-in-flight send -- one whose closure captured the old
  // null activeThreadId, same as the first -- to navigate again and
  // reproduce the SSE thrash. This is a genuinely distinct race from
  // "Chat does not double-navigate when multiple sends resolve before
  // either can navigate away from the empty-thread view" above: that test
  // resolves both sends within a single render and never exercises the
  // rerender in between, so it can't see this bug. Reproducing it needs
  // controllable send resolution order plus a real second render sharing
  // ref state, which is why it's a separate test rather than an extra
  // assertion on the existing one.
  const selections = [];
  let resolveFirst;
  let resolveSecond;
  const hookState = {
    messages: [],
    isProcessing: false,
    pendingGate: null,
    suggestions: [],
    sseStatus: "closed",
    historyLoading: false,
    hasMore: false,
    cooldownSeconds: 0,
    recoveryNotice: null,
    activeRun: null,
    send: async (content) =>
      content === "weather in NY"
        ? new Promise((resolve) => {
            resolveFirst = resolve;
          })
        : new Promise((resolve) => {
            resolveSecond = resolve;
          }),
    cancelRun: async () => {},
    retryMessage: () => {},
    approve: () => {},
    recoverHistory: () => {},
    loadMore: () => {},
    setSuggestions: () => {},
    submitAuthToken: async () => {},
  };
  const refs = [];

  // Render 1: the empty-thread landing view (activeThreadId = null). Both
  // sends are fired from this render, so both handleSend closures capture
  // activeThreadId = null -- exactly like two concurrent "new chat" sends.
  const render1 = renderChat({
    activeThreadId: null,
    onSelectThread: (threadId, options) => selections.push({ threadId, options }),
    hookState,
    refs,
  });
  const emptyState1 = findComponent(render1.tree, render1.components.EmptyState);
  const { onSend } = componentProps(emptyState1, render1.components.EmptyState);

  const firstSend = onSend("weather in NY");
  const secondSend = onSend("weather in LA");

  // The first send resolves and claims the navigation.
  resolveFirst({ thread_id: "thread-for-weather in NY" });
  await firstSend;
  assert.equal(selections.length, 1, "first send should navigate");
  assert.equal(selections[0].threadId, "thread-for-weather in NY");

  // Render 2: simulates the rerender `onSelectThread` triggers once the
  // parent adopts the new thread id -- activeThreadId is now truthy. Shares
  // `refs` with render 1 so the navigation-claim ref persists across
  // renders the way a real mounted component would.
  renderChat({
    activeThreadId: "thread-for-weather in NY",
    onSelectThread: (threadId, options) => selections.push({ threadId, options }),
    hookState,
    refs,
  });

  // The second send -- still holding its stale render-1 closure with
  // activeThreadId = null -- resolves after the rerender.
  resolveSecond({ thread_id: "thread-for-weather in LA" });
  await secondSend;

  assert.equal(
    selections.length,
    1,
    "a stale send resolving after the first navigation's rerender must not navigate again"
  );
});

test("Chat does not let a stale send from an earlier empty-thread cycle hijack a new cycle started by \"+ New\"", async () => {
  // Regression test for PR #6592 review comment (chat.tsx:201, Medium):
  // the previous reset rule cleared the navigation claim on *any*
  // truthy->falsy transition of activeThreadId without regard to which
  // batch of sends the transition belonged to. Sequence: A and B are both
  // fired from the landing composer (both closures capture
  // activeThreadId = null). A resolves first and claims/navigates to
  // threadA. The user clicks "+ New" -- a genuine new empty-thread cycle
  // begins, and the old code reset the claim unconditionally. B, a stale
  // closure from the *original* batch, then resolves and -- because the
  // claim was reset and its captured activeThreadId is still null --
  // hijacks the brand-new empty cycle by navigating to threadB.
  const selections = [];
  let resolveFirst;
  let resolveSecond;
  const hookState = {
    messages: [],
    isProcessing: false,
    pendingGate: null,
    suggestions: [],
    sseStatus: "closed",
    historyLoading: false,
    hasMore: false,
    cooldownSeconds: 0,
    recoveryNotice: null,
    activeRun: null,
    send: async (content) =>
      content === "weather in NY"
        ? new Promise((resolve) => {
            resolveFirst = resolve;
          })
        : new Promise((resolve) => {
            resolveSecond = resolve;
          }),
    cancelRun: async () => {},
    retryMessage: () => {},
    approve: () => {},
    recoverHistory: () => {},
    loadMore: () => {},
    setSuggestions: () => {},
    submitAuthToken: async () => {},
  };
  const refs = [];

  // Render 1: the empty-thread landing view (activeThreadId = null). Both
  // sends are fired from this render, so both handleSend closures capture
  // activeThreadId = null.
  const render1 = renderChat({
    activeThreadId: null,
    onSelectThread: (threadId, options) => selections.push({ threadId, options }),
    hookState,
    refs,
  });
  const emptyState1 = findComponent(render1.tree, render1.components.EmptyState);
  const { onSend } = componentProps(emptyState1, render1.components.EmptyState);

  const firstSend = onSend("weather in NY");
  const secondSend = onSend("weather in LA");

  // A resolves and claims the navigation.
  resolveFirst({ thread_id: "thread-for-weather in NY" });
  await firstSend;
  assert.equal(selections.length, 1, "first send should navigate");
  assert.equal(selections[0].threadId, "thread-for-weather in NY");

  // Render 2: the post-navigation rerender, activeThreadId is now truthy.
  renderChat({
    activeThreadId: "thread-for-weather in NY",
    onSelectThread: (threadId, options) => selections.push({ threadId, options }),
    hookState,
    refs,
  });

  // Render 3: the user clicks "+ New" -- activeThreadId goes back to null,
  // a genuinely new empty-thread cycle begins.
  renderChat({
    activeThreadId: null,
    onSelectThread: (threadId, options) => selections.push({ threadId, options }),
    hookState,
    refs,
  });

  // B -- still holding its stale render-1 closure with activeThreadId =
  // null from the *original* batch -- resolves after the "+ New" reset.
  resolveSecond({ thread_id: "thread-for-weather in LA" });
  await secondSend;

  assert.equal(
    selections.length,
    1,
    "a stale send from the original batch must not hijack the new empty-thread cycle started by \"+ New\""
  );
});
