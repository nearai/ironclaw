import assert from "node:assert/strict";
import { test } from "vitest";

import {
  toolCardFromActivity,
  toolCardFromPreview,
} from "./history-messages";
import {
  createToolActivityState,
  ensureGateToolActivity,
  failGateToolActivity,
  upsertToolActivityMessage,
} from "./tool-activity-state";

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
      invocationId: "invocation-web",
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
      invocationId: "invocation-web",
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
      invocationId: "invocation-nearai",
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
        "declined",
        "gate:web",
      ],
      [
        "tool-invocation-nearai",
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

  assert.equal(messages[0].toolStatus, "declined");

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
      invocationId: "invocation-nearai",
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
        "declined",
        "gate:web",
      ],
      [
        "tool-invocation-nearai",
        "web_search",
        "declined",
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
    invocationId: `invocation-install-${index}`,
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
        "tool-invocation-install-1",
        "extension_install",
        "declined",
        "gate:extension-install:1",
      ],
      [
        "tool-invocation-install-2",
        "extension_install",
        "declined",
        "gate:extension-install:2",
      ],
      [
        "tool-invocation-install-3",
        "extension_install",
        "declined",
        "gate:extension-install:3",
      ],
    ],
  );
});

test("tool activity state does not synthesize gate activity without invocation id", () => {
  const stateRef = { current: createToolActivityState() };
  let messages = [];
  const setMessages = (updater) => {
    messages = typeof updater === "function" ? updater(messages) : updater;
  };

  ensureGateToolActivity(
    setMessages,
    {
      kind: "gate",
      runId: "run-missing-id",
      gateRef: "gate:web-search",
      toolName: "web-access.search",
    },
    stateRef,
  );

  assert.deepEqual(messages, []);
});

test("tool activity state can mark auth gates declined without invocation id", () => {
  const stateRef = { current: createToolActivityState() };
  let messages = [];
  const setMessages = (updater) => {
    messages = typeof updater === "function" ? updater(messages) : updater;
  };

  failGateToolActivity(
    setMessages,
    {
      kind: "auth_required",
      runId: "run-auth-declined",
      gateRef: "gate:auth-required",
      gateKind: "auth",
      headline: "Authentication required",
    },
    stateRef,
  );

  assert.equal(messages.length, 1);
  assert.equal(
    messages[0].id,
    "tool-gate:run-auth-declined:auth_required:gate:auth-required",
  );
  assert.equal(messages[0].toolName, "Authentication required");
  assert.equal(messages[0].toolStatus, "declined");
  assert.equal(messages[0].toolErrorKind, "gate_declined");
  assert.equal(messages[0].gateRef, "gate:auth-required");
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

test("tool activity cards map gate-declined lifecycle frames to declined status", () => {
  const card = toolCardFromActivity({
    invocation_id: "invocation-declined",
    capability_id: "builtin.extension_install",
    status: "failed",
    error_kind: "gate_declined",
  });

  assert.equal(card.toolStatus, "declined");
  assert.equal(card.toolError, "gate_declined");
  assert.equal(card.toolErrorKind, "gate_declined");
});

test("tool preview cards preserve gate-declined error kind as declined status", () => {
  const card = toolCardFromPreview({
    invocation_id: "invocation-preview-declined",
    capability_id: "builtin.extension_install",
    title: "extension_install",
    status: "failed",
    error_kind: "gate_declined",
    output_summary: "gate_declined",
  });

  assert.equal(card.toolStatus, "declined");
  assert.equal(card.toolError, "gate_declined");
  assert.equal(card.toolErrorKind, "gate_declined");
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
      invocationId: "invocation-web-search",
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
    invocationId: "invocation-web-search",
    toolName: "web-access.search",
  };

  ensureGateToolActivity(setMessages, gate, stateRef);
  messages = messages.map((message) => ({
    ...message,
    activityOrder: 4,
  }));
  failGateToolActivity(setMessages, gate, stateRef);

  assert.equal(messages.length, 1);
  assert.equal(messages[0].toolStatus, "declined");
  assert.equal(messages[0].activityOrder, 4);
});

test("tool activity state keeps preview failure detail when a bare-kind activity frame races in after it", () => {
  const stateRef = { current: createToolActivityState() };
  let messages = [];
  const setMessages = (updater) => {
    messages = typeof updater === "function" ? updater(messages) : updater;
  };

  // The display preview lands first with the real failure reason.
  upsertToolActivityMessage(
    setMessages,
    toolCardFromPreview({
      invocation_id: "invocation-json",
      capability_id: "builtin.json",
      status: "failed",
      error_kind: "invalid_input",
      output_summary: "the tool input could not be encoded",
    }),
    stateRef,
  );
  assert.equal(messages[0].toolError, "the tool input could not be encoded");

  // A runtime-payload activity frame (no error_detail → toolError is the bare
  // kind) races in afterward and must NOT clobber the detailed reason.
  upsertToolActivityMessage(
    setMessages,
    toolCardFromActivity({
      invocation_id: "invocation-json",
      capability_id: "builtin.json",
      status: "failed",
      error_kind: "invalid_input",
    }),
    stateRef,
  );

  assert.equal(messages.length, 1);
  assert.equal(messages[0].toolError, "the tool input could not be encoded");
  assert.equal(messages[0].toolErrorKind, "invalid_input");
});

test("tool activity state preserves running input details across metadata-only updates", () => {
  const stateRef = { current: createToolActivityState() };
  let messages = [];
  const setMessages = (updater) => {
    messages = typeof updater === "function" ? updater(messages) : updater;
  };

  upsertToolActivityMessage(
    setMessages,
    toolCardFromActivity({
      invocation_id: "invocation-search",
      capability_id: "nearai.web_search",
      status: "started",
      subtitle: "deploy status",
      input_summary: "query: deploy status",
      output_bytes: 0,
    }),
    stateRef,
  );

  upsertToolActivityMessage(
    setMessages,
    toolCardFromActivity({
      invocation_id: "invocation-search",
      capability_id: "nearai.web_search",
      status: "running",
      output_bytes: 128,
    }),
    stateRef,
  );

  assert.equal(messages.length, 1);
  assert.equal(messages[0].toolStatus, "running");
  assert.equal(messages[0].toolDetail, "deploy status");
  assert.equal(messages[0].toolParameters, "query: deploy status");
  assert.equal(messages[0].outputBytes, 128);
});

test("tool activity state preserves completed output when a late activity frame has no preview", () => {
  const stateRef = { current: createToolActivityState() };
  let messages = [];
  const setMessages = (updater) => {
    messages = typeof updater === "function" ? updater(messages) : updater;
  };

  upsertToolActivityMessage(
    setMessages,
    toolCardFromPreview({
      invocation_id: "invocation-search",
      capability_id: "nearai.web_search",
      title: "nearai.web_search",
      status: "completed",
      input_summary: "query: deploy status",
      output_preview: "No deployment incidents found.",
      result_ref: "result:run.search",
      output_kind: "text",
      output_bytes: 30,
      truncated: true,
    }),
    stateRef,
  );

  upsertToolActivityMessage(
    setMessages,
    toolCardFromActivity({
      invocation_id: "invocation-search",
      capability_id: "nearai.web_search",
      status: "completed",
    }),
    stateRef,
  );

  assert.equal(messages.length, 1);
  assert.equal(messages[0].toolStatus, "success");
  assert.equal(messages[0].toolParameters, "query: deploy status");
  assert.equal(messages[0].toolResultPreview, "No deployment incidents found.");
  assert.equal(messages[0].resultRef, "result:run.search");
  assert.equal(messages[0].outputKind, "text");
  assert.equal(messages[0].outputBytes, 30);
  assert.equal(messages[0].truncated, true);
});

test("tool activity state uses bare-kind flag instead of string equality", () => {
  const stateRef = { current: createToolActivityState() };
  let messages = [];
  const setMessages = (updater) => {
    messages = typeof updater === "function" ? updater(messages) : updater;
  };

  upsertToolActivityMessage(
    setMessages,
    {
      invocationId: "invocation-json",
      capabilityId: "builtin.json",
      toolName: "json",
      toolStatus: "error",
      toolError: "the tool input could not be encoded",
      toolErrorKind: "invalid_input",
      toolErrorIsBareKind: false,
    },
    stateRef,
  );

  upsertToolActivityMessage(
    setMessages,
    {
      invocationId: "invocation-json",
      capabilityId: "builtin.json",
      toolName: "json",
      toolStatus: "error",
      toolError: "invalid_input",
      toolErrorKind: "invalidInput",
      toolErrorIsBareKind: true,
    },
    stateRef,
  );

  assert.equal(messages.length, 1);
  assert.equal(messages[0].toolError, "the tool input could not be encoded");
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
      invocationId: "invocation-web-search",
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
