import assert from "node:assert/strict";
import test from "node:test";

import {
  DESKTOP_SIDEBAR_STORAGE_KEY,
  currentSidebarOpen,
  isDesktopSidebarViewport,
  readDesktopSidebarOpen,
  toggleSidebarState,
  writeDesktopSidebarOpen,
} from "../lib/sidebar-state.js";

function createLocalStorage(initial = {}) {
  const values = new Map(Object.entries(initial));
  return {
    getItem: (key) => (values.has(key) ? values.get(key) : null),
    setItem: (key, value) => values.set(key, String(value)),
    removeItem: (key) => values.delete(key),
    dump: () => Object.fromEntries(values.entries()),
  };
}

test("readDesktopSidebarOpen defaults to open unless the stored value is false", () => {
  assert.equal(readDesktopSidebarOpen(createLocalStorage()), true);
  assert.equal(
    readDesktopSidebarOpen(
      createLocalStorage({ [DESKTOP_SIDEBAR_STORAGE_KEY]: "false" })
    ),
    false
  );
});

test("writeDesktopSidebarOpen persists the desktop preference", () => {
  const storage = createLocalStorage();

  writeDesktopSidebarOpen(false, storage);
  assert.equal(storage.dump()[DESKTOP_SIDEBAR_STORAGE_KEY], "false");

  writeDesktopSidebarOpen(true, storage);
  assert.equal(storage.dump()[DESKTOP_SIDEBAR_STORAGE_KEY], "true");
});

test("isDesktopSidebarViewport reads the responsive desktop breakpoint", () => {
  assert.equal(
    isDesktopSidebarViewport({ matchMedia: () => ({ matches: true }) }),
    true
  );
  assert.equal(
    isDesktopSidebarViewport({ matchMedia: () => ({ matches: false }) }),
    false
  );
});

test("toggleSidebarState targets only the active viewport state", () => {
  assert.deepEqual(
    toggleSidebarState({ mobileOpen: false, desktopOpen: true }, true),
    { mobileOpen: false, desktopOpen: false }
  );
  assert.deepEqual(
    toggleSidebarState({ mobileOpen: false, desktopOpen: true }, false),
    { mobileOpen: true, desktopOpen: true }
  );
});

test("currentSidebarOpen reports the state for the active viewport", () => {
  const state = { mobileOpen: false, desktopOpen: true };

  assert.equal(currentSidebarOpen(state, true), true);
  assert.equal(currentSidebarOpen(state, false), false);
});
