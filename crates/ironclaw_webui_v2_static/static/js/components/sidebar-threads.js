import { NavLink } from "react-router";
import { React, html } from "../lib/html.js";
import { Icon } from "../design-system/icons.js";
import { useT } from "../lib/i18n.js";
import { getPinnedIds, subscribePins, togglePin } from "../lib/pin-store.js";

/* React adapter for the pinned-thread store. Lives here (not in pin-store.js)
 * so the store stays a pure, unit-testable module free of a React import. */
function usePinnedIds() {
  const [set, setSet] = React.useState(getPinnedIds);
  React.useEffect(() => subscribePins(setSet), []);
  return set;
}
import { THREAD_STATE, useThreadStates } from "../lib/thread-state.js";
import {
  byActivityDesc,
  formatThreadActivityLabel,
  formatThreadActivityTooltip,
  threadActivityIso,
} from "../lib/thread-meta.js";
import { cn } from "../utils/cn.js";

/* Single source of truth for how a thread state renders in the sidebar.
 *
 * Adding a state to THREAD_STATE means adding one row here — the
 * partition predicate, the dot, the border, and the label all read
 * from this table so the row component stays free of state-by-state
 * branching. */
const STATE_PRESENTATION = Object.freeze({
  [THREAD_STATE.NEEDS_ATTENTION]: {
    label: "Needs your attention",
    textClass: "text-[var(--v2-warning-text)]",
    dotClass: "bg-[var(--v2-warning-text)]",
    // The colored dot + label badge already signals attention; the solid
    // left border felt visually heavy, so this state carries no border
    // accent. The reserved border-l-2 width stays transparent.
    borderClass: "border-transparent",
  },
  [THREAD_STATE.RUNNING]: {
    label: "Running",
    textClass: "text-[var(--v2-positive-text)]",
    dotClass: "bg-[var(--v2-positive-text)]",
    borderClass: "border-[var(--v2-positive-text)]",
  },
  [THREAD_STATE.FAILED]: {
    label: "Failed",
    textClass: "text-[var(--v2-danger-text)]",
    dotClass: "bg-[var(--v2-danger-text)]",
    borderClass: "border-[var(--v2-danger-text)]",
  },
});

function presentationFor(state) {
  return state ? STATE_PRESENTATION[state] || null : null;
}

function stateFromThreadSummary(thread) {
  const raw = String(thread?.state || "").toLowerCase();
  if (raw === "processing" || raw === "running") return THREAD_STATE.RUNNING;
  if (
    raw === "needs_attention" ||
    raw === "awaitingapproval" ||
    raw === "awaiting_approval"
  ) {
    return THREAD_STATE.NEEDS_ATTENTION;
  }
  if (raw === "failed" || raw === "interrupted") return THREAD_STATE.FAILED;
  return null;
}

function ThreadItem({ thread, isActive, isPinned, presentation, onSelect, onDelete }) {
  const t = useT();
  const activityIso = threadActivityIso(thread);
  const timeLabel = formatThreadActivityLabel(activityIso);
  const timeTitle = formatThreadActivityTooltip(activityIso);

  const handleDelete = React.useCallback(
    (event) => {
      event.preventDefault();
      event.stopPropagation();
      if (!window.confirm("Delete this chat?")) return;
      Promise.resolve(onDelete?.(thread.id)).catch((err) => {
        window.alert(err?.message || "Unable to delete chat");
      });
    },
    [onDelete, thread.id]
  );

  const handleTogglePin = React.useCallback(
    (event) => {
      event.preventDefault();
      event.stopPropagation();
      togglePin(thread.id);
    },
    [thread.id]
  );

  return html`
    <div
      className=${cn(
        "group flex w-full items-stretch rounded-[8px] border-l-2",
        presentation
          ? presentation.borderClass
          : isActive
          ? "border-[var(--v2-accent)]"
          : "border-transparent",
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
          ${presentation &&
          html`<span
            aria-label=${presentation.label}
            className=${cn("h-1.5 w-1.5 shrink-0 rounded-full", presentation.dotClass)}
          />`}
        </div>
        ${(presentation || timeLabel) &&
        html`<span
          className=${cn(
            "block truncate text-[11px]",
            presentation ? presentation.textClass : "text-[var(--v2-text-faint)]"
          )}
        >
          ${presentation ? presentation.label : timeLabel}
        </span>`}
      </button>
      <button
        type="button"
        onClick=${handleTogglePin}
        title=${isPinned ? t("common.unpin") : t("common.pin")}
        aria-label=${isPinned ? t("common.unpin") : t("common.pin")}
        aria-pressed=${isPinned ? "true" : "false"}
        className=${cn(
          "my-1 flex h-7 w-7 shrink-0 items-center justify-center rounded-[6px] transition",
          isPinned
            ? "text-[var(--v2-accent-text)]"
            : "opacity-0 text-[var(--v2-text-faint)] group-hover:opacity-100 focus:opacity-100",
          "hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-accent-text)]"
        )}
      >
        <${Icon} name="pin" className="h-3.5 w-3.5" strokeWidth=${2} />
      </button>
      ${onDelete &&
      html`<button
        type="button"
        onClick=${handleDelete}
        title=${t("common.deleteChat")}
        aria-label=${t("common.deleteChat")}
        className=${cn(
          "my-1 mr-1 flex h-7 w-7 shrink-0 items-center justify-center rounded-[6px]",
          "opacity-0 transition group-hover:opacity-100 focus:opacity-100",
          "text-[var(--v2-text-faint)] hover:bg-[var(--v2-danger-soft)] hover:text-[var(--v2-danger-text)]"
        )}
      >
        <${Icon} name="trash" className="h-3.5 w-3.5" strokeWidth=${2} />
      </button>`}
    </div>
  `;
}

function ThreadGroup({ label, items, activeThreadId, states, pinnedIds, onSelect, onDelete }) {
  if (items.length === 0) return null;
  return html`
    <div className="flex flex-col gap-1">
      <span className="px-3 pt-1 text-[10px] font-semibold uppercase tracking-wider text-[var(--v2-text-faint)]">
        ${label}
      </span>
      ${items.map(
        (thread) => html`
          <${ThreadItem}
            key=${thread.id}
            thread=${thread}
            isActive=${thread.id === activeThreadId}
            isPinned=${pinnedIds.has(thread.id)}
            presentation=${presentationFor(
              states.has(thread.id) ? states.get(thread.id) : stateFromThreadSummary(thread)
            )}
            onSelect=${onSelect}
            onDelete=${onDelete}
          />
        `
      )}
    </div>
  `;
}

export function SidebarThreads({
  threads,
  activeThreadId,
  rebornProjectsEnabled = false,
  onSelect,
  onDelete,
  onNavigate,
}) {
  const [collapsed, setCollapsed] = React.useState(false);
  const [query, setQuery] = React.useState("");
  const states = useThreadStates();
  const pinnedIds = usePinnedIds();
  const t = useT();

  /* Two-group partition (replaces the previous date-bucketed layout):
   *   - Pinned: threads the user has explicitly pinned (see lib/pin-store).
   *     The active thread is no longer auto-pinned — switching threads
   *     used to shuffle the active one into PINNED, which this fixes.
   *   - Recent: everything else, newest-first by updated_at || created_at.
   *
   * Per-thread state (running / needs-attention) still renders its dot
   * wherever the thread lands; it no longer forces a thread into PINNED.
   *
   * Title search runs before partitioning so the filter still matches
   * any thread, pinned or not. */
  const { pinned, recent, totalMatches } = React.useMemo(() => {
    const q = query.trim().toLowerCase();
    const filtered = q
      ? threads.filter((thread) =>
          (thread.title || thread.id || "").toLowerCase().includes(q)
        )
      : threads;

    const pinnedList = [];
    const recentList = [];
    for (const thread of filtered) {
      if (pinnedIds.has(thread.id)) {
        pinnedList.push(thread);
      } else {
        recentList.push(thread);
      }
    }
    pinnedList.sort(byActivityDesc);
    recentList.sort(byActivityDesc);
    return {
      pinned: pinnedList,
      recent: recentList,
      totalMatches: pinnedList.length + recentList.length,
    };
  }, [threads, query, pinnedIds]);

  return html`
    <div className="flex min-h-0 flex-1 flex-col px-2">
      <button
        onClick=${() => setCollapsed((v) => !v)}
        className="flex w-full items-center gap-1 rounded-[6px] px-2 py-1.5 hover:bg-[var(--v2-surface-muted)]"
      >
        <span
          className="flex-1 text-left text-[11px] font-semibold uppercase tracking-wider text-[var(--v2-text-faint)]"
        >
          ${t("chat.conversations")}
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
        ${threads.length > 0 &&
        html`<div className="relative mb-1 mt-1 px-1">
          <span className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-[var(--v2-text-faint)]">
            <${Icon} name="search" className="h-3.5 w-3.5" />
          </span>
          <input
            type="text"
            value=${query}
            onInput=${(event) => setQuery(event.currentTarget.value)}
            placeholder=${t("common.searchChats")}
            className="h-8 w-full rounded-[8px] border border-[var(--v2-panel-border)] bg-[var(--v2-input-bg)] pl-8 pr-2 text-[12px] text-[var(--v2-text-strong)] outline-none placeholder:text-[var(--v2-text-faint)] focus:border-[var(--v2-accent)]"
          />
        </div>`}
        ${rebornProjectsEnabled &&
        html`<div className="mb-1 px-1">
          <${NavLink}
            to="/projects"
            onClick=${onNavigate}
            className=${({ isActive }) =>
              cn(
                "flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",
                isActive
                  ? "bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]"
                  : "text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
              )}
          >
            <${Icon} name="folder" className="h-4 w-4 shrink-0" />
            <span className="min-w-0 truncate">${t("nav.projects")}</span>
          <//>
        </div>`}
        <div
          className="mt-1 flex flex-col gap-2 overflow-y-auto [scrollbar-width:thin]"
        >
          ${threads.length === 0 &&
          html`<div className="px-3 py-2 text-[12px] text-[var(--v2-text-faint)]">
            ${t("chat.noConversations")}
          </div>`}
          ${threads.length > 0 &&
          totalMatches === 0 &&
          html`<div className="px-3 py-2 text-[12px] text-[var(--v2-text-faint)]">
            ${t("common.noChatsMatch").replace("{query}", query)}
          </div>`}

          <${ThreadGroup}
            label=${t("common.pinned")}
            items=${pinned}
            activeThreadId=${activeThreadId}
            states=${states}
            pinnedIds=${pinnedIds}
            onSelect=${onSelect}
            onDelete=${onDelete}
          />
          <${ThreadGroup}
            label=${t("common.recent")}
            items=${recent}
            activeThreadId=${activeThreadId}
            states=${states}
            pinnedIds=${pinnedIds}
            onSelect=${onSelect}
            onDelete=${onDelete}
          />
        </div>
      `}
    </div>
  `;
}
