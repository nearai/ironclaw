import { Icon } from "@ironclaw/design-system";
import { useT } from "../../../lib/i18n";
import { displaySidebarTitle } from "../../../lib/thread-title";

function formatTime(iso) {
  if (!iso) return "";
  const d = new Date(iso);
  const now = new Date();
  const isToday = d.toDateString() === now.toDateString();
  if (isToday)
    return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  return d.toLocaleDateString([], { month: "short", day: "numeric" });
}

export function ThreadSidebar({
  threads,
  activeThreadId,
  onSelect,
  onCreate,
  isCreating,
  compact = false,
}) {
  const t = useT();
  const canCreate = !(
    activeThreadId &&
    threads.some((t) => t.id === activeThreadId && (t.turn_count || 0) === 0)
  );
  const createDisabled = isCreating || !canCreate;

  if (compact) {
    return (
      <div className="flex items-center gap-2">
        <button
          onClick={onCreate}
          disabled={createDisabled}
          className="v2-button h-9 shrink-0 rounded-md border border-[var(--v2-accent)]/25 bg-[var(--v2-accent-soft)] px-3 text-xs font-medium text-[var(--v2-accent-text)] disabled:opacity-50"
        >
          {isCreating ? t("chat.creating") : t("chat.newThread")}
        </button>
        <select
          value={activeThreadId || ""}
          onChange={(event) => onSelect(event.currentTarget.value || null)}
          className="v2-select h-9 min-w-0 flex-1 rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] px-3 text-sm text-[var(--v2-text-strong)] outline-none focus:border-[var(--v2-accent)]/60"
        >
          <option value="">{t("chat.selectConversation")}</option>
          {threads.map(
            (thread) => {
              const title = displaySidebarTitle(
                thread,
                t("notifications.approval.untitled"),
              );
              return (
                <option key={thread.id} value={thread.id}>
                  {title}
                </option>
              );
            }
          )}
        </select>
      </div>
    );
  }

  return (
    <div
      className="flex h-full flex-col border-r border-[var(--v2-panel-border)] bg-[var(--v2-canvas-strong)]/72 backdrop-blur-xl"
    >
      <div
        className="flex items-center justify-between border-b border-[var(--v2-panel-border)] px-5 py-5"
      >
        <div>
          <span className="text-sm font-medium text-[var(--v2-text-strong)]"
            >{t("chat.conversations")}</span
          >
          <p
            className="mt-1 font-mono text-[10px] uppercase tracking-[0.16em] text-[var(--v2-text-muted)]"
          >
            {t("chat.threads", { count: threads.length })}
          </p>
        </div>
        <button
          onClick={onCreate}
          disabled={createDisabled}
          className="v2-button inline-flex h-8 items-center gap-1.5 rounded-md border border-[var(--v2-accent)]/25 bg-[var(--v2-accent-soft)] px-2 text-xs font-medium text-[var(--v2-accent-text)] hover:bg-[var(--v2-accent)]/15 disabled:opacity-50"
        >
          {isCreating
            ? t("chat.creating")
            : (<><Icon name="plus" className="h-3.5 w-3.5" /> {t(
                  "chat.newThread"
                )}</>)}
        </button>
      </div>

      <div className="flex-1 overflow-y-auto p-2">
        {threads.length === 0 &&
        (<div
          className="mx-2 mt-3 rounded-md border border-dashed border-[var(--v2-panel-border)] px-4 py-7 text-left text-xs leading-5 text-[var(--v2-text-muted)]"
        >
          {t("chat.noConversations")}
        </div>)}
        {threads.map((thread) => {
          const active = thread.id === activeThreadId;
          const title = displaySidebarTitle(thread, t("notifications.approval.untitled"));
          return (
            <button
              key={thread.id}
              onClick={() => onSelect(thread.id)}
              className={[
                "v2-button mb-1 flex w-full justify-start items-start flex-col gap-1 rounded-md border px-3 py-3 text-left",
                active
                  ? "border-[var(--v2-accent)]/35 bg-[var(--v2-accent-soft)]"
                  : "border-transparent hover:border-[var(--v2-panel-border)] hover:bg-[var(--v2-surface-soft)]",
              ].join(" ")}
            >
              <div className="flex items-center gap-2">
                <span className="truncate max-w-[150px] text-sm font-medium text-[var(--v2-text-strong)]">
                  {title}
                </span>
                {thread.state === "Processing" &&
                (<span
                  className="v2-breathing-dot ml-auto h-2 w-2 rounded-full bg-[var(--v2-accent)]"
                />)}
              </div>
              <div
                className="flex items-center gap-2 font-mono text-[11px] text-[var(--v2-text-muted)]"
              >
                <span
                  >{t("chat.turns", { count: thread.turn_count || 0 })}</span
                >
                <span>/</span>
                <span>{formatTime(thread.updated_at)}</span>
              </div>
            </button>
          );
        })}
      </div>
    </div>
  );
}
