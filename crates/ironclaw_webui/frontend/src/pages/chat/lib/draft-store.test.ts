// @ts-nocheck
// Unit tests for the per-conversation composer draft store.
//
// Run with Node's built-in test runner (no extra deps):
//   pnpm test -- pages/chat/lib/draft-store.test.ts
//
// NOTE: `build.rs` deliberately excludes `*.test.ts` from the embedded
// asset bundle, so this file is never served to the browser.

import assert from "node:assert/strict";
import { beforeEach, test } from "vitest";
import { setAuthScope } from "../../../lib/auth-scope";
import {
  NEW_DRAFT_KEY,
  clearAllDrafts,
  clearDraft,
  getDraft,
  setDraft,
} from "./draft-store";

// Minimal localStorage stub — the store reads `window.localStorage` lazily
// inside each function, so installing it on the global before the calls is
// enough (the module never touches storage at import time).
function installStorage() {
  const map = new Map();
  globalThis.window = {
    localStorage: {
      getItem: (k) => (map.has(k) ? map.get(k) : null),
      setItem: (k, v) => map.set(k, String(v)),
      removeItem: (k) => map.delete(k),
      get length() {
        return map.size;
      },
      key: (i) => [...map.keys()][i] ?? null,
    },
  };
  return map;
}

beforeEach(() => {
  installStorage();
  setAuthScope(null);
});

test("drafts are isolated per authenticated user across a session switch", () => {
  // Regression: signing out and a different user signing into the same tab
  // must not restore the previous user's unsent draft (the new-conversation
  // slot is shared by key, so identity scoping is what isolates it).
  setAuthScope({ tenant_id: "t1", user_id: "user-A" });
  setDraft(NEW_DRAFT_KEY, "user A's secret draft");

  setAuthScope({ tenant_id: "t1", user_id: "user-B" });
  assert.equal(getDraft(NEW_DRAFT_KEY), "", "user B must not see user A's draft");
  setDraft(NEW_DRAFT_KEY, "user B's draft");

  setAuthScope({ tenant_id: "t1", user_id: "user-A" });
  assert.equal(
    getDraft(NEW_DRAFT_KEY),
    "user A's secret draft",
    "user A's own draft is still scoped to A"
  );
});

test("getDraft returns an empty string when nothing is stored", () => {
  assert.equal(getDraft("thread-1"), "");
});

test("setDraft round-trips a draft for a thread key", () => {
  setDraft("thread-1", "half-written message");
  assert.equal(getDraft("thread-1"), "half-written message");
});

test("drafts are scoped per key and do not leak across conversations", () => {
  setDraft("thread-1", "draft A");
  setDraft("thread-2", "draft B");
  assert.equal(getDraft("thread-1"), "draft A");
  assert.equal(getDraft("thread-2"), "draft B");
});

test("the new-conversation slot is addressable via NEW_DRAFT_KEY", () => {
  setDraft(NEW_DRAFT_KEY, "unsent + New draft");
  assert.equal(getDraft(NEW_DRAFT_KEY), "unsent + New draft");
});

test("a falsy key falls back to the new-conversation slot", () => {
  setDraft(undefined, "fallback draft");
  assert.equal(getDraft(undefined), "fallback draft");
  // Same underlying slot as NEW_DRAFT_KEY.
  assert.equal(getDraft(NEW_DRAFT_KEY), "fallback draft");
});

test("setDraft with empty text clears the slot (so it isn't restored)", () => {
  setDraft("thread-1", "something");
  setDraft("thread-1", "");
  assert.equal(getDraft("thread-1"), "");
});

test("clearDraft removes a stored draft", () => {
  setDraft("thread-1", "to be sent");
  clearDraft("thread-1");
  assert.equal(getDraft("thread-1"), "");
});

test("clearAllDrafts removes every draft but leaves unrelated keys", () => {
  setDraft("thread-1", "a");
  setDraft(NEW_DRAFT_KEY, "b");
  globalThis.window.localStorage.setItem("ironclaw:unrelated", "keep");
  clearAllDrafts();
  assert.equal(getDraft("thread-1"), "");
  assert.equal(getDraft(NEW_DRAFT_KEY), "");
  assert.equal(
    globalThis.window.localStorage.getItem("ironclaw:unrelated"),
    "keep"
  );
});

test("storage failures are swallowed (best-effort persistence)", () => {
  globalThis.window = {
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
    },
  };
  assert.doesNotThrow(() => setDraft("thread-1", "x"));
  assert.equal(getDraft("thread-1"), "");
  assert.doesNotThrow(() => clearDraft("thread-1"));
});
