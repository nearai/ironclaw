import { Card } from "../../../design-system/card.js";
import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { useBudget } from "../hooks/useBudget.js";
import { matchesSearch } from "../lib/settings-search.js";
import { SettingsSearchEmpty } from "./settings-search-empty.js";

export function formatUsd(value, fallback = "-") {
  if (value === null || value === undefined || value === "") return fallback;
  const numeric = Number(value);
  if (!Number.isFinite(numeric)) return String(value);
  return `$${numeric.toLocaleString(undefined, {
    minimumFractionDigits: 0,
    maximumFractionDigits: 4,
  })}`;
}

export function formatPercent(value, fallback = "-") {
  if (value === null || value === undefined || value === "") return fallback;
  const numeric = Number(value);
  if (!Number.isFinite(numeric)) return fallback;
  return `${(numeric * 100).toFixed(1)}%`;
}

function formatTimestamp(value, fallback = "-") {
  if (!value) return fallback;
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? String(value) : parsed.toLocaleString();
}

function Stat({ label, value }) {
  return html`
    <div className="min-w-0">
      <div className="font-mono text-[10px] uppercase tracking-[0.12em] text-[var(--v2-text-faint)]">
        ${label}
      </div>
      <div className="mt-1 truncate font-mono text-sm text-[var(--v2-text-strong)]">
        ${value}
      </div>
    </div>
  `;
}

function BudgetAccountRow({ account }) {
  const t = useT();
  const utilization = Number(account.utilization);
  const utilizationPercent = Number.isFinite(utilization)
    ? Math.max(0, Math.min(100, utilization * 100))
    : null;
  const thresholdText = account.thresholds
    ? `${formatPercent(account.thresholds.warn_at)} / ${formatPercent(account.thresholds.pause_at)}`
    : "-";

  return html`
    <div className="border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0">
      <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
        <div className="min-w-0">
          <h3 className="truncate text-sm font-semibold text-[var(--v2-text-strong)]">
            ${account.account || t("budget.userBudget")}
          </h3>
          <div className="mt-0.5 font-mono text-[11px] text-[var(--v2-text-faint)]">
            ${account.period || t("budget.unconfigured")}
          </div>
        </div>
        <div className="shrink-0 font-mono text-xs text-[var(--v2-text-muted)]">
          ${t("budget.generated")} ${formatTimestamp(account.generated_at)}
        </div>
      </div>

      <div className="grid gap-3 sm:grid-cols-3 lg:grid-cols-6">
        <${Stat} label=${t("budget.usage")} value=${formatUsd(account.usage_usd)} />
        <${Stat} label=${t("budget.reserved")} value=${formatUsd(account.reserved_usd)} />
        <${Stat}
          label=${t("budget.limit")}
          value=${account.unlimited ? t("budget.unlimited") : formatUsd(account.limit_usd)}
        />
        <${Stat} label=${t("budget.utilization")} value=${formatPercent(account.utilization)} />
        <${Stat} label=${t("budget.reset")} value=${formatTimestamp(account.period_end)} />
        <${Stat} label=${t("budget.warnPause")} value=${thresholdText} />
      </div>

      ${utilizationPercent !== null &&
      html`
        <div
          className="mt-3 h-2 overflow-hidden rounded-full bg-[var(--v2-surface-muted)]"
          aria-label=${t("budget.utilization")}
        >
          <div
            className="h-full rounded-full bg-[var(--v2-accent)]"
            style=${{ width: `${utilizationPercent}%` }}
          />
        </div>
      `}
    </div>
  `;
}

function BudgetSkeleton() {
  return html`
    <${Card} padding="md">
      <div className="mb-4 h-3 w-28 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
      <div className="grid gap-3 sm:grid-cols-3 lg:grid-cols-6">
        ${[1, 2, 3, 4, 5, 6].map(
          (i) => html`
            <div key=${i}>
              <div className="h-3 w-16 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
              <div className="mt-2 h-4 w-20 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
            </div>
          `
        )}
      </div>
    <//>
  `;
}

export function BudgetTab({ searchQuery = "" }) {
  const t = useT();
  const { budget, query } = useBudget();

  if (
    !matchesSearch(searchQuery, [
      "budget",
      "usage",
      "limit",
      "cost",
      t("settings.budget"),
      t("budget.title"),
    ])
  ) {
    return html`<${SettingsSearchEmpty} query=${searchQuery} />`;
  }

  if (query.isLoading) {
    return html`<${BudgetSkeleton} />`;
  }

  if (query.isError) {
    return html`
      <${Card} padding="md">
        <p className="text-sm text-[var(--v2-danger-text)]">
          ${t("budget.loadFailed", { message: query.error?.message || t("common.unknown") })}
        </p>
      <//>
    `;
  }

  const accounts = (budget?.accounts || []).map((account) => ({
    ...account,
    generated_at: budget?.generated_at,
  }));

  return html`
    <${Card} padding="md">
      <div className="mb-4 flex items-center justify-between gap-3">
        <div className="min-w-0">
          <h2 className="text-sm font-semibold text-[var(--v2-text-strong)]">
            ${t("budget.title")}
          </h2>
          <p className="mt-1 text-xs text-[var(--v2-text-muted)]">
            ${t("budget.description")}
          </p>
        </div>
      </div>
      ${accounts.length === 0
        ? html`<p className="py-4 text-sm text-[var(--v2-text-muted)]">
            ${t("budget.empty")}
          </p>`
        : accounts.map(
            (account) =>
              html`<${BudgetAccountRow} key=${account.account} account=${account} />`
          )}
    <//>
  `;
}
