import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function messageElement(role, content = "") {
  return {
    role,
    content,
    classList: {
      contains: (name) => name === role || name === "message",
      add: () => {},
      remove: () => {},
    },
    getAttribute: () => "",
    setAttribute: () => {},
    removeAttribute: () => {},
  };
}

function createChatContainer(messages) {
  return {
    querySelectorAll(selector) {
      if (selector === ".message.assistant, .message.system") {
        return messages.filter((message) =>
          message.role === "assistant" || message.role === "system"
        );
      }
      if (selector === ".message" || selector === "#chat-messages .message") {
        return messages;
      }
      return [];
    },
    querySelector: () => null,
  };
}

function createHarness() {
  const messages = [messageElement("user", "hello")];
  let source = null;
  const timers = [];
  const calls = [];

  class FakeEventSource {
    constructor(url) {
      this.url = url;
      this.listeners = {};
      source = this;
    }

    addEventListener(type, listener) {
      this.listeners[type] = listener;
    }

    close() {}

    emit(type, data) {
      this.listeners[type]({
        data: JSON.stringify(data),
        lastEventId: `${type}-1`,
      });
    }
  }

  const context = {
    ActivityEntry: { t: (_key, fallback) => fallback },
    DONE_WITHOUT_RESPONSE_TIMEOUT_MS: 1500,
    EventSource: FakeEventSource,
    I18n: {
      t: (key) =>
        key === "chat.runFinishedWithoutReply"
          ? "The run finished without producing a reply. Try again, or check logs if this keeps happening."
          : key,
    },
    IronClaw: { api: { _dispatch: () => {} } },
    JSON,
    Promise,
    activeWorkStore: {
      clearThread: () => calls.push("clearThread"),
      isThreadBlocked: () => false,
      updateThread: () => {},
      updateJob: () => {},
    },
    addMessage: (role, content) => {
      messages.push(messageElement(role, content));
    },
    clearInterval: () => {},
    clearTimeout: () => {},
    cleanupConnectionState: () => {},
    console,
    currentThreadId: "thread-1",
    debouncedLoadThreads: () => calls.push("debouncedLoadThreads"),
    document: {
      getElementById: (id) =>
        id === "chat-messages" ? createChatContainer(messages) : null,
      querySelector: () => null,
      querySelectorAll: () => [],
    },
    enableChatInput: () => calls.push("enableChatInput"),
    encodeURIComponent,
    eventSource: null,
    finalizeActivityGroup: () => calls.push("finalizeActivityGroup"),
    isCurrentThread: (threadId) => threadId === "thread-1",
    loadHistory: () => {
      calls.push("loadHistory");
      return Promise.resolve();
    },
    oidcProxyAuth: false,
    processingThreads: new Set(),
    setInterval: () => 0,
    setTimeout: (fn, _ms) => {
      timers.push(fn);
      return timers.length;
    },
    token: null,
    window: {},
    _doneWithoutResponseTimer: null,
    _lastSseEventId: null,
    _turnResponseReceived: false,
  };

  vm.runInNewContext(
    readFileSync(new URL("./sse.js", import.meta.url), "utf8"),
    context,
  );

  context.connectSSE();

  return {
    calls,
    context,
    messages,
    source: () => source,
    async runTimers() {
      for (const timer of timers.splice(0)) timer();
      await Promise.resolve();
      await Promise.resolve();
    },
  };
}

test("connectSSE appends fallback when Done has no response and history stays silent", async () => {
  const harness = createHarness();

  harness.source().emit("status", {
    thread_id: "thread-1",
    message: "Done",
  });
  await harness.runTimers();

  assert.deepEqual(harness.calls.filter((call) => call === "loadHistory"), [
    "loadHistory",
  ]);
  assert.equal(harness.messages.length, 2);
  assert.equal(harness.messages[1].role, "system");
  assert.equal(
    harness.messages[1].content,
    "The run finished without producing a reply. Try again, or check logs if this keeps happening.",
  );
});
