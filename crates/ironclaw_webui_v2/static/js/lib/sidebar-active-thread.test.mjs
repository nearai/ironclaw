import assert from "node:assert/strict";
import test from "node:test";

import { activeSidebarThreadIdFromPath } from "./sidebar-active-thread.js";

test("activeSidebarThreadIdFromPath returns the chat route thread id", () => {
  assert.equal(activeSidebarThreadIdFromPath("/chat/thread-123"), "thread-123");
  assert.equal(activeSidebarThreadIdFromPath("/chat/thread-123/"), "thread-123");
  assert.equal(activeSidebarThreadIdFromPath("/chat/thread%20one"), "thread one");
});

test("activeSidebarThreadIdFromPath clears outside chat thread routes", () => {
  assert.equal(activeSidebarThreadIdFromPath("/chat"), null);
  assert.equal(activeSidebarThreadIdFromPath("/automations"), null);
  assert.equal(activeSidebarThreadIdFromPath("/workspace"), null);
  assert.equal(activeSidebarThreadIdFromPath("/settings/inference"), null);
});

test("activeSidebarThreadIdFromPath ignores nested non-chat-thread routes", () => {
  assert.equal(activeSidebarThreadIdFromPath("/projects/project-1/threads/thread-123"), null);
  assert.equal(activeSidebarThreadIdFromPath("/chat/thread-123/details"), null);
});
