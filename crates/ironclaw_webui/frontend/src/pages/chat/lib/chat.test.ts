// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

import { channelConnectionDisplayName } from "../../../lib/channel-connection-events";
import { channelConnectionFromGate } from "./gates";

function chatSourceForTest() {
  const source = readFileSync(new URL("../chat.tsx", import.meta.url), "utf8");
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
  globalAutoApproveEnabled = false,
  showChatLogsShortcut = true,
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
    globalThis: {},
    html: (strings, ...values) => ({ strings: Array.from(strings), values }),
    channelConnectionDisplayName,
    channelConnectionFromGate,
    setThreadState: (threadId, state) =>
      threadStateUpdates.push({ threadId, state }),
    setTimeout: () => 1,
    clearTimeout: () => {},
    window: {
      addEventListener: () => {},
      removeEventListener: () => {},
    },
    useChat: () => hookState,
    useInterfacePreferences: () => ({ showChatLogsShortcut }),
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

test("Chat hides typing indicator once the active run streams assistant text", () => {
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

  assert.equal(findComponent(tree, components.TypingIndicator), null);
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
