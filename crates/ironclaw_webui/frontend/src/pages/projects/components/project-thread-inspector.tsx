import { useT } from "../../../lib/i18n";
import { DetailList, DetailRow, EmptyPanel, Card, SectionHeader, Badge } from "@ironclaw/ui";
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

export function ProjectThreadInspector({ thread }) {
  const t = useT();
  const presentation = threadPresentation(thread, t);

  return (
    <div className="space-y-4">
      <Card className="p-4 sm:p-5">
        <SectionHeader
          eyebrow={presentation.subtitle}
          title={presentation.title}
          actions={<Badge tone={threadTone(thread.state)} label={formatThreadState(thread.state, t)} />}
        />

        {presentation.brief
          ? (
              <div className="mt-4 rounded-2xl border border-mint/15 bg-mint/10 p-4">
                <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-mint">{t("projects.thread.brief")}</div>
                <div className="mt-3 text-sm leading-6 text-iron-100">
                  <MarkdownRenderer content={presentation.brief} />
                </div>
              </div>
            )
          : null}

        <DetailList className="mt-5">
          <DetailRow layout="stacked" term={t("projects.thread.type")}>{formatThreadType(thread.thread_type, t)}</DetailRow>
          <DetailRow layout="stacked" term={t("projects.thread.steps")}>{thread.step_count || 0}</DetailRow>
          <DetailRow layout="stacked" term={t("projects.thread.tokens")}>{(thread.total_tokens || 0).toLocaleString()}</DetailRow>
          <DetailRow layout="stacked" term={t("projects.thread.spend")}>{thread.total_cost_usd ? formatCurrency(thread.total_cost_usd) : t("projects.thread.notMeasured")}</DetailRow>
          <DetailRow layout="stacked" term={t("projects.thread.created")}>{formatProjectDate(thread.created_at, t)}</DetailRow>
          <DetailRow layout="stacked" term={t("projects.thread.completed")}>{thread.completed_at ? formatProjectDate(thread.completed_at, t) : t("projects.thread.stillRunning")}</DetailRow>
        </DetailList>
      </Card>

      <Card className="p-4 sm:p-5">
        <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">{t("projects.thread.timeline")}</div>
        <div className="mt-4 space-y-3">
          {thread.messages?.length
            ? thread.messages.map((message, index) => (
                <article key={index} className="rounded-2xl border border-white/8 bg-iron-950/60 p-4">
                  <div className="text-xs uppercase tracking-[0.16em] text-iron-400">{formatMessageRole(message.role, t)}</div>
                  <div className="mt-3 text-sm leading-6 text-iron-100">
                    <MarkdownRenderer content={messageContent(message)} />
                  </div>
                </article>
              ))
            : (<EmptyPanel variant="dashed" description={t("projects.thread.noMessages")} />)}
        </div>
      </Card>
    </div>
  );
}
