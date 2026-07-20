import assert from "node:assert/strict";
import { test } from "vitest";

import { ATTACHMENTS_ONLY_CONTENT } from "./attachment-sentinel";
import { messagesFromTimeline } from "./history-messages";

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

test("messagesFromTimeline: user records keep durable created_at timestamps", () => {
  const messages = messagesFromTimeline([
    {
      message_id: "message-1",
      kind: "user",
      status: "submitted",
      content: "check my calendar",
      created_at: "2026-05-12T17:08:00Z",
      updated_at: "2026-05-12T17:09:00Z",
    },
  ]);

  assert.equal(messages.length, 1);
  assert.equal(messages[0].timestamp, "2026-05-12T17:08:00Z");
});

test("messagesFromTimeline: finalized assistant records prefer durable updated_at timestamps", () => {
  const messages = messagesFromTimeline([
    {
      message_id: "reply-1",
      kind: "assistant",
      status: "finalized",
      content: "Done.",
      created_at: "2026-05-12T17:06:00Z",
      updated_at: "2026-05-12T17:08:00Z",
    },
  ]);

  assert.equal(messages.length, 1);
  assert.equal(messages[0].timestamp, "2026-05-12T17:08:00Z");
});

test("messagesFromTimeline: received_at takes precedence over durable timestamps", () => {
  const messages = messagesFromTimeline([
    {
      message_id: "reply-received",
      kind: "assistant",
      status: "finalized",
      content: "Done.",
      received_at: "2026-05-12T17:11:00Z",
      created_at: "2026-05-12T17:06:00Z",
      updated_at: "2026-05-12T17:08:00Z",
    },
  ]);

  assert.equal(messages.length, 1);
  assert.equal(messages[0].timestamp, "2026-05-12T17:11:00Z");
});

test("messagesFromTimeline: finalized non-assistant records prefer durable updated_at timestamps", () => {
  const messages = messagesFromTimeline([
    {
      message_id: "tool-preview-1",
      kind: "capability_display_preview",
      status: "finalized",
      turn_run_id: "run-tools",
      content: JSON.stringify({
        version: 1,
        invocation_id: "invocation-extension-a",
        capability_id: "builtin.extension_search",
        status: "completed",
        title: "extension_search",
      }),
      created_at: "2026-05-12T17:06:00Z",
      updated_at: "2026-05-12T17:10:00Z",
    },
  ]);

  assert.equal(messages.length, 1);
  assert.equal(messages[0].role, "tool_activity");
  assert.equal(messages[0].timestamp, "2026-05-12T17:10:00Z");
});

test("messagesFromTimeline: tool previews use timeline sequence as activity order", () => {
  const messages = messagesFromTimeline([
    {
      message_id: "tool-preview-1",
      kind: "capability_display_preview",
      sequence: 2,
      status: "finalized",
      turn_run_id: "run-tools",
      content: JSON.stringify({
        version: 1,
        invocation_id: "invocation-extension-a",
        capability_id: "builtin.extension_search",
        status: "completed",
        title: "extension_search",
        activity_order: 10,
      }),
    },
    {
      message_id: "tool-preview-2",
      kind: "capability_display_preview",
      sequence: 3,
      status: "finalized",
      turn_run_id: "run-tools",
      content: JSON.stringify({
        version: 1,
        invocation_id: "invocation-extension-b",
        capability_id: "builtin.extension_search",
        status: "completed",
        title: "extension_search",
        activity_order: 11,
      }),
    },
  ]);

  assert.deepEqual(
    messages.map((message) => [
      message.id,
      message.toolName,
      message.toolStatus,
      message.sequence,
      message.activityOrder,
      message.activityOrderSource,
    ]),
    [
      ["tool-invocation-extension-a", "extension_search", "success", 2, 10, "projection"],
      ["tool-invocation-extension-b", "extension_search", "success", 3, 11, "projection"],
    ],
  );
});

test("messagesFromTimeline: timeline keeps durable sequence order", () => {
  const messages = messagesFromTimeline(
    [
      {
        message_id: "user-1",
        kind: "user",
        content: "search first",
        sequence: 1,
        status: "accepted",
        turn_run_id: "run-1",
      },
      {
        message_id: "tool-preview-1",
        kind: "capability_display_preview",
        sequence: 2,
        status: "finalized",
        turn_run_id: "run-1",
        content: JSON.stringify({
          version: 1,
          invocation_id: "invocation-web-search",
          capability_id: "builtin.web_search",
          status: "completed",
          title: "web_search",
          activity_order: 1,
        }),
      },
      {
        message_id: "assistant-1",
        kind: "assistant",
        content: "I answered after searching.",
        sequence: 4,
        status: "finalized",
        turn_run_id: "run-1",
      },
    ],
    [],
    "thread-1",
  );

  assert.deepEqual(
    messages.map((message) => message.id),
    ["msg-user-1", "tool-invocation-web-search", "msg-assistant-1"],
  );
  assert.equal(messages[2].keepFollowingActivityAfter, undefined);
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
  // The timeline carries refs only — bytes stay behind the project mount — so
  // `preview_url` is null and the card renders from metadata. `fetch_url` is
  // null here because this call passes no `threadId` (not because it's a
  // document); a landed attachment with a threadId gets one regardless of kind
  // (covered below).
  assert.deepEqual(messages[0].attachments, [
    {
      id: "att-1",
      filename: "report.pdf",
      mime_type: "application/pdf",
      kind: "document",
      size_label: "2 KB",
      preview_url: null,
      fetch_url: null,
    },
  ]);
});

// A landed image gets a `fetch_url` so the bubble can lazily resolve a
// thumbnail through the authenticated bytes endpoint. The URL must carry the
// (thread, message, attachment) triple — the attachment id alone is not unique
// across a thread.
test("messagesFromTimeline: landed image gets a thumbnail fetch_url", () => {
  const messages = messagesFromTimeline(
    [
      {
        message_id: "m9",
        kind: "user",
        content: "look",
        sequence: 1,
        status: "accepted",
        attachments: [
          {
            id: "att-img",
            kind: "image",
            mime_type: "image/png",
            filename: "diagram.png",
            size_bytes: 4,
            storage_key: "attachments/2026-06-14/m9-0-diagram.png",
          },
        ],
      },
    ],
    [],
    "thread-42",
  );

  assert.equal(
    messages[0].attachments[0].fetch_url,
    "/api/webchat/v2/threads/thread-42/messages/m9/attachments/att-img",
  );
});

// Click-to-preview works for every landed attachment, not just images, so a
// landed document/PDF also gets a `fetch_url` to fetch its bytes on demand.
test("messagesFromTimeline: landed non-image attachment gets a fetch_url", () => {
  const messages = messagesFromTimeline(
    [
      {
        message_id: "m11",
        kind: "user",
        content: "see doc",
        sequence: 1,
        status: "accepted",
        attachments: [
          {
            id: "att-pdf",
            kind: "document",
            mime_type: "application/pdf",
            filename: "report.pdf",
            size_bytes: 2048,
            storage_key: "attachments/2026-06-14/m11-0-report.pdf",
          },
        ],
      },
    ],
    [],
    "thread-42",
  );

  assert.equal(
    messages[0].attachments[0].fetch_url,
    "/api/webchat/v2/threads/thread-42/messages/m11/attachments/att-pdf",
  );
});

// Without a thread context (or without a landed storage_key) there is nothing
// to fetch, so `fetch_url` stays null and the card renders the icon fallback.
test("messagesFromTimeline: image without thread context has no fetch_url", () => {
  const messages = messagesFromTimeline([
    {
      message_id: "m10",
      kind: "user",
      content: "pic",
      sequence: 1,
      attachments: [
        {
          id: "a",
          kind: "image",
          mime_type: "image/png",
          filename: "p.png",
          storage_key: "attachments/2026-06-14/m10-0-p.png",
        },
      ],
    },
  ]);
  assert.equal(messages[0].attachments[0].fetch_url, null);
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

test("messagesFromTimeline: hides only the opaque attachment-only sentinel", () => {
  const withSentinel = messagesFromTimeline([
    {
      message_id: "m4",
      kind: "user",
      content: ATTACHMENTS_ONLY_CONTENT,
      sequence: 1,
      attachments: [{ id: "a", mime_type: "text/plain", filename: "notes.txt" }],
    },
  ]);
  assert.equal(withSentinel[0].content, "");

  const userVisibleText = messagesFromTimeline([
    {
      message_id: "m5",
      kind: "user",
      content: "(files attached)",
      sequence: 1,
      attachments: [{ id: "a", mime_type: "text/plain", filename: "notes.txt" }],
    },
  ]);
  assert.equal(userVisibleText[0].content, "(files attached)");
});
