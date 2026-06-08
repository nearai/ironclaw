import { React, html } from "../lib/html.js";
import { useT } from "../lib/i18n.js";
import { useNavigate } from "react-router";
import { Icon } from "../design-system/icons.js";

/* ⌘K / Ctrl+K launcher: jump to a thread, start a new chat, navigate to a
   section, or toggle the theme. Pure frontend — drives existing routes and
   thread state, no new backend. */
export function CommandPalette({ open, onClose, threadsState, onNewChat, onToggleTheme }) {
  const navigate = useNavigate();
  const t = useT();
  const [query, setQuery] = React.useState("");
  const [active, setActive] = React.useState(0);
  const inputRef = React.useRef(null);

  const commands = React.useMemo(() => {
    const actions = [
      { id: "new-chat", label: "New chat", icon: "plus", group: "Actions", run: () => onNewChat?.() },
      { id: "go-chat", label: "Go to Chat", icon: "chat", group: "Navigate", run: () => navigate("/chat") },
      { id: "go-extensions", label: "Go to Extensions", icon: "plug", group: "Navigate", run: () => navigate("/extensions") },
      { id: "go-settings", label: "Go to Settings", icon: "settings", group: "Navigate", run: () => navigate("/settings") },
      { id: "toggle-theme", label: "Toggle theme", icon: "moon", group: "Actions", run: () => onToggleTheme?.() },
    ];
    const threads = (threadsState?.threads || []).map((thread) => ({
      id: `thread-${thread.id}`,
      label: thread.title || `Thread ${thread.id.slice(0, 8)}`,
      icon: "chat",
      group: "Threads",
      run: () => navigate(`/chat/${thread.id}`),
    }));
    return [...actions, ...threads];
  }, [threadsState, navigate, onNewChat, onToggleTheme]);

  const filtered = React.useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return commands;
    return commands.filter((c) => c.label.toLowerCase().includes(q));
  }, [commands, query]);

  React.useEffect(() => {
    if (!open) return;
    setQuery("");
    setActive(0);
    const id = window.requestAnimationFrame(() => inputRef.current?.focus());
    return () => window.cancelAnimationFrame(id);
  }, [open]);

  React.useEffect(() => {
    setActive((i) => Math.min(i, Math.max(0, filtered.length - 1)));
  }, [filtered.length]);

  const exec = React.useCallback(
    (command) => {
      if (!command) return;
      onClose();
      command.run();
    },
    [onClose]
  );

  const onKeyDown = React.useCallback(
    (event) => {
      if (event.key === "ArrowDown") {
        event.preventDefault();
        setActive((i) => Math.min(i + 1, filtered.length - 1));
      } else if (event.key === "ArrowUp") {
        event.preventDefault();
        setActive((i) => Math.max(i - 1, 0));
      } else if (event.key === "Enter") {
        event.preventDefault();
        exec(filtered[active]);
      } else if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    },
    [filtered, active, exec, onClose]
  );

  if (!open) return null;

  let lastGroup = null;

  return html`
    <div className="fixed inset-0 z-50 flex items-start justify-center p-4 pt-[12vh]" role="dialog" aria-modal="true" aria-label="Command palette">
      <button type="button" aria-label="Close" onClick=${onClose} className="absolute inset-0 bg-black/50"></button>
      <div className="relative w-full max-w-lg overflow-hidden rounded-2xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] shadow-[0_30px_60px_-20px_rgba(0,0,0,0.8)]">
        <div className="flex items-center gap-2 border-b border-[var(--v2-panel-border)] px-3">
          <${Icon} name="search" className="h-4 w-4 text-[var(--v2-text-faint)]" />
          <input
            ref=${inputRef}
            value=${query}
            onInput=${(e) => setQuery(e.currentTarget.value)}
            onKeyDown=${onKeyDown}
            placeholder=${t("command.placeholder")}
            className="h-12 w-full border-0 bg-transparent text-sm text-[var(--v2-text-strong)] outline-none placeholder:text-[var(--v2-text-faint)]"
          />
          <kbd className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-faint)]">esc</kbd>
        </div>
        <ul className="max-h-[50vh] overflow-y-auto p-1.5">
          ${filtered.length === 0 &&
          html`<li className="px-3 py-6 text-center text-sm text-[var(--v2-text-faint)]">No matches</li>`}
          ${filtered.map((command, index) => {
            const showGroup = command.group !== lastGroup;
            lastGroup = command.group;
            return html`
              ${showGroup &&
              html`<li key=${`g-${command.group}`} className="px-2 pb-1 pt-2 text-[10px] font-semibold uppercase tracking-wider text-[var(--v2-text-faint)]">${command.group}</li>`}
              <li key=${command.id}>
                <button
                  type="button"
                  onMouseEnter=${() => setActive(index)}
                  onClick=${() => exec(command)}
                  className=${[
                    "flex w-full items-center gap-2.5 rounded-[9px] px-2.5 py-2 text-left text-sm",
                    index === active
                      ? "bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]"
                      : "text-[var(--v2-text)] hover:bg-[var(--v2-surface-soft)]",
                  ].join(" ")}
                >
                  <${Icon} name=${command.icon} className="h-4 w-4 shrink-0" />
                  <span className="min-w-0 truncate">${command.label}</span>
                </button>
              </li>
            `;
          })}
        </ul>
      </div>
    </div>
  `;
}
