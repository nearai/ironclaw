/**
 * useAutomationTasks — owns the automation-task list and the Approve / Modify /
 * Cancel / Revert actions for the OOBE concepts.
 *
 * Every action awaits the API seam and replaces the task with the *returned*
 * (server-confirmed) record before the UI reflects success — no optimistic
 * checkmarks (`.claude/rules/tool-evidence.md`). Per-task busy state disables
 * that card's actions while a command is in flight.
 */
import React from "react";
import type {
  AutomationTask,
  AutomationTaskPatch,
} from "../lib/automation-tasks";
import {
  approveAutomationTask,
  cancelAutomationTask,
  listAutomationTasks,
  modifyAutomationTask,
  revertAutomationTask,
} from "../lib/automation-tasks-api";

interface UseAutomationTasksOptions {
  /** Seed the list explicitly (preview / tests). When set, no fetch runs. */
  initialTasks?: AutomationTask[];
}

/** Which command is in flight for a task, so the matching button can spin. */
export type AutomationActionKind = "approve" | "modify" | "cancel" | "revert";

export interface AutomationTasksApi {
  tasks: AutomationTask[];
  loading: boolean;
  isBusy: (id: string) => boolean;
  pendingAction: (id: string) => AutomationActionKind | null;
  approve: (id: string) => Promise<void>;
  modify: (id: string, patch: AutomationTaskPatch) => Promise<void>;
  cancel: (id: string) => Promise<void>;
  revert: (id: string) => Promise<void>;
}

export function useAutomationTasks(
  options: UseAutomationTasksOptions = {},
): AutomationTasksApi {
  const { initialTasks } = options;
  const [tasks, setTasks] = React.useState<AutomationTask[]>(
    () => initialTasks ?? [],
  );
  const [loading, setLoading] = React.useState<boolean>(() => !initialTasks);
  const [pending, setPending] = React.useState<
    Record<string, AutomationActionKind>
  >({});

  React.useEffect(() => {
    if (initialTasks) {
      setTasks(initialTasks);
      setLoading(false);
      return undefined;
    }
    let cancelled = false;
    setLoading(true);
    listAutomationTasks()
      .then((loaded) => {
        if (!cancelled) setTasks(loaded);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
    // initialTasks is a stable seed for a given mount; re-run only if identity changes.
  }, [initialTasks]);

  const replaceTask = React.useCallback((updated: AutomationTask) => {
    setTasks((current) =>
      current.map((task) => (task.id === updated.id ? updated : task)),
    );
  }, []);

  const runAction = React.useCallback(
    async (
      id: string,
      kind: AutomationActionKind,
      action: (task: AutomationTask) => Promise<AutomationTask>,
    ) => {
      const target = tasks.find((task) => task.id === id);
      if (!target || pending[id]) return;
      setPending((current) => ({ ...current, [id]: kind }));
      try {
        const updated = await action(target);
        replaceTask(updated);
      } catch {
        // Prototype: a failed command leaves the task in its prior state; the
        // real wiring surfaces the redacted error through the toast/gate path.
      } finally {
        setPending((current) => {
          const next = { ...current };
          delete next[id];
          return next;
        });
      }
    },
    [tasks, pending, replaceTask],
  );

  const approve = React.useCallback(
    (id: string) => runAction(id, "approve", approveAutomationTask),
    [runAction],
  );
  const cancel = React.useCallback(
    (id: string) => runAction(id, "cancel", cancelAutomationTask),
    [runAction],
  );
  const revert = React.useCallback(
    (id: string) => runAction(id, "revert", revertAutomationTask),
    [runAction],
  );
  const modify = React.useCallback(
    (id: string, patch: AutomationTaskPatch) =>
      runAction(id, "modify", (task) => modifyAutomationTask(task, patch)),
    [runAction],
  );

  const isBusy = React.useCallback((id: string) => Boolean(pending[id]), [pending]);
  const pendingAction = React.useCallback(
    (id: string) => pending[id] ?? null,
    [pending],
  );

  return { tasks, loading, isBusy, pendingAction, approve, modify, cancel, revert };
}
