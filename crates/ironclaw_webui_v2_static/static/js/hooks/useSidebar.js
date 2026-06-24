import { React } from "../lib/html.js";
import { useNavigate } from "react-router";

const DESKTOP_SIDEBAR_STORAGE_KEY = "ironclaw:v2-sidebar-open";

export function readDesktopSidebarOpen() {
  try {
    return window.localStorage.getItem(DESKTOP_SIDEBAR_STORAGE_KEY) !== "false";
  } catch (_) {
    return true;
  }
}

function writeDesktopSidebarOpen(open) {
  try {
    window.localStorage.setItem(
      DESKTOP_SIDEBAR_STORAGE_KEY,
      open ? "true" : "false"
    );
  } catch (_) {
    // Best-effort: storage failures should never block navigation.
  }
}

export function isDesktopSidebarViewport() {
  try {
    return window.matchMedia("(min-width: 768px)").matches;
  } catch (_) {
    return false;
  }
}

export function toggleSidebarState(state, isDesktop) {
  return isDesktop
    ? { ...state, desktopOpen: !state.desktopOpen }
    : { ...state, mobileOpen: !state.mobileOpen };
}

export function useSidebar({ onNewChat } = {}) {
  const navigate = useNavigate();
  const [state, setState] = React.useState(() => ({
    mobileOpen: false,
    desktopOpen: readDesktopSidebarOpen(),
  }));

  React.useEffect(() => {
    writeDesktopSidebarOpen(state.desktopOpen);
  }, [state.desktopOpen]);

  const close = React.useCallback(() => {
    setState((current) => ({ ...current, mobileOpen: false }));
  }, []);

  const toggle = React.useCallback(() => {
    setState((current) =>
      toggleSidebarState(current, isDesktopSidebarViewport())
    );
  }, []);

  // "+ New" eagerly creates a thread because v2 requires a
  // pre-existing `thread_id` before `POST /threads/{id}/messages`
  // is accepted. The callback returns the new thread id; we route
  // to `/chat/<id>` so the composer is bound to the right thread
  // from the first keystroke.
  const newChat = React.useCallback(async () => {
    const result = await onNewChat?.();
    const newThreadId =
      typeof result === "string" && result.length > 0 ? result : null;
    navigate(newThreadId ? `/chat/${newThreadId}` : "/chat");
    close();
  }, [navigate, close, onNewChat]);

  const selectThread = React.useCallback(
    (id) => {
      navigate(`/chat/${id}`);
      close();
    },
    [navigate, close]
  );

  return {
    mobileOpen: state.mobileOpen,
    desktopOpen: state.desktopOpen,
    close,
    toggle,
    newChat,
    selectThread,
  };
}
