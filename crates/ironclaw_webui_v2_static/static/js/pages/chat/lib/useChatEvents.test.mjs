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

test("useChatEvents: projection activity preserves reasoning/tool chronology", () => {
  const threadId = "thread-1";
  let messages = [];

  const context = {
    Date,
    React: {
      useCallback: (fn) => fn,
      useRef: (value) => ({ current: value }),
    },
    failureMessageForRunStatus: () => "run failed",
    gateFromEvent: () => null,
    globalThis: {},
    isTerminalToolStatus,
    toolCardFromActivity,
    toolCardFromPreview,
  };

  vm.runInNewContext(useChatEventsSourceForTest(), context);

  const handleEvent = context.globalThis.__testExports.useChatEvents({
    threadId,
    setMessages: (updater) => {
      messages = typeof updater === "function" ? updater(messages) : updater;
    },
    setIsProcessing: () => {},
    setPendingGate: () => {},
    setActiveRun: () => {},
    onRunCompleted: () => {},
  });

  handleEvent({
    type: "projection_update",
    frame: {
      state: {
        items: [
          { thinking: { id: "run-1:1", body: "before tool" } },
          {
            capability_activity: {
              invocation_id: "invocation-1",
              thread_id: threadId,
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
          { thinking: { id: "run-1:2", body: "after tool" } },
        ],
      },
    },
  });

  assert.deepEqual(
    Array.from(messages, (message) => message.id),
    ["thinking-run-1:1", "tool-invocation-1", "thinking-run-1:2"],
  );
  assert.deepEqual(
    Array.from(messages, (message) => message.role),
    ["thinking", "tool_activity", "thinking"],
  );
  assert.equal(messages[1].toolName, "builtin.http");
  assert.equal(messages[1].toolStatus, "running");
});
