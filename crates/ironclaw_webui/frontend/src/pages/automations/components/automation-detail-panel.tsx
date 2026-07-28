import React from "react";
import { Button } from "../../../design-system/button";
import { ConfirmDialog } from "../../../design-system/confirm-dialog";
import { Icon } from "../../../design-system/icons";
import { Input } from "../../../design-system/input";
import { EmptyPanel, Panel, StatusPill } from "../../../design-system/primitives";
import { useT } from "../../../lib/i18n";
import { cn } from "../../../utils/cn";
import {
  RecentRunRow,
  recentRunKey,
  RunDots,
  RunHistorySummary,
} from "./automation-recent-runs";

const AUTOMATION_NAME_MAX_BYTES = 256;

function MetaItem({ label, value, tone = "muted" }) {
  return (
    <div className="min-w-0 rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.14em] text-iron-400">
        {label}
      </div>
      <div
        className={cn(
          "mt-2 min-w-0 break-words text-sm text-iron-100",
          tone === "success" && "text-[var(--v2-positive-text)]",
          tone === "danger" && "text-red-200",
          tone === "info" && "text-sky-200"
        )}
      >
        {value || "—"}
      </div>
    </div>
  );
}

export function AutomationDetailPanel({
  automation,
  isMutating = false,
  onPauseAutomation,
  onResumeAutomation,
  onRenameAutomation,
  onDeleteAutomation,
}) {
  const t = useT();
  const [isEditingName, setIsEditingName] = React.useState(false);
  const [draftName, setDraftName] = React.useState("");
  const [nameError, setNameError] = React.useState("");
  const [deleteDialogOpen, setDeleteDialogOpen] = React.useState(false);

  React.useEffect(() => {
    setIsEditingName(false);
    setDraftName(automation?.display_name || "");
    setNameError("");
    setDeleteDialogOpen(false);
  }, [automation?.automation_id]);

  if (!automation) {
    return (
      <Panel className="p-4 sm:p-5">
        <EmptyPanel
          boxed={false}
          title={t("automations.detail.emptyTitle")}
          description={t("automations.detail.emptyDescription")}
        />
      </Panel>
    );
  }

  const activeRun = automation.current_run;
  const canResume = automation.state === "paused";
  const canPause = automation.state === "active" || automation.state === "scheduled";
  const canRename = Boolean(onRenameAutomation);
  const actionLabel = canResume ? t("missions.action.resume") : t("missions.action.pause");
  const actionTitle = `${actionLabel}: ${automation.display_name}`;
  const renameTitle = `${t("automations.rename.action")}: ${automation.display_name}`;
  const handleAction = () => {
    if (canResume) {
      onResumeAutomation?.(automation.automation_id);
      return;
    }
    if (canPause) {
      onPauseAutomation?.(automation.automation_id);
    }
  };
  const deleteTitle = `${t("common.delete")}: ${automation.display_name}`;
  const handleDelete = () => {
    setDeleteDialogOpen(true);
  };
  const handleConfirmDelete = () => {
    onDeleteAutomation?.(automation.automation_id);
  };
  const handleRenameStart = () => {
    setDraftName(automation.display_name);
    setNameError("");
    setIsEditingName(true);
  };
  const handleRenameCancel = () => {
    setDraftName(automation.display_name);
    setNameError("");
    setIsEditingName(false);
  };
  const handleRenameSubmit = (event) => {
    event.preventDefault();
    const name = draftName.trim();
    if (!name) {
      setNameError(t("automations.rename.nameRequired"));
      return;
    }
    if (byteLength(name) > AUTOMATION_NAME_MAX_BYTES) {
      setNameError(t("automations.rename.nameTooLong"));
      return;
    }
    setNameError("");
    if (name !== automation.display_name) {
      onRenameAutomation?.({ automationId: automation.automation_id, name });
    }
    setIsEditingName(false);
  };

  return (
    <Panel className="overflow-hidden" data-testid="automation-detail-panel">
      <div className="border-b border-[var(--v2-panel-border)] p-4 sm:p-5">
        <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
          <div className="min-w-0">
            {isEditingName
              ? (
                  <form
                    className="flex min-w-0 flex-col gap-2"
                    onSubmit={handleRenameSubmit}
                  >
                    <div className="flex min-w-0 items-start gap-2">
                      <Input
                        size="sm"
                        value={draftName}
                        data-testid="automation-rename-input"
                        aria-label={t("automations.rename.nameLabel")}
                        disabled={isMutating}
                        error={Boolean(nameError)}
                        className="min-w-0"
                        onInput={(event) => {
                          setDraftName(event.currentTarget.value);
                          if (nameError) setNameError("");
                        }}
                      />
                      <Button
                        type="submit"
                        variant="primary"
                        size="icon-sm"
                        data-testid="automation-rename-save"
                        aria-label={t("common.save")}
                        title={t("common.save")}
                        disabled={isMutating}
                      >
                        <Icon name="check" className="h-4 w-4" />
                      </Button>
                      <Button
                        type="button"
                        variant="secondary"
                        size="icon-sm"
                        aria-label={t("common.cancel")}
                        title={t("common.cancel")}
                        disabled={isMutating}
                        onClick={handleRenameCancel}
                      >
                        <Icon name="close" className="h-4 w-4" />
                      </Button>
                    </div>
                    {nameError && (
                      <div className="text-xs text-[var(--v2-danger-text)]" role="alert">
                        {nameError}
                      </div>
                    )}
                  </form>
                )
              : (
                  <div className="flex min-w-0 items-center gap-2">
                    <h3
                      data-testid="automation-detail-title"
                      className="min-w-0 flex-1 truncate text-xl font-semibold tracking-tight text-iron-100"
                    >
                      {automation.display_name}
                    </h3>
                    {canRename && (
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon-sm"
                        className="shrink-0"
                        data-testid="automation-rename-button"
                        aria-label={renameTitle}
                        title={renameTitle}
                        disabled={isMutating}
                        onClick={handleRenameStart}
                      >
                        <Icon name="edit" className="h-4 w-4" />
                      </Button>
                    )}
                  </div>
                )}
            <div className="mt-2 truncate font-mono text-[11px] uppercase tracking-[0.12em] text-iron-400">
              {automation.automation_id}
            </div>
            {automation.hold_meta_label && (
              <div
                data-testid="automation-hold-meta"
                className="mt-1 text-xs text-iron-300"
              >
                {automation.hold_meta_label}
              </div>
            )}
          </div>
          <div className="flex shrink-0 items-center gap-2">
            <StatusPill
              tone={automation.primary_status_tone}
              label={automation.primary_status_label}
            />
            {(canPause || canResume) &&
            (
              <Button
                type="button"
                variant={canResume ? "primary" : "secondary"}
                size="icon-sm"
                aria-label={actionTitle}
                title={actionTitle}
                disabled={isMutating}
                onClick={handleAction}
              >
                <Icon name={canResume ? "play" : "pause"} className="h-4 w-4" />
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
      </div>

      <div className="space-y-5 p-4 sm:p-5">
        <div className="grid gap-3 sm:grid-cols-2">
          <MetaItem label={t("automations.detail.schedule")} value={automation.schedule_label} />
          <MetaItem
            label={t("automations.detail.successRate")}
            value={automation.success_rate_label}
            tone={automation.has_failed_runs ? "danger" : "success"}
          />
          <MetaItem label={t("automations.detail.lastCompleted")} value={automation.last_run_label} />
          <MetaItem
            label={t("automations.detail.currentRun")}
            value={activeRun?.run_id || activeRun?.thread_id || t("automations.detail.noCurrentRun")}
            tone={automation.has_running_run ? "info" : null}
          />
        </div>

        <div>
          <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
            <h4 className="text-sm font-semibold text-iron-100">
              {t("automations.detail.recentRuns")}
            </h4>
            <div className="flex flex-col items-end gap-1">
              <RunDots runs={automation.recent_runs} />
              <RunHistorySummary runs={automation.recent_runs} />
            </div>
          </div>

          {automation.recent_runs.length
            ? (
                <div>
                  {automation.recent_runs.map((run) => (
                    <RecentRunRow
                      key={recentRunKey(run)}
                      run={run}
                    />
                  ))}
                </div>
              )
            : (
                <div className="rounded-xl border border-dashed border-[var(--v2-panel-border)] p-4 text-sm text-iron-300">
                  {t("automations.detail.noRuns")}
                </div>
              )}
        </div>
      </div>
      <ConfirmDialog
        open={deleteDialogOpen}
        title={deleteTitle}
        confirmLabel={t("common.delete")}
        isConfirming={isMutating}
        onConfirm={handleConfirmDelete}
        onCancel={() => setDeleteDialogOpen(false)}
      />
    </Panel>
  );
}

function byteLength(value) {
  if (typeof TextEncoder === "function") {
    return new TextEncoder().encode(value).length;
  }
  return value.length;
}
