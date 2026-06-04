import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

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
    return { values: node.values.slice(componentIndex) };
  }
  for (const value of node.values) {
    const found = findComponent(value, component);
    if (found) return found;
  }
  return null;
}

function renderChat({ hookState, activeThreadId = "thread-1" }) {
  const components = {
    ApprovalCard() {},
    AuthGenericCard() {},
    AuthOauthCard() {},
    AuthTokenCard() {},
    ChatInput() {},
    ConnectionStatus() {},
    EmptyState() {},
    KeyboardShortcuts() {},
    MessageList() {},
    RecoveryNotice() {},
    SuggestionChips() {},
    TypingIndicator() {},
  };
  const context = {
    ...components,
    React: {
      useCallback: (fn) => fn,
      useEffect: () => {},
      useMemo: (fn) => fn(),
      useState: (initial) => [initial, () => {}],
    },
    THREAD_STATE: { NEEDS_ATTENTION: "needs_attention", RUNNING: "running" },
    buildRuntimeContext: () => ({}),
    clearThreadState: () => {},
    globalThis: {},
    html: (_strings, ...values) => ({ values }),
    setThreadState: () => {},
    useChat: () => hookState,
  };

  vm.runInNewContext(chatSourceForTest(), context);
  const tree = context.globalThis.__testExports.Chat({
    threads: [{ id: activeThreadId }],
    activeThreadId,
    onSelectThread: () => {},
    isCreatingThread: false,
    gatewayStatus: {},
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
  assert.equal(chatInput.values[7], true);
  await chatInput.values[8]();
  assert.deepEqual(cancelReasons, ["user_requested"]);
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
  assert.equal(chatInput.values[7], false);
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
  approvalCard.values[3]();
  assert.deepEqual(approveCalls, [["request-1", "deny", "gate"]]);
});
