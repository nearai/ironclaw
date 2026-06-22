import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function chatPageSourceForTest() {
  const source = readFileSync(new URL("./chat-page.js", import.meta.url), "utf8");
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
    lines.push(line.replace("export function ChatPage", "function ChatPage"));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { ChatPage };`;
}

function html(strings, ...values) {
  return { strings: Array.from(strings), values };
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

function plain(value) {
  return JSON.parse(JSON.stringify(value));
}

function renderChatPage({
  activeThreadId = null,
  isFetching = false,
  threads = [{ id: "thread-1", title: "Existing chat" }],
  isLoaded = true,
  urlThreadId = null,
} = {}) {
  const navigateCalls = [];
  const setActiveThreadIdCalls = [];
  const threadsState = {
    activeThreadId,
    createThread: async () => null,
    deleteThread: async () => {},
    isCreating: false,
    isFetching,
    isLoaded,
    setActiveThreadId: (id) => {
      threadsState.activeThreadId = id;
      setActiveThreadIdCalls.push(id);
    },
    threads,
  };
  const context = {
    Chat() {},
    React: {
      useCallback: (fn) => fn,
      useEffect: (fn) => fn(),
    },
    globalThis: {},
    html,
    useLocation: () => ({ key: "location-key", state: null }),
    useNavigate: () => (path, options) => {
      navigateCalls.push([path, options]);
    },
    useOutletContext: () => ({
      gatewayStatus: { ok: true },
      threadsState,
    }),
    useParams: () => (urlThreadId ? { threadId: urlThreadId } : {}),
  };

  vm.runInNewContext(chatPageSourceForTest(), context);
  const tree = context.globalThis.__testExports.ChatPage();
  return {
    chatProps: componentProps(tree, context.Chat),
    navigateCalls,
    setActiveThreadIdCalls,
  };
}

test("ChatPage redirects the reserved newchat route to blank chat", () => {
  const result = renderChatPage({
    activeThreadId: "thread-1",
    urlThreadId: "newchat",
  });

  assert.deepEqual(plain(result.navigateCalls), [["/chat", { replace: true }]]);
  assert.deepEqual(result.setActiveThreadIdCalls, [null]);
  assert.equal(result.chatProps.activeThreadId, null);
});

test("ChatPage redirects a loaded but missing thread route to blank chat", () => {
  const result = renderChatPage({
    activeThreadId: "thread-1",
    threads: [{ id: "thread-1", title: "Existing chat" }],
    urlThreadId: "missing-thread",
  });

  assert.deepEqual(plain(result.navigateCalls), [["/chat", { replace: true }]]);
  assert.deepEqual(result.setActiveThreadIdCalls, [null]);
  assert.equal(result.chatProps.activeThreadId, null);
});

test("ChatPage redirects a stale active thread when the loaded thread list is settled", () => {
  const result = renderChatPage({
    activeThreadId: "missing-thread",
    threads: [{ id: "thread-1", title: "Existing chat" }],
    urlThreadId: "missing-thread",
  });

  assert.deepEqual(plain(result.navigateCalls), [["/chat", { replace: true }]]);
  assert.deepEqual(result.setActiveThreadIdCalls, [null]);
  assert.equal(result.chatProps.activeThreadId, null);
});

test("ChatPage keeps a valid loaded thread route active", () => {
  const result = renderChatPage({
    activeThreadId: null,
    threads: [{ id: "thread-1", title: "Existing chat" }],
    urlThreadId: "thread-1",
  });

  assert.deepEqual(result.navigateCalls, []);
  assert.deepEqual(result.setActiveThreadIdCalls, ["thread-1"]);
  assert.equal(result.chatProps.activeThreadId, "thread-1");
});

test("ChatPage keeps a locally selected route active while the thread list catches up", () => {
  const result = renderChatPage({
    activeThreadId: "thread-new",
    isFetching: true,
    threads: [{ id: "thread-1", title: "Existing chat" }],
    urlThreadId: "thread-new",
  });

  assert.deepEqual(result.navigateCalls, []);
  assert.deepEqual(result.setActiveThreadIdCalls, []);
  assert.equal(result.chatProps.activeThreadId, "thread-new");
});

test("ChatPage waits for the thread list before redirecting a deep link", () => {
  const result = renderChatPage({
    activeThreadId: null,
    isLoaded: false,
    threads: [],
    urlThreadId: "thread-1",
  });

  assert.deepEqual(result.navigateCalls, []);
  assert.deepEqual(result.setActiveThreadIdCalls, []);
  assert.equal(result.chatProps.activeThreadId, null);
});

test("ChatPage keeps bare chat route in blank chat state", () => {
  const result = renderChatPage({
    activeThreadId: "thread-1",
    urlThreadId: null,
  });

  assert.deepEqual(result.navigateCalls, []);
  assert.deepEqual(result.setActiveThreadIdCalls, [null]);
  assert.equal(result.chatProps.activeThreadId, null);
});
