import assert from "node:assert/strict";
import test from "node:test";

import {
  toolCardFromActivity,
  toolCardFromPreview,
} from "./history-messages.js";
import {
  createToolActivityState,
  ensureGateToolActivity,
  failGateToolActivity,
  upsertToolActivityMessage,
} from "./tool-activity-state.js";

test("tool activity state keeps denied tools visible through a follow-up gate", () => {
  const runId = "run-deny-sequence";
  const stateRef = { current: createToolActivityState() };
  let messages = [];
  const setMessages = (updater) => {
    messages = typeof updater === "function" ? updater(messages) : updater;
  };

  upsertToolActivityMessage(
    setMessages,
    toolCardFromActivity({
      invocation_id: "invocation-web",
      turn_run_id: runId,
      capability_id: "web-access.search",
      status: "started",
    }),
    stateRef,
  );
  ensureGateToolActivity(
    setMessages,
    {
      kind: "gate",
      runId,
      gateRef: "gate:web",
      toolName: "web-access.search",
    },
    stateRef,
  );
  failGateToolActivity(
    setMessages,
    {
      kind: "gate",
      runId,
      gateRef: "gate:web",
      toolName: "web-access.search",
    },
    stateRef,
  );

  ensureGateToolActivity(
    setMessages,
    {
      kind: "gate",
      runId,
      gateRef: "gate:nearai",
      toolName: "nearai.web_search",
    },
    stateRef,
  );

  assert.deepEqual(
    messages.map((message) => [
      message.id,
      message.toolName,
      message.toolStatus,
      message.gateRef,
    ]),
    [
      [
        "tool-invocation-web",
        "search",
        "error",
        "gate:web",
      ],
      [
        `tool-gate:${runId}:gate:nearai`,
        "web_search",
        "running",
        "gate:nearai",
      ],
    ],
  );

  upsertToolActivityMessage(
    setMessages,
    toolCardFromActivity({
      invocation_id: "invocation-web",
      turn_run_id: runId,
      capability_id: "web-access.search",
      status: "started",
    }),
    stateRef,
  );

  assert.equal(messages[0].toolStatus, "error");

  upsertToolActivityMessage(
    setMessages,
    toolCardFromActivity({
      invocation_id: "invocation-nearai",
      turn_run_id: runId,
      capability_id: "nearai.web_search",
      status: "started",
    }),
    stateRef,
  );
  failGateToolActivity(
    setMessages,
    {
      kind: "gate",
      runId,
      gateRef: "gate:nearai",
      toolName: "nearai.web_search",
    },
    stateRef,
  );

  assert.deepEqual(
    messages.map((message) => [
      message.id,
      message.toolName,
      message.toolStatus,
      message.gateRef,
    ]),
    [
      [
        "tool-invocation-web",
        "search",
        "error",
        "gate:web",
      ],
      [
        "tool-invocation-nearai",
        "web_search",
        "error",
        "gate:nearai",
      ],
    ],
  );
});

test("tool activity state keeps repeated same-tool approval gates separate", () => {
  const runId = "run-repeated-installs";
  const stateRef = { current: createToolActivityState() };
  let messages = [];
  const setMessages = (updater) => {
    messages = typeof updater === "function" ? updater(messages) : updater;
  };
  const gate = (index) => ({
    kind: "gate",
    runId,
    gateRef: `gate:extension-install:${index}`,
    toolName: "builtin.extension_install",
  });

  for (const index of [1, 2, 3]) {
    ensureGateToolActivity(setMessages, gate(index), stateRef);
    failGateToolActivity(setMessages, gate(index), stateRef);
  }

  assert.deepEqual(
    messages.map((message) => [
      message.id,
      message.toolName,
      message.toolStatus,
      message.gateRef,
    ]),
    [
      [
        `tool-gate:${runId}:gate:extension-install:1`,
        "extension_install",
        "error",
        "gate:extension-install:1",
      ],
      [
        `tool-gate:${runId}:gate:extension-install:2`,
        "extension_install",
        "error",
        "gate:extension-install:2",
      ],
      [
        `tool-gate:${runId}:gate:extension-install:3`,
        "extension_install",
        "error",
        "gate:extension-install:3",
      ],
    ],
  );
});

test("tool activity cards use unprefixed display names", () => {
  assert.equal(
    toolCardFromActivity({
      invocation_id: "invocation-web",
      capability_id: "web-access.search",
      status: "failed",
    }).toolName,
    "search",
  );
  assert.equal(
    toolCardFromActivity({
      invocation_id: "invocation-install",
      capability_id: "builtin.extension_install",
      status: "failed",
    }).toolName,
    "extension_install",
  );
  assert.equal(
    toolCardFromPreview({
      invocation_id: "invocation-nearai",
      capability_id: "nearai.web_search",
      title: "nearai.web_search",
      status: "failed",
    }).toolName,
    "web_search",
  );
});

test("tool activity state leaves pending gates unnumbered after existing timeline activity", () => {
  const runId = "run-refresh-order";
  const stateRef = { current: createToolActivityState() };
  let messages = [
    {
      id: "tool-extension-a",
      role: "tool_activity",
      invocationId: "extension-a",
      turnRunId: runId,
      toolName: "extension_search",
      toolStatus: "success",
      activityOrder: 2,
    },
    {
      id: "tool-extension-b",
      role: "tool_activity",
      invocationId: "extension-b",
      turnRunId: runId,
      toolName: "extension_search",
      toolStatus: "success",
      activityOrder: 3,
    },
  ];
  const setMessages = (updater) => {
    messages = typeof updater === "function" ? updater(messages) : updater;
  };

  ensureGateToolActivity(
    setMessages,
    {
      kind: "gate",
      runId,
      gateRef: "gate:web-search",
      toolName: "web-access.search",
    },
    stateRef,
  );

  assert.deepEqual(
    messages.map((message) => [message.toolName, message.activityOrder]),
    [
      ["extension_search", 2],
      ["extension_search", 3],
      ["search", undefined],
    ],
  );
});

test("tool activity state preserves existing order when a gate is denied", () => {
  const runId = "run-deny-rebased";
  const stateRef = { current: createToolActivityState() };
  let messages = [];
  const setMessages = (updater) => {
    messages = typeof updater === "function" ? updater(messages) : updater;
  };
  const gate = {
    kind: "gate",
    runId,
    gateRef: "gate:web-search",
    toolName: "web-access.search",
  };

  ensureGateToolActivity(setMessages, gate, stateRef);
  messages = messages.map((message) => ({
    ...message,
    activityOrder: 4,
  }));
  failGateToolActivity(setMessages, gate, stateRef);

  assert.equal(messages.length, 1);
  assert.equal(messages[0].toolStatus, "error");
  assert.equal(messages[0].activityOrder, 4);
});

test("tool activity state applies durable projection order to live activity", () => {
  const runId = "run-projection-order";
  const stateRef = { current: createToolActivityState() };
  let messages = [];
  const setMessages = (updater) => {
    messages = typeof updater === "function" ? updater(messages) : updater;
  };

  upsertToolActivityMessage(
    setMessages,
    {
      invocationId: "invocation-web",
      callId: "invocation-web",
      capabilityId: "web-access.search",
      toolName: "search",
      toolStatus: "running",
      turnRunId: runId,
    },
    stateRef,
  );
  assert.equal(messages[0].activityOrder, undefined);

  upsertToolActivityMessage(
    setMessages,
    {
      invocationId: "invocation-web",
      callId: "invocation-web",
      capabilityId: "web-access.search",
      toolName: "search",
      toolStatus: "running",
      turnRunId: runId,
      activityOrder: 42,
      activityOrderSource: "projection",
    },
    stateRef,
  );

  assert.equal(messages.length, 1);
  assert.equal(messages[0].activityOrder, 42);
  assert.equal(messages[0].activityOrderSource, "projection");
});

test("tool activity state applies durable projection order to gate activity", () => {
  const runId = "run-snapshot-order";
  const stateRef = { current: createToolActivityState() };
  let messages = [];
  const setMessages = (updater) => {
    messages = typeof updater === "function" ? updater(messages) : updater;
  };

  ensureGateToolActivity(
    setMessages,
    {
      kind: "gate",
      runId,
      gateRef: "gate:web-search",
      toolName: "web-access.search",
    },
    stateRef,
  );
  assert.equal(messages[0].activityOrder, undefined);
  assert.equal(messages[0].activityOrderSource, undefined);

  upsertToolActivityMessage(
    setMessages,
    {
      invocationId: messages[0].invocationId,
      callId: messages[0].callId,
      capabilityId: "web-access.search",
      toolName: "search",
      toolStatus: "running",
      turnRunId: runId,
      activityOrder: 43,
      activityOrderSource: "projection",
    },
    stateRef,
  );

  assert.equal(messages.length, 1);
  assert.equal(messages[0].activityOrder, 43);
  assert.equal(messages[0].activityOrderSource, "projection");
});
