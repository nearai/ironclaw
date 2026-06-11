// Unit tests for the per-conversation composer draft store (#4724).
//
// Run with Node's built-in test runner (no extra deps):
//   node --test crates/ironclaw_webui_v2_static/static/js/pages/chat/lib/draft-store.test.js
//
// NOTE: `build.rs` deliberately excludes `*.test.js` from the embedded
// asset bundle, so this file is never served to the browser.

import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";
import {
  NEW_DRAFT_KEY,
  clearDraft,
  getDraft,
  setDraft,
} from "./draft-store.js";

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
    },
  };
  return map;
}

beforeEach(() => {
  installStorage();
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
