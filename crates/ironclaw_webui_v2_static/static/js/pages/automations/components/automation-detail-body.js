import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { cn } from "../../../utils/cn.js";
import {
  RecentRunRow,
  recentRunKey,
  RunHistorySummary,
} from "./automation-recent-runs.js";

// A labelled metadata tile. Shared by the detail modal and the full-screen
// detail page so both surfaces read identically.
function MetaItem({ label, value, tone }) {
  return html`
    <div className="min-w-0 rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.14em] text-iron-400">
        ${label}
      </div>
      <div
        className=${cn(
          "mt-2 min-w-0 break-words text-sm text-iron-100",
          tone === "success" && "text-emerald-200",
          tone === "danger" && "text-red-200",
          tone === "info" && "text-sky-200"
        )}
      >
        ${value || "—"}
      </div>
    </div>
  `;
}

// The read-only detail content for a single automation: a metadata grid plus
// the recent-run history. Rendered inside the per-automation modal and the
// persistent full-screen page. Container chrome (panel / modal / header) is
// owned by the caller so this stays layout-agnostic.
export function AutomationDetailBody({ automation, onOpenRun, onOpenLogs }) {
  const t = useT();
  const activeRun = automation.current_run;

  return html`
    <div className="space-y-5">
      <div className="grid gap-3 sm:grid-cols-2">
        <${MetaItem}
          label=${t("automations.detail.schedule")}
          value=${automation.schedule_label}
        />
        <${MetaItem}
          label=${t("automations.detail.successRate")}
          value=${automation.success_rate_label}
          tone=${automation.has_failed_runs ? "danger" : "success"}
        />
        <${MetaItem}
          label=${t("automations.detail.lastCompleted")}
          value=${automation.last_run_label}
        />
        <${MetaItem}
          label=${t("automations.detail.currentRun")}
          value=${activeRun?.run_id ||
          activeRun?.thread_id ||
          t("automations.detail.noCurrentRun")}
          tone=${automation.has_running_run ? "info" : null}
        />
      </div>

      <div>
        <div className="mb-3 flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
          <h4 className="text-sm font-semibold text-iron-100">
            ${t("automations.detail.recentRuns")}
          </h4>
          <${RunHistorySummary} runs=${automation.recent_runs} />
        </div>

        ${automation.recent_runs.length
          ? html`
              <div>
                ${automation.recent_runs.map(
                  (run) => html`
                    <${RecentRunRow}
                      key=${recentRunKey(run)}
                      run=${run}
                      onOpenRun=${onOpenRun}
                      onOpenLogs=${onOpenLogs}
                    />
                  `
                )}
              </div>
            `
          : html`
              <div className="rounded-xl border border-dashed border-[var(--v2-panel-border)] p-4 text-sm text-iron-300">
                ${t("automations.detail.noRuns")}
              </div>
            `}
      </div>
    </div>
  `;
}
