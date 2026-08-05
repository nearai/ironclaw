/**
 * Automation tasks — the typed model behind the two OOBE concepts:
 *
 *   1. the completed-automations carousel above the landing composer, and
 *   2. the inline calendar reschedule rich-preview inside a thread.
 *
 * A task is either *suggested* (proposed by the agent, awaiting the
 * Approve / Modify / Cancel decision) or *automated* (already carried out — the
 * user gets Modify / Revert). The `state` field drives which action set renders.
 *
 * SEAM (design prototype): the shapes here mirror the intended durable event +
 * projection contract (see ../AUTOMATION-TASKS-CONTRACT.md). Data is mock and
 * lives in-memory; `automation-tasks-api.ts` is the swap point for the real
 * projection reads and command endpoints.
 */

/** Third-party app a task acts in. Drives the icon, brand tint, and launch copy. */
export type AutomationApp =
  | "gmail"
  | "google_calendar"
  | "google_docs"
  | "slack"
  | "notion";

/** What the task does. Drives the modify editor and the inline renderer. */
export type AutomationTaskKind =
  | "email_triage"
  | "calendar_accept"
  | "calendar_reschedule"
  | "doc_insights";

/**
 * Lifecycle state. `suggested` → (approve) → `automated`, or (cancel) →
 * `cancelled`. `automated` → (revert) → `reverted`. `in_progress` is the
 * transient approved-but-not-confirmed state.
 */
export type AutomationTaskState =
  | "suggested"
  | "in_progress"
  | "automated"
  | "reverted"
  | "cancelled";

export interface TaskMetric {
  labelKey: string;
  value: string;
}

/** One drafted reply in an email-triage task; editable/toggleable in Modify. */
export interface TriagedEmail {
  id: string;
  from: string;
  subject: string;
  /** Why the agent classified this as safe to auto-handle. */
  rationaleKey: string;
  /** The drafted reply / action, editable in the Modify dialog. */
  draft: string;
  /** Whether this reply is included when the task runs. */
  include: boolean;
}

export interface CalendarSlot {
  /** Human-friendly day label, e.g. "Thu, Jul 24". */
  day: string;
  /** Human-friendly time range, e.g. "2:00 – 2:30 PM". */
  time: string;
}

/** Payload for a calendar_reschedule task. */
export interface CalendarReschedule {
  meetingTitle: string;
  attendees: string[];
  /** The meeting this one collided with (why it moved). */
  conflictWith: string;
  from: CalendarSlot;
  to: CalendarSlot;
  /** Alternative destination slots offered in the Modify dialog. */
  alternativeSlots: CalendarSlot[];
  note?: string;
}

export interface AutomationTask {
  id: string;
  app: AutomationApp;
  kind: AutomationTaskKind;
  state: AutomationTaskState;
  title: string;
  /** One-line result/proposal for the thumbnail and card subtitle. */
  summary: string;
  detail?: string;
  /** ISO-ish display timestamps (prototype uses friendly strings). */
  completedAt?: string;
  suggestedAt?: string;
  /** Deep link into the third-party app. */
  launchUrl?: string;
  metrics?: TaskMetric[];
  emails?: TriagedEmail[];
  reschedule?: CalendarReschedule;
}

/** Patch accepted by Modify — a shallow, kind-aware set of editable fields. */
export interface AutomationTaskPatch {
  emails?: TriagedEmail[];
  reschedule?: Partial<CalendarReschedule>;
  note?: string;
}

/* ── State machine (pure, unit-testable) ─────────────────────────────── */

export function isSuggested(task: AutomationTask): boolean {
  return task.state === "suggested" || task.state === "in_progress";
}

export function isAutomated(task: AutomationTask): boolean {
  return task.state === "automated";
}

/** Approve a suggested task → it becomes automated. */
export function applyApprove(task: AutomationTask): AutomationTask {
  if (!isSuggested(task)) return task;
  return { ...task, state: "automated", completedAt: task.completedAt ?? "Just now" };
}

/** Apply a Modify patch, preserving lifecycle state. */
export function applyModify(
  task: AutomationTask,
  patch: AutomationTaskPatch,
): AutomationTask {
  const next: AutomationTask = { ...task };
  if (patch.emails) next.emails = patch.emails;
  if (patch.reschedule && next.reschedule) {
    next.reschedule = { ...next.reschedule, ...patch.reschedule };
  }
  if (patch.note !== undefined) {
    if (next.reschedule) next.reschedule = { ...next.reschedule, note: patch.note };
  }
  return next;
}

/** Cancel a suggested task (dismiss before running). */
export function applyCancel(task: AutomationTask): AutomationTask {
  return { ...task, state: "cancelled" };
}

/** Revert an already-automated task (undo the effect). */
export function applyRevert(task: AutomationTask): AutomationTask {
  if (!isAutomated(task)) return task;
  return { ...task, state: "reverted" };
}

/**
 * Re-run an already-automated task after a Modify. Modifying a completed task
 * doesn't just record a preference — it re-executes with the user's changes and
 * produces a fresh result. Refreshes the completion stamp and recomputes any
 * derived summary/metrics the change affects (e.g. how many email replies now
 * send). Calendar slots reflect the patch directly, so no recompute is needed
 * there. Only meaningful for automated tasks; a suggested edit is a proposal
 * change, not a run.
 */
export function rerunModified(task: AutomationTask): AutomationTask {
  if (!isAutomated(task)) return task;
  const next: AutomationTask = { ...task, completedAt: "Just now" };
  if (next.kind === "email_triage" && next.emails) {
    const replied = next.emails.filter((email) => email.include).length;
    if (next.metrics) {
      next.metrics = next.metrics.map((metric) =>
        metric.labelKey === "automation.metric.replied"
          ? { ...metric, value: String(replied) }
          : metric,
      );
    }
  }
  return next;
}

/* ── Mock data (design prototype) ────────────────────────────────────── */

export const MOCK_COMPLETED_TASKS: AutomationTask[] = [
  {
    id: "task-email-triage",
    app: "gmail",
    kind: "email_triage",
    state: "automated",
    title: "Triaged your inbox",
    summary: "Replied to 3 routine messages, archived 9 newsletters.",
    detail:
      "Handled the low-stakes mail so your inbox is down to what needs you. Nothing sensitive was sent — replies were confirmations and acknowledgements.",
    completedAt: "8 min ago",
    launchUrl: "https://mail.google.com/mail/u/0/#inbox",
    metrics: [
      { labelKey: "automation.metric.replied", value: "3" },
      { labelKey: "automation.metric.archived", value: "9" },
      { labelKey: "automation.metric.flagged", value: "2" },
    ],
    emails: [
      {
        id: "mail-1",
        from: "Priya Natarajan",
        subject: "Access to the Q3 Roadmap doc",
        rationaleKey: "automation.email.rationaleAccess",
        draft: "Done — I've shared the Q3 Roadmap doc with you (comment access).",
        include: true,
      },
      {
        id: "mail-2",
        from: "Notion",
        subject: "Weekly digest",
        rationaleKey: "automation.email.rationaleNewsletter",
        draft: "Archived — recurring newsletter, no action needed.",
        include: true,
      },
      {
        id: "mail-3",
        from: "Marcus Lee",
        subject: "Re: Lunch Thursday?",
        rationaleKey: "automation.email.rationaleConfirm",
        draft: "Thursday works for me — see you at 12:30. 👍",
        include: true,
      },
    ],
  },
  {
    id: "task-calendar-accept",
    app: "google_calendar",
    kind: "calendar_accept",
    state: "automated",
    title: "Accepted 2 team invites",
    summary: "No conflicts with your availability — both auto-accepted.",
    detail:
      "Design Review (Wed 10:00) and Sprint Planning (Fri 9:30) had open slots on your calendar, so I accepted on your behalf.",
    completedAt: "22 min ago",
    launchUrl: "https://calendar.google.com/calendar/u/0/r",
    metrics: [
      { labelKey: "automation.metric.accepted", value: "2" },
      { labelKey: "automation.metric.conflicts", value: "0" },
    ],
  },
  {
    id: "task-doc-insights",
    app: "google_docs",
    kind: "doc_insights",
    state: "automated",
    title: "3 insights from this week's docs",
    summary: "Summarized the Q3 launch retro and two incoming PRDs.",
    detail:
      "The retro flags two recurring themes; the PRDs overlap on the notifications workstream. Full summary is in your workspace notes.",
    completedAt: "1 hr ago",
    launchUrl: "https://docs.google.com/document/u/0/",
    metrics: [
      { labelKey: "automation.metric.docs", value: "3" },
      { labelKey: "automation.metric.insights", value: "3" },
    ],
  },
];

/** A conflicting-meeting reschedule the agent is *proposing* (needs approval). */
export const MOCK_SUGGESTED_RESCHEDULE: AutomationTask = {
  id: "task-reschedule-suggested",
  app: "google_calendar",
  kind: "calendar_reschedule",
  state: "suggested",
  title: "Reschedule a conflicting meeting",
  summary: "“Design sync” overlaps your 1:1 with Dana — I can move it.",
  detail:
    "The Design sync you were invited to lands on top of your recurring 1:1 with Dana. There's a clear 30-minute slot later the same afternoon that works for every attendee.",
  suggestedAt: "Just now",
  launchUrl: "https://calendar.google.com/calendar/u/0/r",
  reschedule: {
    meetingTitle: "Design sync",
    attendees: ["You", "Dana Ortiz", "Sam Cole", "Priya Natarajan"],
    conflictWith: "1:1 with Dana",
    from: { day: "Thu, Jul 24", time: "2:00 – 2:30 PM" },
    to: { day: "Thu, Jul 24", time: "4:00 – 4:30 PM" },
    alternativeSlots: [
      { day: "Thu, Jul 24", time: "4:00 – 4:30 PM" },
      { day: "Thu, Jul 24", time: "4:30 – 5:00 PM" },
      { day: "Fri, Jul 25", time: "11:00 – 11:30 AM" },
    ],
  },
};

/** The same reschedule after it has been auto-run (Auto mode) — Modify / Revert. */
export const MOCK_AUTOMATED_RESCHEDULE: AutomationTask = {
  ...MOCK_SUGGESTED_RESCHEDULE,
  id: "task-reschedule-automated",
  state: "automated",
  title: "Rescheduled a conflicting meeting",
  summary: "Moved “Design sync” to 4:00 PM to clear your 1:1 with Dana.",
  completedAt: "Just now",
  suggestedAt: undefined,
};

/* ── Plan mode (batched approval) ────────────────────────────────────── */

/**
 * A plan groups several suggested tasks the agent proposes together so the user
 * can approve the whole set at once (Plan mode). The items are ordinary
 * `suggested` tasks — the plan is just the batching + one-shot approval around
 * them.
 */
export interface AutomationPlan {
  id: string;
  title: string;
  summary: string;
}

export const MOCK_PLAN: AutomationPlan = {
  id: "plan-morning",
  title: "Your morning productivity plan",
  summary: "Three things I can handle now — approve all, or adjust any first.",
};

export const MOCK_PLAN_TASKS: AutomationTask[] = [
  {
    id: "plan-email-triage",
    app: "gmail",
    kind: "email_triage",
    state: "suggested",
    title: "Triage 12 new emails",
    summary: "Reply to 3 routine messages, archive 9 newsletters.",
    suggestedAt: "Just now",
    launchUrl: "https://mail.google.com/mail/u/0/#inbox",
    metrics: [
      { labelKey: "automation.metric.replied", value: "3" },
      { labelKey: "automation.metric.archived", value: "9" },
    ],
    emails: [
      {
        id: "plan-mail-1",
        from: "Priya Natarajan",
        subject: "Access to the Q3 Roadmap doc",
        rationaleKey: "automation.email.rationaleAccess",
        draft: "Done — I've shared the Q3 Roadmap doc with you (comment access).",
        include: true,
      },
      {
        id: "plan-mail-2",
        from: "Marcus Lee",
        subject: "Re: Lunch Thursday?",
        rationaleKey: "automation.email.rationaleConfirm",
        draft: "Thursday works — see you at 12:30. 👍",
        include: true,
      },
      {
        id: "plan-mail-3",
        from: "GitHub",
        subject: "Weekly digest",
        rationaleKey: "automation.email.rationaleNewsletter",
        draft: "Archive — recurring digest, no action needed.",
        include: true,
      },
    ],
  },
  {
    id: "plan-calendar-accept",
    app: "google_calendar",
    kind: "calendar_accept",
    state: "suggested",
    title: "Accept 2 non-conflicting invites",
    summary: "Design Review (Wed 10:00) and Sprint Planning (Fri 9:30) — both open.",
    suggestedAt: "Just now",
    launchUrl: "https://calendar.google.com/calendar/u/0/r",
    metrics: [
      { labelKey: "automation.metric.accepted", value: "2" },
      { labelKey: "automation.metric.conflicts", value: "0" },
    ],
  },
  {
    ...MOCK_SUGGESTED_RESCHEDULE,
    id: "plan-reschedule",
    title: "Reschedule the Design sync conflict",
    summary: "Move “Design sync” to 4:00 PM so it clears your 1:1 with Dana.",
    reschedule: { ...MOCK_SUGGESTED_RESCHEDULE.reschedule },
  },
];
