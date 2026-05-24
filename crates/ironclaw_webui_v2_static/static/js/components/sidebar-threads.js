import { React, html } from "../lib/html.js";
import { Icon } from "../design-system/icons.js";
import { cn } from "../utils/cn.js";

function formatRelativeTime(iso) {
  if (!iso) return "";
  const d = new Date(iso);
  const now = new Date();
  if (d.toDateString() === now.toDateString())
    return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  return d.toLocaleDateString([], { month: "short", day: "numeric" });
}

function formatExactTime(iso) {
  if (!iso) return "";
  return new Date(iso).toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function threadTimeLabel(thread) {
  const started = formatRelativeTime(thread.created_at);
  const updated = formatRelativeTime(thread.updated_at);
  if (!started && !updated) return "";
  if (!updated || started === updated) return `Started ${started}`;
  return `Started ${started} · Last ${updated}`;
}

function threadTimeTitle(thread) {
  const started = formatExactTime(thread.created_at);
  const updated = formatExactTime(thread.updated_at);
  if (!started && !updated) return "";
  if (!updated || started === updated) return `Started ${started}`;
  return `Started ${started}\nLast activity ${updated}`;
}

function ThreadItem({ thread, isActive, onSelect, onDelete }) {
  const isProcessing = thread.state === "Processing";
  const timeLabel = threadTimeLabel(thread);
  const timeTitle = threadTimeTitle(thread);
  const handleDelete = React.useCallback(
    (event) => {
      event.preventDefault();
      event.stopPropagation();
      if (isProcessing || !window.confirm("Delete this chat?")) return;
      Promise.resolve(onDelete?.(thread.id)).catch((err) => {
        window.alert(err?.message || "Unable to delete chat");
      });
    },
    [isProcessing, onDelete, thread.id]
  );

  return html`
    <div
      className=${cn(
        "group flex w-full items-stretch rounded-[8px]",
        isActive
          ? "bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]"
          : "text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
      )}
    >
      <button
        onClick=${() => onSelect(thread.id)}
        className="min-w-0 flex-1 px-3 py-2 text-left"
        title=${timeTitle || undefined}
      >
        <div className="flex w-full items-center gap-1.5">
          <span className="min-w-0 flex-1 truncate text-[13px] font-medium leading-snug">
            ${thread.title || `Thread ${thread.id.slice(0, 8)}`}
          </span>
          ${isProcessing &&
          html`<span
            className="h-1.5 w-1.5 shrink-0 rounded-full bg-[var(--v2-accent)]"
          />`}
        </div>
        ${timeLabel &&
        html`<span className="block truncate text-[11px] text-[var(--v2-text-faint)]">
          ${timeLabel}
        </span>`}
      </button>
      <button
        type="button"
        onClick=${handleDelete}
        disabled=${isProcessing}
        title=${isProcessing ? "Cannot delete while processing" : "Delete chat"}
        aria-label="Delete chat"
        className=${cn(
          "my-1 mr-1 flex h-7 w-7 shrink-0 items-center justify-center rounded-[6px]",
          "opacity-0 transition group-hover:opacity-100 focus:opacity-100",
          isProcessing
            ? "cursor-not-allowed text-[var(--v2-text-faint)]"
            : "text-[var(--v2-text-faint)] hover:bg-[var(--v2-danger-soft)] hover:text-[var(--v2-danger-text)]"
        )}
      >
        <${Icon} name="trash" className="h-3.5 w-3.5" strokeWidth=${2} />
      </button>
    </div>
  `;
}

export function SidebarThreads({ threads, activeThreadId, onSelect, onDelete }) {
  const [collapsed, setCollapsed] = React.useState(false);

  return html`
    <div className="flex min-h-0 flex-1 flex-col px-2">
      <button
        onClick=${() => setCollapsed((v) => !v)}
        className="flex w-full items-center gap-1 rounded-[6px] px-2 py-1.5 hover:bg-[var(--v2-surface-muted)]"
      >
        <span
          className="flex-1 text-left text-[11px] font-semibold uppercase tracking-wider text-[var(--v2-text-faint)]"
        >
          Recent
        </span>
        <${Icon}
          name="chevron"
          className=${cn(
            "h-3.5 w-3.5 text-[var(--v2-text-faint)]",
            collapsed ? "-rotate-90" : ""
          )}
          strokeWidth=${2.2}
        />
      </button>

      ${!collapsed &&
      html`
        <div
          className="mt-1 flex flex-col gap-1 overflow-y-auto [scrollbar-width:thin]"
        >
          ${threads.length === 0 &&
          html`<div className="px-3 py-2 text-[12px] text-[var(--v2-text-faint)]">
            No conversations yet
          </div>`}
          ${threads.map(
            (thread) => html`
              <${ThreadItem}
                key=${thread.id}
                thread=${thread}
                isActive=${thread.id === activeThreadId}
                onSelect=${onSelect}
                onDelete=${onDelete}
              />
            `
          )}
        </div>
      `}
    </div>
  `;
}
