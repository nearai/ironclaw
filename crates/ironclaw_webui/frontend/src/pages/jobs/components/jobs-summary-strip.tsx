import { StatStrip, StatTile, type BadgeTone } from "@ironclaw/ui";

const SUMMARY_CARDS: { key: string; label: string; tone: BadgeTone; detail: string }[] = [
  { key: "total", label: "Total jobs", tone: "muted", detail: "All tracked work across agent and sandbox execution." },
  { key: "pending", label: "Pending", tone: "warning", detail: "Queued work waiting for a worker or container slot." },
  { key: "in_progress", label: "In progress", tone: "signal", detail: "Actively running jobs and live bridges." },
  { key: "completed", label: "Completed", tone: "success", detail: "Finished without intervention." },
  { key: "failed", label: "Failed", tone: "danger", detail: "Runs that terminated with an error or interruption." },
  { key: "stuck", label: "Stuck", tone: "danger", detail: "Agent work needing recovery or operator attention." },
];

export function JobsSummaryStrip({ summary }) {
  return (
    <StatStrip columns={3}>
      {SUMMARY_CARDS.map((card) => (
        <StatTile
          key={card.key}
          label={card.label}
          value={summary?.[card.key] ?? 0}
          tone={card.tone}
          detail={card.detail}
        />
      ))}
    </StatStrip>
  );
}
