import { Button, Icon, Select, Text } from "@ironclaw/design-system";
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

/* The new-thread action carries the accent-soft treatment (quiet accent
   fill) on top of the DS Button chrome. */
const NEW_THREAD_CLASSES =
  "border-[var(--v2-accent)]/25 bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)] hover:bg-[var(--v2-accent)]/15 hover:border-[var(--v2-accent)]/25";

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
        <Button
          variant="secondary"
          size="sm"
          disabled={createDisabled}
          onClick={onCreate}
          className={`h-9 shrink-0 ${NEW_THREAD_CLASSES}`}
        >
          {isCreating ? t("chat.creating") : t("chat.newThread")}
        </Button>
        <Select
          value={activeThreadId || ""}
          onChange={(event) => onSelect(event.currentTarget.value || null)}
          className="h-9"
          aria-label={t("chat.selectConversation")}
        >
          <option value="">{t("chat.selectConversation")}</option>
          {threads.map((thread) => {
            const title = displaySidebarTitle(
              thread,
              t("notifications.approval.untitled"),
            );
            return (
              <option key={thread.id} value={thread.id}>
                {title}
              </option>
            );
          })}
        </Select>
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
          <Text variant="body" tone="strong" weight="medium" as="span">
            {t("chat.conversations")}
          </Text>
          <Text variant="eyebrow" tone="muted" as="p" className="mt-1">
            {t("chat.threads", { count: threads.length })}
          </Text>
        </div>
        <Button
          variant="secondary"
          size="sm"
          disabled={createDisabled}
          onClick={onCreate}
          className={NEW_THREAD_CLASSES}
        >
          {isCreating
            ? t("chat.creating")
            : (<><Icon name="plus" className="h-3.5 w-3.5" /> {t(
                  "chat.newThread"
                )}</>)}
        </Button>
      </div>

      <div className="flex-1 overflow-y-auto p-2">
        {threads.length === 0 &&
        (<Text
          as="div"
          variant="caption"
          tone="muted"
          className="mx-2 mt-3 block rounded-md border border-dashed border-[var(--v2-panel-border)] px-4 py-7 text-left leading-5"
        >
          {t("chat.noConversations")}
        </Text>)}
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
                <Text
                  as="span"
                  variant="body"
                  tone="strong"
                  weight="medium"
                  className="max-w-[150px] truncate"
                >
                  {title}
                </Text>
                {thread.state === "Processing" &&
                (<span
                  className="v2-breathing-dot ml-auto h-2 w-2 rounded-full bg-[var(--v2-accent)]"
                />)}
              </div>
              <Text
                as="div"
                variant="mono"
                tone="muted"
                className="flex items-center gap-2 text-[length:var(--v2-font-size-label)]"
              >
                <span
                  >{t("chat.turns", { count: thread.turn_count || 0 })}</span
                >
                <span>/</span>
                <span>{formatTime(thread.updated_at)}</span>
              </Text>
            </button>
          );
        })}
      </div>
    </div>
  );
}
