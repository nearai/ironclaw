/**
 * SuggestedTaskSurface — the feature-gated landing surface that shows the OOBE
 * first-run suggestion cards above the composer (PROPOSAL §2A, dep D-F5).
 *
 * Slice 1 is presentational: it reads the `oobe_suggestions` deployment flag and
 * renders `null` when off, so the empty-state is unchanged for real users. When
 * the flag is on it renders a STATIC in-memory demo list — no projection reads,
 * no background jobs, no approve/connect wiring (card callbacks are omitted for
 * now). The real projection-backed feed lands in a later slice (D-F1/D-F2).
 */
import { useOobeSuggestionsEnabled } from "../../../app/auth";
import { useT } from "../../../lib/i18n";
import type { SuggestedTask } from "../lib/suggested-tasks";
import { SuggestedTaskCard } from "./suggested-task-card";

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
  },
  {
    id: "demo-calendar-suggested",
    app: "google_calendar",
    title: "Reschedule a conflicting meeting",
    summary: "“Design sync” overlaps your 1:1 with Dana — I can move it to a free slot.",
    state: "suggested",
  },
  {
    id: "demo-docs-completed",
    app: "google_docs",
    title: "Summarized this week's docs",
    summary: "Pulled 3 insights from the launch retro and two incoming PRDs.",
    state: "completed",
  },
];

export function SuggestedTaskSurface() {
  const enabled = useOobeSuggestionsEnabled();
  const t = useT();
  if (!enabled) return null;

  return (
    <section
      aria-label={t("chat.oobe.heading")}
      className="mt-8 w-full max-w-5xl text-left"
    >
      <div className="mb-2 text-[11px] font-medium uppercase tracking-wide text-[var(--v2-text-faint)]">
        {t("chat.oobe.heading")}
      </div>
      <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-3">
        {DEMO_TASKS.map((task) => (
          <SuggestedTaskCard key={task.id} task={task} />
        ))}
      </div>
    </section>
  );
}
