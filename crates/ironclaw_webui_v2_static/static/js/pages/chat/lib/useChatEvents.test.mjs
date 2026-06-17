import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

import {
  isTerminalToolStatus,
  toolCardFromActivity,
  toolCardFromPreview,
} from "./history-messages.js";

function useChatEventsSourceForTest() {
  const source = readFileSync(
    new URL("./useChatEvents.js", import.meta.url),
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
  gateFromEvent = () => null,
  failureMessageForRunStatus = () => "run failed",
} = {}) {
  let messages = [];
  let pendingGate = null;
  let isProcessing = false;
  let activeRun = null;
  const activeRunRef = { current: null };
  // [{ runId, success }] in fire order; one entry per settled run.
  const settledRuns = [];
  const context = {
    Date,
    React: {
      useCallback: (fn) => fn,
      useRef: (value) => ({ current: value }),
    },
    failureMessageForRunStatus,
    gateFromEvent,
    globalThis: {},
    isTerminalToolStatus,
    toolCardFromActivity,
    toolCardFromPreview,
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
    get settledRuns() {
      return settledRuns;
    },
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
  assert.equal(harness.messages[1].toolName, "builtin.http");
  assert.equal(harness.messages[1].toolStatus, "running");
  assert.deepEqual(
    Array.from(harness.messages, (message) => message.turnRunId),
    ["run-1", "run-1", "run-1"],
  );
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
    runId,
    gateRef: "gate:resource",
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
              gate_ref: "gate:approval",
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
    runId,
    gateRef: "gate:approval",
    headline: "Approval required",
    body: "",
    allowAlways: true,
  });
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
