import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { Panel, StatCard } from "../../../design-system/primitives.js";
import { useUsage } from "../hooks/useAdminUsage.js";
import {
  formatCost,
  formatTokenCount,
  truncateId,
  aggregateUsageByUser,
  aggregateUsageByModel,
  totalUsage,
} from "../lib/admin-presenters.js";

const PERIODS = [
  { value: "day", label: "24h" },
  { value: "week", label: "7d" },
  { value: "month", label: "30d" },
];

function UsageBar({ value, max }) {
  const pct = max > 0 ? (value / max) * 100 : 0;
  return html`
    <div className="h-2 w-full overflow-hidden rounded-full bg-white/[0.06]">
      <div
        className="h-full rounded-full bg-signal/50"
        style=${{ width: `${Math.max(pct, 1)}%` }}
      />
    </div>
  `;
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
    return html`
      <${Panel} className="p-5 sm:p-6">
        <div className="v2-skeleton mb-4 h-4 w-32 rounded" />
        <div className="grid gap-4 sm:grid-cols-4">
          ${[1, 2, 3, 4].map((i) => html`<div key=${i} className="v2-skeleton h-28 rounded-lg" />`)}
        </div>
      <//>
    `;
  }

  return html`
    <div className="space-y-5">
      <${Panel} className="p-5 sm:p-6">
        <div className="mb-5 flex items-center justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${t("admin.usage.overview")}</h3>
          <div className="flex gap-1">
            ${PERIODS.map(
              (p) => html`
                <button
                  key=${p.value}
                  onClick=${() => setPeriod(p.value)}
                  className=${[
                    "rounded-md px-3 py-1.5 text-[11px] font-medium",
                    period === p.value
                      ? "border border-signal/35 bg-signal/10 text-white"
                      : "border border-transparent text-iron-300 hover:text-white",
                  ].join(" ")}
                >
                  ${p.label}
                </button>
              `
            )}
          </div>
        </div>

        ${entries.length === 0
          ? html`<p className="py-4 text-sm text-iron-300">${t("admin.usage.noData")}</p>`
          : html`
              <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
                <${StatCard} label=${t("admin.usage.totalCalls")} value=${totals.calls.toLocaleString()} tone="muted" />
                <${StatCard} label=${t("admin.usage.inputTokens")} value=${formatTokenCount(totals.input_tokens)} tone="muted" />
                <${StatCard} label=${t("admin.usage.outputTokens")} value=${formatTokenCount(totals.output_tokens)} tone="muted" />
                <${StatCard} label=${t("admin.usage.totalCost")} value=${formatCost(totals.cost.toFixed(2))} tone="signal" />
              </div>
            `}
      <//>

      ${byUser.length > 0 && html`
        <${Panel} className="p-5 sm:p-6">
          <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${t("admin.usage.perUser")}</h3>
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-white/10 text-left">
                  <th className="pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${t("admin.usage.user")}</th>
                  <th className="pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${t("admin.usage.calls")}</th>
                  <th className="hidden pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300 sm:table-cell">${t("admin.usage.input")}</th>
                  <th className="hidden pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300 sm:table-cell">${t("admin.usage.output")}</th>
                  <th className="pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${t("admin.usage.cost")}</th>
                  <th className="hidden pb-3 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300 md:table-cell" />
                </tr>
              </thead>
              <tbody>
                ${byUser.map(
                  (u) => html`
                    <tr key=${u.user_id} className="border-b border-white/[0.06] last:border-0">
                      <td className="py-3 pr-4">
                        <button
                          onClick=${() => onSelectUser(u.user_id)}
                          className="font-mono text-xs text-signal hover:underline"
                        >
                          ${truncateId(u.user_id)}
                        </button>
                      </td>
                      <td className="py-3 pr-4 font-mono text-xs text-iron-300">${u.calls.toLocaleString()}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${formatTokenCount(u.input_tokens)}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${formatTokenCount(u.output_tokens)}</td>
                      <td className="py-3 pr-4 font-mono text-xs text-iron-100">${formatCost(u.cost.toFixed(2))}</td>
                      <td className="hidden py-3 md:table-cell">
                        <${UsageBar} value=${u.cost} max=${maxCost} />
                      </td>
                    </tr>
                  `
                )}
              </tbody>
            </table>
          </div>
        <//>
      `}

      ${byModel.length > 0 && html`
        <${Panel} className="p-5 sm:p-6">
          <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">${t("admin.usage.perModel")}</h3>
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-white/10 text-left">
                  <th className="pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${t("admin.usage.model")}</th>
                  <th className="pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${t("admin.usage.calls")}</th>
                  <th className="hidden pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300 sm:table-cell">${t("admin.usage.input")}</th>
                  <th className="hidden pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300 sm:table-cell">${t("admin.usage.output")}</th>
                  <th className="pb-3 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${t("admin.usage.cost")}</th>
                </tr>
              </thead>
              <tbody>
                ${byModel.map(
                  (m) => html`
                    <tr key=${m.model} className="border-b border-white/[0.06] last:border-0">
                      <td className="py-3 pr-4 font-mono text-xs text-iron-100">${m.model}</td>
                      <td className="py-3 pr-4 font-mono text-xs text-iron-300">${m.calls.toLocaleString()}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${formatTokenCount(m.input_tokens)}</td>
                      <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">${formatTokenCount(m.output_tokens)}</td>
                      <td className="py-3 font-mono text-xs text-iron-100">${formatCost(m.cost.toFixed(2))}</td>
                    </tr>
                  `
                )}
              </tbody>
            </table>
          </div>
        <//>
      `}
    </div>
  `;
}
