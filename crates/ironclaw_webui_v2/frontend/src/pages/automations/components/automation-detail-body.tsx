// @ts-nocheck
import { useT } from "../../../lib/i18n";
import { cn } from "../../../utils/cn";
import {
  RecentRunRow,
  recentRunKey,
  RunHistorySummary,
} from "./automation-recent-runs";

// A labelled metadata tile. Shared by the detail modal and the full-screen
// detail page so both surfaces read identically.
function MetaItem({ label, value, tone = null }) {
  return (
    <div className="min-w-0 rounded-[var(--v2-radius-md)] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.14em] text-[var(--v2-text-faint)]">
        {label}
      </div>
      <div
        className={cn(
          "mt-2 min-w-0 break-words text-sm text-[var(--v2-text-strong)]",
          tone === "success" && "text-[var(--v2-positive-text)]",
          tone === "danger" && "text-[var(--v2-danger-text)]",
          tone === "signal" && "text-[var(--v2-positive-text)]",
          tone === "info" && "text-[var(--v2-info-text)]"
        )}
      >
        {value || "—"}
      </div>
    </div>
  );
}

// The read-only detail content for a single automation: a metadata grid plus
// the recent-run history. Rendered inside the per-automation modal and the
// persistent full-screen page. Container chrome (panel / modal / header) is
// owned by the caller so this stays layout-agnostic.
export function AutomationDetailBody({ automation }) {
  const t = useT();
  const activeRun = automation.current_run;

  return (
    <div className="space-y-5">
      <div className="grid gap-3 sm:grid-cols-2">
        <MetaItem
          label={t("automations.detail.schedule")}
          value={automation.schedule_label}
        />
        <MetaItem
          label={t("automations.detail.successRate")}
          value={automation.success_rate_label}
          tone={automation.has_failed_runs ? "danger" : "success"}
        />
        <MetaItem
          label={t("automations.detail.lastCompleted")}
          value={automation.last_run_label}
        />
        <MetaItem
          label={t("automations.detail.currentRun")}
          value={
            activeRun?.run_id ||
            activeRun?.thread_id ||
            t("automations.detail.noCurrentRun")
          }
          tone={automation.has_running_run ? "signal" : null}
        />
      </div>

      <div>
        <RunHistorySummary runs={automation.recent_runs} className="mb-4" />

        {(automation.recent_runs || []).length ? (
          <div>
            <h4 className="border-b border-[var(--v2-panel-border)] pb-2 text-base font-semibold text-[var(--v2-text-strong)]">
              {t("automations.detail.recentRuns")}
            </h4>
            {(automation.recent_runs || []).map((run) => (
              <RecentRunRow key={recentRunKey(run)} run={run} />
            ))}
          </div>
        ) : (
          <div className="rounded-[var(--v2-radius-md)] border border-dashed border-[var(--v2-panel-border)] p-4 text-sm text-[var(--v2-text-muted)]">
            {t("automations.detail.noRuns")}
          </div>
        )}
      </div>
    </div>
  );
}
