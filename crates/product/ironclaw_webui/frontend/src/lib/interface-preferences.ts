import React from "react";

// `ironclaw:v2-*` is the WebUI v2 browser-local preference namespace.
export const CHAT_LOGS_SHORTCUT_STORAGE_KEY = "ironclaw:v2-chat-logs-shortcut";
const STORED_BOOLEAN_TRUE = "true";
const STORED_BOOLEAN_FALSE = "false";

type PreferenceStorage = Pick<Storage, "getItem" | "setItem">;

function browserWindow(): Window | null {
  return typeof window === "undefined" ? null : window;
}

function browserStorage(): Storage | null {
  try {
    return browserWindow()?.localStorage || null;
  } catch (_) {
    return null;
  }
}

function parseStoredBoolean(
  value: string | null | undefined,
  defaultValue: boolean,
): boolean {
  if (value === STORED_BOOLEAN_TRUE) return true;
  if (value === STORED_BOOLEAN_FALSE) return false;
  return defaultValue;
}

export function readShowChatLogsShortcut(
  storage: Pick<PreferenceStorage, "getItem"> | null = browserStorage(),
): boolean {
  try {
    return parseStoredBoolean(
      storage?.getItem(CHAT_LOGS_SHORTCUT_STORAGE_KEY),
      true
    );
  } catch (_) {
    return true;
  }
}

export function writeShowChatLogsShortcut(
  show: boolean,
  storage: Pick<PreferenceStorage, "setItem"> | null = browserStorage(),
): void {
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

  const setShowChatLogsShortcut = React.useCallback((show: boolean) => {
    const next = Boolean(show);
    setShowChatLogsShortcutState(next);
    writeShowChatLogsShortcut(next);
  }, []);

  React.useEffect(() => {
    const win = browserWindow();
    if (!win?.addEventListener) return undefined;
    const onStorage = (event: StorageEvent) => {
      if (event.key !== CHAT_LOGS_SHORTCUT_STORAGE_KEY) return;
      setShowChatLogsShortcutState(parseStoredBoolean(event.newValue, true));
    };
    win.addEventListener("storage", onStorage);
    return () => win.removeEventListener("storage", onStorage);
  }, []);

  return { showChatLogsShortcut, setShowChatLogsShortcut };
}
