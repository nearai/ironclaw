/**
 * SuggestedTaskSurface — the landing surface that shows the OOBE first-run
 * suggestion cards above the composer (PROPOSAL §2A, dep D-F5).
 *
 * Gating happens in the eager parent (`empty-state.tsx`) before this lazy chunk
 * is even requested, so this component itself is unconditional/presentational:
 * whenever it's mounted, it renders a STATIC in-memory demo list — no
 * projection reads, no background jobs.
 *
 * Slice 2 wires Approve: the card's approve action submits the task's
 * `approvePrompt` through the app's existing send path (via `onApproveTask`),
 * running the agent as a real foreground turn whose activity streams in the
 * thread by reuse. The just-approved card flips to `running` optimistically.
 *
 * Slice 3 wires Connect WITHOUT cloning the OAuth flow: an unconnected card's
 * Connect resolves the card's `app` to a real catalog extension
 * (`resolveConnectExtension` over `useExtensions()`), then opens the EXISTING
 * extensions setup/OAuth modal (`ConfigureModal`) — the one real connect path,
 * lazy-loaded so its weight never touches the surface chunk until Connect is
 * clicked. On a successful save the card flips `unconnected → suggested`.
 *
 * Slice 4 wires "+ Automation" the same way as Approve: a completed card's
 * automation action submits the task's `automationPrompt` (via
 * `onAutomationTask`) so the agent schedules a recurring automation (it calls
 * `builtin.trigger_create` — prompt injection is the design, no REST create).
 * The card flips to a "scheduled" chip optimistically.
 */
import React from "react";
import type { ReactNode } from "react";

import { useT } from "../../../lib/i18n";
import { useExtensions } from "../../extensions/hooks/useExtensions";
import { resolveConnectExtension } from "../lib/connect-extension";
import type { SuggestedTask } from "../lib/suggested-tasks";
import { SuggestedTaskCard } from "./suggested-task-card";

// The real Connect UI (extensions setup/OAuth modal) loads only when a user
// actually clicks Connect, so its weight — and the whole OAuth watcher/state
// machine it pulls in — never pads the surface chunk. Same React.lazy pattern
// empty-state.tsx uses to keep the surface itself out of eager /chat.
const ConfigureModal = React.lazy(() =>
  import("../../extensions/components/configure-modal").then(({ ConfigureModal }) => ({
    default: ConfigureModal,
  })),
);

// Static demo cards spanning the card states (§2A). Gate-guarded: never reached
// unless `oobe_suggestions` is on, so real users never see fabricated tasks.
const DEMO_TASKS: SuggestedTask[] = [
  {
    id: "demo-gmail-connect",
    app: "gmail",
    title: "Triage your inbox",
    summary: "Reply to routine mail and archive newsletters so your inbox is clear.",
    state: "unconnected",
    connectLabel: "Gmail",
    approvePrompt: "Triage my inbox — reply to routine mail and archive newsletters.",
    automationPrompt:
      "Set this up as a recurring automation: triage my inbox every morning — reply to routine mail and archive newsletters.",
  },
  {
    id: "demo-calendar-suggested",
    app: "google_calendar",
    title: "Reschedule a conflicting meeting",
    summary: "“Design sync” overlaps your 1:1 with Dana — I can move it to a free slot.",
    state: "suggested",
    approvePrompt:
      "Reschedule my “Design sync” so it no longer overlaps my 1:1 with Dana — move it to a free slot.",
    automationPrompt:
      "Set this up as a recurring automation: each morning, scan my calendar for conflicts and reschedule them to free slots.",
  },
  {
    id: "demo-docs-completed",
    app: "google_docs",
    title: "Summarized this week's docs",
    summary: "Pulled 3 insights from the launch retro and two incoming PRDs.",
    state: "completed",
    approvePrompt:
      "Summarize this week's docs — pull the key insights from the launch retro and the two incoming PRDs.",
    automationPrompt:
      "Set this up as a recurring automation: summarize my docs each week and share the key insights.",
  },
];

export function SuggestedTaskSurface({
  onApproveTask,
  onAutomationTask,
  renderRunningIndicator,
}: {
  onApproveTask?: (task: SuggestedTask) => void;
  onAutomationTask?: (task: SuggestedTask) => void;
  renderRunningIndicator?: (label: string) => ReactNode;
} = {}) {
  const t = useT();
  // Reuse the extensions catalog so Connect resolves to a REAL extension and
  // drives the real setup modal — no parallel connect implementation.
  const { extensions, registry } = useExtensions();
  // The id of the just-approved card, flipped to `running` optimistically so
  // the surface reflects the kicked-off turn immediately (live status streams
  // in the thread once the user is in it).
  const [runningId, setRunningId] = React.useState<string | null>(null);
  // The id of the just-scheduled card, flipped to its "scheduled" chip
  // optimistically once "+ Automation" fires (same pattern as `runningId`).
  const [scheduledId, setScheduledId] = React.useState<string | null>(null);
  // The task whose Connect was clicked — drives the setup modal below.
  const [connectingTask, setConnectingTask] = React.useState<SuggestedTask | null>(null);
  // Ids of tasks whose extension is now connected — flips `unconnected` cards
  // to `suggested` so the natural next step (Approve) becomes available.
  const [connectedIds, setConnectedIds] = React.useState<string[]>([]);

  const connectExtension = connectingTask
    ? resolveConnectExtension(connectingTask.app, extensions, registry)
    : null;
  // Connect was clicked but the app resolves to no installable extension (not
  // in the catalog yet, or offline): surface a plain notice instead of opening
  // an empty modal or leaving a dead button.
  const connectUnavailable = Boolean(connectingTask) && !connectExtension;

  function effectiveState(task: SuggestedTask): SuggestedTask["state"] {
    if (runningId === task.id) return "running";
    if (task.state === "unconnected" && connectedIds.includes(task.id)) return "suggested";
    return task.state;
  }

  return (
    <section
      aria-label={t("chat.oobe.heading")}
      className="mt-8 w-full max-w-5xl text-left"
    >
      <div className="mb-2 text-[11px] font-medium uppercase tracking-wide text-[var(--v2-text-faint)]">
        {t("chat.oobe.heading")}
      </div>
      <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-3">
        {DEMO_TASKS.map((task) => {
          const state = effectiveState(task);
          return (
            <SuggestedTaskCard
              key={task.id}
              task={state === task.state ? task : { ...task, state }}
              scheduled={scheduledId === task.id}
              renderRunningIndicator={renderRunningIndicator}
              // §2A change 3: only one suggested job may run at a time — every
              // OTHER card disables (no queuing, no concurrent approve/connect/
              // automation) while one is active. The acting card itself stays
              // interactive so its own running/completed state still reads.
              locked={runningId !== null && runningId !== task.id}
              onConnect={() => setConnectingTask(task)}
              onApprove={() => {
                setRunningId(task.id);
                onApproveTask?.(task);
              }}
              onAutomation={() => {
                setScheduledId(task.id);
                onAutomationTask?.(task);
              }}
            />
          );
        })}
      </div>

      {connectUnavailable && (
        <p
          role="status"
          className="mt-2 text-[11px] text-[var(--v2-text-faint)]"
        >
          {t("chat.oobe.connectUnavailable")}
        </p>
      )}

      {connectingTask && connectExtension && (
        <React.Suspense fallback={null}>
          <ConfigureModal
            extension={connectExtension}
            returnFocusTo={null}
            onClose={() => setConnectingTask(null)}
            onSaved={() => {
              setConnectedIds((ids) =>
                ids.includes(connectingTask.id) ? ids : [...ids, connectingTask.id],
              );
              setConnectingTask(null);
            }}
          />
        </React.Suspense>
      )}
    </section>
  );
}
