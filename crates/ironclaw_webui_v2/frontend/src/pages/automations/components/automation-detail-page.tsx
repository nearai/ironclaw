// @ts-nocheck
import { useNavigate } from "react-router";
import { Button } from "../../../design-system/button";
import { Icon } from "../../../design-system/icons";
import { EmptyPanel, Panel, StatusPill } from "../../../design-system/primitives";
import { useT } from "../../../lib/i18n";
import { buildScopedLogsPath } from "../../logs/lib/logs-data";
import { AutomationDetailBody } from "./automation-detail-body";
import { EditableAutomationName } from "./automation-name";

// The persistent, full-screen view for one automation. Reached by popping out
// of the detail modal (or by deep link to `/automations/:automationId`). Unlike
// the modal, this is a routed page: it survives reloads and exposes a direct
// jump into the scoped logs for the most recent run.
export function AutomationDetailPage({
  automation,
  isLoading,
  error,
  isMutating = false,
  onPauseAutomation,
  onResumeAutomation,
  onRenameAutomation,
  onDeleteAutomation,
}) {
  const t = useT();
  const navigate = useNavigate();
  const goBack = () => navigate("/automations");

  const backButton = (
    <button
      type="button"
      onClick={goBack}
      className="inline-flex items-center gap-1.5 text-sm font-medium text-[var(--v2-text-muted)] hover:text-[var(--v2-text-strong)]"
    >
      <Icon name="chevron" className="h-4 w-4 rotate-90" />
      {t("automations.detail.backToList")}
    </button>
  );

  if (isLoading) {
    return (
      <div className="flex h-full flex-col overflow-y-auto">
        <div className="v2-page-entrance flex-1 space-y-5 p-4 sm:p-6">
          {backButton}
          <div className="v2-skeleton h-28 rounded-[1.25rem]" />
          <div className="v2-skeleton h-64 rounded-[1.25rem]" />
        </div>
      </div>
    );
  }

  if (error || !automation) {
    return (
      <div className="flex h-full flex-col overflow-y-auto">
        <div className="v2-page-entrance flex-1 space-y-5 p-4 sm:p-6">
          {backButton}
          <EmptyPanel
            title={t("automations.detail.notFoundTitle")}
            description={t("automations.detail.notFoundDescription")}
          >
            <Button variant="secondary" size="sm" onClick={goBack}>
              {t("automations.detail.backToList")}
            </Button>
          </EmptyPanel>
        </div>
      </div>
    );
  }

  const canResume = automation.state === "paused";
  const canPause = automation.state === "active" || automation.state === "scheduled";
  const actionTitle = `${
    canResume ? t("missions.action.resume") : t("missions.action.pause")
  }: ${automation.display_name}`;
  const handleAction = () => {
    if (canResume) onResumeAutomation?.(automation.automation_id);
    else if (canPause) onPauseAutomation?.(automation.automation_id);
  };
  const deleteTitle = `${t("common.delete")}: ${automation.display_name}`;
  const handleDelete = () => {
    if (window.confirm(deleteTitle)) {
      onDeleteAutomation?.(automation.automation_id);
      goBack();
    }
  };

  // Jump straight to the scoped logs for the most recent run that has a thread
  // or run id; falls back to the unscoped logs page when nothing is attached.
  const latestScoped = (automation.recent_runs || []).find(
    (run) => run.thread_id || run.run_id
  );
  const logsPath = buildScopedLogsPath({
    threadId: latestScoped?.thread_id,
    runId: latestScoped?.run_id,
  });

  return (
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 space-y-5 p-4 sm:p-6">
        {backButton}

        <Panel variant="flat" className="p-4 sm:p-5">
          <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
            <div className="flex min-w-0 items-start gap-3">
              <span className="grid h-10 w-10 shrink-0 place-items-center rounded-[var(--v2-radius-md)] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text)]">
                <Icon name={automation.icon} className="h-5 w-5" />
              </span>
              <div className="min-w-0">
                <EditableAutomationName
                  automation={automation}
                  isMutating={isMutating}
                  onRenameAutomation={onRenameAutomation}
                  headingClassName="truncate text-xl font-semibold tracking-tight text-[var(--v2-text-strong)] md:text-2xl"
                />
                <div className="mt-1.5 truncate font-mono text-[11px] uppercase tracking-[0.12em] text-[var(--v2-text-faint)]">
                  {automation.automation_id}
                </div>
              </div>
            </div>
            {/* One control size (sm) across the whole action row. */}
            <div className="flex flex-wrap items-center gap-2">
              <StatusPill
                tone={automation.primary_status_tone}
                label={automation.primary_status_label}
              />
              <Button
                variant="secondary"
                size="sm"
                onClick={() => navigate(logsPath)}
              >
                <Icon name="file" className="mr-1.5 h-4 w-4" />
                {t("automations.detail.viewLogs")}
              </Button>
              {(canPause || canResume) && (
                <Button
                  type="button"
                  variant={canResume ? "primary" : "secondary"}
                  size="sm"
                  disabled={isMutating}
                  onClick={handleAction}
                >
                  <Icon
                    name={canResume ? "play" : "pause"}
                    className={canResume ? "h-4 w-4" : "mr-1.5 h-4 w-4"}
                  />
                  {canResume ? t("missions.action.resume") : t("missions.action.pause")}
                </Button>
              )}
              <Button
                type="button"
                variant="danger"
                size="icon-sm"
                aria-label={deleteTitle}
                title={deleteTitle}
                disabled={isMutating}
                onClick={handleDelete}
              >
                <Icon name="trash" className="h-4 w-4" />
              </Button>
            </div>
          </div>
        </Panel>

        <Panel variant="flat" className="p-4 sm:p-5">
          <AutomationDetailBody
            automation={automation}
            onOpenRun={navigate}
            onOpenLogs={navigate}
          />
        </Panel>
      </div>
    </div>
  );
}
