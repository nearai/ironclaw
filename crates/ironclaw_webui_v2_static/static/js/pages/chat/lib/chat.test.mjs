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
    lines.push(
      line
        .replace("export function Chat", "function Chat")
        .replace(
          "export function enrichApprovalGateWithActivityArguments",
          "function enrichApprovalGateWithActivityArguments",
        ),
    );
  }
  return `${lines.join(
    "\n",
  )}\nglobalThis.__testExports = { Chat, enrichApprovalGateWithActivityArguments };`;
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

function renderChat({ hookState, activeThreadId = "thread-1" }) {
  const components = {
    ApprovalCard() {},
    AuthGenericCard() {},
    AuthOauthCard() {},
    AuthTokenCard() {},
    ChatInput() {},
    ConnectionStatus() {},
    EmptyState() {},
    Icon() {},
    KeyboardShortcuts() {},
    Link() {},
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
      useRef: (initial) => ({ current: initial }),
      useState: (initial) => [initial, () => {}],
    },
    THREAD_STATE: { NEEDS_ATTENTION: "needs_attention", RUNNING: "running" },
    buildScopedLogsPath: (
      { threadId, runId } = {},
      { absolute = false } = {},
    ) => {
      const params = [];
      if (threadId) params.push(`thread_id=${encodeURIComponent(threadId)}`);
      if (runId) params.push(`run_id=${encodeURIComponent(runId)}`);
      const query = params.length > 0 ? `?${params.join("&")}` : "";
      return `${absolute ? "/v2" : ""}/logs${query}`;
    },
    buildRuntimeContext: () => ({}),
    clearThreadState: () => {},
    globalThis: {},
    html: (strings, ...values) => ({ strings: Array.from(strings), values }),
    setThreadState: () => {},
    useChat: () => hookState,
    useT: () => (key) => key,
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
      channelConnectAction: null,
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
      dismissChannelConnectAction: () => {},
    },
  });

  const chatInput = findComponent(tree, components.ChatInput);
  const props = componentProps(chatInput, components.ChatInput);
  assert.equal(props.disabled, false);
  assert.equal(props.sendDisabled, false);
});

test("Chat queues composer sends while a run is processing", async () => {
  let sendCalls = 0;
  const { tree, components } = renderChat({
    hookState: {
      messages: [{ id: "message-1" }],
      isProcessing: true,
      pendingGate: null,
      channelConnectAction: null,
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
      dismissChannelConnectAction: () => {},
    },
  });

  const chatInput = findComponent(tree, components.ChatInput);
  const props = componentProps(chatInput, components.ChatInput);
  const response = await props.onSend("draft while busy");

  assert.deepEqual(response, {});
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
    "Resolve the approval request before sending another message.",
  );
  await assert.rejects(
    props.onSend("draft while approval is open"),
    /Resolve the approval request before sending another message/,
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
      channelConnectAction: null,
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
      dismissChannelConnectAction: () => {},
    },
  });

  const alert = findNode(tree, (node) =>
    node.strings.some((part) => part.includes('role="alert"')),
  );
  assert.ok(alert, "history load failure should render a role=alert banner");
  assert.ok(alert.values.includes(historyLoadError));
  assert.equal(findComponent(tree, components.EmptyState), null);
});

test("Chat links to scoped logs for the active thread run", () => {
  const { tree, components } = renderChat({
    hookState: {
      messages: [{ id: "message-1" }],
      isProcessing: true,
      pendingGate: null,
      channelConnectAction: null,
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
      dismissChannelConnectAction: () => {},
    },
  });

  const logsLink = findComponent(tree, components.Link);
  assert.ok(logsLink, "active chat should render a scoped logs link");
  assert.equal(
    componentProps(logsLink, components.Link).to,
    "/v2/logs?thread_id=thread-1&run_id=run-1",
  );
  assert.ok(logsLink.values.includes("nav.logs"));

  const messageList = findComponent(tree, components.MessageList);
  assert.equal(
    findComponent(messageList, components.Link),
    null,
    "active run logs link should not render in the message list footer near the composer",
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
  props.onDeny();
  assert.deepEqual(approveCalls, [["request-1", "deny", "gate"]]);
});

test("Chat approval card includes matching tool activity arguments", () => {
  const pendingGate = {
    kind: "gate",
    requestId: "request-1",
    invocationId: "invocation-search",
    toolName: "nearai.web_search",
    description: "Run tool",
    approvalDetails: [{ label: "Capability", value: "nearai.web_search" }],
  };
  const { tree, components } = renderChat({
    hookState: {
      messages: [
        {
          id: "tool-invocation-search",
          role: "tool_activity",
          invocationId: "invocation-search",
          toolName: "web_search",
          toolStatus: "running",
          toolParameters: "query: deploy status",
        },
      ],
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
      approve: () => {},
      recoverHistory: () => {},
      loadMore: () => {},
      setSuggestions: () => {},
      submitAuthToken: async () => {},
    },
  });

  const approvalCard = findComponent(tree, components.ApprovalCard);
  const props = componentProps(approvalCard, components.ApprovalCard);

  assert.deepEqual(JSON.parse(JSON.stringify(props.gate.approvalDetails)), [
    { label: "Capability", value: "nearai.web_search" },
    { label: "Arguments", value: "query: deploy status" },
  ]);
  assert.equal(props.gate.parameters, "query: deploy status");
});

test("Chat approval card falls back to same-run tool arguments for nested approval gates", () => {
  const pendingGate = {
    kind: "gate",
    requestId: "request-1",
    runId: "run-1",
    invocationId: "invocation-http",
    toolName: "builtin.http",
    description: "Network approval",
    approvalDetails: [{ label: "Method", value: "GET" }],
  };
  const { tree, components } = renderChat({
    hookState: {
      messages: [
        {
          id: "tool-invocation-search",
          role: "tool_activity",
          invocationId: "invocation-search",
          turnRunId: "run-1",
          toolName: "web_search",
          toolStatus: "running",
          toolParameters: "query: deploy status",
        },
        {
          id: "tool-invocation-http",
          role: "tool_activity",
          invocationId: "invocation-http",
          turnRunId: "run-1",
          toolName: "http",
          toolStatus: "running",
        },
      ],
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
      approve: () => {},
      recoverHistory: () => {},
      loadMore: () => {},
      setSuggestions: () => {},
      submitAuthToken: async () => {},
    },
  });

  const approvalCard = findComponent(tree, components.ApprovalCard);
  const props = componentProps(approvalCard, components.ApprovalCard);

  assert.deepEqual(JSON.parse(JSON.stringify(props.gate.approvalDetails)), [
    { label: "Method", value: "GET" },
    { label: "Arguments", value: "query: deploy status" },
  ]);
});

test("approval argument enrichment uses same-run arguments when gate has no invocation id", () => {
  const context = { globalThis: {} };
  vm.runInNewContext(chatSourceForTest(), {
    ...context,
    React: {},
    THREAD_STATE: {},
    buildScopedLogsPath: () => "",
    buildRuntimeContext: () => ({}),
    clearThreadState: () => {},
    html: () => ({}),
    setThreadState: () => {},
    useChat: () => ({}),
    useT: () => () => "",
  });
  const { enrichApprovalGateWithActivityArguments } = context.globalThis.__testExports;
  const gate = {
    kind: "gate",
    runId: "run-1",
    description: "capability requires approval",
    approvalDetails: [],
  };

  const enriched = enrichApprovalGateWithActivityArguments(gate, [
    {
      role: "tool_activity",
      turnRunId: "run-1",
      invocationId: "invocation-search",
      toolParameters: "query: total market cap of companies that IPOed in 2025",
    },
  ]);

  assert.deepEqual(JSON.parse(JSON.stringify(enriched.approvalDetails)), [
    {
      label: "Arguments",
      value: "query: total market cap of companies that IPOed in 2025",
    },
  ]);
  assert.equal(
    enriched.parameters,
    "query: total market cap of companies that IPOed in 2025",
  );
});

test("approval argument enrichment does not duplicate existing argument details", () => {
  const context = { globalThis: {} };
  vm.runInNewContext(chatSourceForTest(), {
    ...context,
    React: {},
    THREAD_STATE: {},
    buildScopedLogsPath: () => "",
    buildRuntimeContext: () => ({}),
    clearThreadState: () => {},
    html: () => ({}),
    setThreadState: () => {},
    useChat: () => ({}),
    useT: () => () => "",
  });
  const { enrichApprovalGateWithActivityArguments } = context.globalThis.__testExports;
  const gate = {
    kind: "gate",
    invocationId: "invocation-search",
    approvalDetails: [{ label: "Parameters", value: "query: existing" }],
  };

  const enriched = enrichApprovalGateWithActivityArguments(gate, [
    {
      role: "tool_activity",
      invocationId: "invocation-search",
      toolParameters: "query: deploy status",
    },
  ]);

  assert.deepEqual(enriched.approvalDetails, [
    { label: "Parameters", value: "query: existing" },
  ]);
  assert.equal(enriched.parameters, "query: deploy status");
});
