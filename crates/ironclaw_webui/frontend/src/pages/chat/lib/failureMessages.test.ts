import assert from "node:assert/strict";
import { test } from "vitest";

import {
  CONNECTION_LOST_RUN_FAILURE_KEY,
  failureMessageForRequestError,
  failureMessageForRunStatus,
  failureMessageForStreamError,
  rewriteConnectionLostRunFailures,
  upsertConnectionLostRunFailure,
} from "./failureMessages";
import { CONNECTION_STATUS } from "./connection-status";

const ENGLISH_FAILURE_COPY = {
  "chat.failure.connectionLost":
    "Connection to the server was lost. Please reconnect and try again.",
  "chat.failure.request": "The request failed before it could be sent.",
  "chat.failure.runCategory": "The run failed: {detail}.",
  "chat.failure.recoveryRequired":
    "The run is awaiting recovery — backend reported `recovery_required`.",
  "chat.failure.run": "The run failed before producing a reply.",
  "chat.failure.streamRetryable":
    "The chat stream hit a retryable error: {detail}.",
  "chat.failure.stream": "The chat stream failed: {detail}.",
};

function testTranslator(
  copy: Record<string, string> = ENGLISH_FAILURE_COPY,
) {
  return (key, params = {}) =>
    (copy[key] || key).replace(/\{(\w+)\}/g, (match, name) =>
      Object.hasOwn(params, name) ? String(params[name]) : match,
    );
}

const t = testTranslator();

test("failureMessageForRunStatus prefers trimmed failureSummary", () => {
  assert.equal(
    failureMessageForRunStatus({
      status: "failed",
      failureCategory: "driver_failed",
      failureSummary: "  The driver stopped unexpectedly.  ",
    }, t),
    "The driver stopped unexpectedly.",
  );
});

test("failureMessageForRunStatus formats category underscores", () => {
  assert.equal(
    failureMessageForRunStatus({
      status: "failed",
      failureCategory: "driver_invalid_request",
      failureSummary: null,
    }, t),
    "The run failed: driver invalid request.",
  );
});

test("failureMessageForRunStatus uses recovery_required fallback", () => {
  assert.equal(
    failureMessageForRunStatus({
      status: "recovery_required",
      failureCategory: null,
      failureSummary: null,
    }, t),
    "The run is awaiting recovery — backend reported `recovery_required`.",
  );
});

test("failureMessageForRunStatus handles whitespace-only summary", () => {
  assert.equal(
    failureMessageForRunStatus({
      status: "failed",
      failureCategory: "lease_expired",
      failureSummary: "   ",
    }, t),
    "The run failed: lease expired.",
  );
});

test("failureMessageForRequestError prefers a safe throwable message", () => {
  assert.equal(
    failureMessageForRequestError(
      new Error("  AI provider account is out of credits.  "),
      t,
    ),
    "AI provider account is out of credits.",
  );
});

test("failureMessageForRequestError suppresses credential-bearing messages", () => {
  assert.equal(
    failureMessageForRequestError(
      new Error("Authorization: Bearer sk-proj-1234567890abcdef"),
      t,
    ),
    "The request failed before it could be sent.",
  );
  assert.equal(
    failureMessageForRequestError({
      message: "Provider rejected API key abcdef1234567890abcdef1234567890.",
    }, t),
    "The request failed before it could be sent.",
  );
});

test("failureMessageForRequestError has a stable fallback", () => {
  assert.equal(
    failureMessageForRequestError({ message: "   " }, t),
    "The request failed before it could be sent.",
  );
});

test("failureMessageForRequestError localizes client-derived API errors", () => {
  assert.equal(
    failureMessageForRequestError(
      {
        name: "ApiError",
        message: "Service unavailable",
        payload: { kind: "service_unavailable" },
      },
      t,
    ),
    "The request failed before it could be sent.",
  );
  assert.equal(
    failureMessageForRequestError(
      {
        name: "ApiError",
        message: "Bad Gateway",
        body: "",
        statusText: "Bad Gateway",
      },
      t,
    ),
    "The request failed before it could be sent.",
  );
});

test("failureMessageForRequestError preserves safe server prose", () => {
  assert.equal(
    failureMessageForRequestError(
      {
        name: "ApiError",
        message: "The selected model is temporarily unavailable.",
        body: "The selected model is temporarily unavailable.",
      },
      t,
    ),
    "The selected model is temporarily unavailable.",
  );
});

test("failureMessageForStreamError humanizes redacted stream tokens", () => {
  assert.equal(
    failureMessageForStreamError({
      error: "unavailable",
      kind: "service_unavailable",
      retryable: true,
    }, t),
    "The chat stream hit a retryable error: Service unavailable.",
  );
});

test("failureMessageForRunStatus prefers connection copy for disconnected driver_unavailable failures", () => {
  assert.equal(
    failureMessageForRunStatus({
      status: "failed",
      failureCategory: "driver_unavailable",
      failureSummary:
        "The run failed because the execution driver was temporarily unavailable.",
      connectionStatus: CONNECTION_STATUS.DISCONNECTED,
    }, t),
    t(CONNECTION_LOST_RUN_FAILURE_KEY),
  );
});

test("failureMessageForRunStatus keeps non-connection driver failures unchanged", () => {
  assert.equal(
    failureMessageForRunStatus({
      status: "failed",
      failureCategory: "driver_unavailable",
      failureSummary:
        "The run failed because the execution driver was temporarily unavailable.",
      connectionStatus: CONNECTION_STATUS.CONNECTED,
    }, t),
    "The run failed because the execution driver was temporarily unavailable.",
  );
});

test("failureMessageForRunStatus does not infer driver_unavailable from summary copy", () => {
  const summary =
    "The run failed because the execution driver was temporarily unavailable.";

  assert.equal(
    failureMessageForRunStatus({
      status: "failed",
      failureCategory: null,
      failureSummary: summary,
      connectionStatus: CONNECTION_STATUS.DISCONNECTED,
    }, t),
    summary,
  );
});

test("failureMessageForRunStatus does not hide unrelated failures while disconnected", () => {
  assert.equal(
    failureMessageForRunStatus({
      status: "failed",
      failureCategory: "driver_invalid_request",
      failureSummary:
        "The run failed because the execution driver rejected the request.",
      connectionStatus: CONNECTION_STATUS.DISCONNECTED,
    }, t),
    "The run failed because the execution driver rejected the request.",
  );
});

test("rewriteConnectionLostRunFailures updates existing driver_unavailable bubbles after a disconnect", () => {
  const messages = [
    {
      id: "err-run-1",
      role: "error",
      content:
        "The run failed because the execution driver was temporarily unavailable.",
      failureCategory: "driver_unavailable",
      failureSummary:
        "The run failed because the execution driver was temporarily unavailable.",
    },
    {
      id: "err-run-2",
      role: "error",
      content:
        "The run failed because the execution driver rejected the request.",
      failureCategory: "driver_invalid_request",
      failureSummary:
        "The run failed because the execution driver rejected the request.",
    },
  ];

  const next = rewriteConnectionLostRunFailures(messages, { runId: "run-1", t });

  assert.notEqual(next, messages);
  assert.equal(next[0].content, t(CONNECTION_LOST_RUN_FAILURE_KEY));
  assert.equal(
    next[1].content,
    "The run failed because the execution driver rejected the request.",
  );
  assert.equal(
    rewriteConnectionLostRunFailures(next, { runId: "run-1", t }),
    next,
  );
});

test("rewriteConnectionLostRunFailures leaves history alone without a run id", () => {
  const messages = [
    {
      id: "err-old-run",
      role: "error",
      content:
        "The run failed because the execution driver was temporarily unavailable.",
      failureCategory: "driver_unavailable",
      failureSummary:
        "The run failed because the execution driver was temporarily unavailable.",
    },
  ];

  assert.equal(
    rewriteConnectionLostRunFailures(messages, { runId: null, t }),
    messages,
  );
  assert.equal(rewriteConnectionLostRunFailures(messages, { t }), messages);
});

test("upsertConnectionLostRunFailure appends scoped connection error", () => {
  const messages = [
    {
      id: "err-old-run",
      role: "error",
      content:
        "The run failed because the execution driver was temporarily unavailable.",
      failureCategory: "driver_unavailable",
    },
  ];

  const next = upsertConnectionLostRunFailure(messages, {
    runId: "run-1",
    timestamp: "2026-06-02T00:00:00.000Z",
    t,
  });

  assert.equal(messages.length, 1);
  assert.equal(next.length, 2);
  assert.equal(next[0].content, messages[0].content);
  assert.deepEqual(next[1], {
    id: "err-run-1",
    role: "error",
    content: t(CONNECTION_LOST_RUN_FAILURE_KEY),
    timestamp: "2026-06-02T00:00:00.000Z",
    failureStatus: "failed",
    failureCategory: "connection_lost",
    failureSummary: t(CONNECTION_LOST_RUN_FAILURE_KEY),
  });
});

test("upsertConnectionLostRunFailure does not rewrite history when run id is unknown", () => {
  const historicalFailure =
    "The run failed because the execution driver was temporarily unavailable.";
  const messages = [
    {
      id: "err-old-run",
      role: "error",
      content: historicalFailure,
      failureCategory: "driver_unavailable",
    },
  ];

  const next = upsertConnectionLostRunFailure(messages, {
    timestamp: "2026-06-02T00:00:00.000Z",
    t,
  });

  assert.equal(next.length, 2);
  assert.equal(next[0].content, historicalFailure);
  assert.equal(next[1].id, "err-connection-lost");
  assert.equal(next[1].content, t(CONNECTION_LOST_RUN_FAILURE_KEY));
});

test("client-generated failure copy uses the selected translator", () => {
  const zh = testTranslator({
    "chat.failure.connectionLost": "与服务器的连接已断开。请重新连接后重试。",
    "chat.failure.request": "请求在发送前失败。",
    "chat.failure.run": "运行在生成回复前失败。",
    "chat.failure.stream": "聊天流失败：{detail}。",
  });

  assert.equal(
    failureMessageForRequestError(
      { name: "TypeError", message: "Failed to fetch" },
      zh,
    ),
    "请求在发送前失败。",
  );
  assert.equal(
    failureMessageForRunStatus({ status: "failed" }, zh),
    "运行在生成回复前失败。",
  );
  assert.equal(
    failureMessageForStreamError(
      { kind: "service_unavailable", retryable: false },
      zh,
    ),
    "聊天流失败：Service unavailable。",
  );
  assert.equal(
    upsertConnectionLostRunFailure([], { runId: "run-zh", t: zh })[0].content,
    "与服务器的连接已断开。请重新连接后重试。",
  );
});
