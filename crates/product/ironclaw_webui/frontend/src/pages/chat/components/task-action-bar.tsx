/**
 * TaskActionBar — the decision row for an automation task, shared by the
 * carousel thumbnails and the inline calendar card so the two concepts can
 * never drift on wording or button order.
 *
 *   suggested  → Approve (primary) · Modify · Cancel
 *   automated  → Modify · Revert (danger)
 *   terminal   → a muted status line (Reverted / Dismissed), no actions
 */
import { Button } from "../../../design-system/button";
import { Icon } from "../../../design-system/icons";
import { useT } from "../../../lib/i18n";
import type { AutomationTask } from "../lib/automation-tasks";
import { NearProcessIndicator } from "./near-process-indicator";

// While an action runs, the row collapses to the branded NEAR live indicator
// with a per-action progress label (shared with the streaming design language).
const BUSY_LABEL: Record<"approve" | "modify" | "cancel" | "revert", string> = {
  approve: "automation.action.approving",
  modify: "automation.action.rerunning",
  cancel: "automation.action.cancelling",
  revert: "automation.action.reverting",
};

export function TaskActionBar({
  task,
  busy = false,
  pendingAction = null,
  size = "md",
  onApprove,
  onModify,
  onCancel,
  onRevert,
}: {
  task: AutomationTask;
  busy?: boolean;
  pendingAction?: "approve" | "modify" | "cancel" | "revert" | null;
  size?: "sm" | "md";
  onApprove?: () => void;
  onModify?: () => void;
  onCancel?: () => void;
  onRevert?: () => void;
}) {
  const t = useT();
  const compact = size === "sm";
  const buttonSize = compact ? "xs" : "md";
  const iconClass = compact ? "mr-1 h-3.5 w-3.5" : "mr-1.5 h-4 w-4";
  // Compact (carousel) keeps the buttons on ONE row, each sharing the width so
  // Modify + Revert sit side by side inside the narrow card. Full-size wraps.
  const rowClass = compact
    ? "flex w-full items-center gap-1.5"
    : "flex flex-wrap items-center gap-2";
  const btnClass = compact ? "flex-1" : "";

  if (task.state === "reverted" || task.state === "cancelled") {
    const labelKey =
      task.state === "reverted"
        ? "automation.action.revertedNote"
        : "automation.action.cancelledNote";
    return (
      <div className="flex items-center gap-1.5 text-[11px] text-[var(--v2-text-muted)]">
        <Icon name="undo" className="h-3.5 w-3.5" />
        <span>{t(labelKey)}</span>
      </div>
    );
  }

  if (busy && pendingAction) {
    return (
      <div className={rowClass}>
        <NearProcessIndicator state="working" label={t(BUSY_LABEL[pendingAction])} />
      </div>
    );
  }

  if (task.state === "automated") {
    return (
      <div className={rowClass}>
        <Button
          variant="secondary"
          size={buttonSize}
          className={btnClass}
          onClick={onModify}
          disabled={busy}
          loading={pendingAction === "modify"}
        >
          {pendingAction !== "modify" && (
            <Icon name="edit" className={iconClass} />
          )}
          {pendingAction === "modify"
            ? t("automation.action.rerunning")
            : t("automation.action.modify")}
        </Button>
        <Button
          variant="danger"
          size={buttonSize}
          className={btnClass}
          onClick={onRevert}
          disabled={busy}
          loading={pendingAction === "revert"}
        >
          {pendingAction !== "revert" && (
            <Icon name="undo" className={iconClass} />
          )}
          {t("automation.action.revert")}
        </Button>
      </div>
    );
  }

  // suggested / in_progress
  return (
    <div className={rowClass}>
      <Button
        variant="primary"
        size={buttonSize}
        className={btnClass}
        onClick={onApprove}
        disabled={busy}
        loading={pendingAction === "approve"}
      >
        {pendingAction !== "approve" && (
          <Icon name="check" className={iconClass} />
        )}
        {t("automation.action.approve")}
      </Button>
      <Button
        variant="secondary"
        size={buttonSize}
        className={btnClass}
        onClick={onModify}
        disabled={busy}
      >
        <Icon name="edit" className={iconClass} />
        {t("automation.action.modify")}
      </Button>
      <Button
        variant="ghost"
        size={buttonSize}
        className={btnClass}
        onClick={onCancel}
        disabled={busy}
        loading={pendingAction === "cancel"}
      >
        {t("automation.action.cancel")}
      </Button>
    </div>
  );
}
