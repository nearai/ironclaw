import { Panel, StatCard } from "@ironclaw/design-system";

const SUMMARY_CARDS = [
  {
    key: "total",
    label: "Total routines",
    tone: "muted" as const,
    detail: "All saved schedules and event handlers.",
  },
  {
    key: "enabled",
    label: "Enabled",
    tone: "signal" as const,
    detail: "Ready to run from schedule, event, or manual trigger.",
  },
  {
    key: "disabled",
    label: "Disabled",
    tone: "muted" as const,
    detail: "Paused until explicitly re-enabled.",
  },
  {
    key: "unverified",
    label: "Unverified",
    tone: "warning" as const,
    detail: "Needs a successful validation run.",
  },
  {
    key: "failing",
    label: "Failing",
    tone: "danger" as const,
    detail: "Recent run status needs operator attention.",
  },
  {
    key: "runs_today",
    label: "Runs today",
    tone: "success" as const,
    detail: "Routines with activity since local day start.",
  },
];

export function RoutinesSummaryStrip({ summary }) {
  return (
    <Panel className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-6">
        {SUMMARY_CARDS.map(
          (card) => (
            <div
              key={card.key}
              className="rounded-2xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4"
            >
              <StatCard
                label={card.label}
                value={summary?.[card.key] ?? 0}
                tone={card.tone}
                detail={card.detail}
                showDivider={false}
                className="px-0 py-0"
              />
            </div>
          )
        )}
      </div>
    </Panel>
  );
}
