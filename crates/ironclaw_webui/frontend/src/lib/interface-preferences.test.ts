// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";

import { runVmModuleForTest } from "../test-support/vm-module-harness";

function loadPreferenceHelpers() {
  return runVmModuleForTest(
    "./interface-preferences.ts",
    [
      "CHAT_LOGS_SHORTCUT_STORAGE_KEY",
      "readShowChatLogsShortcut",
      "writeShowChatLogsShortcut",
    ],
    {
      React: {
        useCallback: (fn) => fn,
        useEffect: () => {},
        useState: (initial) => [
          typeof initial === "function" ? initial() : initial,
          () => {},
        ],
      },
    },
    import.meta.url
  );
}

function createLocalStorage(initial = {}) {
  const values = new Map(Object.entries(initial));
  return {
    getItem: (key) => (values.has(key) ? values.get(key) : null),
    setItem: (key, value) => values.set(key, String(value)),
    dump: () => Object.fromEntries(values.entries()),
  };
}

test("readShowChatLogsShortcut defaults to visible unless stored false", () => {
  const { CHAT_LOGS_SHORTCUT_STORAGE_KEY, readShowChatLogsShortcut } =
    loadPreferenceHelpers();

  assert.equal(readShowChatLogsShortcut(createLocalStorage()), true);
  assert.equal(
    readShowChatLogsShortcut(
      createLocalStorage({ [CHAT_LOGS_SHORTCUT_STORAGE_KEY]: "false" })
    ),
    false
  );
  assert.equal(
    readShowChatLogsShortcut(
      createLocalStorage({ [CHAT_LOGS_SHORTCUT_STORAGE_KEY]: "true" })
    ),
    true
  );
  assert.equal(
    readShowChatLogsShortcut(
      createLocalStorage({ [CHAT_LOGS_SHORTCUT_STORAGE_KEY]: "unexpected" })
    ),
    true
  );
});

test("useInterfacePreferences initializes from localStorage as a boolean", () => {
  const localStorage = createLocalStorage();
  const { CHAT_LOGS_SHORTCUT_STORAGE_KEY, useInterfacePreferences } =
    runVmModuleForTest(
      "./interface-preferences.ts",
      ["CHAT_LOGS_SHORTCUT_STORAGE_KEY", "useInterfacePreferences"],
      {
        React: {
          useCallback: (fn) => fn,
          useEffect: () => {},
          useState: (initial) => [
            typeof initial === "function" ? initial() : initial,
            () => {},
          ],
        },
        window: { localStorage },
      },
      import.meta.url
    );
  localStorage.setItem(CHAT_LOGS_SHORTCUT_STORAGE_KEY, "false");

  const preferences = useInterfacePreferences();
  assert.equal(preferences.showChatLogsShortcut, false);
  assert.equal(typeof preferences.showChatLogsShortcut, "boolean");
  assert.equal(typeof preferences.setShowChatLogsShortcut, "function");
});

test("writeShowChatLogsShortcut persists the chat terminal shortcut preference", () => {
  const { CHAT_LOGS_SHORTCUT_STORAGE_KEY, writeShowChatLogsShortcut } =
    loadPreferenceHelpers();
  const storage = createLocalStorage();

  writeShowChatLogsShortcut(false, storage);
  assert.equal(storage.dump()[CHAT_LOGS_SHORTCUT_STORAGE_KEY], "false");

  writeShowChatLogsShortcut(true, storage);
  assert.equal(storage.dump()[CHAT_LOGS_SHORTCUT_STORAGE_KEY], "true");
});
