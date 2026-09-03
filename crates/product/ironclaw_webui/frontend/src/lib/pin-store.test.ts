// Unit tests for the client-side pinned-thread store.
//
// Run with Node's built-in test runner (no extra deps):
//   pnpm test -- lib/pin-store.test.ts
//
// NOTE: `build.rs` deliberately excludes `*.test.ts` from the embedded
// asset bundle, so this file is never served to the browser.

import assert from "node:assert/strict";
import { afterEach, beforeEach, test } from "vitest";
import {
  createMemoryStorage,
  replaceBrowserGlobal,
} from "../test-support/browser-mocks";
import { setAuthScope } from "./auth-scope";
import {
  clearAllPins,
  getPinnedIds,
  isPinned,
  subscribePins,
  togglePin,
} from "./pin-store";

// Minimal localStorage stub. The store reads `window.localStorage` lazily, so
// installing it on the global before the calls is enough.
let restoreWindow: (() => void) | null = null;

function installStorage() {
  const storage = createMemoryStorage();
  restoreWindow = replaceBrowserGlobal("window", { localStorage: storage });
  return storage;
}

beforeEach(() => {
  installStorage();
  setAuthScope(null);
  // Reset module state between tests (the in-memory Set persists per process).
  clearAllPins();
});

afterEach(() => {
  restoreWindow?.();
  restoreWindow = null;
});

test("togglePin round-trips with isPinned", () => {
  assert.equal(isPinned("t1"), false);
  togglePin("t1");
  assert.equal(isPinned("t1"), true);
  togglePin("t1");
  assert.equal(isPinned("t1"), false);
});

test("getPinnedIds returns a snapshot that can't mutate the store", () => {
  togglePin("t1");
  const snap = getPinnedIds();
  snap.add("t2");
  assert.equal(isPinned("t2"), false, "mutating the snapshot must not pin t2");
});

test("a falsy thread id is a no-op", () => {
  togglePin("");
  togglePin(null);
  togglePin(undefined);
  assert.equal(getPinnedIds().size, 0);
});

test("subscribePins fires on change and unsubscribe stops it", () => {
  let calls = 0;
  const unsub = subscribePins(() => {
    calls += 1;
  });
  togglePin("t1");
  assert.equal(calls, 1);
  unsub();
  togglePin("t2");
  assert.equal(calls, 1, "no further notifications after unsubscribe");
});

test("pins are isolated per authenticated user and persist across a return", () => {
  setAuthScope({ tenant_id: "t", user_id: "user-A" });
  togglePin("thread-A");
  assert.equal(isPinned("thread-A"), true);

  setAuthScope({ tenant_id: "t", user_id: "user-B" });
  assert.equal(isPinned("thread-A"), false, "user B must not see user A's pin");

  // Back to A: the pin is reloaded from A's namespaced storage.
  setAuthScope({ tenant_id: "t", user_id: "user-A" });
  assert.equal(isPinned("thread-A"), true);
});

test("clearAllPins resets the set and removes pin keys but leaves others", () => {
  setAuthScope({ tenant_id: "t", user_id: "user-A" });
  togglePin("thread-A");
  globalThis.window.localStorage.setItem("ironclaw:unrelated", "keep");
  clearAllPins();
  assert.equal(isPinned("thread-A"), false);
  assert.equal(
    globalThis.window.localStorage.getItem("ironclaw:unrelated"),
    "keep"
  );
});

test("storage failures are swallowed (in-memory still works)", () => {
  restoreWindow?.();
  restoreWindow = replaceBrowserGlobal("window", {
    localStorage: {
      getItem: () => {
        throw new Error("quota / private mode");
      },
      setItem: () => {
        throw new Error("quota / private mode");
      },
      removeItem: () => {
        throw new Error("quota / private mode");
      },
      clear: () => {
        throw new Error("quota / private mode");
      },
      get length(): number {
        throw new Error("quota / private mode");
      },
      key: () => {
        throw new Error("quota / private mode");
      },
    } satisfies Storage,
  });
  assert.doesNotThrow(() => togglePin("t1"));
  assert.equal(isPinned("t1"), true, "in-memory pin works without storage");
});
