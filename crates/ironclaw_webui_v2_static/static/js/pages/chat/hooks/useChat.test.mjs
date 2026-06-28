import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
// The harness strips useChat.js's imports, so its collaborators are injected
// as context globals below. retry eligibility is one of them — inject the real
// predicate so the test exercises the same guard the production code uses.
import { isRetryableMessage } from "../lib/retry-eligibility.js";

// Load useChat.js into a fresh VM context with its imports stripped, the same
// harness pattern useThreads.test.mjs uses. The hook's many collaborators
// (React, the api.js requests, the query client, useHistory/useSSE/
// useChatEvents, the pending-message + attachment helpers) are injected as
// context globals so the test can drive `retryMessage` directly — the
// test-through-the-caller pattern: retryMessage delegates to send(), so the
// regression is observable as a real sendMessage() call, not a unit assertion
// on a helper in isolation.
function useChatSourceForTest() {
  const source = readFileSync(new URL("./useChat.js", import.meta.url), "utf8");
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
    lines.push(line.replace(/^export function /, "function "));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { useChat };`;
}

// Minimal synchronous React stub: effects run inline, refs are real mutable
// boxes (collected so a test can flip a busy flag), state setters apply
// functional updates against a per-hook-call closure value. The hook is only
// invoked once per test, so this single-render model is sufficient.
function createReactStub(refs) {
  return {
    useCallback: (fn) => fn,
    useMemo: (fn) => fn(),
    useEffect: (fn) => {
      fn();
    },
    useRef: (value) => {
      const ref = { current: value };
      refs.push(ref);
      return ref;
    },
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

function instantiate({
  threadId = "thread-1",
  initialMessages = [],
  // Lets a test simulate the network layer: e.g. a throwing send (network
  // failure) so the catch path in retryMessage is exercised.
  sendImpl = (arg) => ({
    run_id: "run-retry-1",
    thread_id: arg.threadId,
    status: "queued",
  }),
} = {}) {
  const sendCalls = [];
  const loadHistoryCalls = [];
  let chatEventsConfig = null;
  let messagesState = [...initialMessages];
  const refs = [];

  const setMessages = (updater) => {
    messagesState =
      typeof updater === "function" ? updater(messagesState) : updater;
  };

  const context = {
    console,
    React: createReactStub(refs),
    // api.js requests
    sendMessage: async (arg) => {
      sendCalls.push(arg);
      return sendImpl(arg);
    },
    cancelRunRequest: async () => {},
    createThreadRequest: async () => ({ thread: { thread_id: threadId } }),
    resolveGateRequest: async () => ({}),
    submitManualToken: async () => ({}),
    // channel-connect helpers — never matched in these tests
    listConnectableChannels: async () => ({ channels: [] }),
    looksLikeChannelConnectCommand: () => false,
    resolveChannelConnectCommand: () => null,
    // query client
    queryClient: {
      getQueryData: () => undefined,
      invalidateQueries: () => {},
      fetchQuery: async () => ({ channels: [] }),
    },
    // hooks
    useHistory: () => ({
      messages: messagesState,
      hasMore: false,
      nextCursor: null,
      isLoading: false,
      loadError: null,
      loadHistory: (cursor, opts) => {
        loadHistoryCalls.push({ cursor, opts });
      },
      seedThreadMessages: () => {},
      setMessages,
    }),
    useSSE: () => ({ status: "open" }),
    // Capture the config passed to useChatEvents so a test can invoke the
    // onRunSettled callback that fires the post-run history reload.
    useChatEvents: (cfg) => {
      chatEventsConfig = cfg;
      return () => {};
    },
    // pending-message helpers
    addPending: () => {},
    recordAcceptedMessageRef: () => null,
    removePending: () => {},
    timelineMessageIdFromAcceptedRef: () => null,
    // tool-activity helpers
    createToolActivityState: () => ({}),
    failGateToolActivity: () => {},
    resetToolActivityState: () => {},
    // attachment helpers
    toRenderAttachment: (a) => a,
    toWireAttachment: (a) => a,
    // retry eligibility predicate (shared with message-bubble's render guard)
    isRetryableMessage,
    globalThis: {},
    window: {},
  };

  vm.runInNewContext(useChatSourceForTest(), context);
  const hook = context.globalThis.__testExports.useChat(threadId);
  return {
    hook,
    sendCalls,
    refs,
    loadHistoryCalls,
    getChatEventsConfig: () => chatEventsConfig,
    getMessages: () => messagesState,
  };
}

function erroredUserMessage(overrides = {}) {
  return {
    id: "pending-7",
    role: "user",
    content: "hello world",
    status: "error",
    error: "Network error",
    ...overrides,
  };
}

test("retryMessage re-sends a failed user message via send()", async () => {
  const failed = erroredUserMessage();
  const { hook, sendCalls } = instantiate({ initialMessages: [failed] });

  await hook.retryMessage(failed);

  // The regression: on the unpatched base retryMessage is a truthy no-op, so
  // the button renders but sendMessage() is never invoked. A real
  // implementation must re-dispatch through send(), which calls sendMessage.
  assert.equal(sendCalls.length, 1, "retry must re-send through send()/sendMessage");
  assert.equal(sendCalls[0].content, "hello world");
  assert.equal(sendCalls[0].threadId, "thread-1");
});

test("retryMessage drops the failed bubble once the re-send is accepted", async () => {
  const failed = erroredUserMessage();
  const { hook, getMessages } = instantiate({ initialMessages: [failed] });

  await hook.retryMessage(failed);

  const remaining = getMessages();
  assert.ok(
    !remaining.some((m) => m.id === failed.id),
    "the old failed bubble is removed after a successful re-send",
  );
});

test("retryMessage keeps the failed bubble when the thread is busy (send returns null)", async () => {
  const failed = erroredUserMessage();
  const { hook, sendCalls, refs, getMessages } = instantiate({
    initialMessages: [failed],
  });

  // Flip the busy/processing flags so send() short-circuits to null before
  // it ever reaches sendMessage(). These are the only refs initialised to a
  // literal `false` (isProcessingRef + submitBusyRef).
  refs.filter((r) => r.current === false).forEach((r) => (r.current = true));

  await hook.retryMessage(failed);

  assert.equal(sendCalls.length, 0, "a blocked retry must not dispatch a send");
  assert.ok(
    getMessages().some((m) => m.id === failed.id),
    "a blocked retry must keep the failed bubble so the message never vanishes",
  );
});

test("retryMessage ignores non-error, non-user, and non-string-content messages", async () => {
  const { hook, sendCalls } = instantiate();

  await hook.retryMessage({ id: "a1", role: "assistant", status: "error", content: "hi" });
  await hook.retryMessage({ id: "u1", role: "user", status: undefined, content: "hi" });
  await hook.retryMessage({ id: "u2", role: "user", status: "error", content: "   " });
  // Non-string content must short-circuit before `.trim()` (which would throw
  // a TypeError that the catch block would silently swallow).
  await hook.retryMessage({ id: "u3", role: "user", status: "error", content: null });
  await hook.retryMessage({ id: "u4", role: "user", status: "error", content: 42 });
  await hook.retryMessage(null);

  assert.equal(sendCalls.length, 0, "only non-empty failed user messages retry");
});

test("retryMessage ignores an attachment-bearing failed message", async () => {
  // Retry is content-only: a failed bubble's attachments are render-shape and
  // the File blobs are gone, so attachment-bearing failures are not retryable
  // (the Retry button is hidden for them). Prove the hook itself short-circuits
  // — no send(), no bubble removal — not just the shared predicate in isolation.
  const failed = erroredUserMessage({
    id: "msg-withfiles",
    attachments: [{ id: "att-1", name: "photo.png" }],
  });
  const { hook, sendCalls, getMessages } = instantiate({ initialMessages: [failed] });

  await hook.retryMessage(failed);

  assert.equal(sendCalls.length, 0, "an attachment-bearing message must not re-send");
  assert.ok(
    getMessages().some((m) => m.id === failed.id),
    "the failed bubble is retained (retry is suppressed, not silently dropped)",
  );
});

test("retryMessage keeps the failed bubble when send() throws", async () => {
  const failed = erroredUserMessage();
  const { hook, sendCalls, getMessages } = instantiate({
    initialMessages: [failed],
    // Simulate a network/API failure on the retry's POST. send() marks its
    // own fresh optimistic bubble as error and re-throws; retryMessage must
    // swallow the throw and leave the original failed bubble in place.
    sendImpl: () => {
      throw new Error("network down");
    },
  });

  await assert.doesNotReject(() => hook.retryMessage(failed));

  assert.equal(sendCalls.length, 1, "the retry attempted a send");
  assert.ok(
    getMessages().some((m) => m.id === failed.id),
    "the original failed bubble is retained when the re-send throws",
  );
});

test("retryMessage returns the send() response so callers can route to a new thread", async () => {
  // chat.js handleRetry routes a landing-screen retry to the thread send()
  // creates implicitly; that only works if retryMessage hands the response
  // (carrying thread_id) back to the caller.
  const failed = erroredUserMessage();
  const { hook } = instantiate({
    initialMessages: [failed],
    sendImpl: () => ({ run_id: "run-x", thread_id: "thread-new", status: "queued" }),
  });

  const response = await hook.retryMessage(failed);

  assert.ok(response, "retryMessage must return the send() response");
  assert.equal(response.thread_id, "thread-new");
});

test("retryMessage suppresses a retried msg-* timeline row on the post-run reload", async () => {
  // A persisted rejected_busy row rehydrates as { id: "msg-*", status: "error" }.
  // After a successful retry, onRunSettled reloads history; without suppression
  // the server re-emits that row and the old failed bubble reappears as a
  // duplicate next to the retried message.
  const failed = erroredUserMessage({ id: "msg-abc123" });
  const { hook, getChatEventsConfig, loadHistoryCalls } = instantiate({
    initialMessages: [failed],
  });

  await hook.retryMessage(failed);
  // Fire the run-settled callback the way useChatEvents would on a terminal run.
  getChatEventsConfig().onRunSettled("run-retry-1", { success: true });

  const reload = loadHistoryCalls.at(-1);
  assert.ok(reload, "onRunSettled must trigger a history reload");
  // The Set is constructed inside the vm realm, so duck-type rather than
  // `instanceof Set` (which compares against this realm's constructor).
  assert.equal(
    typeof reload.opts.suppressIds?.has,
    "function",
    "reload carries a suppressIds set",
  );
  assert.ok(
    reload.opts.suppressIds.has("msg-abc123"),
    "the retried timeline row id is suppressed across the reload",
  );
});

test("the suppressIds passed to loadHistory is a snapshot, immune to a mid-refresh thread reset", async () => {
  // loadHistory is async; the per-thread reset clears retriedTimelineIdsRef on
  // a threadId change. If onRunSettled passed the LIVE ref, a thread switch
  // before the refetch resolved would empty the set and resurrect the row.
  // Passing `new Set(...)` snapshots it, so clearing the live set afterward
  // (what the reset does) must NOT affect the captured value.
  const failed = erroredUserMessage({ id: "msg-snap1" });
  const { hook, getChatEventsConfig, loadHistoryCalls, refs } = instantiate({
    initialMessages: [failed],
  });

  await hook.retryMessage(failed);
  getChatEventsConfig().onRunSettled("run-retry-1", { success: true });

  // Simulate the per-thread reset clearing every Set-typed ref (the live
  // retriedTimelineIdsRef). The captured snapshot lives in loadHistoryCalls,
  // not in refs, so this must not touch it.
  refs
    .filter((r) => r.current && typeof r.current.clear === "function" && typeof r.current.has === "function")
    .forEach((r) => r.current.clear());

  const reload = loadHistoryCalls.at(-1);
  assert.ok(
    reload.opts.suppressIds.has("msg-snap1"),
    "captured suppressIds must still hold the retried id after the live set is cleared",
  );
});

test("retryMessage does not suppress client-only pending-* bubbles", async () => {
  // pending-* bubbles are never re-emitted by loadHistory, so they must not be
  // added to the suppression set (which would be dead weight at best).
  const failed = erroredUserMessage({ id: "pending-7" });
  const { hook, getChatEventsConfig, loadHistoryCalls } = instantiate({
    initialMessages: [failed],
  });

  await hook.retryMessage(failed);
  getChatEventsConfig().onRunSettled("run-retry-1", { success: true });

  const reload = loadHistoryCalls.at(-1);
  assert.ok(
    !reload.opts.suppressIds.has("pending-7"),
    "client-only pending-* ids are not added to suppression",
  );
});
