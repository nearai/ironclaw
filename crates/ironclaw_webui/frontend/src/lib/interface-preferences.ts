// @ts-nocheck
import React from "react";

// `ironclaw:v2-*` is the WebUI v2 browser-local preference namespace.
export const CHAT_LOGS_SHORTCUT_STORAGE_KEY = "ironclaw:v2-chat-logs-shortcut";
const STORED_BOOLEAN_TRUE = "true";
const STORED_BOOLEAN_FALSE = "false";

function browserWindow() {
  return typeof window === "undefined" ? null : window;
}

function browserStorage() {
  try {
    return browserWindow()?.localStorage || null;
  } catch (_) {
    return null;
  }
}

function parseStoredBoolean(value, defaultValue) {
  if (value === STORED_BOOLEAN_TRUE) return true;
  if (value === STORED_BOOLEAN_FALSE) return false;
  return defaultValue;
}

export function readShowChatLogsShortcut(storage = browserStorage()) {
  try {
    return parseStoredBoolean(
      storage?.getItem(CHAT_LOGS_SHORTCUT_STORAGE_KEY),
      true
    );
  } catch (_) {
    return true;
  }
}

export function writeShowChatLogsShortcut(show, storage = browserStorage()) {
  try {
    storage?.setItem(
      CHAT_LOGS_SHORTCUT_STORAGE_KEY,
      show ? STORED_BOOLEAN_TRUE : STORED_BOOLEAN_FALSE
    );
  } catch (_) {
    // Best-effort UI preference; storage failures should not block chat.
  }
}

export function useInterfacePreferences() {
  const [showChatLogsShortcut, setShowChatLogsShortcutState] = React.useState(
    () => readShowChatLogsShortcut()
  );

  const setShowChatLogsShortcut = React.useCallback((show) => {
    const next = Boolean(show);
    setShowChatLogsShortcutState(next);
    writeShowChatLogsShortcut(next);
  }, []);

  React.useEffect(() => {
    const win = browserWindow();
    if (!win?.addEventListener) return undefined;
    const onStorage = (event) => {
      if (event.key !== CHAT_LOGS_SHORTCUT_STORAGE_KEY) return;
      setShowChatLogsShortcutState(parseStoredBoolean(event.newValue, true));
    };
    win.addEventListener("storage", onStorage);
    return () => win.removeEventListener("storage", onStorage);
  }, []);

  return { showChatLogsShortcut, setShowChatLogsShortcut };
}
