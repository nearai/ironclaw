/**
 * AutomationTaskCard — a single completed-automation thumbnail in the landing
 * carousel. The card body is a launch target into the third-party app; the
 * action row (Modify / Revert) and launch button stop propagation so they never
 * trigger the launch.
 */
import { Badge } from "../../../design-system/badge";
import { Icon } from "../../../design-system/icons";
import { useT } from "../../../lib/i18n";
import { appChipStyle, appMeta } from "../lib/automation-app-meta";
import type { AutomationTask } from "../lib/automation-tasks";
import { TaskActionBar } from "./task-action-bar";

function launch(url?: string) {
  if (!url) return;
  window.open(url, "_blank", "noopener,noreferrer");
}

export function AutomationTaskCard({
  task,
  busy = false,
  pendingAction = null,
  onModify,
  onRevert,
}: {
  task: AutomationTask;
  busy?: boolean;
  pendingAction?: "approve" | "modify" | "cancel" | "revert" | null;
  onModify?: () => void;
  onRevert?: () => void;
}) {
  const t = useT();
  const meta = appMeta(task.app);
  const appLabel = t(meta.labelKey);
  const isTerminal = task.state === "reverted" || task.state === "cancelled";

  const stateBadge =
    task.state === "reverted"
      ? { tone: "muted" as const, key: "automation.state.reverted" }
      : task.state === "automated"
        ? { tone: "success" as const, key: "automation.state.automated" }
        : { tone: "accent" as const, key: "automation.state.suggested" };

  const hasMetrics = Boolean(task.metrics && task.metrics.length > 0) && !isTerminal;

  return (
    <div
      role="group"
      aria-label={task.title}
      className={[
        "flex w-[14.5rem] shrink-0 flex-col rounded-[13px] border bg-[var(--v2-card-bg)] p-2.5 text-left transition-colors",
        isTerminal
          ? "border-[var(--v2-panel-border)] opacity-70"
          : "border-[var(--v2-panel-border)] hover:border-[color-mix(in_srgb,var(--v2-accent)_32%,var(--v2-panel-border))]",
      ].join(" ")}
    >
      {/* Launch target — the identity + title block opens the app. */}
      <button
        type="button"
        onClick={() => launch(task.launchUrl)}
        disabled={isTerminal || !task.launchUrl}
        className="group/launch -m-0.5 mb-1.5 rounded-[10px] p-0.5 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[color-mix(in_srgb,var(--v2-accent)_32%,transparent)] disabled:cursor-default"
        title={task.launchUrl ? t("automation.launch", { app: appLabel }) : undefined}
      >
        <div className="mb-1 flex items-center gap-1.5">
          <span
            className="grid h-5 w-5 shrink-0 place-items-center rounded-[6px] border"
            style={appChipStyle(task.app)}
          >
            <Icon name={meta.icon} className="h-3 w-3" />
          </span>
          <span className="truncate text-[11px] font-medium text-[var(--v2-text-muted)]">
            {appLabel}
          </span>
          <span className="ml-auto flex shrink-0 items-center gap-1.5">
            {task.completedAt && !isTerminal && (
              <span className="whitespace-nowrap text-[10px] text-[var(--v2-text-faint)]">
                {task.completedAt}
              </span>
            )}
            <Badge
              tone={stateBadge.tone}
              label={t(stateBadge.key)}
              dot={task.state === "automated"}
              size="sm"
            />
          </span>
        </div>
        <div className="flex items-center justify-between gap-1.5">
          <div className="truncate text-[13px] font-semibold leading-tight text-[var(--v2-text-strong)]">
            {task.title}
          </div>
          {!isTerminal && task.launchUrl && (
            <Icon
              name="external"
              className="h-3.5 w-3.5 shrink-0 text-[var(--v2-text-faint)] transition-colors group-hover/launch:text-[var(--v2-accent-text)]"
            />
          )}
        </div>
        {!hasMetrics && (
          <p className="mt-0.5 line-clamp-1 text-[11px] leading-4 text-[var(--v2-text-muted)]">
            {task.summary}
          </p>
        )}
      </button>

      {hasMetrics && (
        <div className="mb-2 flex flex-wrap items-baseline gap-x-2.5 gap-y-0.5 text-[11px] text-[var(--v2-text-muted)]">
          {task.metrics.map((metric) => (
            <span key={metric.labelKey}>
              <span className="font-semibold text-[var(--v2-text-strong)]">
                {metric.value}
              </span>{" "}
              {t(metric.labelKey)}
            </span>
          ))}
        </div>
      )}

      <div className="mt-auto">
        <TaskActionBar
          task={task}
          busy={busy}
          pendingAction={pendingAction}
          size="sm"
          onModify={onModify}
          onRevert={onRevert}
        />
      </div>
    </div>
  );
}
