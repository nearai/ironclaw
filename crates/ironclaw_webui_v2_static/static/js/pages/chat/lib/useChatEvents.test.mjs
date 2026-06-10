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

function createUseChatEventsHarness({ gateFromEvent = () => null } = {}) {
  let messages = [];
  let pendingGate = null;
  let isProcessing = false;
  let activeRun = null;
  const activeRunRef = { current: null };
  const completedRuns = [];
  const context = {
    Date,
    React: {
      useCallback: (fn) => fn,
      useRef: (value) => ({ current: value }),
    },
    failureMessageForRunStatus: () => "run failed",
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
    onRunCompleted: (runId) => completedRuns.push(runId),
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
    get completedRuns() {
      return completedRuns;
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

  assert.deepEqual(harness.completedRuns, []);
  assert.equal(harness.isProcessing, true);
  assert.deepEqual(plain(harness.activeRun), {
    runId: "run-2",
    threadId: "thread-1",
    status: "running",
  });
});
