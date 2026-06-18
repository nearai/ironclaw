import { useNavigate } from "react-router";
import { Button } from "../../../design-system/button.js";
import { Icon } from "../../../design-system/icons.js";
import { EmptyPanel, Panel, StatusPill } from "../../../design-system/primitives.js";
import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { buildScopedLogsPath } from "../../logs/lib/logs-data.js";
import { AutomationDetailBody } from "./automation-detail-body.js";

// The persistent, full-screen view for one automation. Reached by popping out
// of the detail modal (or by deep link to `/automations/:automationId`). Unlike
// the modal, this is a routed page: it survives reloads and exposes a direct
// jump into the scoped logs for the most recent run.
export function AutomationDetailPage({ automation, isLoading, error }) {
  const t = useT();
  const navigate = useNavigate();
  const goBack = () => navigate("/automations");

  const backButton = html`
    <button
      type="button"
      onClick=${goBack}
      className="inline-flex items-center gap-1.5 text-sm font-medium text-iron-300 hover:text-iron-100"
    >
      <${Icon} name="chevron" className="h-4 w-4 rotate-90" />
      ${t("automations.detail.backToList")}
    </button>
  `;

  if (isLoading) {
    return html`
      <div className="flex h-full flex-col overflow-y-auto">
        <div className="v2-page-entrance flex-1 space-y-5 p-4 sm:p-6">
          ${backButton}
          <div className="v2-skeleton h-28 rounded-[18px]" />
          <div className="v2-skeleton h-64 rounded-[18px]" />
        </div>
      </div>
    `;
  }

  if (error || !automation) {
    return html`
      <div className="flex h-full flex-col overflow-y-auto">
        <div className="v2-page-entrance flex-1 space-y-5 p-4 sm:p-6">
          ${backButton}
          <${EmptyPanel}
            title=${t("automations.detail.notFoundTitle")}
            description=${t("automations.detail.notFoundDescription")}
          >
            <${Button} variant="secondary" size="sm" onClick=${goBack}>
              ${t("automations.detail.backToList")}
            <//>
          <//>
        </div>
      </div>
    `;
  }

  const statusTone = automation.has_running_run ? "info" : automation.state_tone;
  const statusLabel = automation.has_running_run
    ? t("automations.status.running")
    : automation.state_label;

  // Jump straight to the scoped logs for the most recent run that has a thread
  // or run id; falls back to the unscoped logs page when nothing is attached.
  const latestScoped = automation.recent_runs.find(
    (run) => run.thread_id || run.run_id
  );
  const logsPath = buildScopedLogsPath({
    threadId: latestScoped?.thread_id,
    runId: latestScoped?.run_id,
  });

  return html`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 space-y-5 p-4 sm:p-6">
        ${backButton}

        <${Panel} className="p-4 sm:p-5">
          <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
            <div className="flex min-w-0 items-start gap-3">
              <span
                className="grid h-11 w-11 shrink-0 place-items-center rounded-[12px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-iron-200"
              >
                <${Icon} name=${automation.icon} className="h-5 w-5" />
              </span>
              <div className="min-w-0">
                <h1 className="truncate text-2xl font-semibold tracking-tight text-iron-100">
                  ${automation.display_name}
                </h1>
                <div className="mt-1.5 truncate font-mono text-[11px] uppercase tracking-[0.12em] text-iron-400">
                  ${automation.automation_id}
                </div>
              </div>
            </div>
            <div className="flex items-center gap-3">
              <${StatusPill} tone=${statusTone} label=${statusLabel} />
              <${Button}
                variant="secondary"
                size="sm"
                onClick=${() => navigate(logsPath)}
              >
                <${Icon} name="file" className="mr-1.5 h-4 w-4" />
                ${t("automations.detail.viewLogs")}
              <//>
            </div>
          </div>
        <//>

        <${Panel} className="p-4 sm:p-5">
          <${AutomationDetailBody}
            automation=${automation}
            onOpenRun=${navigate}
            onOpenLogs=${navigate}
          />
        <//>
      </div>
    </div>
  `;
}
