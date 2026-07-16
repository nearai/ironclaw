// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

import {
  isTerminalToolStatus,
  toolCardFromActivity,
  toolCardFromPreview,
} from "./history-messages";
import {
  CONNECTION_LOST_RUN_FAILURE_MESSAGE,
  failureMessageForRunStatus,
} from "./failureMessages";
import { CONNECTION_STATUS } from "./connection-status";
import { gateFromProjectionGate } from "./gates";
import {
  createToolActivityState,
  ensureGateToolActivity,
  upsertToolActivityMessage,
} from "./tool-activity-state";
import {
  isFinalAssistantForRun,
  replaceAssistantReplyForRun,
} from "./stream-order-memory";
import {
  createErrorChatMessage,
  isErrorChatMessage,
  isRunFailureMessageId,
  RUN_FAILURE_ID_PREFIX,
  STREAM_FAILURE_ID_PREFIX,
  UNKNOWN_RUN_FAILURE_ID,
} from "./message-types";
import { groupMessages } from "./message-groups";

function useChatEventsSourceForTest() {
  const source = readFileSync(
    new URL("./useChatEvents.ts", import.meta.url),
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
    lines.push(
      line.replace("export function useChatEvents", "function useChatEvents"),
    );
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { useChatEvents };`;
}

function createUseChatEventsHarness({
  DateImpl = Date,
  gateFromEvent = () => null,
  failureMessageForRunStatus = () => "run failed",
  failureMessageForStreamError = ({ error, kind, retryable }) =>
    `stream:${error}:${kind}:${retryable}`,
  locallyResolvedGatesRef = { current: new Map() },
  noteConnectionInterruptedRunId = () => {},
  connectionContextForRunFailure = () => ({}),
  onStreamError = () => {},
} = {}) {
  let messages = [];
  let pendingGate = null;
  let isProcessing = false;
  let activeRun = null;
  const activeRunRef = { current: null };
  const toolActivityStateRef = { current: createToolActivityState() };
  // [{ runId, success }] in fire order; one entry per settled run.
  const settledRuns = [];
  const context = {
    Date: DateImpl,
    createErrorChatMessage,
    React: {
      useCallback: (fn) => fn,
      useEffect: (fn) => fn(),
      useRef: (value) => ({ current: value }),
    },
    failureMessageForRunStatus,
    failureMessageForStreamError,
    gateFromEvent,
    gateFromProjectionGate,
    globalThis: {},
    ensureGateToolActivity,
    isErrorChatMessage,
    isRunFailureMessageId,
    isTerminalToolStatus,
    isFinalAssistantForRun,
    replaceAssistantReplyForRun,
    RUN_FAILURE_ID_PREFIX,
    STREAM_FAILURE_ID_PREFIX,
    toolCardFromActivity,
    toolCardFromPreview,
    UNKNOWN_RUN_FAILURE_ID,
    upsertToolActivityMessage,
  };

  vm.runInNewContext(useChatEventsSourceForTest(), context);

  const handleEvent = context.globalThis.__testExports.useChatEvents({
    threadId: "thread-1",
    setMessages: (updater) => {
      messages = typeof updater === "function" ? updater(messages) : updater;
    },
    setIsProcessing: (updater) => {
      isProcessing =
        typeof updater === "function" ? updater(isProcessing) : updater;
    },
    setPendingGate: (updater) => {
      pendingGate =
        typeof updater === "function" ? updater(pendingGate) : updater;
    },
    setActiveRun: (updater) => {
      activeRun = typeof updater === "function" ? updater(activeRun) : updater;
      activeRunRef.current = activeRun;
    },
    activeRunRef,
    locallyResolvedGatesRef,
    toolActivityStateRef,
    noteConnectionInterruptedRunId,
    connectionContextForRunFailure,
    onStreamError,
    onRunSettled: (runId, { success }) => settledRuns.push({ runId, success }),
  });

  return {
    handleEvent,
    get messages() {
      return messages;
    },
    get pendingGate() {
      return pendingGate;
    },
    get isProcessing() {
      return isProcessing;
    },
    get activeRun() {
      return activeRun;
    },
    setCurrentActiveRun(run) {
      activeRun = run;
      activeRunRef.current = run;
    },
    replaceMessages(next) {
      messages = next;
    },
    get settledRuns() {
      return settledRuns;
    },
    toolActivityStateRef,
  };
}

function plain(value) {
  return JSON.parse(JSON.stringify(value));
}

test("useChatEvents: projection activity preserves reasoning/tool chronology", () => {
  const harness = createUseChatEventsHarness();

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [
          { run_status: { run_id: "run-1", status: "running" } },
          { thinking: { id: "run-1:1", run_id: "run-1", body: "before tool" } },
          {
            capability_activity: {
              invocation_id: "invocation-1",
              turn_run_id: "run-1",
              thread_id: "thread-1",
              capability_id: "builtin.http",
              status: "started",
              provider: null,
              runtime: null,
              process_id: null,
              output_bytes: null,
              error_kind: null,
              updated_at: "2026-06-03T11:44:43Z",
            },
          },
          { thinking: { id: "run-1:2", run_id: "run-1", body: "after tool" } },
        ],
      },
    },
  });

  assert.deepEqual(
    Array.from(harness.messages, (message) => message.id),
    ["thinking-run-1:1", "tool-invocation-1", "thinking-run-1:2"],
  );
  assert.deepEqual(
    Array.from(harness.messages, (message) => message.role),
    ["thinking", "tool_activity", "thinking"],
  );
  assert.equal(harness.messages[1].toolName, "http");
  assert.equal(harness.messages[1].toolStatus, "running");
  assert.deepEqual(
    Array.from(harness.messages, (message) => message.turnRunId),
    ["run-1", "run-1", "run-1"],
  );
});

test("useChatEvents: projection text streams into one assistant bubble without ending the run", () => {
  const harness = createUseChatEventsHarness();

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [
          { run_status: { run_id: "run-1", status: "running" } },
          { text: { id: "text:run-1", run_id: "run-1", body: "partial" } },
        ],
      },
    },
  });

  assert.equal(harness.isProcessing, true);
  assert.deepEqual(plain(harness.activeRun), {
    runId: "run-1",
    threadId: "thread-1",
    status: "running",
  });
  assert.equal(harness.messages.length, 1);
  assert.equal(harness.messages[0].id, "text-text:run-1");
  assert.equal(harness.messages[0].role, "assistant");
  assert.equal(harness.messages[0].content, "partial");
  assert.equal(harness.messages[0].turnRunId, "run-1");
  assert.equal(
    harness.messages[0].isFinalReply,
    false,
    "streamed projection text is still in-flight until terminal reply/timeline finalizes it",
  );

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [{ text: { id: "text:run-1", run_id: "run-1", body: "partial answer" } }],
      },
    },
  });

  assert.equal(harness.isProcessing, true);
  assert.equal(harness.messages.length, 1);
  assert.equal(harness.messages[0].content, "partial answer");
  assert.equal(harness.messages[0].turnRunId, "run-1");
  assert.equal(harness.messages[0].isFinalReply, false);

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [{ run_status: { run_id: "run-1", status: "completed" } }],
      },
    },
  });

  assert.equal(harness.isProcessing, false);
  assert.equal(harness.activeRun, null);
  assert.deepEqual(harness.settledRuns, [{ runId: "run-1", success: true }]);
});

test("useChatEvents: final_reply replaces matching streamed projection bubble", () => {
  const harness = createUseChatEventsHarness();

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [
          { run_status: { run_id: "run-1", status: "running" } },
          { text: { id: "text:run-1", run_id: "run-1", body: "part" } },
        ],
      },
    },
  });

  harness.handleEvent({
    type: "final_reply",
    frame: {
      reply: {
        turn_run_id: "run-1",
        text: "partial answer finalized",
        generated_at: "2026-07-08T13:00:00Z",
      },
    },
  });

  assert.equal(harness.messages.length, 1);
  assert.deepEqual(plain(harness.messages[0]), {
    id: "reply-run-1",
    role: "assistant",
    content: "partial answer finalized",
    timestamp: "2026-07-08T13:00:00Z",
    turnRunId: "run-1",
    isFinalReply: true,
  });
  assert.equal(harness.isProcessing, false);
});

test("useChatEvents: replayed final_reply replaces existing same-run assistant in place", () => {
  const harness = createUseChatEventsHarness();
  harness.replaceMessages([
    {
      id: "msg-user-1",
      role: "user",
      content: "search first",
      turnRunId: "run-1",
    },
    {
      id: "tool-web-search",
      role: "tool_activity",
      toolName: "web_search",
      turnRunId: "run-1",
    },
    {
      id: "msg-assistant-1",
      role: "assistant",
      content: "old final",
      timestamp: "2026-07-08T12:59:00Z",
      isFinalReply: true,
      turnRunId: "run-1",
      keepFollowingActivityAfter: true,
    },
  ]);

  harness.handleEvent({
    type: "final_reply",
    frame: {
      reply: {
        turn_run_id: "run-1",
        text: "new final",
        generated_at: "2026-07-08T13:00:00Z",
      },
    },
  });

  assert.deepEqual(
    Array.from(harness.messages, (message) => message.id),
    ["msg-user-1", "tool-web-search", "msg-assistant-1"],
  );
  assert.equal(harness.messages[2].content, "new final");
  assert.equal(harness.messages[2].timestamp, "2026-07-08T12:59:00Z");
  assert.equal(harness.messages[2].keepFollowingActivityAfter, true);
  assert.equal(harness.isProcessing, false);
});

test("useChatEvents: unscoped activity uses only unambiguous run candidates", () => {
  const runId = "run-google";
  const replyMessage = {
    id: `reply-${runId}`,
    role: "assistant",
    content: "Gmail, Calendar, Drive, and Sheets are connected.",
    timestamp: "2026-07-08T13:00:00Z",
    turnRunId: runId,
    isFinalReply: true,
  };
  const groupedIds = (messages) =>
    groupMessages(messages).map((item) =>
      item.type === "activity-run" ? item.id : item.message.id,
    );
  const finalReplyEvent = {
    type: "final_reply",
    frame: {
      reply: {
        turn_run_id: runId,
        text: replyMessage.content,
        generated_at: replyMessage.timestamp,
      },
    },
  };

  for (const scenario of [
    {
      label: "active run",
      toolId: "tool-invocation-active-extension-search",
      expectedOrder: ["activity-run-tool-invocation-active-extension-search"],
      arrange: (harness) => {
        // Mirrors useChat's POST /messages response before any final_reply.
        harness.setCurrentActiveRun({
          runId,
          threadId: "thread-1",
          status: "running",
          source: "local",
        });
      },
      emit: (harness) =>
        harness.handleEvent({
          type: "capability_activity",
          frame: {
            activity: {
              invocation_id: "invocation-active-extension-search",
              capability_id: "builtin.extension_search",
              status: "completed",
            },
          },
        }),
    },
    {
      label: "final reply",
      toolId: "tool-invocation-final-preview",
      expectedOrder: [
        "activity-run-tool-invocation-final-preview",
        `reply-${runId}`,
        "follow-up-final-reply",
      ],
      arrange: (harness) => {
        // Production path: the run id comes from the submit response, then
        // final_reply clears activeRun before delayed live activity arrives.
        harness.setCurrentActiveRun({
          runId,
          threadId: "thread-1",
          status: "running",
          source: "local",
        });
        harness.handleEvent(finalReplyEvent);
        assert.equal(harness.activeRun, null);
        harness.replaceMessages([
          ...harness.messages,
          {
            id: "follow-up-final-reply",
            role: "user",
            content: "thanks",
            timestamp: "2026-07-08T13:00:20Z",
          },
        ]);
      },
      emit: (harness) =>
        harness.handleEvent({
          type: "capability_display_preview",
          frame: {
            preview: {
              invocation_id: "invocation-final-preview",
              capability_id: "builtin.extension_install",
              status: "completed",
              title: "extension_install",
            },
          },
        }),
    },
    {
      label: "projection batch",
      toolId: "tool-invocation-projection-install",
      expectedOrder: [
        "activity-run-tool-invocation-projection-install",
        `reply-${runId}`,
      ],
      arrange: (harness) => {
        harness.replaceMessages([replyMessage]);
      },
      emit: (harness) =>
        harness.handleEvent({
          type: "projection_update",
          frame: {
            state: {
              items: [
                { run_status: { run_id: runId, status: "completed" } },
                {
                  capability_activity: {
                    invocation_id: "invocation-projection-install",
                    capability_id: "builtin.extension_install",
                    status: "completed",
                  },
                },
              ],
            },
          },
        }),
    },
    {
      label: "mixed terminal and active projection batch",
      toolId: "tool-invocation-projection-active-install",
      expectedOrder: [
        "reply-run-old",
        "follow-up-projection-active",
        "activity-run-tool-invocation-projection-active-install",
      ],
      expectedRunId: "run-new",
      arrange: (harness) => {
        harness.replaceMessages([
          {
            id: "reply-run-old",
            role: "assistant",
            content: "The first run is done.",
            timestamp: "2026-07-08T13:00:00Z",
            turnRunId: "run-old",
            isFinalReply: true,
          },
          {
            id: "follow-up-projection-active",
            role: "user",
            content: "run something else",
            timestamp: "2026-07-08T13:00:20Z",
          },
        ]);
      },
      emit: (harness) =>
        harness.handleEvent({
          type: "projection_update",
          frame: {
            state: {
              items: [
                { run_status: { run_id: "run-old", status: "completed" } },
                { run_status: { run_id: "run-new", status: "running" } },
                {
                  capability_activity: {
                    invocation_id: "invocation-projection-active-install",
                    capability_id: "builtin.extension_install",
                    status: "completed",
                  },
                },
              ],
            },
          },
        }),
    },
  ]) {
    const harness = createUseChatEventsHarness();
    scenario.arrange(harness);
    scenario.emit(harness);

    const toolMessage = harness.messages.find((message) => message.id === scenario.toolId);
    assert.equal(toolMessage?.turnRunId, scenario.expectedRunId || runId, scenario.label);
    assert.deepEqual(groupedIds(harness.messages), scenario.expectedOrder, scenario.label);
  }
});

test("useChatEvents: unscoped activity stays unscoped when run candidates conflict", () => {
  const groupedIds = (messages) =>
    groupMessages(messages).map((item) =>
      item.type === "activity-run" ? item.id : item.message.id,
    );

  {
    const harness = createUseChatEventsHarness();
    harness.setCurrentActiveRun({
      runId: "run-old",
      threadId: "thread-1",
      status: "running",
      source: "local",
    });
    harness.handleEvent({
      type: "final_reply",
      frame: {
        reply: {
          turn_run_id: "run-old",
          text: "The first run is done.",
          generated_at: "2026-07-08T13:00:00Z",
        },
      },
    });
    harness.setCurrentActiveRun({
      runId: "run-new",
      threadId: "thread-1",
      status: "running",
      source: "local",
    });
    harness.replaceMessages([
      ...harness.messages,
      {
        id: "follow-up-conflicting-live",
        role: "user",
        content: "run something else",
        timestamp: "2026-07-08T13:00:20Z",
      },
    ]);
    harness.handleEvent({
      type: "capability_activity",
      frame: {
        activity: {
          invocation_id: "invocation-conflicting-live",
          capability_id: "builtin.extension_search",
          status: "completed",
        },
      },
    });

    const toolMessage = harness.messages.find(
      (message) => message.id === "tool-invocation-conflicting-live",
    );
    assert.equal(toolMessage?.turnRunId, null);
    assert.deepEqual(groupedIds(harness.messages), [
      "reply-run-old",
      "follow-up-conflicting-live",
      "activity-run-tool-invocation-conflicting-live",
    ]);
  }

  {
    const harness = createUseChatEventsHarness();
    harness.replaceMessages([
      {
        id: "reply-run-old",
        role: "assistant",
        content: "The first run is done.",
        timestamp: "2026-07-08T13:00:00Z",
        turnRunId: "run-old",
        isFinalReply: true,
      },
      {
        id: "follow-up-conflicting-projection",
        role: "user",
        content: "run something else",
        timestamp: "2026-07-08T13:00:20Z",
      },
    ]);
    harness.handleEvent({
      type: "projection_update",
      frame: {
        state: {
          items: [
            { run_status: { run_id: "run-old", status: "completed" } },
            { run_status: { run_id: "run-new", status: "completed" } },
            {
              capability_activity: {
                invocation_id: "invocation-conflicting-projection",
                capability_id: "builtin.extension_install",
                status: "completed",
              },
            },
          ],
        },
      },
    });

    const toolMessage = harness.messages.find(
      (message) => message.id === "tool-invocation-conflicting-projection",
    );
    assert.equal(toolMessage?.turnRunId, null);
    assert.deepEqual(groupedIds(harness.messages), [
      "reply-run-old",
      "follow-up-conflicting-projection",
      "activity-run-tool-invocation-conflicting-projection",
    ]);
  }
});

test("useChatEvents: stale projection text does not duplicate finalized same-run reply", () => {
  const harness = createUseChatEventsHarness();
  harness.replaceMessages([
    {
      id: "msg-user-1",
      role: "user",
      content: "search first",
      turnRunId: "run-1",
    },
    {
      id: "tool-web-search",
      role: "tool_activity",
      toolName: "web_search",
      turnRunId: "run-1",
    },
    {
      id: "msg-assistant-1",
      role: "assistant",
      content: "final answer",
      isFinalReply: true,
      turnRunId: "run-1",
    },
  ]);

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [
          { text: { id: "text:run-1", run_id: "run-1", body: "stale final answer" } },
        ],
      },
    },
  });

  assert.equal(harness.messages.length, 3);
  assert.deepEqual(
    Array.from(harness.messages, (message) => message.id),
    ["msg-user-1", "tool-web-search", "msg-assistant-1"],
  );
  assert.equal(harness.messages[2].content, "final answer");
  assert.equal(harness.messages[2].isFinalReply, true);
});

test("useChatEvents: replayed text before activity keeps finalized reply after activity", () => {
  const harness = createUseChatEventsHarness();
  harness.replaceMessages([
    {
      id: "msg-user-1",
      role: "user",
      content: "search first",
      turnRunId: "run-1",
    },
    {
      id: "tool-invocation-1",
      role: "tool_activity",
      toolName: "web_search",
      turnRunId: "run-1",
    },
    {
      id: "msg-assistant-1",
      role: "assistant",
      content: "final answer",
      isFinalReply: true,
      turnRunId: "run-1",
    },
  ]);

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [
          { text: { id: "text:run-1", run_id: "run-1", body: "stale final answer" } },
          {
            capability_activity: {
              invocation_id: "invocation-1",
              turn_run_id: "run-1",
              thread_id: "thread-1",
              capability_id: "builtin.web_search",
              status: "completed",
              updated_at: "2026-07-08T13:00:00Z",
            },
          },
        ],
      },
    },
  });

  assert.deepEqual(
    Array.from(harness.messages, (message) => message.id),
    ["msg-user-1", "tool-invocation-1", "msg-assistant-1"],
  );
  assert.equal(harness.messages[2].content, "final answer");
  assert.equal(harness.messages[2].isFinalReply, true);
  assert.equal(harness.messages[2].keepFollowingActivityAfter, undefined);
});

test("useChatEvents: replayed activity before text keeps finalized reply after activity", () => {
  const harness = createUseChatEventsHarness();
  harness.replaceMessages([
    {
      id: "msg-user-1",
      role: "user",
      content: "search first",
      turnRunId: "run-1",
    },
    {
      id: "tool-invocation-1",
      role: "tool_activity",
      toolName: "web_search",
      turnRunId: "run-1",
    },
    {
      id: "msg-assistant-1",
      role: "assistant",
      content: "final answer",
      isFinalReply: true,
      turnRunId: "run-1",
    },
  ]);

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [
          {
            capability_activity: {
              invocation_id: "invocation-1",
              turn_run_id: "run-1",
              thread_id: "thread-1",
              capability_id: "builtin.web_search",
              status: "completed",
              updated_at: "2026-07-08T13:00:00Z",
            },
          },
          { text: { id: "text:run-1", run_id: "run-1", body: "stale final answer" } },
        ],
      },
    },
  });

  assert.deepEqual(
    Array.from(harness.messages, (message) => message.id),
    ["msg-user-1", "tool-invocation-1", "msg-assistant-1"],
  );
  assert.equal(harness.messages[2].content, "final answer");
  assert.equal(harness.messages[2].isFinalReply, true);
  assert.equal(harness.messages[2].keepFollowingActivityAfter, undefined);
});

test("useChatEvents: text replay before a later activity frame keeps tools before final reply", () => {
  const harness = createUseChatEventsHarness();
  harness.replaceMessages([
    {
      id: "msg-user-1",
      role: "user",
      content: "search first",
      turnRunId: "run-1",
    },
    {
      id: "tool-invocation-1",
      role: "tool_activity",
      toolName: "web_search",
      turnRunId: "run-1",
    },
    {
      id: "msg-assistant-1",
      role: "assistant",
      content: "final answer",
      isFinalReply: true,
      turnRunId: "run-1",
    },
  ]);

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [{ text: { id: "text:run-1", run_id: "run-1", body: "pre-tool text" } }],
      },
    },
  });

  assert.deepEqual(
    Array.from(harness.messages, (message) => message.id),
    ["msg-user-1", "tool-invocation-1", "msg-assistant-1"],
  );

  harness.handleEvent({
    type: "capability_activity",
    frame: {
      activity: {
        invocation_id: "invocation-1",
        turn_run_id: "run-1",
        thread_id: "thread-1",
        capability_id: "builtin.web_search",
        status: "completed",
        updated_at: "2026-07-08T13:00:00Z",
      },
    },
  });

  assert.deepEqual(
    Array.from(harness.messages, (message) => message.id),
    ["msg-user-1", "tool-invocation-1", "msg-assistant-1"],
  );
  assert.equal(harness.messages[2].keepFollowingActivityAfter, undefined);
});

test("useChatEvents: text replay after activity does not move finalized reply on preview", () => {
  const harness = createUseChatEventsHarness();
  harness.replaceMessages([
    {
      id: "msg-user-1",
      role: "user",
      content: "search first",
      turnRunId: "run-1",
    },
    {
      id: "tool-invocation-1",
      role: "tool_activity",
      toolName: "web_search",
      turnRunId: "run-1",
    },
    {
      id: "msg-assistant-1",
      role: "assistant",
      content: "final answer",
      isFinalReply: true,
      turnRunId: "run-1",
    },
  ]);

  harness.handleEvent({
    type: "capability_activity",
    frame: {
      activity: {
        invocation_id: "invocation-1",
        turn_run_id: "run-1",
        thread_id: "thread-1",
        capability_id: "builtin.web_search",
        status: "completed",
        updated_at: "2026-07-08T13:00:00Z",
      },
    },
  });
  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [{ text: { id: "text:run-1", run_id: "run-1", body: "post-tool text" } }],
      },
    },
  });
  harness.handleEvent({
    type: "capability_display_preview",
    frame: {
      preview: {
        invocation_id: "invocation-1",
        turn_run_id: "run-1",
        thread_id: "thread-1",
        capability_id: "builtin.web_search",
        status: "completed",
        title: "builtin.web_search",
        updated_at: "2026-07-08T13:00:01Z",
      },
    },
  });

  assert.deepEqual(
    Array.from(harness.messages, (message) => message.id),
    ["msg-user-1", "tool-invocation-1", "msg-assistant-1"],
  );
  assert.equal(harness.messages[2].keepFollowingActivityAfter, undefined);
});

test("useChatEvents: skill activation projection stays out of chat transcript", () => {
  const harness = createUseChatEventsHarness();

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [
          {
            skill_activation: {
              id: "activation-1",
              skill_names: ["github"],
              feedback: ["github: activated after model selection"],
            },
          },
        ],
      },
    },
  });

  assert.deepEqual(harness.messages, []);
});

test("useChatEvents: auth gate stays visible through progress events", () => {
  const runId = "run-auth-1";
  const authGate = {
    kind: "auth_required",
    challengeKind: "manual_token",
    runId,
    gateRef: "gate:auth",
  };
  const harness = createUseChatEventsHarness({ gateFromEvent: () => authGate });

  harness.handleEvent({
    type: "auth_required",
    frame: {
      prompt: {
        turn_run_id: runId,
        auth_request_ref: "gate:auth",
      },
    },
  });
  assert.deepEqual(harness.pendingGate, authGate);

  harness.handleEvent({
    type: "capability_progress",
    frame: {
      progress: {
        turn_run_id: runId,
        kind: "tool_running",
      },
    },
  });

  assert.deepEqual(harness.pendingGate, authGate);
});

test("useChatEvents: progress clears non-auth gates for the resumed run", () => {
  const runId = "run-approval-1";
  const approvalGate = {
    kind: "gate",
    runId,
    gateRef: "gate:approval",
  };
  const harness = createUseChatEventsHarness({
    gateFromEvent: () => approvalGate,
  });

  harness.handleEvent({
    type: "gate",
    frame: {
      prompt: {
        turn_run_id: runId,
        gate_ref: "gate:approval",
      },
    },
  });
  assert.deepEqual(harness.pendingGate, approvalGate);

  harness.handleEvent({
    type: "running",
    frame: {
      progress: {
        turn_run_id: runId,
        kind: "typing",
      },
    },
  });

  assert.equal(harness.pendingGate, null);
});

test("useChatEvents: final_reply clears the active run", () => {
  const runId = "run-final-reply";
  const harness = createUseChatEventsHarness();

  harness.setCurrentActiveRun({
    runId,
    threadId: "thread-1",
    status: "running",
  });

  harness.handleEvent({
    type: "final_reply",
    frame: {
      reply: {
        turn_run_id: runId,
        text: "Done.",
        generated_at: "2026-06-02T00:00:00Z",
      },
    },
  });

  assert.equal(harness.isProcessing, false);
  assert.equal(harness.pendingGate, null);
  assert.equal(harness.activeRun, null);
  assert.equal(harness.messages.length, 1);
  assert.equal(harness.messages[0].id, `reply-${runId}`);
  assert.equal(harness.messages[0].content, "Done.");
});

test("useChatEvents: observed run ids are passed to the connection observer", () => {
  const notedRunIds = [];
  const harness = createUseChatEventsHarness({
    noteConnectionInterruptedRunId: (runId) => notedRunIds.push(runId),
  });

  harness.handleEvent({
    type: "accepted",
    frame: {
      ack: {
        run_id: "run-accepted-1",
        thread_id: "thread-1",
        status: "queued",
      },
    },
  });
  harness.handleEvent({
    type: "capability_progress",
    frame: {
      progress: {
        turn_run_id: "run-progress-1",
        kind: "tool_running",
      },
    },
  });

  assert.deepEqual(notedRunIds, ["run-accepted-1", "run-progress-1"]);
});

test("useChatEvents: approval gate annotates an existing tool activity", () => {
  const runId = "run-gated-existing";
  const gateRef = "gate:web-access";
  const gate = {
    kind: "gate",
    runId,
    gateRef,
    invocationId: "invocation-web-access",
    toolName: "web-access.search",
  };
  const harness = createUseChatEventsHarness({
    gateFromEvent: () => gate,
  });

  harness.handleEvent({
    type: "capability_activity",
    frame: {
      activity: {
        invocation_id: "invocation-web-access",
        turn_run_id: runId,
        capability_id: "web-access.search",
        status: "started",
      },
    },
  });
  harness.handleEvent({
    type: "gate",
    frame: {
      prompt: {
        turn_run_id: runId,
        gate_ref: gateRef,
      },
    },
  });

  assert.equal(harness.messages.length, 1);
  assert.equal(harness.messages[0].id, "tool-invocation-web-access");
  assert.equal(harness.messages[0].toolName, "search");
  assert.equal(harness.messages[0].toolStatus, "running");
  assert.equal(harness.messages[0].gateRef, gateRef);
  assert.deepEqual(harness.pendingGate, gate);
});

test("useChatEvents: approval gate creates activity from stable invocation id before lifecycle metadata arrives", () => {
  const runId = "run-gated-synthetic";
  const gateRef = "gate:nearai";
  const gate = {
    kind: "gate",
    runId,
    gateRef,
    invocationId: "invocation-nearai",
    toolName: "nearai.web_search",
  };
  const harness = createUseChatEventsHarness({
    gateFromEvent: () => gate,
  });

  harness.handleEvent({
    type: "gate",
    frame: {
      prompt: {
        turn_run_id: runId,
        gate_ref: gateRef,
      },
    },
  });

  assert.equal(harness.messages.length, 1);
  assert.equal(harness.messages[0].id, "tool-invocation-nearai");
  assert.equal(harness.messages[0].toolName, "web_search");
  assert.equal(harness.messages[0].toolStatus, "running");
  assert.equal(harness.messages[0].gateRef, gateRef);

  harness.handleEvent({
    type: "capability_activity",
    frame: {
      activity: {
        invocation_id: "invocation-nearai",
        turn_run_id: runId,
        capability_id: "nearai.web_search",
        status: "started",
      },
    },
  });

  assert.equal(harness.messages.length, 1);
  assert.equal(harness.messages[0].id, "tool-invocation-nearai");
  assert.equal(harness.messages[0].invocationId, "invocation-nearai");
  assert.equal(harness.messages[0].toolName, "web_search");
  assert.equal(harness.messages[0].toolStatus, "running");
  assert.equal(harness.messages[0].gateRef, gateRef);
  assert.equal(harness.messages[0].gateActivity, false);
});

test("useChatEvents: an extension activation preview becomes a tool card (pairing rides the gate rail)", () => {
  const harness = createUseChatEventsHarness();

  harness.handleEvent({
    type: "capability_display_preview",
    frame: {
      preview: {
        invocation_id: "invocation-extension-activate",
        turn_run_id: "run-1",
        thread_id: "thread-1",
        capability_id: "builtin.extension_activate",
        status: "completed",
        title: "extension_activate",
        output_preview: JSON.stringify({
          package_ref: { kind: "extension", id: "telegram" },
          phase: "active",
          message:
            "Telegram is installed as an external channel, but the user's account still needs channel-specific connection or pairing. Tell the user to open the extension's app or bot, get the pairing code or connection challenge, and paste it into the WebChat connection panel rather than normal chat.",
          payload: {
            kind: "extension_activate",
            activated: true,
            visible_capability_ids: [],
          },
        }),
      },
    },
  });

  // The event stream only materializes the activation tool card. A connectable
  // channel that needs pairing now blocks the turn as a standard auth gate
  // (manual_token + connection), so the pairing card is driven by pendingGate —
  // there is no timeline-derived panel for this preview to open.
  assert.equal(harness.messages.length, 1);
  assert.equal(harness.messages[0].toolName, "extension_activate");
});

test("useChatEvents: cleared non-auth gates are not restored by later projections", () => {
  const runId = "run-resource-1";
  const harness = createUseChatEventsHarness();

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [
          { run_status: { run_id: runId, status: "blocked_resource" } },
          {
            gate: {
              run_id: runId,
              gate_kind: "resource",
              gate_ref: "gate:resource",
              headline: "Resource unavailable",
            },
          },
        ],
      },
    },
  });
  assert.deepEqual(plain(harness.pendingGate), {
    kind: "gate",
    gateKind: "resource",
    runId,
    gateRef: "gate:resource",
    invocationId: null,
    headline: "Resource unavailable",
    body: "",
    allowAlways: false,
  });

  harness.handleEvent({
    type: "running",
    frame: {
      progress: {
        turn_run_id: runId,
        kind: "typing",
      },
    },
  });
  assert.equal(harness.pendingGate, null);

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [
          {
            gate: {
              run_id: runId,
              gate_kind: "resource",
              gate_ref: "gate:resource",
              headline: "Resource unavailable",
            },
          },
        ],
      },
    },
  });

  assert.equal(harness.pendingGate, null);
});

test("useChatEvents: projection approval gate preserves always-allow affordance", () => {
  const runId = "run-approval";
  const harness = createUseChatEventsHarness();

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [
          { run_status: { run_id: runId, status: "blocked_approval" } },
          {
            gate: {
              run_id: runId,
              gate_kind: "approval",
              gate_ref: "gate:approval",
              invocation_id: "invocation-approval",
              headline: "Approval required",
              allow_always: true,
            },
          },
        ],
      },
    },
  });

  assert.deepEqual(plain(harness.pendingGate), {
    kind: "gate",
    gateKind: "approval",
    runId,
    gateRef: "gate:approval",
    invocationId: "invocation-approval",
    headline: "Approval required",
    body: "",
    allowAlways: true,
  });
  const activity = harness.messages.find((message) => message.id === "tool-invocation-approval");
  assert.equal(activity?.gateRef, "gate:approval");
  assert.equal(activity?.toolStatus, "running");
  assert.equal(activity?.toolName, "Approval required");
});

test("useChatEvents: projection gate visibility is independent of item order", () => {
  const runId = "run-gate-before-status";
  const harness = createUseChatEventsHarness();
  harness.setCurrentActiveRun({
    runId,
    threadId: "thread-1",
    status: "running",
  });

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [
          {
            gate: {
              run_id: runId,
              gate_kind: "approval",
              gate_ref: "gate:ordered",
              invocation_id: "invocation-ordered",
              headline: "Approve ordered action",
              allow_always: false,
            },
          },
          { run_status: { run_id: runId, status: "blocked_approval" } },
        ],
      },
    },
  });

  assert.deepEqual(plain(harness.pendingGate), {
    kind: "gate",
    gateKind: "approval",
    runId,
    gateRef: "gate:ordered",
    invocationId: "invocation-ordered",
    headline: "Approve ordered action",
    body: "",
    allowAlways: false,
  });
  const activity = harness.messages.find((message) => message.id === "tool-invocation-ordered");
  assert.equal(activity?.gateRef, "gate:ordered");
  assert.equal(activity?.toolStatus, "running");
});

test("useChatEvents: delayed old projection does not restore a previous run gate", () => {
  const currentRunId = "run-current";
  const oldRunId = "run-old";
  const harness = createUseChatEventsHarness();

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [
          { run_status: { run_id: currentRunId, status: "running" } },
        ],
      },
    },
  });
  assert.deepEqual(plain(harness.activeRun), {
    runId: currentRunId,
    threadId: "thread-1",
    status: "running",
  });

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [
          { run_status: { run_id: oldRunId, status: "blocked_approval" } },
          {
            gate: {
              run_id: oldRunId,
              gate_kind: "approval",
              gate_ref: "gate:old",
              invocation_id: "invocation-old",
              headline: "Old approval",
              allow_always: false,
            },
          },
        ],
      },
    },
  });

  assert.equal(harness.pendingGate, null);
  assert.deepEqual(plain(harness.activeRun), {
    runId: currentRunId,
    threadId: "thread-1",
    status: "running",
  });
  assert.equal(
    harness.messages.some((message) => message.id === "tool-invocation-old"),
    false,
  );
});

test("useChatEvents: gate-only projection rebuilds pending gate from gate identity", () => {
  const runId = "run-gate-only";
  const harness = createUseChatEventsHarness();

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [
          {
            gate: {
              run_id: runId,
              gate_kind: "auth",
              gate_ref: "gate:auth-only",
              headline: "Authentication required",
              allow_always: false,
            },
          },
        ],
      },
    },
  });

  assert.deepEqual(plain(harness.pendingGate), {
    kind: "auth_required",
    gateKind: "auth",
    runId,
    gateRef: "gate:auth-only",
    invocationId: null,
    headline: "Authentication required",
    body: "",
    allowAlways: false,
    challengeKind: "other",
    provider: null,
    accountLabel: "",
    authorizationUrl: null,
    expiresAt: null,
    connection: null,
  });
  assert.deepEqual(plain(harness.activeRun), {
    runId,
    threadId: "thread-1",
    status: "awaiting_gate",
  });
  assert.equal(harness.isProcessing, false);
});

test("useChatEvents: failed terminal projection appends visible error", () => {
  const seenFailureInputs = [];
  const harness = createUseChatEventsHarness({
    failureMessageForRunStatus: (input) => {
      seenFailureInputs.push(input);
      return input.failureSummary || "run failed";
    },
  });

  harness.setCurrentActiveRun({
    runId: "run-failed-1",
    threadId: "thread-1",
    status: "running",
  });

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [
          {
            run_status: {
              run_id: "run-failed-1",
              status: "failed",
              failure_category: "driver_invalid_request",
              failure_summary:
                "The run failed because the execution driver rejected the request.",
            },
          },
        ],
      },
    },
  });

  assert.equal(harness.isProcessing, false);
  assert.equal(harness.pendingGate, null);
  assert.equal(harness.activeRun, null);
  assert.deepEqual(plain(seenFailureInputs), [
    {
      status: "failed",
      failureCategory: "driver_invalid_request",
      failureSummary:
        "The run failed because the execution driver rejected the request.",
    },
  ]);
  assert.equal(harness.messages.length, 1);
  assert.equal(harness.messages[0].id, "err-run-failed-1");
  assert.equal(harness.messages[0].role, "error");
  assert.equal(
    harness.messages[0].content,
    "The run failed because the execution driver rejected the request.",
  );
});

test("useChatEvents: interrupted driver_unavailable projection shows connection error", () => {
  const runId = "run-disconnected-driver";
  const contextRunIds = [];
  const harness = createUseChatEventsHarness({
    failureMessageForRunStatus,
    connectionContextForRunFailure: (actualRunId) => {
      contextRunIds.push(actualRunId);
      return {
        connectionStatus: CONNECTION_STATUS.CONNECTED,
        connectionInterrupted: actualRunId === runId,
      };
    },
  });

  harness.setCurrentActiveRun({
    runId,
    threadId: "thread-1",
    status: "running",
  });

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [
          {
            run_status: {
              run_id: runId,
              status: "failed",
              failure_category: "driver_unavailable",
              failure_summary:
                "The run failed because the execution driver was temporarily unavailable.",
            },
          },
        ],
      },
    },
  });

  assert.equal(harness.messages.length, 1);
  assert.equal(harness.messages[0].id, `err-${runId}`);
  assert.equal(harness.messages[0].role, "error");
  assert.equal(harness.messages[0].content, CONNECTION_LOST_RUN_FAILURE_MESSAGE);
  assert.deepEqual(contextRunIds, [runId]);
});

test("useChatEvents: repeated failed projection updates existing error content", () => {
  const harness = createUseChatEventsHarness({
    failureMessageForRunStatus: (input) =>
      input.failureSummary || input.failureCategory || "run failed",
  });

  harness.setCurrentActiveRun({
    runId: "run-failed-update",
    threadId: "thread-1",
    status: "running",
  });

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [
          {
            run_status: {
              run_id: "run-failed-update",
              status: "failed",
              failure_category: "driver_invalid_request",
            },
          },
        ],
      },
    },
  });
  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [
          {
            run_status: {
              run_id: "run-failed-update",
              status: "failed",
              failure_category: "driver_protocol_violation",
            },
          },
        ],
      },
    },
  });

  assert.equal(harness.messages.length, 1);
  assert.equal(harness.messages[0].id, "err-run-failed-update");
  assert.equal(harness.messages[0].content, "driver_protocol_violation");
});

test("useChatEvents: typed failed event appends visible error", () => {
  const seenFailureInputs = [];
  const harness = createUseChatEventsHarness({
    failureMessageForRunStatus: (input) => {
      seenFailureInputs.push(input);
      return `category:${input.failureCategory}`;
    },
  });

  harness.handleEvent({
    type: "failed",
    frame: {
      run_state: {
        run_id: "run-typed-failed-1",
        status: "Failed",
        failure: { category: "model_unavailable" },
      },
    },
  });

  assert.equal(harness.isProcessing, false);
  assert.equal(harness.pendingGate, null);
  assert.equal(harness.activeRun, null);
  assert.equal(harness.messages.length, 1);
  assert.equal(harness.messages[0].id, "err-run-typed-failed-1");
  assert.equal(harness.messages[0].role, "error");
  assert.equal(harness.messages[0].content, "category:model_unavailable");
  assert.deepEqual(plain(seenFailureInputs), [
    {
      status: "Failed",
      failureCategory: "model_unavailable",
      failureSummary: null,
    },
  ]);
});

test("useChatEvents: category-only failure update upgrades existing error", () => {
  const harness = createUseChatEventsHarness({
    failureMessageForRunStatus: ({ failureCategory, failureSummary }) =>
      failureSummary || `category:${failureCategory || "unknown"}`,
  });
  harness.replaceMessages([
    {
      id: "err-run-category-upgrade",
      role: "error",
      content: "category:unknown",
      timestamp: "2026-06-03T11:44:43Z",
    },
  ]);

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [
          {
            run_status: {
              run_id: "run-category-upgrade",
              status: "failed",
              failure_category: "model_unavailable",
            },
          },
        ],
      },
    },
  });

  assert.equal(harness.messages.length, 1);
  assert.equal(harness.messages[0].id, "err-run-category-upgrade");
  assert.equal(harness.messages[0].content, "category:model_unavailable");
});

test("useChatEvents: adjacent duplicate run failures collapse across unknown and known run ids", () => {
  const harness = createUseChatEventsHarness({
    failureMessageForRunStatus: ({ failureCategory }) =>
      failureCategory || "run failed",
  });
  const failureCategory = "model_credentials_invalid";

  harness.handleEvent({
    type: "failed",
    frame: {
      run_state: {
        status: "failed",
        failure: { category: failureCategory },
      },
    },
  });
  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [
          {
            run_status: {
              run_id: "run-known-failure",
              status: "failed",
              failure_category: failureCategory,
            },
          },
        ],
      },
    },
  });

  assert.equal(harness.messages.length, 1);
  assert.equal(harness.messages[0].id, "err-run-known-failure");
  assert.equal(harness.messages[0].content, failureCategory);

  harness.replaceMessages([
    ...harness.messages,
    { id: "pending-next", role: "user", content: "try again" },
  ]);
  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [
          {
            run_status: {
              run_id: "run-next-failure",
              status: "failed",
              failure_category: failureCategory,
            },
          },
        ],
      },
    },
  });

  assert.equal(harness.messages.length, 3);
  assert.equal(harness.messages[2].id, "err-run-next-failure");
  assert.equal(harness.messages[2].content, failureCategory);
});

test("useChatEvents: locally resolved approval gate is not restored by stale projection", () => {
  const runId = "run-denied";
  const gateRef = "gate:approval-denied";
  const harness = createUseChatEventsHarness({
    locallyResolvedGatesRef: {
      current: new Map([[`${runId}\n${gateRef}`, "denied"]]),
    },
  });
  harness.setCurrentActiveRun({
    runId,
    threadId: "thread-1",
    status: "awaiting_gate",
  });

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [
          { run_status: { run_id: runId, status: "blocked_approval" } },
          {
            gate: {
              run_id: runId,
              gate_kind: "approval",
              gate_ref: gateRef,
              headline: "Approval required",
              allow_always: true,
            },
          },
          {
            capability_activity: {
              invocation_id: "invocation-denied",
              turn_run_id: runId,
              capability_id: "builtin.shell",
              status: "running",
            },
          },
        ],
      },
    },
  });

  assert.equal(harness.pendingGate, null);
  assert.equal(harness.isProcessing, false);
  assert.equal(harness.activeRun, null);
  assert.equal(harness.messages.length, 1);
  assert.equal(harness.messages[0].role, "tool_activity");
  assert.equal(harness.messages[0].toolName, "shell");
  assert.equal(harness.messages[0].toolStatus, "running");
});

test("useChatEvents: locally resumed deny allows follow-up activity without restoring gate", () => {
  const runId = "run-denied-resumed";
  const gateRef = "gate:approval-denied";
  const harness = createUseChatEventsHarness({
    locallyResolvedGatesRef: {
      current: new Map([
        [`${runId}\n${gateRef}`, { resolution: "denied", outcome: "resumed" }],
      ]),
    },
  });
  harness.setCurrentActiveRun({
    runId,
    threadId: "thread-1",
    status: "awaiting_gate",
  });

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [
          { run_status: { run_id: runId, status: "blocked_approval" } },
          {
            gate: {
              run_id: runId,
              gate_kind: "approval",
              gate_ref: gateRef,
              headline: "Approval required",
              allow_always: true,
            },
          },
          {
            capability_activity: {
              invocation_id: "invocation-follow-up",
              turn_run_id: runId,
              capability_id: "nearai.web_search",
              status: "running",
            },
          },
        ],
      },
    },
  });

  assert.equal(harness.pendingGate, null);
  assert.equal(harness.isProcessing, true);
  assert.deepEqual(plain(harness.activeRun), {
    runId,
    threadId: "thread-1",
    status: "queued",
  });
  assert.equal(harness.messages.length, 1);
  assert.equal(harness.messages[0].id, "tool-invocation-follow-up");
  assert.equal(harness.messages[0].role, "tool_activity");
  assert.equal(harness.messages[0].toolName, "web_search");
  assert.equal(harness.messages[0].toolStatus, "running");
  assert.equal(harness.messages[0].turnRunId, runId);
});

test("useChatEvents: parent completion after resumed auth cancel clears typing and refetches", () => {
  const parentRunId = "turn-run-after-auth-cancel";
  const authRunId = "auth-run-cancelled";
  const gateRef = "gate:auth-token";
  const harness = createUseChatEventsHarness({
    locallyResolvedGatesRef: {
      current: new Map([
        [
          `${authRunId}\n${gateRef}`,
          { resolution: "cancelled", outcome: "resumed" },
        ],
      ]),
    },
  });
  harness.setCurrentActiveRun({
    runId: authRunId,
    threadId: "thread-1",
    status: "awaiting_gate",
  });

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [
          { run_status: { run_id: authRunId, status: "blocked_auth" } },
        ],
      },
    },
  });
  assert.equal(harness.isProcessing, true);
  assert.deepEqual(plain(harness.activeRun), {
    runId: authRunId,
    threadId: "thread-1",
    status: "queued",
  });

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [
          { run_status: { run_id: parentRunId, status: "completed" } },
        ],
      },
    },
  });

  assert.equal(harness.isProcessing, false);
  assert.equal(harness.pendingGate, null);
  assert.equal(harness.activeRun, null);
  assert.deepEqual(harness.settledRuns, [
    { runId: parentRunId, success: true },
  ]);
});

test("useChatEvents: failed parent terminal after resumed auth cancel clears typing and shows error", () => {
  const parentRunId = "turn-run-after-auth-cancel-failed";
  const authRunId = "auth-run-cancelled-before-failure";
  const gateRef = "gate:auth-token";
  const harness = createUseChatEventsHarness({
    locallyResolvedGatesRef: {
      current: new Map([
        [
          `${authRunId}\n${gateRef}`,
          { resolution: "cancelled", outcome: "resumed" },
        ],
      ]),
    },
    failureMessageForRunStatus: ({ failureSummary }) =>
      failureSummary || "run failed after auth cancel",
  });
  harness.setCurrentActiveRun({
    runId: authRunId,
    threadId: "thread-1",
    status: "awaiting_gate",
  });

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [
          { run_status: { run_id: authRunId, status: "blocked_auth" } },
        ],
      },
    },
  });
  assert.equal(harness.isProcessing, true);

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [
          {
            run_status: {
              run_id: parentRunId,
              status: "failed",
              failure_summary:
                "The run failed after the resolved auth prompt resumed.",
            },
          },
        ],
      },
    },
  });

  assert.equal(harness.isProcessing, false);
  assert.equal(harness.pendingGate, null);
  assert.equal(harness.activeRun, null);
  assert.deepEqual(harness.settledRuns, [
    { runId: parentRunId, success: false },
  ]);
  assert.equal(harness.messages.length, 1);
  assert.equal(harness.messages[0].id, `err-${parentRunId}`);
  assert.equal(
    harness.messages[0].content,
    "The run failed after the resolved auth prompt resumed.",
  );
});

test("useChatEvents: late started activity cannot downgrade remembered declined tool", () => {
  const runId = "run-terminal-tool";
  const invocationId = "invocation-terminal-tool";
  const harness = createUseChatEventsHarness();

  harness.handleEvent({
    type: "capability_activity",
    frame: {
      activity: {
        invocation_id: invocationId,
        turn_run_id: runId,
        capability_id: "nearai.web_search",
        status: "failed",
        error_kind: "gate_declined",
      },
    },
  });
  assert.equal(harness.messages[0].toolStatus, "declined");

  // A full history refresh can temporarily rebuild messages from the
  // transcript, which does not include capability_display_preview records for
  // denied gates. The event handler still must remember terminal activity so a
  // later stale projection replay cannot recreate the same invocation as RUN.
  harness.replaceMessages([]);
  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [
          {
            capability_activity: {
              invocation_id: invocationId,
              turn_run_id: runId,
              capability_id: "nearai.web_search",
              status: "started",
            },
          },
        ],
      },
    },
  });

  assert.equal(harness.messages.length, 1);
  assert.equal(harness.messages[0].id, `tool-${invocationId}`);
  assert.equal(harness.messages[0].toolName, "web_search");
  assert.equal(harness.messages[0].toolStatus, "declined");
  assert.equal(harness.messages[0].toolError, "gate_declined");
  assert.equal(harness.messages[0].toolErrorKind, "gate_declined");
});

test("useChatEvents: projection order annotates replayed terminal activity", () => {
  const runId = "run-replayed-order";
  const harness = createUseChatEventsHarness();

  harness.handleEvent({
    type: "capability_activity",
    frame: {
      activity: {
        invocation_id: "invocation-nearai",
        turn_run_id: runId,
        capability_id: "nearai.web_search",
        status: "failed",
        error_kind: "authorization",
      },
    },
  });

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [
          {
            capability_activity: {
              invocation_id: "invocation-web",
              turn_run_id: runId,
              capability_id: "web-access.search",
              status: "started",
              activity_order: 1,
            },
          },
          {
            capability_activity: {
              invocation_id: "invocation-install",
              turn_run_id: runId,
              capability_id: "builtin.extension_install",
              status: "started",
              activity_order: 2,
            },
          },
          {
            capability_activity: {
              invocation_id: "invocation-nearai",
              turn_run_id: runId,
              capability_id: "nearai.web_search",
              status: "started",
              activity_order: 3,
            },
          },
        ],
      },
    },
  });

  const orderById = new Map(
    harness.messages.map((message) => [message.id, message.activityOrder]),
  );
  assert.equal(orderById.get("tool-invocation-web"), 1);
  assert.equal(orderById.get("tool-invocation-install"), 2);
  assert.equal(orderById.get("tool-invocation-nearai"), 3);
  assert.equal(
    harness.messages.find((message) => message.id === "tool-invocation-nearai")
      .toolStatus,
    "error",
  );
});

test("useChatEvents: durable activity order updates live activity", () => {
  const runId = "run-live-then-durable-order";
  const harness = createUseChatEventsHarness();

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [
          {
            capability_activity: {
              invocation_id: "invocation-web",
              turn_run_id: runId,
              capability_id: "web-access.search",
              status: "started",
            },
          },
        ],
      },
    },
  });

  assert.deepEqual(
    harness.messages.map((message) => [message.id, message.activityOrder]),
    [["tool-invocation-web", null]],
  );

  for (const [invocationId, capabilityId, activityOrder] of [
    ["invocation-extension-a", "builtin.extension_search", 2],
    ["invocation-extension-b", "builtin.extension_search", 3],
    ["invocation-web", "web-access.search", 4],
  ]) {
    harness.handleEvent({
      type: "capability_activity",
      frame: {
        activity: {
          invocation_id: invocationId,
          turn_run_id: runId,
          capability_id: capabilityId,
          status: invocationId === "invocation-web" ? "started" : "completed",
          activity_order: activityOrder,
        },
      },
    });
  }

  assert.deepEqual(
    harness.messages.map((message) => [
      message.id,
      message.toolName,
      message.activityOrder,
    ]),
    [
      ["tool-invocation-web", "search", 4],
      ["tool-invocation-extension-a", "extension_search", 2],
      ["tool-invocation-extension-b", "extension_search", 3],
    ],
  );
});

test("useChatEvents: durable activity order updates gate activity", () => {
  const runId = "run-gate-then-snapshot-order";
  const gateRef = "gate:web-search";
  const harness = createUseChatEventsHarness({
    gateFromEvent: () => ({
      kind: "gate",
      runId,
      gateRef,
      invocationId: "invocation-web-search",
      toolName: "web-access.search",
    }),
  });

  harness.handleEvent({
    type: "gate",
    frame: {
      prompt: {
        turn_run_id: runId,
        approval_request_ref: gateRef,
      },
    },
  });
  assert.deepEqual(
    harness.messages.map((message) => [
      message.id,
      message.toolName,
      message.activityOrder,
      message.activityOrderSource,
    ]),
    [["tool-invocation-web-search", "search", undefined, undefined]],
  );

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [
          {
            capability_activity: {
              invocation_id: "invocation-extension-a",
              turn_run_id: runId,
              capability_id: "builtin.extension_search",
              status: "completed",
              activity_order: 1,
            },
          },
          {
            capability_activity: {
              invocation_id: "invocation-extension-b",
              turn_run_id: runId,
              capability_id: "builtin.extension_search",
              status: "completed",
              activity_order: 2,
            },
          },
          {
            capability_activity: {
              invocation_id: "invocation-web-search",
              turn_run_id: runId,
              capability_id: "web-access.search",
              status: "started",
              activity_order: 3,
            },
          },
        ],
      },
    },
  });

  assert.deepEqual(
    harness.messages.map((message) => [
      message.id,
      message.toolName,
      message.activityOrder,
      message.activityOrderSource,
    ]),
    [
      ["tool-invocation-web-search", "search", 3, "projection"],
      [
        "tool-invocation-extension-a",
        "extension_search",
        1,
        "projection",
      ],
      [
        "tool-invocation-extension-b",
        "extension_search",
        2,
        "projection",
      ],
    ],
  );
});

test("useChatEvents: stale terminal run status does not clear newer run", () => {
  const harness = createUseChatEventsHarness();

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [{ run_status: { run_id: "run-1", status: "running" } }],
      },
    },
  });
  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [
          { run_status: { run_id: "run-2", status: "running" } },
          { run_status: { run_id: "run-1", status: "cancelled" } },
        ],
      },
    },
  });

  assert.equal(harness.isProcessing, true);
  assert.deepEqual(plain(harness.activeRun), {
    runId: "run-2",
    threadId: "thread-1",
    status: "running",
  });
});

test("useChatEvents: stale terminal status before newer projection does not clear newer run", () => {
  const harness = createUseChatEventsHarness();

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [{ run_status: { run_id: "run-1", status: "running" } }],
      },
    },
  });
  harness.setCurrentActiveRun({
    runId: "run-2",
    threadId: "thread-1",
    status: "queued",
    source: "local",
  });
  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [{ run_status: { run_id: "run-1", status: "cancelled" } }],
      },
    },
  });

  assert.equal(harness.isProcessing, true);
  assert.deepEqual(plain(harness.activeRun), {
    runId: "run-2",
    threadId: "thread-1",
    status: "queued",
    source: "local",
  });
});

test("useChatEvents: stale running status before newer projection does not replace newer run", () => {
  const harness = createUseChatEventsHarness();

  harness.setCurrentActiveRun({
    runId: "run-2",
    threadId: "thread-1",
    status: "queued",
    source: "local",
  });
  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [{ run_status: { run_id: "run-1", status: "running" } }],
      },
    },
  });

  assert.deepEqual(plain(harness.activeRun), {
    runId: "run-2",
    threadId: "thread-1",
    status: "queued",
    source: "local",
  });
});

test("useChatEvents: stale failed run status does not append error", () => {
  const harness = createUseChatEventsHarness();

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [{ run_status: { run_id: "run-1", status: "running" } }],
      },
    },
  });
  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [
          { run_status: { run_id: "run-2", status: "running" } },
          { run_status: { run_id: "run-1", status: "failed" } },
        ],
      },
    },
  });

  assert.equal(harness.isProcessing, true);
  assert.deepEqual(harness.messages, []);
  assert.deepEqual(plain(harness.activeRun), {
    runId: "run-2",
    threadId: "thread-1",
    status: "running",
  });
});

test("useChatEvents: stale completed run status does not refetch timeline", () => {
  const harness = createUseChatEventsHarness();

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [{ run_status: { run_id: "run-1", status: "running" } }],
      },
    },
  });
  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [
          { run_status: { run_id: "run-2", status: "running" } },
          { run_status: { run_id: "run-1", status: "completed" } },
        ],
      },
    },
  });

  assert.deepEqual(harness.settledRuns, []);
  assert.equal(harness.isProcessing, true);
  assert.deepEqual(plain(harness.activeRun), {
    runId: "run-2",
    threadId: "thread-1",
    status: "running",
  });
});

test("useChatEvents: terminal success settles the run once", () => {
  const harness = createUseChatEventsHarness();

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [{ run_status: { run_id: "run-1", status: "running" } }],
      },
    },
  });
  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [{ run_status: { run_id: "run-1", status: "completed" } }],
      },
    },
  });
  // Replay of the same terminal projection must not settle twice.
  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [{ run_status: { run_id: "run-1", status: "completed" } }],
      },
    },
  });

  assert.deepEqual(harness.settledRuns, [{ runId: "run-1", success: true }]);
});

test("useChatEvents: terminal failure settles the run as not successful", () => {
  const harness = createUseChatEventsHarness();

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [{ run_status: { run_id: "run-1", status: "running" } }],
      },
    },
  });
  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [{ run_status: { run_id: "run-1", status: "failed" } }],
      },
    },
  });

  // A failed run still settles so the timeline reload recovers tool
  // input/output previews for tools that ran before it terminated.
  assert.deepEqual(harness.settledRuns, [{ runId: "run-1", success: false }]);
});

test("useChatEvents: terminal cancellation settles the run as not successful", () => {
  const harness = createUseChatEventsHarness();

  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [{ run_status: { run_id: "run-1", status: "running" } }],
      },
    },
  });
  harness.handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [{ run_status: { run_id: "run-1", status: "cancelled" } }],
      },
    },
  });

  assert.deepEqual(harness.settledRuns, [{ runId: "run-1", success: false }]);
});

test("useChatEvents: typed failed event settles the run as not successful", () => {
  const harness = createUseChatEventsHarness();

  harness.handleEvent({
    type: "failed",
    frame: {
      run_state: {
        run_id: "run-typed-failed-1",
        status: "Failed",
        failure: { category: "model_unavailable" },
      },
    },
  });

  assert.deepEqual(harness.settledRuns, [
    { runId: "run-typed-failed-1", success: false },
  ]);
});

test("useChatEvents: stream error event appends inline error and clears active state", () => {
  const seenStreamErrors = [];
  const callerStreamErrors = [];
  const harness = createUseChatEventsHarness({
    failureMessageForStreamError: (input) => {
      seenStreamErrors.push(input);
      return "The chat stream failed inline.";
    },
    onStreamError: (input) => callerStreamErrors.push(input),
  });
  harness.setCurrentActiveRun({
    runId: "run-stream-error",
    threadId: "thread-1",
    status: "running",
  });

  harness.handleEvent({
    type: "error",
    frame: {
      error: "unavailable",
      kind: "service_unavailable",
      retryable: true,
    },
  });
  harness.handleEvent({
    type: "error",
    frame: {
      error: "unavailable",
      kind: "service_unavailable",
      retryable: true,
    },
  });

  assert.equal(harness.isProcessing, false);
  assert.equal(harness.pendingGate, null);
  assert.equal(harness.activeRun, null);
  assert.equal(harness.messages.length, 1);
  assert.equal(harness.messages[0].role, "error");
  assert.equal(harness.messages[0].content, "The chat stream failed inline.");
  assert.deepEqual(plain(seenStreamErrors), [
    {
      error: "unavailable",
      kind: "service_unavailable",
      retryable: true,
    },
  ]);
  assert.deepEqual(plain(callerStreamErrors), [
    {
      error: "unavailable",
      kind: "service_unavailable",
      retryable: true,
    },
    {
      error: "unavailable",
      kind: "service_unavailable",
      retryable: true,
    },
  ]);
});

test("useChatEvents: stream error dedupe only suppresses adjacent repeats", () => {
  const seenStreamErrors = [];
  const harness = createUseChatEventsHarness({
    failureMessageForStreamError: (input) => {
      seenStreamErrors.push(input);
      return `stream:${input.kind}`;
    },
  });
  const streamErrorFrame = {
    error: "unavailable",
    kind: "service_unavailable",
    retryable: true,
  };

  harness.handleEvent({ type: "error", frame: streamErrorFrame });
  harness.handleEvent({ type: "error", frame: streamErrorFrame });

  assert.equal(harness.messages.length, 1);
  const firstError = harness.messages[0];
  assert.equal(firstError.role, "error");
  assert.match(
    firstError.id,
    /^err-stream-unavailable-service_unavailable-retryable-/,
  );

  harness.replaceMessages([
    ...harness.messages,
    { id: "assistant-between-errors", role: "assistant", content: "between" },
  ]);
  harness.handleEvent({ type: "error", frame: streamErrorFrame });
  harness.handleEvent({ type: "error", frame: streamErrorFrame });

  assert.equal(harness.messages.length, 3);
  const secondError = harness.messages[2];
  assert.equal(secondError.role, "error");
  assert.match(
    secondError.id,
    /^err-stream-unavailable-service_unavailable-retryable-/,
  );
  assert.notEqual(secondError.id, firstError.id);
  assert.equal(secondError.content, "stream:service_unavailable");
  assert.equal(seenStreamErrors.length, 2);
});

test("useChatEvents: stream error ids avoid timestamp collisions", () => {
  class FixedDate extends Date {
    constructor(...args) {
      super(args.length > 0 ? args[0] : 1_788_259_200_000);
    }

    static now() {
      return 1_788_259_200_000;
    }
  }

  const harness = createUseChatEventsHarness({ DateImpl: FixedDate });
  const baseId = `${STREAM_FAILURE_ID_PREFIX}unavailable-service_unavailable-retryable`;
  harness.replaceMessages([
    { id: `${baseId}-1788259200000`, role: "error", content: "first" },
    { id: "assistant-between-errors", role: "assistant", content: "between" },
  ]);

  harness.handleEvent({
    type: "error",
    frame: {
      error: "unavailable",
      kind: "service_unavailable",
      retryable: true,
    },
  });

  assert.equal(harness.messages.length, 3);
  assert.equal(harness.messages[2].id, `${baseId}-1788259200000-1`);
});
