// Unit tests for transient tool activity merge behavior.
//
// Run with Node's built-in test runner (no extra deps):
//   node --test crates/ironclaw_webui_v2_static/static/js/pages/chat/lib/tool-activity-state.test.js
//
// NOTE: `build.rs` deliberately excludes `*.test.js` from the embedded
// asset bundle, so this file is never served to the browser.

import assert from "node:assert/strict";
import { test } from "node:test";
import {
  createToolActivityState,
  ensureGateToolActivity,
  failGateToolActivity,
  upsertToolActivityMessage,
} from "./tool-activity-state.js";

function messageHarness() {
  let messages = [];
  return {
    stateRef: { current: createToolActivityState() },
    get messages() {
      return messages;
    },
    setMessages(updater) {
      messages = typeof updater === "function" ? updater(messages) : updater;
    },
  };
}

function runtimeActivity(overrides = {}) {
  return {
    invocationId: "invocation-1",
    callId: "invocation-1",
    capabilityId: "web-access.search",
    toolName: "search",
    toolStatus: "running",
    toolDetail: null,
    toolParameters: null,
    toolResultPreview: null,
    toolError: null,
    toolDurationMs: null,
    updatedAt: "2026-06-17T01:00:00.000Z",
    resultRef: null,
    truncated: false,
    outputBytes: null,
    outputKind: null,
    turnRunId: "run-1",
    activityOrder: 42,
    activityOrderSource: "projection",
    ...overrides,
  };
}

function approvalGate(overrides = {}) {
  return {
    kind: "gate",
    runId: "run-1",
    gateRef: "gate:approval-1",
    invocationId: "invocation-1",
    toolName: "web-access.search",
    ...overrides,
  };
}

test("approval gate with invocation id merges into an existing runtime activity", () => {
  const harness = messageHarness();
  upsertToolActivityMessage(
    harness.setMessages,
    runtimeActivity(),
    harness.stateRef,
  );

  ensureGateToolActivity(harness.setMessages, approvalGate(), harness.stateRef);
  failGateToolActivity(harness.setMessages, approvalGate(), harness.stateRef);

  assert.equal(harness.messages.length, 1);
  assert.equal(harness.messages[0].id, "tool-invocation-1");
  assert.equal(harness.messages[0].gateRef, "gate:approval-1");
  assert.equal(harness.messages[0].toolStatus, "error");
  assert.equal(harness.messages[0].toolError, "authorization");
  assert.equal(harness.messages[0].activityOrder, 42);
});

test("runtime activity adopts an earlier gate card by invocation id", () => {
  const harness = messageHarness();

  ensureGateToolActivity(harness.setMessages, approvalGate(), harness.stateRef);
  failGateToolActivity(harness.setMessages, approvalGate(), harness.stateRef);
  upsertToolActivityMessage(
    harness.setMessages,
    runtimeActivity({ toolStatus: "error" }),
    harness.stateRef,
  );

  assert.equal(harness.messages.length, 1);
  assert.equal(harness.messages[0].id, "tool-invocation-1");
  assert.equal(harness.messages[0].toolStatus, "error");
  assert.equal(harness.messages[0].activityOrder, 42);
  assert.equal(harness.messages[0].activityOrderSource, "projection");
});
