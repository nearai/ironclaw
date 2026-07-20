import { Panel, StatCard } from "../../../design-system/primitives";

const SUMMARY_CARDS = [
  {
    key: "total",
    label: "Total routines",
    tone: "muted",
    detail: "All saved schedules and event handlers.",
  },
  {
    key: "enabled",
    label: "Enabled",
    tone: "signal",
    detail: "Ready to run from schedule, event, or manual trigger.",
  },
  {
    key: "disabled",
    label: "Disabled",
    tone: "muted",
    detail: "Paused until explicitly re-enabled.",
  },
  {
    key: "unverified",
    label: "Unverified",
    tone: "warning",
    detail: "Needs a successful validation run.",
  },
  {
    key: "failing",
    label: "Failing",
    tone: "danger",
    detail: "Recent run status needs operator attention.",
  },
  {
    key: "runs_today",
    label: "Runs today",
    tone: "success",
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
              className="rounded-2xl border border-white/8 bg-white/[0.03] p-4"
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
