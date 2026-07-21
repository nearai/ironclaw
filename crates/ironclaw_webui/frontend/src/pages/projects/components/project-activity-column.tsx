// @ts-nocheck
import { useT } from "../../../lib/i18n";
import { Panel, StatusPill } from "@ironclaw/design-system";
import { Button } from "@ironclaw/design-system";
import {
  formatThreadState,
  formatProjectRelativeTime,
  projectCount,
  threadPresentation,
  threadTone,
} from "../lib/projects-presenters";

export function ProjectActivityColumn({
  threads,
  selectedThreadId,
  onSelectThread,
  onNewConversation,
  isStartingConversation,
}) {
  const t = useT();
  const sortedThreads = [...threads].sort((a, b) => new Date(b.updated_at || b.created_at) - new Date(a.updated_at || a.created_at));

  return (
    <Panel className="p-4 sm:p-5">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-[var(--v2-text-muted)]">{t("projects.activity.label")}</div>
          <h2 className="mt-2 text-2xl font-medium tracking-tight text-[var(--v2-text-strong)]">{t("projects.activity.title")}</h2>
        </div>
        {onNewConversation &&
        (
          <Button onClick={onNewConversation} disabled={isStartingConversation}>
            {isStartingConversation ? t("projects.activity.starting") : t("projects.activity.newConversation")}
          </Button>
        )}
      </div>

      <div className="mt-5 space-y-3">
        {sortedThreads.length
          ? sortedThreads.slice(0, 18).map((thread) => {
              const presentation = threadPresentation(thread, t);
              return (
                <button
                  key={thread.id}
                  onClick={() => onSelectThread(thread.id)}
                  className={[
                    "w-full rounded-[20px] border p-4 text-left",
                    selectedThreadId === thread.id
                      ? "border-[var(--v2-accent)]/35 bg-[var(--v2-accent-soft)]"
                      : "border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] hover:border-[var(--v2-accent)]/25 hover:bg-[var(--v2-surface-muted)]",
                  ].join(" ")}
                >
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0">
                      <div className="truncate text-base font-medium text-[var(--v2-text-strong)]">{presentation.title}</div>
                      <div className="mt-1 text-xs uppercase tracking-[0.16em] text-[var(--v2-text-faint)]">{presentation.subtitle}</div>
                      {presentation.brief
                        ? (<p className="mt-3 line-clamp-2 text-sm leading-6 text-[var(--v2-text-muted)]">{presentation.brief}</p>)
                        : null}
                    </div>
                    <StatusPill tone={threadTone(thread.state)} label={formatThreadState(thread.state, t)} />
                  </div>
                  <div className="mt-4 flex flex-wrap gap-x-4 gap-y-2 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-text-faint)]">
                    <span>{projectCount(t, "steps", thread.step_count || 0)}</span>
                    <span>{projectCount(t, "tokens", thread.total_tokens || 0)}</span>
                    <span>{formatProjectRelativeTime(thread.updated_at || thread.created_at, t)}</span>
                  </div>
                </button>
              );
            })
          : (
              <div className="rounded-[20px] border border-dashed border-[var(--v2-panel-border)] px-4 py-8 text-sm leading-6 text-[var(--v2-text-muted)]">
                {t("projects.activity.empty")}
              </div>
            )}
      </div>
    </Panel>
  );
}
