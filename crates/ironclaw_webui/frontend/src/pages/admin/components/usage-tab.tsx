// @ts-nocheck
import React from "react";
import { useT } from "../../../lib/i18n";
import { Button, Panel, StatCard, Text } from "@ironclaw/design-system";
import { useUsage } from "../hooks/useAdminUsage";
import {
  formatCost,
  formatTokenCount,
  truncateId,
  aggregateUsageByUser,
  aggregateUsageByModel,
  totalUsage,
} from "../lib/admin-presenters";

const PERIODS = [
  { value: "day", label: "24h" },
  { value: "week", label: "7d" },
  { value: "month", label: "30d" },
];

function UsageBar({ value, max }) {
  const pct = max > 0 ? (value / max) * 100 : 0;
  return (
    <div className="h-2 w-full overflow-hidden rounded-full bg-[var(--v2-surface-soft)]">
      <div
        className="h-full rounded-full bg-[color-mix(in_srgb,var(--v2-accent)_50%,transparent)]"
        style={{ width: `${Math.max(pct, 1)}%` }}
      />
    </div>
  );
}

export function UsageTab({ onSelectUser }) {
  const t = useT();
  const [period, setPeriod] = React.useState("day");
  const usageQuery = useUsage(period);
  const entries = usageQuery.data?.usage || [];

  const byUser = aggregateUsageByUser(entries);
  const byModel = aggregateUsageByModel(entries);
  const totals = totalUsage(byUser);
  const maxCost = byUser.length > 0 ? byUser[0].cost : 0;

  if (usageQuery.isLoading) {
    return (
      <Panel className="p-5 sm:p-6">
        <div className="v2-skeleton mb-4 h-4 w-32 rounded" />
        <div className="grid gap-4 sm:grid-cols-4">
          {[1, 2, 3, 4].map((i) => (<div key={i} className="v2-skeleton h-28 rounded-lg" />))}
        </div>
      </Panel>
    );
  }

  return (
    <div className="space-y-5">
      <Panel className="p-5 sm:p-6">
        <div className="mb-5 flex items-center justify-between">
          <Text as="h3" variant="eyebrow" tone="accent">{t("admin.usage.overview")}</Text>
          <div className="flex gap-1">
            {PERIODS.map(
              (p) => (
                <Button
                  key={p.value}
                  variant="ghost"
                  size="sm"
                  onClick={() => setPeriod(p.value)}
                  className={
                    period === p.value
                      ? "border-[color-mix(in_srgb,var(--v2-accent)_35%,transparent)] bg-[var(--v2-accent-soft)] text-[var(--v2-text-strong)]"
                      : undefined
                  }
                >
                  {p.label}
                </Button>
              )
            )}
          </div>
        </div>

        {entries.length === 0
          ? (<Text variant="body" tone="muted" className="py-4">{t("admin.usage.noData")}</Text>)
          : (
              <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
                <StatCard label={t("admin.usage.totalCalls")} value={totals.calls.toLocaleString()} tone="muted" />
                <StatCard label={t("admin.usage.inputTokens")} value={formatTokenCount(totals.input_tokens)} tone="muted" />
                <StatCard label={t("admin.usage.outputTokens")} value={formatTokenCount(totals.output_tokens)} tone="muted" />
                <StatCard label={t("admin.usage.totalCost")} value={formatCost(totals.cost.toFixed(2))} tone="signal" />
              </div>
            )}
      </Panel>

      {byUser.length > 0 && (
        <Panel className="p-5 sm:p-6">
          <Text as="h3" variant="eyebrow" tone="accent" className="mb-4">{t("admin.usage.perUser")}</Text>
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-[var(--v2-panel-border)] text-left">
                  <Text as="th" variant="eyebrow" tone="muted" className="pb-3 pr-4">{t("admin.usage.user")}</Text>
                  <Text as="th" variant="eyebrow" tone="muted" className="pb-3 pr-4">{t("admin.usage.calls")}</Text>
                  <Text as="th" variant="eyebrow" tone="muted" className="hidden pb-3 pr-4 sm:table-cell">{t("admin.usage.input")}</Text>
                  <Text as="th" variant="eyebrow" tone="muted" className="hidden pb-3 pr-4 sm:table-cell">{t("admin.usage.output")}</Text>
                  <Text as="th" variant="eyebrow" tone="muted" className="pb-3 pr-4">{t("admin.usage.cost")}</Text>
                  <Text as="th" variant="eyebrow" tone="muted" className="hidden pb-3 md:table-cell" />
                </tr>
              </thead>
              <tbody>
                {byUser.map(
                  (u) => (
                    <tr key={u.user_id} className="border-b border-[var(--v2-panel-border)] last:border-0">
                      <td className="py-3 pr-4">
                        <button
                          onClick={() => onSelectUser(u.user_id)}
                          className="font-mono text-xs text-[var(--v2-accent-text)] hover:underline"
                        >
                          {truncateId(u.user_id)}
                        </button>
                      </td>
                      <Text as="td" variant="mono" tone="muted" className="py-3 pr-4">{u.calls.toLocaleString()}</Text>
                      <Text as="td" variant="mono" tone="muted" className="hidden py-3 pr-4 sm:table-cell">{formatTokenCount(u.input_tokens)}</Text>
                      <Text as="td" variant="mono" tone="muted" className="hidden py-3 pr-4 sm:table-cell">{formatTokenCount(u.output_tokens)}</Text>
                      <Text as="td" variant="mono" tone="strong" className="py-3 pr-4">{formatCost(u.cost.toFixed(2))}</Text>
                      <td className="hidden py-3 md:table-cell">
                        <UsageBar value={u.cost} max={maxCost} />
                      </td>
                    </tr>
                  )
                )}
              </tbody>
            </table>
          </div>
        </Panel>
      )}

      {byModel.length > 0 && (
        <Panel className="p-5 sm:p-6">
          <Text as="h3" variant="eyebrow" tone="accent" className="mb-4">{t("admin.usage.perModel")}</Text>
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-[var(--v2-panel-border)] text-left">
                  <Text as="th" variant="eyebrow" tone="muted" className="pb-3 pr-4">{t("admin.usage.model")}</Text>
                  <Text as="th" variant="eyebrow" tone="muted" className="pb-3 pr-4">{t("admin.usage.calls")}</Text>
                  <Text as="th" variant="eyebrow" tone="muted" className="hidden pb-3 pr-4 sm:table-cell">{t("admin.usage.input")}</Text>
                  <Text as="th" variant="eyebrow" tone="muted" className="hidden pb-3 pr-4 sm:table-cell">{t("admin.usage.output")}</Text>
                  <Text as="th" variant="eyebrow" tone="muted" className="pb-3">{t("admin.usage.cost")}</Text>
                </tr>
              </thead>
              <tbody>
                {byModel.map(
                  (m) => (
                    <tr key={m.model} className="border-b border-[var(--v2-panel-border)] last:border-0">
                      <Text as="td" variant="mono" tone="strong" className="py-3 pr-4">{m.model}</Text>
                      <Text as="td" variant="mono" tone="muted" className="py-3 pr-4">{m.calls.toLocaleString()}</Text>
                      <Text as="td" variant="mono" tone="muted" className="hidden py-3 pr-4 sm:table-cell">{formatTokenCount(m.input_tokens)}</Text>
                      <Text as="td" variant="mono" tone="muted" className="hidden py-3 pr-4 sm:table-cell">{formatTokenCount(m.output_tokens)}</Text>
                      <Text as="td" variant="mono" tone="strong" className="py-3">{formatCost(m.cost.toFixed(2))}</Text>
                    </tr>
                  )
                )}
              </tbody>
            </table>
          </div>
        </Panel>
      )}
    </div>
  );
}
