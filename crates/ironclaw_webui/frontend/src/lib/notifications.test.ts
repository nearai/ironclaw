// @ts-nocheck
// Run with:
//   pnpm test -- lib/notifications.test.ts

import assert from "node:assert/strict";
import { beforeEach, test } from "vitest";
import { setAuthScope } from "./auth-scope";
import {
  approvalThreadNotificationId,
  approvalThreadNotifications,
  getNotificationState,
  isApprovalThread,
  markNotificationIdsSeen,
} from "./notifications";

let testScopeId = 0;

function installStorage() {
  const map = new Map();
  globalThis.window = {
    localStorage: {
      getItem: (key) => (map.has(key) ? map.get(key) : null),
      setItem: (key, value) => map.set(key, String(value)),
      removeItem: (key) => map.delete(key),
      get length() {
        return map.size;
      },
      key: (index) => [...map.keys()][index] ?? null,
    },
  };
}

beforeEach(() => {
  installStorage();
  testScopeId += 1;
  setAuthScope({ tenant_id: "tenant", user_id: `user-${testScopeId}` });
});

test("approvalThreadNotifications returns generic clickable approval messages", () => {
  const messages = approvalThreadNotifications(
    [
      {
        id: "thread-1",
        title: "Daily report",
        state: "awaiting_approval",
        updated_at: "2026-06-30T07:43:00Z",
      },
    ],
    new Map(),
    (key) =>
      ({
        "notifications.approval.title": "Approval required",
        "notifications.approval.detail": "Needs your approval",
      })[key] || key,
  );

  assert.equal(messages.length, 1);
  assert.equal(messages[0].type, "approval");
  assert.equal(messages[0].title, "Approval required");
  assert.equal(messages[0].body, "Daily report");
  assert.equal(messages[0].href, "/chat/thread-1");
});

test("approvalThreadNotificationId includes approval freshness", () => {
  const id = approvalThreadNotificationId({
    thread_id: "thread-1",
    updated_at: "2026-06-30T00:00:00Z",
  });
  assert.equal(id, "approval:thread-1:2026-06-30T00%3A00%3A00Z");

  const runScoped = approvalThreadNotificationId({
    thread_id: "thread-1",
    run_id: "run-123",
    updated_at: "2026-06-30T00:00:00Z",
  });
  assert.equal(runScoped, "approval:thread-1:run-123");

  const fallback = approvalThreadNotificationId({ id: "thread-1" });
  assert.equal(fallback, "approval:thread-1:pending");
});

test("approvalThreadNotifications can use local thread state", () => {
  assert.equal(isApprovalThread({ state: "idle" }, "needs_attention"), true);
  assert.equal(isApprovalThread({ state: "awaiting_approval" }, "running"), true);

  const messages = approvalThreadNotifications(
    [{ id: "thread-1", title: "Open gate", state: "idle" }],
    new Map([["thread-1", "needs_attention"]]),
    (key) => key,
  );

  assert.equal(messages.length, 1);
  assert.equal(messages[0].body, "Open gate");
});

test("dismissed notification ids are persisted as seen ids", () => {
  let state = getNotificationState();
  assert.equal(state.initialized, false);

  state = markNotificationIdsSeen(["m1", "m2"]);
  assert.equal(state.initialized, true);
  assert.equal(state.seenIds.has("m1"), true);
  assert.equal(state.seenIds.has("m2"), true);
  assert.equal(state.seenIds.has("m3"), false);

  state = markNotificationIdsSeen(["m3"]);
  assert.equal(state.seenIds.has("m3"), true);
});

test("notification persistence is optional outside the browser", () => {
  delete globalThis.window;
  setAuthScope({ tenant_id: "tenant", user_id: `node-${testScopeId}` });

  let state = getNotificationState();
  assert.equal(state.initialized, false);

  state = markNotificationIdsSeen(["m1"]);
  assert.equal(state.initialized, true);
  assert.equal(state.seenIds.has("m1"), true);

  state = markNotificationIdsSeen(["m2"]);
  assert.equal(state.seenIds.has("m2"), true);
});

test("explicit notification scopes stay isolated from auth scope", () => {
  setAuthScope({ tenant_id: "tenant", user_id: `auth-${testScopeId}` });

  markNotificationIdsSeen(["profile-message"], "tenant:profile-user");
  markNotificationIdsSeen(["profile-read"], "tenant:profile-user");
  markNotificationIdsSeen(["auth-read"]);

  const profileState = getNotificationState("tenant:profile-user");
  assert.equal(profileState.seenIds.has("profile-message"), true);
  assert.equal(profileState.seenIds.has("profile-read"), true);
  assert.equal(profileState.seenIds.has("auth-read"), false);

  const authState = getNotificationState();
  assert.equal(authState.seenIds.has("auth-read"), true);
  assert.equal(authState.seenIds.has("profile-message"), false);
});

test("seen ids are capped in memory", () => {
  const ids = Array.from({ length: 260 }, (_, index) => `m${index}`);
  let state = markNotificationIdsSeen(ids);

  assert.equal(state.seenIds.size, 250);
  assert.equal(state.seenIds.has("m0"), false);
  assert.equal(state.seenIds.has("m9"), false);
  assert.equal(state.seenIds.has("m10"), true);
  assert.equal(state.seenIds.has("m259"), true);

  state = markNotificationIdsSeen(["m260"]);
  assert.equal(state.seenIds.size, 250);
  assert.equal(state.seenIds.has("m10"), false);
  assert.equal(state.seenIds.has("m260"), true);
});
