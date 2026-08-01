/**
 * CalendarRescheduleCard — the inline thread rich-preview of a conflicting-meeting
 * reschedule. Shows the before → after move as a compact agenda so the user can
 * read the change at a glance, then act on it:
 *
 *   suggested → Approve / Modify / Cancel   (agent is proposing the move)
 *   automated → Modify / Revert             (Auto mode already moved it)
 *
 * Rendered as a MessageList child in the thread; in the design prototype it is
 * driven by mock data, but the shape is the intended rich-preview event payload.
 */
import React from "react";
import { Badge } from "../../../design-system/badge";
import { Button } from "../../../design-system/button";
import { Icon } from "../../../design-system/icons";
import { useT } from "../../../lib/i18n";
import { appChipStyle, appMeta } from "../lib/automation-app-meta";
import type {
  AutomationTask,
  AutomationTaskPatch,
  CalendarSlot,
} from "../lib/automation-tasks";
import { TaskActionBar } from "./task-action-bar";
import { ModifyTaskDialog } from "./modify-task-dialog";

function SlotBlock({
  variant,
  captionKey,
  slot,
  meetingTitle,
  conflictNote,
}: {
  variant: "was" | "now";
  captionKey: string;
  slot: CalendarSlot;
  meetingTitle: string;
  conflictNote?: string;
}) {
  const t = useT();
  const isWas = variant === "was";
  return (
    <div
      className={[
        "flex-1 rounded-[13px] border p-3",
        isWas
          ? "border-[color-mix(in_srgb,var(--v2-danger-text)_28%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)]"
          : "border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)]",
      ].join(" ")}
    >
      <div className="mb-1.5 flex items-center gap-1.5">
        <span
          className={[
            "font-mono text-[0.625rem] font-semibold uppercase tracking-[0.14em]",
            isWas ? "text-[var(--v2-danger-text)]" : "text-[var(--v2-positive-text)]",
          ].join(" ")}
        >
          {t(captionKey)}
        </span>
        <Icon
          name={isWas ? "close" : "check"}
          className={[
            "h-3.5 w-3.5",
            isWas ? "text-[var(--v2-danger-text)]" : "text-[var(--v2-positive-text)]",
          ].join(" ")}
        />
      </div>
      <div className="text-xs font-medium text-[var(--v2-text-muted)]">{slot.day}</div>
      <div
        className={[
          "text-sm font-semibold",
          isWas
            ? "text-[var(--v2-text-muted)] line-through decoration-[color-mix(in_srgb,var(--v2-danger-text)_60%,transparent)]"
            : "text-[var(--v2-text-strong)]",
        ].join(" ")}
      >
        {slot.time}
      </div>
      <div className="mt-1 truncate text-xs text-[var(--v2-text-muted)]">
        {meetingTitle}
      </div>
      {conflictNote && (
        <div className="mt-1.5 inline-flex items-center gap-1 text-[0.6875rem] text-[var(--v2-danger-text)]">
          <Icon name="bolt" className="h-3 w-3" />
          {conflictNote}
        </div>
      )}
    </div>
  );
}

function launch(url?: string) {
  if (!url) return;
  window.open(url, "_blank", "noopener,noreferrer");
}

export function CalendarRescheduleCard({
  task,
  busy = false,
  pendingAction = null,
  onApprove,
  onModify,
  onCancel,
  onRevert,
}: {
  task: AutomationTask;
  busy?: boolean;
  pendingAction?: "approve" | "modify" | "cancel" | "revert" | null;
  onApprove?: () => void;
  onModify?: (patch: AutomationTaskPatch) => void;
  onCancel?: () => void;
  onRevert?: () => void;
}) {
  const t = useT();
  const [modifyOpen, setModifyOpen] = React.useState(false);
  const reschedule = task.reschedule;
  if (!reschedule) return null;

  const meta = appMeta(task.app);
  const appLabel = t(meta.labelKey);
  const suggested = task.state === "suggested" || task.state === "in_progress";
  const terminal = task.state === "reverted" || task.state === "cancelled";

  const stateBadge = suggested
    ? { tone: "accent" as const, key: "automation.state.suggested" }
    : task.state === "automated"
      ? { tone: "success" as const, key: "automation.state.automated" }
      : task.state === "reverted"
        ? { tone: "muted" as const, key: "automation.state.reverted" }
        : { tone: "muted" as const, key: "automation.state.cancelled" };

  return (
    <div
      data-testid="calendar-reschedule-card"
      className="mx-auto w-full max-w-xl rounded-[18px] border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] shadow-[var(--v2-card-shadow)]"
    >
      <div className="flex items-center gap-2.5 border-b border-[var(--v2-panel-border)] px-4 py-3">
        <span
          className="grid h-8 w-8 shrink-0 place-items-center rounded-[10px] border"
          style={appChipStyle(task.app)}
        >
          <Icon name={meta.icon} className="h-4 w-4" />
        </span>
        <div className="min-w-0">
          <div className="truncate text-sm font-semibold text-[var(--v2-text-strong)]">
            {task.title}
          </div>
          <div className="text-xs text-[var(--v2-text-muted)]">{appLabel}</div>
        </div>
        <Badge
          tone={stateBadge.tone}
          label={t(stateBadge.key)}
          dot={task.state === "automated"}
          size="sm"
          className="ml-auto"
        />
      </div>

      <div className="px-4 py-4">
        <p className="mb-3 text-sm leading-6 text-[var(--v2-text-base)]">
          {suggested
            ? t("automation.reschedule.proposeLine", {
                meeting: reschedule.meetingTitle,
                conflict: reschedule.conflictWith,
              })
            : t("automation.reschedule.doneLine", {
                meeting: reschedule.meetingTitle,
                conflict: reschedule.conflictWith,
              })}
        </p>

        <div className="flex items-stretch gap-2">
          <SlotBlock
            variant="was"
            captionKey="automation.reschedule.was"
            slot={reschedule.from}
            meetingTitle={reschedule.meetingTitle}
            conflictNote={t("automation.reschedule.conflictWith", {
              conflict: reschedule.conflictWith,
            })}
          />
          <div className="flex shrink-0 items-center px-0.5 text-[var(--v2-text-faint)]">
            <Icon name="chevron" className="h-5 w-5 -rotate-90" />
          </div>
          <SlotBlock
            variant="now"
            captionKey={suggested ? "automation.reschedule.proposed" : "automation.reschedule.now"}
            slot={reschedule.to}
            meetingTitle={reschedule.meetingTitle}
          />
        </div>

        <div className="mt-3 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-[var(--v2-text-muted)]">
          <Icon name="layers" className="h-3.5 w-3.5" />
          <span className="font-medium text-[var(--v2-text-base)]">
            {t("automation.reschedule.attendees")}
          </span>
          {reschedule.attendees.join(", ")}
        </div>

        {reschedule.note && (
          <div className="mt-2 rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 py-2 text-xs text-[var(--v2-text-muted)]">
            {reschedule.note}
          </div>
        )}
      </div>

      <div className="flex flex-wrap items-center gap-2 border-t border-[var(--v2-panel-border)] px-4 py-3">
        <TaskActionBar
          task={task}
          busy={busy}
          pendingAction={pendingAction}
          size="md"
          onApprove={onApprove}
          onModify={() => setModifyOpen(true)}
          onCancel={onCancel}
          onRevert={onRevert}
        />
        {!terminal && task.launchUrl && (
          <Button
            variant="ghost"
            size="md"
            className="ml-auto"
            onClick={() => launch(task.launchUrl)}
          >
            <Icon name="external" className="mr-1.5 h-4 w-4" />
            {t("automation.launch", { app: appLabel })}
          </Button>
        )}
      </div>

      <ModifyTaskDialog
        task={task}
        open={modifyOpen}
        onClose={() => setModifyOpen(false)}
        onSave={(patch) => onModify?.(patch)}
      />
    </div>
  );
}
