import assert from "node:assert/strict";
import test from "node:test";

import { isRetryableMessage } from "./retry-eligibility.js";

test("isRetryableMessage: user + error + text → retryable", () => {
  assert.ok(isRetryableMessage({ role: "user", status: "error", content: "hello" }));
});

test("isRetryableMessage: user + error + text + empty attachments array → retryable", () => {
  assert.ok(
    isRetryableMessage({ role: "user", status: "error", content: "hello", attachments: [] }),
  );
});

test("isRetryableMessage: user + error + attachments → not retryable", () => {
  assert.ok(
    !isRetryableMessage({ role: "user", status: "error", content: "with file", attachments: [{ id: "1" }] }),
    "file blobs are gone after first send; retry would silently drop attachments",
  );
});

test("isRetryableMessage: non-user roles → not retryable", () => {
  assert.ok(!isRetryableMessage({ role: "assistant", status: "error", content: "sorry" }));
  assert.ok(!isRetryableMessage({ role: "system", status: "error", content: "note" }));
});

test("isRetryableMessage: empty or whitespace content → not retryable", () => {
  assert.ok(!isRetryableMessage({ role: "user", status: "error", content: "" }));
  assert.ok(!isRetryableMessage({ role: "user", status: "error", content: "   " }));
});

test("isRetryableMessage: non-string content → not retryable", () => {
  assert.ok(!isRetryableMessage({ role: "user", status: "error", content: null }));
  assert.ok(!isRetryableMessage({ role: "user", status: "error", content: 42 }));
});

test("isRetryableMessage: status not 'error' → not retryable", () => {
  assert.ok(!isRetryableMessage({ role: "user", status: "sent", content: "hello" }));
  assert.ok(!isRetryableMessage({ role: "user", status: undefined, content: "hello" }));
});

test("isRetryableMessage: null or undefined message → not retryable", () => {
  assert.ok(!isRetryableMessage(null));
  assert.ok(!isRetryableMessage(undefined));
});
