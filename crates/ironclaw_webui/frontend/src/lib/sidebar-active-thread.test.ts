import assert from "node:assert/strict";
import { test } from "vitest";

import {
  activeRouteThreadIdFromPath,
  routeSynchronizedThreadsState,
} from "./sidebar-active-thread";

test("activeRouteThreadIdFromPath returns the chat route thread id", () => {
  assert.equal(activeRouteThreadIdFromPath("/chat/thread-123"), "thread-123");
  assert.equal(activeRouteThreadIdFromPath("/chat/thread-123/"), "thread-123");
  assert.equal(activeRouteThreadIdFromPath("/chat/thread%20one"), "thread one");
  assert.equal(activeRouteThreadIdFromPath("/chat/thread-123?ref=sidebar"), "thread-123");
  assert.equal(activeRouteThreadIdFromPath("/chat/thread-123#bottom"), "thread-123");
});

test("activeRouteThreadIdFromPath clears outside chat thread routes", () => {
  assert.equal(activeRouteThreadIdFromPath("/chat"), null);
  assert.equal(activeRouteThreadIdFromPath("/automations"), null);
  assert.equal(activeRouteThreadIdFromPath("/workspace"), null);
  assert.equal(activeRouteThreadIdFromPath("/settings/inference"), null);
});

test("activeRouteThreadIdFromPath ignores nested non-chat-thread routes", () => {
  assert.equal(activeRouteThreadIdFromPath("/projects/project-1/threads/thread-123"), null);
  assert.equal(activeRouteThreadIdFromPath("/chat/thread-123/details"), null);
});

test("routeSynchronizedThreadsState uses the route as the active thread source", () => {
  const deleteThread = () => {};
  const loadMore = () => {};
  const threadsState = {
    activeThreadId: "stale-thread",
    threads: [{ id: "route-thread" }],
    isCreating: false,
    hasMore: true,
    isLoadingMore: false,
    loadMoreError: new Error("page failed"),
    loadMore,
    deleteThread,
  };

  const onChatRoute = routeSynchronizedThreadsState(threadsState, "/chat/route-thread");
  assert.equal(onChatRoute.activeThreadId, "route-thread");
  assert.equal(onChatRoute.threads, threadsState.threads);
  assert.equal(onChatRoute.hasMore, true);
  assert.equal(onChatRoute.isLoadingMore, false);
  assert.equal(onChatRoute.loadMoreError, threadsState.loadMoreError);
  assert.equal(onChatRoute.loadMore, loadMore);
  assert.equal(onChatRoute.deleteThread, deleteThread);

  const outsideChat = routeSynchronizedThreadsState(threadsState, "/automations");
  assert.equal(outsideChat.activeThreadId, null);
  assert.equal(outsideChat.loadMore, loadMore);
});
