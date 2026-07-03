import { html } from "../../../lib/html.js";
import { Card } from "../../../design-system/card.js";
import { useT } from "../../../lib/i18n.js";
import { useTraceCredits } from "../hooks/useTraceCredits.js";
import { matchesSearch } from "../lib/settings-search.js";
import { SettingsSearchEmpty } from "./settings-search-empty.js";

function formatCredit(value) {
  return (Number(value) || 0).toFixed(2);
}

function formatSignedCredit(value) {
  const numeric = Number(value) || 0;
  return `${numeric >= 0 ? "+" : ""}${numeric.toFixed(2)}`;
}

function formatTimestamp(value, t) {
  if (!value) return t("traceCommons.never");
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? t("traceCommons.never") : parsed.toLocaleString();
}

function StatRow({ label, value, description }) {
  return html`
    <div
      className="flex items-center justify-between gap-3 border-t border-[var(--v2-panel-border)] py-3 first:border-0"
    >
      <div className="min-w-0">
        <div className="text-sm text-[var(--v2-text-strong)]">${label}</div>
        ${description &&
        html`<div className="mt-0.5 text-xs text-[var(--v2-text-muted)]">${description}</div>`}
      </div>
      <div className="shrink-0 font-mono text-sm text-[var(--v2-text-strong)]">${value}</div>
    </div>
  `;
}

export function TraceCommonsTab({ searchQuery = "" }) {
  const t = useT();
  const { credits, query, authorize } = useTraceCredits();

  if (
    !matchesSearch(searchQuery, [
      "trace commons",
      "credits",
      t("settings.traceCommons"),
      t("traceCommons.title"),
    ])
  ) {
    return html`<${SettingsSearchEmpty} query=${searchQuery} />`;
  }

  let body;
  if (query.isLoading) {
    body = html`
      <div className="mt-4">
        ${[1, 2, 3].map(
          (i) => html`
            <div
              key=${i}
              className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-3 first:border-0"
            >
              <div className="h-4 w-32 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
              <div className="h-4 w-16 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
            </div>
          `
        )}
      </div>
    `;
  } else if (query.isError) {
    body = html`
      <div
        className="mt-4 rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
      >
        ${t("traceCommons.loadFailed")}
      </div>
    `;
  } else if (!credits || (!credits.enrolled && !(credits.submissions_total > 0))) {
    body = html`
      <div
        className="mt-4 rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-6 text-center text-sm text-[var(--v2-text-muted)]"
      >
        ${t("traceCommons.emptyState")}
      </div>
    `;
  } else {
    const explanations = credits.recent_explanations || [];
    const holds = credits.holds || [];
    body = html`
      <div className="mt-4">
        <${StatRow}
          label=${t("traceCommons.enrollment")}
          value=${credits.enrolled ? t("traceCommons.enrolled") : t("traceCommons.notEnrolled")}
        />
        <${StatRow}
          label=${t("traceCommons.pendingCredit")}
          description=${t("traceCommons.pendingCreditDesc")}
          value=${formatCredit(credits.pending_credit)}
        />
        <${StatRow}
          label=${t("traceCommons.finalCredit")}
          description=${t("traceCommons.finalCreditDesc")}
          value=${formatCredit(credits.final_credit)}
        />
        <${StatRow}
          label=${t("traceCommons.delayedLedger")}
          description=${t("traceCommons.delayedLedgerDesc")}
          value=${formatSignedCredit(credits.delayed_credit_delta)}
        />
        <${StatRow}
          label=${t("traceCommons.submissions")}
          value=${t("traceCommons.submissionsValue", {
            submitted: credits.submissions_submitted || 0,
            accepted: credits.submissions_accepted || 0,
            total: credits.submissions_total || 0,
          })}
        />
        <${StatRow}
          label=${t("traceCommons.lastSubmission")}
          value=${formatTimestamp(credits.last_submission_at, t)}
        />
        <${StatRow}
          label=${t("traceCommons.lastSync")}
          description=${t("traceCommons.lastSyncDesc")}
          value=${formatTimestamp(credits.last_credit_sync_at, t)}
        />
      </div>
      ${explanations.length > 0 &&
      html`
        <div className="mt-5">
          <h4
            className="mb-2 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
          >
            ${t("traceCommons.recentExplanations")}
          </h4>
          <ul className="ml-4 list-disc space-y-1 text-xs text-[var(--v2-text-muted)]">
            ${explanations.map((line, index) => html`<li key=${index}>${line}</li>`)}
          </ul>
        </div>
      `}
      ${holds.length > 0 &&
      html`
        <div className="mt-5">
          <h4
            className="mb-1 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
          >
            ${t("traceCommons.heldTitle")}
          </h4>
          <p className="mb-2 text-xs leading-5 text-[var(--v2-text-muted)]">
            ${t("traceCommons.heldDescription")}
          </p>
          <ul className="space-y-2">
            ${holds.map(
              (hold) => html`
                <li
                  key=${hold.submission_id}
                  className="flex items-start justify-between gap-3 rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 py-2"
                >
                  <div className="min-w-0">
                    <div className="text-xs text-[var(--v2-text-strong)]">${hold.reason}</div>
                    <div className="mt-0.5 truncate font-mono text-[10px] text-[var(--v2-text-faint)]">
                      ${hold.submission_id}
                    </div>
                  </div>
                  <button
                    type="button"
                    onClick=${() => authorize.mutate(hold.submission_id)}
                    disabled=${authorize.isPending}
                    className="shrink-0 rounded-lg border border-[var(--v2-accent-soft)] px-2.5 py-1 text-xs font-medium text-[var(--v2-accent-text)] transition-colors hover:bg-[var(--v2-accent-soft)] disabled:cursor-not-allowed disabled:opacity-50"
                  >
                    ${authorize.isPending
                      ? t("traceCommons.authorizing")
                      : t("traceCommons.authorize")}
                  </button>
                </li>
              `
            )}
          </ul>
        </div>
      `}
    `;
  }

  return html`
    <${Card} padding="md">
      <h3
        className="mb-2 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
      >
        ${t("traceCommons.title")}
      </h3>
      <p className="text-sm leading-6 text-[var(--v2-text-muted)]">
        ${t("traceCommons.description")}
      </p>

      ${body}

      <p className="mt-5 text-xs leading-5 text-[var(--v2-text-faint)]">
        ${t("traceCommons.note")}
      </p>
    <//>
  `;
}
