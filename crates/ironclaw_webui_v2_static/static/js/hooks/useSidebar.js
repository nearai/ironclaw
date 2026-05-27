import { React } from "../lib/html.js";
import { useNavigate } from "react-router";

export function useSidebar({ onNewChat } = {}) {
  const navigate = useNavigate();
  const [open, setOpen] = React.useState(false);

  const close = React.useCallback(() => setOpen(false), []);
  const toggle = React.useCallback(() => setOpen((v) => !v), []);

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

  return { open, close, toggle, newChat, selectThread };
}
