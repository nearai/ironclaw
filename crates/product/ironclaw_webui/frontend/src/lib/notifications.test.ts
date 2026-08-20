// @ts-nocheck
import assert from "node:assert/strict";
import { beforeEach, test } from "vitest";
import { setAuthScope } from "./auth-scope";
import {
  approvalThreadNotificationId,
  approvalThreadNotifications,
  getNotificationState,
  markNotificationIdsSeen,
} from "./notification-approval-compat";
import { notificationMessages } from "./notifications";

let testScopeId = 0;

beforeEach(() => {
  const storage = new Map();
  globalThis.window = {
    localStorage: {
      getItem: (key) => storage.get(key) ?? null,
      setItem: (key, value) => storage.set(key, String(value)),
      removeItem: (key) => storage.delete(key),
      get length() { return storage.size; },
      key: (index) => [...storage.keys()][index] ?? null,
    },
  };
  testScopeId += 1;
  setAuthScope({ tenant_id: "tenant", user_id: `user-${testScopeId}` });
});

const t = (key) => ({
  "notifications.approval.title": "Approval required",
  "notifications.approval.body": "A run is waiting for your approval.",
  "notifications.approval.detail": "Needs your approval",
  "notifications.failed.title": "Run failed",
  "notifications.failed.body": "A background run did not complete.",
  "notifications.failed.detail": "Open the thread to review",
  "notifications.resolved": "Resolved",
}[key] || key);

test("notificationMessages presents typed server notifications", () => {
  const messages = notificationMessages([
    {
      id: "notification-1",
      kind: "approval_required",
      severity: "warning",
      action: { kind: "open_thread", thread_id: "thread/1" },
      created_at: "2026-06-30T07:43:00Z",
      read_at: null,
      resolved_at: null,
    },
  ], t);

  assert.equal(messages.length, 1);
  assert.equal(messages[0].type, "approval_required");
  assert.equal(messages[0].icon, "shield");
  assert.equal(messages[0].title, "Approval required");
  assert.equal(messages[0].body, "A run is waiting for your approval.");
  assert.equal(messages[0].href, "/chat/thread%2F1");
  assert.equal(messages[0].read, false);
});

test("a kind the frontend does not know yet still presents", () => {
  const messages = notificationMessages([
    {
      id: "notification-1",
      kind: "quota_exhausted",
      severity: "warning",
      action: { kind: "open_thread", thread_id: "thread-1" },
      created_at: "2026-06-30T07:43:00Z",
      read_at: null,
      resolved_at: null,
    },
  ], t);

  /* The backend can ship a kind before this map learns it, so the fallback is
   * reachable in production rather than only defensive. */
  assert.equal(messages.length, 1);
  assert.equal(messages[0].type, "quota_exhausted");
  assert.equal(messages[0].icon, "bell");
  assert.equal(messages[0].title, "notifications.generic.title");
  assert.equal(messages[0].body, "notifications.generic.body");
  assert.equal(messages[0].href, "/chat/thread-1");
});

test("a run completion carries the run and thread the acknowledgement needs", () => {
  const messages = notificationMessages([
    {
      id: "notification-1",
      kind: "run_completed",
      severity: "info",
      action: { kind: "open_thread", thread_id: "thread-1" },
      turn_run_id: "run-1",
      created_at: "2026-06-30T07:43:00Z",
      read_at: null,
      resolved_at: null,
    },
  ], t);

  /* The deferred acknowledgement matches on exactly these two fields, so
   * dropping either from the presentation contract would send the completion
   * notification back to dismissing itself on open, silently. */
  assert.equal(messages.length, 1);
  assert.equal(messages[0].threadId, "thread-1");
  assert.equal(messages[0].turnRunId, "run-1");
});

test("notificationMessages preserves resolved records and sorts newest first", () => {
  const messages = notificationMessages([
    {
      id: "older",
      kind: "run_failed",
      action: { kind: "open_thread", thread_id: "thread-old" },
      created_at: "2026-06-30T07:43:00Z",
      read_at: "2026-06-30T07:44:00Z",
      resolved_at: "2026-06-30T07:44:00Z",
    },
    {
      id: "newer",
      kind: "run_failed",
      action: { kind: "open_thread", thread_id: "thread-new" },
      created_at: "2026-06-30T08:43:00Z",
      read_at: null,
      resolved_at: null,
    },
  ], t);

  assert.deepEqual(messages.map((message) => message.id), ["newer", "older"]);
  assert.equal(messages[1].detail, "Resolved");
  assert.equal(messages[1].read, true);
});

test("notificationMessages does not trust arbitrary action URLs", () => {
  const [message] = notificationMessages([
    {
      id: "unsafe-action",
      kind: "run_failed",
      action: { kind: "open_url", url: "https://example.invalid" },
      created_at: "2026-06-30T08:43:00Z",
    },
  ], t);
  assert.equal(message.href, null);
});

test("approval compatibility messages keep freshness and typed thread navigation", () => {
  const messages = approvalThreadNotifications([
    {
      id: "thread/1",
      title: "Daily report",
      state: "awaiting_approval",
      gate_ref: "gate-1",
      updated_at: "2026-06-30T07:43:00Z",
    },
  ], new Map(), t);

  assert.equal(messages.length, 1);
  assert.equal(messages[0].id, "approval:thread/1:gate-1");
  assert.equal(messages[0].href, "/chat/thread%2F1");
  assert.equal(
    approvalThreadNotificationId({ id: "thread/1", gate_ref: "gate-1" }),
    "approval:thread/1:gate-1",
  );
});

test("compatibility seen state remains scoped and bounded", () => {
  const firstScope = "tenant:first";
  const secondScope = "tenant:second";
  markNotificationIdsSeen(
    Array.from({ length: 260 }, (_, index) => `notification-${index}`),
    firstScope,
  );
  assert.equal(getNotificationState(firstScope).seenIds.size, 250);
  assert.equal(getNotificationState(secondScope).seenIds.size, 0);
});
