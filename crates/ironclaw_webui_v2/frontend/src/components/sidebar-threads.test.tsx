import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { test, vi } from "vitest";

vi.mock("react-router", async () => {
  const { createElement } = await import("react");
  return {
    NavLink: ({ children, to, onClick, className }) =>
      createElement(
        "a",
        {
          href: typeof to === "string" ? to : "#",
          onClick,
          className: typeof className === "function" ? className({ isActive: false }) : className,
        },
        children,
      ),
  };
});

vi.mock("../design-system/icons", async () => {
  const { createElement } = await import("react");
  return {
    Icon: ({ name, className }) =>
      createElement("span", { className, "data-icon": name }),
  };
});

vi.mock("../lib/i18n", () => ({ useT: () => (key) => key }));
vi.mock("../lib/pin-store", () => ({
  getPinnedIds: () => new Set(),
  subscribePins: () => () => {},
  togglePin: () => {},
}));
vi.mock("../lib/thread-state", () => ({
  THREAD_STATE: {
    FAILED: "failed",
    NEEDS_ATTENTION: "needs_attention",
    RUNNING: "running",
  },
  useThreadStates: () => new Map(),
}));

test("SidebarThreads exposes a visible delete action for listed threads", async () => {
  const { SidebarThreads } = await import("./sidebar-threads");
  const html = renderToStaticMarkup(
    <SidebarThreads
      threads={[
        {
          id: "thread-old",
          title: "Old investigation",
          created_at: "2026-07-01T12:00:00.000Z",
          updated_at: "2026-07-02T12:00:00.000Z",
        },
      ]}
      activeThreadId="thread-old"
      onSelect={() => {}}
      onDelete={() => {}}
      onNavigate={() => {}}
    />,
  );

  assert.match(html, /data-testid="thread-delete"/);
  assert.match(html, /data-thread-id="thread-old"/);
  assert.match(html, /aria-label="common.deleteChat"/);

  const deleteButton = html.match(/<button[^>]*data-testid="thread-delete"[^>]*>/)?.[0];
  assert.ok(deleteButton, "thread delete button should render in each thread row");
  assert.match(deleteButton, /\bopacity-70\b/);
  assert.doesNotMatch(
    deleteButton,
    /\bopacity-0\b/,
    "delete action must not be invisible until hover, because touch users cannot discover it",
  );
});
