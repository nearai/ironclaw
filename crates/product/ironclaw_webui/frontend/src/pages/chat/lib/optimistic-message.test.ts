// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";

import { buildOptimisticMessage } from "./optimistic-message";

test("buildOptimisticMessage carries side metadata through `extra`", () => {
  const message = buildOptimisticMessage({
    id: "pending-1",
    content: "queued follow-up",
    attachments: [{ id: "att-1" }],
    extra: { retryContent: "queued follow-up", retryAttachments: [{ id: "att-1" }] },
  });

  assert.equal(message.id, "pending-1");
  assert.equal(message.role, "user");
  assert.equal(message.content, "queued follow-up");
  assert.deepEqual(message.attachments, [{ id: "att-1" }]);
  assert.equal(message.isOptimistic, true);
  assert.equal(typeof message.timestamp, "string");
  assert.equal(message.retryContent, "queued follow-up");
  assert.deepEqual(message.retryAttachments, [{ id: "att-1" }]);
});

// This function exists so the pending-ref record and the rendered message can
// never drift. That guarantee is only real if `extra` cannot reshape the
// identity fields — a colliding key must lose, not silently win.
test("buildOptimisticMessage identity fields win over colliding `extra` keys", () => {
  const message = buildOptimisticMessage({
    id: "pending-2",
    content: "real content",
    attachments: [],
    extra: {
      id: "hijacked",
      role: "assistant",
      content: "hijacked content",
      attachments: [{ id: "hijacked" }],
      isOptimistic: false,
      timestamp: "1970-01-01T00:00:00.000Z",
    },
  });

  assert.equal(message.id, "pending-2");
  assert.equal(message.role, "user");
  assert.equal(message.content, "real content");
  assert.deepEqual(message.attachments, []);
  assert.equal(message.isOptimistic, true);
  assert.notEqual(message.timestamp, "1970-01-01T00:00:00.000Z");
});
