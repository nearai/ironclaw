// @vitest-environment happy-dom
//
// Regression coverage for the module-level command-inventory cache: the
// inventory is role-filtered per caller (see useChatCommands.ts), so the
// cache must not survive an in-tab identity swap. `clearChatCommandsCache()`
// is called from the auth identity-change purge effect
// (`app/auth.ts`, beside `clearHistoryCache()`/`clearAllDrafts()`/
// `clearAllPins()`); after that purge, `RequireAuth` (app/app.tsx) briefly
// renders `<AuthLoading />` while the new session resolves, unmounting and
// then remounting the authenticated subtree — so the next mount of this hook
// must refetch rather than keep serving the previous identity's cached list.

import assert from "node:assert/strict";
import { beforeEach, test, vi } from "vitest";
import React, { act } from "react";
import { createRoot } from "react-dom/client";

const api = vi.hoisted(() => ({
  listChatCommands: vi.fn(),
}));

vi.mock("../../../lib/api", () => api);

import { clearChatCommandsCache, useChatCommands } from "./useChatCommands";

beforeEach(() => {
  vi.clearAllMocks();
  clearChatCommandsCache();
});

function Probe({ onCommands }) {
  const commands = useChatCommands();
  onCommands(commands);
  return null;
}

async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

test("useChatCommands refetches after clearChatCommandsCache instead of serving the previous identity's cached inventory", async () => {
  const adminCommands = [
    { name: "admin-only", title: "Admin only", usage: "/admin-only" },
  ];
  const memberCommands = [{ name: "status", title: "Status", usage: "/status" }];
  api.listChatCommands
    .mockResolvedValueOnce({ commands: adminCommands })
    .mockResolvedValueOnce({ commands: memberCommands });

  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  let latest = null;
  const onCommands = (commands) => {
    latest = commands;
  };

  try {
    act(() => {
      root.render(<Probe key="identity-a" onCommands={onCommands} />);
    });
    await flush();

    assert.deepEqual(
      latest,
      adminCommands,
      "first mount serves the fetched inventory for the resolved identity",
    );
    assert.equal(api.listChatCommands.mock.calls.length, 1);

    // Simulate the identity-change purge, then the natural remount
    // RequireAuth performs while the new session resolves.
    clearChatCommandsCache();
    act(() => {
      root.render(<Probe key="identity-b" onCommands={onCommands} />);
    });
    await flush();

    assert.deepEqual(
      latest,
      memberCommands,
      "remount after the purge must refetch, not keep serving the previous identity's cached inventory",
    );
    assert.equal(
      api.listChatCommands.mock.calls.length,
      2,
      "the cleared cache forces a second fetch instead of reusing the stale one",
    );
  } finally {
    act(() => {
      root.unmount();
    });
    container.remove();
  }
});
