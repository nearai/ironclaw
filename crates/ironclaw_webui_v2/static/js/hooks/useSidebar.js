import { React } from "../lib/html.js";
import { useNavigate } from "react-router";
import {
  currentSidebarOpen,
  isDesktopSidebarViewport,
  readDesktopSidebarOpen,
  toggleSidebarState,
  writeDesktopSidebarOpen,
} from "../lib/sidebar-state.js";

export function useSidebar({ onNewChat } = {}) {
  const navigate = useNavigate();
  const [state, setState] = React.useState(() => ({
    mobileOpen: false,
    desktopOpen: readDesktopSidebarOpen(),
  }));
  const [isDesktopViewport, setIsDesktopViewport] = React.useState(() =>
    isDesktopSidebarViewport()
  );

  React.useEffect(() => {
    const query = window.matchMedia("(min-width: 768px)");
    const handleChange = () => setIsDesktopViewport(query.matches);
    handleChange();
    query.addEventListener?.("change", handleChange);
    return () => query.removeEventListener?.("change", handleChange);
  }, []);

  React.useEffect(() => {
    writeDesktopSidebarOpen(state.desktopOpen);
  }, [state.desktopOpen]);

  const close = React.useCallback(() => {
    setState((current) => ({ ...current, mobileOpen: false }));
  }, []);

  const toggle = React.useCallback(() => {
    setState((current) => toggleSidebarState(current, isDesktopViewport));
  }, [isDesktopViewport]);

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
    currentOpen: currentSidebarOpen(state, isDesktopViewport),
    close,
    toggle,
    newChat,
    selectThread,
  };
}
