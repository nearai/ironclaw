import { useT } from "../../../lib/i18n";
import { StatStrip, StatTile, type BadgeTone } from "@ironclaw/ui";

type SummaryCard = {
  key: string;
  label: string;
  value: string | number;
  tone: BadgeTone;
  detail: string;
  filter?: string | null;
  valueClassName?: string;
};

export function AutomationsSummaryStrip({ summary, activeFilter, onSelectFilter }) {
  const t = useT();
  const cards: SummaryCard[] = [
    {
      key: "scheduled",
      label: t("automations.summary.scheduled"),
      value: summary?.scheduled ?? 0,
      tone: "muted",
      detail: t("automations.summary.scheduledDetail"),
      filter: "all",
    },
    {
      key: "active",
      label: t("automations.summary.active"),
      value: summary?.active ?? 0,
      tone: "signal",
      detail: t("automations.summary.activeDetail"),
      filter: "active",
    },
    {
      key: "running",
      label: t("automations.summary.running"),
      value: summary?.running ?? 0,
      tone: "info",
      detail: t("automations.summary.runningDetail"),
      filter: "running",
    },
    {
      key: "failures",
      label: t("automations.summary.failures"),
      value: summary?.failures ?? 0,
      tone: (summary?.failures ?? 0) > 0 ? "danger" : "success",
      detail: t("automations.summary.failuresDetail"),
      // The failures card is the primary actionable card (#5004): clicking it
      // filters the list down to the automations with failed runs so the user
      // can jump straight to what went wrong instead of hunting through
      // history. Only offer the jump when there is at least one failure.
      filter: (summary?.failures ?? 0) > 0 ? "failures" : null,
    },
    {
      key: "nextRun",
      label: t("automations.summary.nextRun"),
      value: summary?.nextRun || t("automations.summary.none"),
      tone: "info",
      detail: t("automations.summary.nextRunDetail"),
      // NEXT RUN is a date string, not a count — use a smaller size so it isn't
      // truncated to "Jun…" inside a narrow card.
      valueClassName: "text-lg md:text-xl",
    },
  ];

  return (
    <StatStrip columns={3}>
      {cards.map((card) => {
        const interactive = Boolean(card.filter && onSelectFilter);
        return (
          <StatTile
            key={card.key}
            label={card.label}
            value={card.value}
            tone={card.tone}
            badgeLabel={t(`automations.badge.${card.tone}`)}
            detail={card.detail}
            valueClassName={card.valueClassName}
            onSelect={interactive ? () => onSelectFilter(card.filter) : undefined}
            isActive={interactive && activeFilter === card.filter}
            selectTitle={
              interactive
                ? t("automations.summary.filterAction", { label: card.label })
                : undefined
            }
          />
        );
      })}
    </StatStrip>
  );
}
