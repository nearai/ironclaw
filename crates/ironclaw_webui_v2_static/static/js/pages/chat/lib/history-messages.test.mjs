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

// Refresh-persistence contract (#3272): the timeline returns
// `ThreadMessageRecord.attachments`; the projection must surface them as
// render cards so they survive a reload / thread switch.
test("messagesFromTimeline: projects attachment refs into render cards", () => {
  const messages = messagesFromTimeline([
    {
      message_id: "m1",
      kind: "user",
      content: "see attached",
      sequence: 1,
      status: "accepted",
      attachments: [
        {
          id: "att-1",
          kind: "document",
          mime_type: "application/pdf",
          filename: "report.pdf",
          size_bytes: 2048,
          storage_key: "attachments/2026-06-10/m1-0-report.pdf",
          extracted_text: "quarterly numbers",
        },
      ],
    },
  ]);

  assert.equal(messages.length, 1);
  // The timeline carries refs only — bytes stay behind the project mount —
  // so `preview_url` is null and the card renders from metadata.
  assert.deepEqual(messages[0].attachments, [
    {
      id: "att-1",
      filename: "report.pdf",
      mime_type: "application/pdf",
      kind: "document",
      size_label: "2 KB",
      preview_url: null,
    },
  ]);
});

test("messagesFromTimeline: derives attachment kind from MIME when omitted", () => {
  const messages = messagesFromTimeline([
    {
      message_id: "m2",
      kind: "user",
      content: "pic",
      sequence: 1,
      attachments: [{ id: "a", mime_type: "image/png", filename: "p.png" }],
    },
  ]);
  assert.equal(messages[0].attachments[0].kind, "image");
  assert.equal(messages[0].attachments[0].size_label, "");
});

test("messagesFromTimeline: attachments are undefined when a record has none", () => {
  const messages = messagesFromTimeline([
    { message_id: "m3", kind: "user", content: "text only", sequence: 1 },
  ]);
  assert.equal(messages[0].attachments, undefined);
});
