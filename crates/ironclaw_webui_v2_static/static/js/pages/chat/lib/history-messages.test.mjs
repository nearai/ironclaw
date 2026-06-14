import assert from "node:assert/strict";
import test from "node:test";

import { messagesFromTimeline } from "./history-messages.js";

test("messagesFromTimeline: pending messages default to optimistic user messages", () => {
  const messages = messagesFromTimeline([], [
    {
      id: "pending-1",
      content: "check my calendar",
      timestamp: "2026-06-02T10:00:00.000Z",
    },
  ]);

  assert.deepEqual(messages, [
    {
      id: "pending-1",
      role: "user",
      content: "check my calendar",
      timestamp: "2026-06-02T10:00:00.000Z",
      isOptimistic: true,
    },
  ]);
});

test("messagesFromTimeline: confirmed user records replace matching pending by timeline id", () => {
  const messages = messagesFromTimeline(
    [
      {
        message_id: "message-1",
        kind: "user",
        content: "check my calendar",
        sequence: 1,
        status: "accepted",
      },
    ],
    [
      {
        id: "pending-1",
        role: "user",
        content: "check my calendar",
        timestamp: "2026-06-02T10:00:00.000Z",
        isOptimistic: true,
        timelineMessageId: "message-1",
      },
    ],
  );

  assert.equal(messages.length, 1);
  assert.equal(messages[0].id, "msg-message-1");
  assert.equal(messages[0].role, "user");
  assert.equal(messages[0].content, "check my calendar");
});

test("messagesFromTimeline: mismatched pending timeline id is preserved", () => {
  const messages = messagesFromTimeline(
    [
      {
        message_id: "message-1",
        kind: "user",
        content: "check my calendar",
        sequence: 1,
        status: "accepted",
      },
    ],
    [
      {
        id: "pending-1",
        role: "user",
        content: "check my calendar",
        timestamp: "2026-06-02T10:00:00.000Z",
        isOptimistic: true,
        timelineMessageId: "message-2",
      },
    ],
  );

  assert.deepEqual(
    messages.map((message) => message.id),
    ["msg-message-1", "pending-1"],
  );
});

test("messagesFromTimeline: equal pending text without timeline id is preserved", () => {
  const messages = messagesFromTimeline(
    [
      {
        message_id: "message-1",
        kind: "user",
        content: "check my calendar",
        sequence: 1,
        status: "accepted",
      },
    ],
    [
      {
        id: "pending-1",
        role: "user",
        content: "check my calendar",
        timestamp: "2026-06-02T10:00:00.000Z",
        isOptimistic: true,
      },
    ],
  );

  assert.deepEqual(
    messages.map((message) => message.id),
    ["msg-message-1", "pending-1"],
  );
});

test("messagesFromTimeline: rejected_busy user record maps to error status with durable resend copy", () => {
  const messages = messagesFromTimeline([
    {
      message_id: "msg-rb",
      kind: "user",
      content: "do something",
      sequence: 1,
      status: "rejected_busy",
    },
  ]);

  assert.equal(messages.length, 1);
  assert.equal(messages[0].id, "msg-msg-rb");
  assert.equal(messages[0].role, "user");
  assert.equal(messages[0].status, "error");
  assert.equal(
    messages[0].error,
    "This message wasn't sent because Ironclaw was busy. Resend it to try again.",
  );
});

test("messagesFromTimeline: deferred_busy user record maps to error status with durable resend copy", () => {
  const messages = messagesFromTimeline([
    {
      message_id: "msg-db",
      kind: "user",
      content: "do something else",
      sequence: 1,
      status: "deferred_busy",
    },
  ]);

  assert.equal(messages.length, 1);
  assert.equal(messages[0].id, "msg-msg-db");
  assert.equal(messages[0].role, "user");
  assert.equal(messages[0].status, "error");
  assert.equal(
    messages[0].error,
    "This message wasn't sent because Ironclaw was busy. Resend it to try again.",
  );
});

test("messagesFromTimeline: finalized assistant records are marked as final replies", () => {
  const messages = messagesFromTimeline([
    {
      message_id: "final",
      kind: "assistant",
      status: "finalized",
      content: "Done.",
    },
    {
      message_id: "draft",
      kind: "assistant",
      status: "draft",
      content: "I will check.",
    },
  ]);

  assert.equal(messages[0].id, "msg-final");
  assert.equal(messages[0].isFinalReply, true);
  assert.equal(messages[1].id, "msg-draft");
  assert.equal(messages[1].isFinalReply, false);
});
