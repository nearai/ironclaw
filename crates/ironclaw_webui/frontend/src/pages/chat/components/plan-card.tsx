/**
 * PlanCard — the Plan-mode surface. When the agent is asked to plan a batch of
 * productivity tasks, it proposes them together and the user approves the whole
 * set at once (or adjusts / drops individual items first).
 *
 * The items are ordinary suggested `AutomationTask`s driven by the shared
 * `useAutomationTasks` hook, so a per-item Approve/Modify/Cancel and the
 * plan-level "Approve all" run through the exact same command seam.
 */
import React from "react";
import { Badge } from "../../../design-system/badge";
import { Button } from "../../../design-system/button";
import { Icon } from "../../../design-system/icons";
import { useT } from "../../../lib/i18n";
import { appChipStyle, appMeta } from "../lib/automation-app-meta";
import {
  isAutomated,
  isSuggested,
  type AutomationPlan,
  type AutomationTask,
} from "../lib/automation-tasks";
import type { AutomationTasksApi } from "../hooks/useAutomationTasks";
import { ModifyTaskDialog } from "./modify-task-dialog";
import { NearProcessIndicator } from "./near-process-indicator";

function itemStateBadge(task: AutomationTask) {
  if (isAutomated(task)) return { tone: "success" as const, key: "automation.state.automated" };
  if (task.state === "reverted") return { tone: "muted" as const, key: "automation.state.reverted" };
  if (task.state === "cancelled") return { tone: "muted" as const, key: "automation.state.cancelled" };
  return { tone: "accent" as const, key: "automation.state.suggested" };
}

export function PlanCard({
  plan,
  automations,
}: {
  plan: AutomationPlan;
  automations: AutomationTasksApi;
}) {
  const t = useT();
  const [modifyTask, setModifyTask] = React.useState<AutomationTask | null>(null);
  const { tasks, isBusy, pendingAction, approve, modify, cancel } = automations;

  const suggested = tasks.filter(isSuggested);
  const remaining = suggested.length;
  const anyBusy = tasks.some((task) => isBusy(task.id));
  const allResolved = tasks.length > 0 && remaining === 0;

  const approveAll = () => {
    for (const task of suggested) approve(task.id);
  };
  const dismissAll = () => {
    for (const task of suggested) cancel(task.id);
  };

  return (
    <div
      data-testid="plan-card"
      className="mx-auto w-full max-w-xl rounded-[18px] border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] shadow-[var(--v2-card-shadow)]"
    >
      <div className="flex items-center gap-2.5 border-b border-[var(--v2-panel-border)] px-4 py-3">
        <span className="grid h-8 w-8 shrink-0 place-items-center rounded-[10px] border border-[color-mix(in_srgb,var(--v2-accent-text)_36%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]">
          <Icon name="list" className="h-4 w-4" />
        </span>
        <div className="min-w-0">
          <div className="truncate text-sm font-semibold text-[var(--v2-text-strong)]">
            {plan.title}
          </div>
          <div className="truncate text-xs text-[var(--v2-text-muted)]">
            {plan.summary}
          </div>
        </div>
        <Badge
          tone={allResolved ? "success" : "accent"}
          label={
            allResolved
              ? t("plan.allDone")
              : t("plan.remaining", { count: String(remaining) })
          }
          dot={allResolved}
          size="sm"
          className="ml-auto"
        />
      </div>

      <ul className="divide-y divide-[var(--v2-panel-border)]">
        {tasks.map((task) => {
          const meta = appMeta(task.app);
          const badge = itemStateBadge(task);
          const busy = isBusy(task.id);
          const done = !isSuggested(task);
          return (
            <li key={task.id} className="flex items-center gap-3 px-4 py-3">
              <span
                className="grid h-7 w-7 shrink-0 place-items-center rounded-[9px] border"
                style={appChipStyle(task.app)}
              >
                <Icon name={meta.icon} className="h-3.5 w-3.5" />
              </span>
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="truncate text-sm font-medium text-[var(--v2-text-strong)]">
                    {task.title}
                  </span>
                  <Badge tone={badge.tone} label={t(badge.key)} dot={false} size="sm" />
                </div>
                <p className="truncate text-xs text-[var(--v2-text-muted)]">
                  {task.summary}
                </p>
              </div>
              {!done && (
                <div className="flex shrink-0 items-center gap-1">
                  <button
                    type="button"
                    onClick={() => setModifyTask(task)}
                    disabled={busy}
                    aria-label={t("automation.action.modify")}
                    title={t("automation.action.modify")}
                    className="grid h-8 w-8 place-items-center rounded-[9px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)] hover:text-[var(--v2-text-strong)] disabled:opacity-50"
                  >
                    <Icon name="edit" className="h-4 w-4" />
                  </button>
                  <button
                    type="button"
                    onClick={() => cancel(task.id)}
                    disabled={busy}
                    aria-label={t("automation.action.cancel")}
                    title={t("automation.action.cancel")}
                    className="grid h-8 w-8 place-items-center rounded-[9px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)] hover:text-[var(--v2-danger-text)] disabled:opacity-50"
                  >
                    <Icon name={pendingAction(task.id) === "cancel" ? "clock" : "close"} className="h-4 w-4" />
                  </button>
                </div>
              )}
              {done && isAutomated(task) && (
                <Icon name="check" className="h-4 w-4 shrink-0 text-[var(--v2-positive-text)]" />
              )}
            </li>
          );
        })}
      </ul>

      <div className="flex flex-wrap items-center gap-2 border-t border-[var(--v2-panel-border)] px-4 py-3">
        {allResolved ? (
          <span className="inline-flex items-center gap-2 text-sm text-[var(--v2-text-muted)]">
            <Icon name="check" className="h-4 w-4 text-[var(--v2-positive-text)]" />
            {t("plan.finished")}
          </span>
        ) : anyBusy ? (
          <NearProcessIndicator state="working" label={t("plan.running")} />
        ) : (
          <>
            <Button variant="primary" onClick={approveAll} disabled={anyBusy}>
              <Icon name="check" className="mr-1.5 h-4 w-4" />
              {t("plan.approveAll", { count: String(remaining) })}
            </Button>
            <Button variant="ghost" onClick={dismissAll} disabled={anyBusy}>
              {t("plan.dismiss")}
            </Button>
          </>
        )}
      </div>

      <ModifyTaskDialog
        task={modifyTask}
        open={Boolean(modifyTask)}
        onClose={() => setModifyTask(null)}
        onSave={(patch) => {
          if (modifyTask) modify(modifyTask.id, patch);
        }}
      />
    </div>
  );
}
