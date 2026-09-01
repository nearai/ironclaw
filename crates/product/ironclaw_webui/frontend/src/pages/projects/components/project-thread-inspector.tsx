import { useT } from "../../../lib/i18n";
import { Panel, StatusPill } from "../../../design-system/primitives";
import { MarkdownRenderer } from "../../chat/components/markdown-renderer";
import {
  formatMessageRole,
  formatCurrency,
  formatProjectDate,
  formatThreadState,
  formatThreadType,
  messageContent,
  threadPresentation,
  threadTone,
} from "../lib/projects-presenters";

function MetaCard({ label, value }) {
  return (
    <div className="rounded-2xl border border-[var(--v2-panel-border)] bg-[color-mix(in_srgb,var(--v2-canvas-strong)_88%,transparent)] p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-[var(--v2-text-muted)]">{label}</div>
      <div className="mt-2 text-ui leading-6 text-[var(--v2-text-strong)]">{value}</div>
    </div>
  );
}

export function ProjectThreadInspector({ thread }) {
  const t = useT();
  const presentation = threadPresentation(thread, t);

  return (
    <div className="space-y-4">
      <Panel className="p-4 sm:p-5">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-[var(--v2-text-muted)]">{presentation.subtitle}</div>
            <h2 className="mt-2 text-title-lg font-semibold tracking-tight text-[var(--v2-text-strong)]">{presentation.title}</h2>
          </div>
          <StatusPill tone={threadTone(thread.state)} label={formatThreadState(thread.state, t)} />
        </div>

        {presentation.brief
          ? (
              <div className="mt-4 rounded-2xl border border-mint/15 bg-mint/10 p-4">
                <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-[var(--v2-accent-text)]">{t("projects.thread.brief")}</div>
                <div className="mt-3 text-ui leading-6 text-[var(--v2-text-strong)]">
                  <MarkdownRenderer content={presentation.brief} />
                </div>
              </div>
            )
          : null}

        <div className="mt-5 grid gap-3 sm:grid-cols-2">
          <MetaCard label={t("projects.thread.type")} value={formatThreadType(thread.thread_type, t)} />
          <MetaCard label={t("projects.thread.steps")} value={thread.step_count || 0} />
          <MetaCard label={t("projects.thread.tokens")} value={(thread.total_tokens || 0).toLocaleString()} />
          <MetaCard label={t("projects.thread.spend")} value={thread.total_cost_usd ? formatCurrency(thread.total_cost_usd) : t("projects.thread.notMeasured")} />
          <MetaCard label={t("projects.thread.created")} value={formatProjectDate(thread.created_at, t)} />
          <MetaCard label={t("projects.thread.completed")} value={thread.completed_at ? formatProjectDate(thread.completed_at, t) : t("projects.thread.stillRunning")} />
        </div>
      </Panel>

      <Panel className="p-4 sm:p-5">
        <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-[var(--v2-text-muted)]">{t("projects.thread.timeline")}</div>
        <div className="mt-4 space-y-3">
          {thread.messages?.length
            ? thread.messages.map((message, index) => (
                <article key={index} className="rounded-2xl border border-[var(--v2-panel-border)] bg-[color-mix(in_srgb,var(--v2-canvas-strong)_88%,transparent)] p-4">
                  <div className="text-ui-sm uppercase tracking-[0.16em] text-[var(--v2-text-muted)]">{formatMessageRole(message.role, t)}</div>
                  <div className="mt-3 text-ui leading-6 text-[var(--v2-text-strong)]">
                    <MarkdownRenderer content={messageContent(message)} />
                  </div>
                </article>
              ))
            : (
                <div className="rounded-2xl border border-dashed border-[var(--v2-panel-border)] px-4 py-8 text-ui leading-6 text-[var(--v2-text-muted)]">
                  {t("projects.thread.noMessages")}
                </div>
              )}
        </div>
      </Panel>
    </div>
  );
}
