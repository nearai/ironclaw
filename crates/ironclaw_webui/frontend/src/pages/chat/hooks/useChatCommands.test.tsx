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
import { setAuthScope } from "../../../lib/auth-scope";

const api = vi.hoisted(() => ({
  listChatCommands: vi.fn(),
}));

vi.mock("../../../lib/api", () => api);

import { clearChatCommandsCache, useChatCommands } from "./useChatCommands";

beforeEach(() => {
  vi.clearAllMocks();
  clearChatCommandsCache();
  setAuthScope(null);
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

test("an in-flight fetch from a since-abandoned identity must not poison the cache for a later mount", async () => {
  // Race in the finding: admin A's fetch is in flight when the
  // identity-change purge fires (a no-op here — the cache is still empty at
  // that point) and the swap moves on to member B. A's fetch is the only
  // thing that could still write the cache, so the `.then()` itself must
  // refuse to write a response for an identity the app has already left.
  const adminCommands = [
    { name: "admin-only", title: "Admin only", usage: "/admin-only" },
  ];
  const memberCommands = [{ name: "status", title: "Status", usage: "/status" }];

  let resolveStaleFetch;
  const staleFetch = new Promise((resolve) => {
    resolveStaleFetch = resolve;
  });
  api.listChatCommands
    .mockImplementationOnce(() => staleFetch)
    .mockResolvedValueOnce({ commands: memberCommands });

  setAuthScope({ tenant_id: "t", user_id: "admin-a" });

  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  let latest = null;
  const onCommands = (commands) => {
    latest = commands;
  };

  try {
    // Identity A (admin) mounts; its fetch is deliberately left in flight.
    act(() => {
      root.render(<Probe key="identity-a" onCommands={onCommands} />);
    });
    await flush();
    assert.equal(api.listChatCommands.mock.calls.length, 1);

    // The identity-change purge fires while A's fetch is still pending — the
    // purge itself can't help (the cache is already empty), so only the
    // `.then()`'s own scope check can protect what happens next.
    clearChatCommandsCache();
    setAuthScope({ tenant_id: "t", user_id: "member-b" });

    // A's stale fetch resolves only now, after the identity has already
    // moved on.
    resolveStaleFetch({ commands: adminCommands });
    await flush();

    // A later mount under the new identity must not read a cache poisoned by
    // A's discarded response — it must issue its own (second) fetch and
    // serve that result instead.
    act(() => {
      root.render(<Probe key="identity-b" onCommands={onCommands} />);
    });
    await flush();

    assert.equal(
      api.listChatCommands.mock.calls.length,
      2,
      "the discarded response must not satisfy the next mount from cache — a second fetch happens instead",
    );
    assert.deepEqual(
      latest,
      memberCommands,
      "the later mount serves its own identity's inventory, never the discarded cross-identity response",
    );
  } finally {
    act(() => {
      root.unmount();
    });
    container.remove();
  }
});
