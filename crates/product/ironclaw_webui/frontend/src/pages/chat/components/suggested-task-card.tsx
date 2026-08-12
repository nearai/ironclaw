/**
 * SuggestedTaskCard — one OOBE first-run suggestion rendered as a status
 * affordance (PROPOSAL §2A). Pure and presentational: it takes a `SuggestedTask`
 * plus optional action callbacks and renders exactly one action row for the
 * task's `state`. No data fetching, no `useAuth`, no automation wiring — the
 * callbacks are handed in by the surface.
 *
 *   unconnected → Connect <tool>   (onConnect)
 *   suggested   → Approve          (onApprove)      — no Modify
 *   running     → live NEAR "working" indicator (activity lives in the thread)
 *   completed   → Completed chip + "+ Automation" (onAutomation) — no Revert/Modify
 *   failed      → "Couldn't complete" chip + Try again (onApprove)
 *
 * `locked` (another job is running — §2A change 3) visually disables the card.
 */
import { Button } from "../../../design-system/button";
import { Icon } from "../../../design-system/icons";
import { useT } from "../../../lib/i18n";
import { appChipStyle, appMeta, type SuggestedTask } from "../lib/suggested-tasks";
import { NearProcessIndicator } from "./near-process-indicator";

export function SuggestedTaskCard({
  task,
  onConnect,
  onApprove,
  onAutomation,
  onDismiss,
  locked = false,
}: {
  task: SuggestedTask;
  onConnect?: () => void;
  onApprove?: () => void;
  onAutomation?: () => void;
  onDismiss?: () => void;
  locked?: boolean;
}) {
  const t = useT();
  const meta = appMeta(task.app);
  const appLabel = t(meta.labelKey);

  return (
    <div
      role="group"
      aria-label={task.title}
      aria-disabled={locked || undefined}
      className={[
        "flex flex-col rounded-[13px] border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] p-3 text-left transition-colors",
        locked
          ? "pointer-events-none opacity-50"
          : "hover:border-[color-mix(in_srgb,var(--v2-accent)_32%,var(--v2-panel-border))]",
      ].join(" ")}
    >
      {/* Identity + dismiss */}
      <div className="mb-1.5 flex items-center gap-1.5">
        <span
          className="grid h-5 w-5 shrink-0 place-items-center rounded-[6px] border"
          style={appChipStyle(task.app)}
        >
          <Icon name={meta.icon} className="h-3 w-3" />
        </span>
        <span className="truncate text-[11px] font-medium text-[var(--v2-text-muted)]">
          {appLabel}
        </span>
        <button
          type="button"
          onClick={() => onDismiss?.()}
          disabled={locked}
          aria-label={t("chat.oobe.dismiss")}
          className="ml-auto grid h-5 w-5 shrink-0 place-items-center rounded-[6px] text-[var(--v2-text-faint)] transition-colors hover:text-[var(--v2-text-strong)] disabled:cursor-not-allowed"
        >
          <Icon name="close" className="h-3.5 w-3.5" />
        </button>
      </div>

      {/* Title + summary */}
      <div className="text-[13px] font-semibold leading-tight text-[var(--v2-text-strong)]">
        {task.title}
      </div>
      <p className="mt-0.5 line-clamp-2 text-[11px] leading-4 text-[var(--v2-text-muted)]">
        {task.summary}
      </p>

      {/* One action row per state */}
      <div className="mt-2.5">{renderActions()}</div>
    </div>
  );

  function renderActions() {
    switch (task.state) {
      case "unconnected":
        return (
          <Button
            variant="secondary"
            size="sm"
            onClick={() => onConnect?.()}
            disabled={locked}
          >
            <Icon name="plug" className="mr-1 h-3.5 w-3.5" />
            {t("chat.oobe.action.connect", {
              tool: task.connectLabel ?? appLabel,
            })}
          </Button>
        );
      case "suggested":
        return (
          <Button
            variant="primary"
            size="sm"
            onClick={() => onApprove?.()}
            disabled={locked}
          >
            <Icon name="check" className="mr-1 h-3.5 w-3.5" />
            {t("chat.oobe.action.approve")}
          </Button>
        );
      case "running":
        return (
          <NearProcessIndicator
            state="working"
            label={t("chat.oobe.status.running")}
          />
        );
      case "completed":
        return (
          <div className="flex flex-wrap items-center gap-2">
            <span className="inline-flex items-center gap-1 rounded-full border border-[color-mix(in_srgb,var(--v2-positive-text)_45%,transparent)] bg-[var(--v2-positive-soft)] px-2 py-0.5 text-[11px] font-medium text-[var(--v2-positive-text)]">
              <Icon name="check" className="h-3 w-3" />
              {t("chat.oobe.status.completed")}
            </span>
            <Button
              variant="secondary"
              size="sm"
              onClick={() => onAutomation?.()}
              disabled={locked}
            >
              <Icon name="plus" className="mr-1 h-3.5 w-3.5" />
              {t("chat.oobe.action.automation")}
            </Button>
          </div>
        );
      case "failed":
        return (
          <div className="flex flex-wrap items-center gap-2">
            <span className="inline-flex items-center gap-1 rounded-full border border-[color-mix(in_srgb,var(--v2-danger-text)_45%,transparent)] bg-[var(--v2-danger-soft)] px-2 py-0.5 text-[11px] font-medium text-[var(--v2-danger-text)]">
              <Icon name="alert" className="h-3 w-3" />
              {t("chat.oobe.status.failed")}
            </span>
            <Button
              variant="secondary"
              size="sm"
              onClick={() => onApprove?.()}
              disabled={locked}
            >
              <Icon name="retry" className="mr-1 h-3.5 w-3.5" />
              {t("chat.oobe.action.tryAgain")}
            </Button>
          </div>
        );
      default:
        return null;
    }
  }
}
