import { StatStrip, StatTile, type BadgeTone } from "@ironclaw/ui";

const SUMMARY_CARDS: { key: string; label: string; tone: BadgeTone; detail: string }[] = [
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
    <StatStrip columns={3}>
      {SUMMARY_CARDS.map(
        (card) => (
          <StatTile
            key={card.key}
            label={card.label}
            value={summary?.[card.key] ?? 0}
            tone={card.tone}
            detail={card.detail}
          />
        )
      )}
    </StatStrip>
  );
}
