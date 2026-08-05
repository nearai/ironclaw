/**
 * AutomationCarousel — the strip of completed-automation thumbnails that sits
 * directly above the landing composer. It is the OOBE's first proof that the
 * agent has already been working: each tile launches into the third-party app
 * and carries Modify / Revert for the automated result.
 *
 * Renders nothing when there are no automations, so a fresh account (or a
 * backend with an empty projection) sees the unchanged landing.
 */
import React from "react";
import { Icon } from "../../../design-system/icons";
import { useT } from "../../../lib/i18n";
import type { AutomationTask, AutomationTaskPatch } from "../lib/automation-tasks";
import type { AutomationTasksApi } from "../hooks/useAutomationTasks";
import { AutomationTaskCard } from "./automation-task-card";
import { ModifyTaskDialog } from "./modify-task-dialog";

const SCROLL_STEP_PX = 244;

export function AutomationCarousel({
  automations,
}: {
  automations: AutomationTasksApi;
}) {
  const t = useT();
  const scrollerRef = React.useRef<HTMLDivElement | null>(null);
  const [modifyTask, setModifyTask] = React.useState<AutomationTask | null>(null);
  const [overflow, setOverflow] = React.useState(false);

  const { tasks, loading, isBusy, pendingAction, modify, revert } = automations;

  const updateOverflow = React.useCallback(() => {
    const el = scrollerRef.current;
    if (!el) return;
    setOverflow(el.scrollWidth - el.clientWidth > 8);
  }, []);

  React.useEffect(() => {
    updateOverflow();
    window.addEventListener("resize", updateOverflow);
    return () => window.removeEventListener("resize", updateOverflow);
  }, [updateOverflow, tasks.length]);

  const scrollBy = (direction: number) => {
    scrollerRef.current?.scrollBy({
      left: direction * SCROLL_STEP_PX,
      behavior: "smooth",
    });
  };

  const handleSave = (patch: AutomationTaskPatch) => {
    if (modifyTask) modify(modifyTask.id, patch);
  };

  if (loading || tasks.length === 0) return null;

  return (
    <section
      aria-label={t("automation.carousel.title")}
      className="w-full"
      data-testid="automation-carousel"
    >
      <div className="mb-2 flex items-center gap-1.5 px-1">
        <Icon name="spark" className="h-3.5 w-3.5 text-[var(--v2-accent-text)]" />
        <span className="text-[13px] font-semibold text-[var(--v2-text-strong)]">
          {t("automation.carousel.title")}
        </span>
        <span className="text-[11px] text-[var(--v2-text-faint)]">
          {t("automation.carousel.subtitle")}
        </span>
        {overflow && (
          <div className="ml-auto flex items-center gap-1">
            <button
              type="button"
              onClick={() => scrollBy(-1)}
              aria-label={t("automation.carousel.scrollPrev")}
              className="grid h-6 w-6 place-items-center rounded-full border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_36%,var(--v2-panel-border))] hover:text-[var(--v2-text-strong)]"
            >
              <Icon name="chevron" className="h-3.5 w-3.5 rotate-90" />
            </button>
            <button
              type="button"
              onClick={() => scrollBy(1)}
              aria-label={t("automation.carousel.scrollNext")}
              className="grid h-6 w-6 place-items-center rounded-full border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_36%,var(--v2-panel-border))] hover:text-[var(--v2-text-strong)]"
            >
              <Icon name="chevron" className="h-3.5 w-3.5 -rotate-90" />
            </button>
          </div>
        )}
      </div>

      <div
        ref={scrollerRef}
        onScroll={updateOverflow}
        className="flex gap-2.5 overflow-x-auto pb-1 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
      >
        {tasks.map((task) => (
          <AutomationTaskCard
            key={task.id}
            task={task}
            busy={isBusy(task.id)}
            pendingAction={pendingAction(task.id)}
            onModify={() => setModifyTask(task)}
            onRevert={() => revert(task.id)}
          />
        ))}
      </div>

      <ModifyTaskDialog
        task={modifyTask}
        open={Boolean(modifyTask)}
        onClose={() => setModifyTask(null)}
        onSave={handleSave}
      />
    </section>
  );
}
