// @ts-nocheck
import { React } from "./html.js";

export const CHAT_LOGS_SHORTCUT_STORAGE_KEY = "ironclaw:v2-chat-logs-shortcut";

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

export function readShowChatLogsShortcut(storage = browserStorage()) {
  try {
    return storage?.getItem(CHAT_LOGS_SHORTCUT_STORAGE_KEY) !== "false";
  } catch (_) {
    return true;
  }
}

export function writeShowChatLogsShortcut(show, storage = browserStorage()) {
  try {
    storage?.setItem(CHAT_LOGS_SHORTCUT_STORAGE_KEY, show ? "true" : "false");
  } catch (_) {
    // Best-effort UI preference; storage failures should not block chat.
  }
}

export function useInterfacePreferences() {
  const [showChatLogsShortcut, setShowChatLogsShortcutState] = React.useState(
    readShowChatLogsShortcut
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
      setShowChatLogsShortcutState(event.newValue !== "false");
    };
    win.addEventListener("storage", onStorage);
    return () => win.removeEventListener("storage", onStorage);
  }, []);

  return { showChatLogsShortcut, setShowChatLogsShortcut };
}
