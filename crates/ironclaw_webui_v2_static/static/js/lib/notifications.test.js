// Run with:
//   node --test crates/ironclaw_webui_v2_static/static/js/lib/notifications.test.js

import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";
import { setAuthScope } from "./auth-scope.js";
import {
  automationRunNotificationId,
  automationRunNotifications,
  ensureNotificationBaseline,
  getNotificationState,
  markNotificationIdsSeen,
} from "./notifications.js";

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

test("automationRunNotifications returns generic clickable automation messages", () => {
  const messages = automationRunNotifications(
    [
      {
        automation_id: "auto-1",
        display_name: "Daily report",
        recent_runs: [
          {
            run_id: "run-1",
            thread_id: "thread-1",
            chat_path: "/chat/thread-1",
            status_label: "Running",
            fired_label: "Jun 30, 3:43 PM",
            timestamp: 20,
          },
        ],
      },
    ],
    (key) => ({ "notifications.automation.title": "Automation task started" })[key] || key,
  );

  assert.equal(messages.length, 1);
  assert.equal(messages[0].type, "automation");
  assert.equal(messages[0].title, "Automation task started");
  assert.equal(messages[0].body, "Daily report");
  assert.equal(messages[0].href, "/chat/thread-1");
});

test("automationRunNotificationId falls back to thread and timestamp keys", () => {
  const id = automationRunNotificationId(
    { automation_id: "auto-1" },
    { thread_id: "thread-1", timestamp_source: "2026-06-30T00:00:00Z" },
  );
  assert.equal(id, "automation:auto-1:run:thread-1");

  const fallback = automationRunNotificationId(
    { automation_id: "auto-1" },
    { timestamp_source: "2026-06-30T00:00:00Z" },
  );
  assert.equal(fallback, "automation:auto-1:run:2026-06-30T00:00:00Z");
});

test("first baseline marks existing messages seen; later ids remain unread", () => {
  let state = getNotificationState();
  assert.equal(state.initialized, false);

  state = ensureNotificationBaseline(["m1", "m2"]);
  assert.equal(state.initialized, true);
  assert.equal(state.seenIds.has("m1"), true);
  assert.equal(state.seenIds.has("m2"), true);
  assert.equal(state.seenIds.has("m3"), false);

  state = ensureNotificationBaseline(["m1", "m2", "m3"]);
  assert.equal(state.seenIds.has("m3"), false);

  state = markNotificationIdsSeen(["m3"]);
  assert.equal(state.seenIds.has("m3"), true);
});

test("notification persistence is optional outside the browser", () => {
  delete globalThis.window;
  setAuthScope({ tenant_id: "tenant", user_id: `node-${testScopeId}` });

  let state = getNotificationState();
  assert.equal(state.initialized, false);

  state = ensureNotificationBaseline(["m1"]);
  assert.equal(state.initialized, true);
  assert.equal(state.seenIds.has("m1"), true);

  state = markNotificationIdsSeen(["m2"]);
  assert.equal(state.seenIds.has("m2"), true);
});

test("explicit notification scopes stay isolated from auth scope", () => {
  setAuthScope({ tenant_id: "tenant", user_id: `auth-${testScopeId}` });

  ensureNotificationBaseline(["profile-message"], "tenant:profile-user");
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
