// DEMO routine fixtures for `/api/routines/*`.
//
// Shapes follow the Routines page consumers: the list is `{ routines }`, the
// summary strip reads `{ total, enabled, disabled, unverified, failing,
// runs_today }`, and detail is the bare routine object (`useRoutineDetail`
// renders the response directly).
//
// Narrative anchors from the chat fixtures (routes/core.ts): "nightly-backup"
// failed yesterday from disk pressure on the runner (12 GB since freed,
// retry scheduled tonight) and "docs-sync" drafted 2 doc updates.

import { DAY, HOUR, MINUTE, iso, isoIn } from "./clock";

type RoutineRun = {
  id: string;
  status: "ok" | "error" | "running";
  started_at: string;
  result_summary?: string;
};

export type DemoRoutine = {
  id: string;
  name: string;
  description: string;
  enabled: boolean;
  status: "active" | "running" | "failing" | "attention" | "disabled";
  verification_status: "verified" | "unverified";
  trigger_type: string;
  trigger_summary: string;
  action_type: string;
  run_count: number;
  consecutive_failures: number;
  next_fire_at: string | null;
  last_run_at: string | null;
  created_at: string;
  trigger: Record<string, unknown>;
  action: Record<string, unknown>;
  recent_runs: RoutineRun[];
};

let runCounter = 0;
function run(
  status: RoutineRun["status"],
  startedMsAgo: number,
  resultSummary?: string
): RoutineRun {
  runCounter += 1;
  return {
    id: `routine-run-${String(runCounter).padStart(4, "0")}`,
    status,
    started_at: iso(startedMsAgo),
    ...(resultSummary ? { result_summary: resultSummary } : {}),
  };
}

export const routines: DemoRoutine[] = [
  {
    id: "routine-nightly-backup",
    name: "nightly-backup",
    description:
      "Snapshot the workspace and memory stores to cold storage, verify the archive, and prune snapshots older than 30 days.",
    enabled: true,
    status: "failing",
    verification_status: "verified",
    trigger_type: "schedule",
    trigger_summary: "Every day at 02:30 UTC",
    action_type: "full_job",
    run_count: 214,
    consecutive_failures: 1,
    next_fire_at: isoIn(9 * HOUR + 30 * MINUTE),
    last_run_at: iso(26 * HOUR + 30 * MINUTE),
    created_at: iso(214 * DAY),
    trigger: { type: "cron", cron: "30 2 * * *", timezone: "UTC" },
    action: {
      type: "full_job",
      prompt: "Run the backup pipeline: snapshot, verify, prune (>30d).",
      timeout_secs: 1800,
    },
    recent_runs: [
      run(
        "error",
        26 * HOUR + 30 * MINUTE,
        "Runner out of disk during archive verify (412 MB free). 12 GB freed since; retry scheduled for tonight's 02:30 slot."
      ),
      run("ok", 2 * DAY + 2 * HOUR + 30 * MINUTE, "Snapshot 7.9 GB verified; pruned 3 old snapshots."),
      run("ok", 3 * DAY + 2 * HOUR + 30 * MINUTE, "Snapshot 7.8 GB verified."),
      run("ok", 4 * DAY + 2 * HOUR + 30 * MINUTE, "Snapshot 7.8 GB verified."),
    ],
  },
  {
    id: "routine-docs-sync",
    name: "docs-sync",
    description:
      "Scan merged PRs each weekday morning and draft matching documentation updates for anything that changed a public surface.",
    enabled: true,
    status: "active",
    verification_status: "verified",
    trigger_type: "schedule",
    trigger_summary: "Weekdays at 06:00 UTC",
    action_type: "lightweight",
    run_count: 96,
    consecutive_failures: 0,
    next_fire_at: isoIn(13 * HOUR),
    last_run_at: iso(23 * HOUR),
    created_at: iso(130 * DAY),
    trigger: { type: "cron", cron: "0 6 * * 1-5", timezone: "UTC" },
    action: {
      type: "lightweight",
      prompt: "Draft doc updates for merged PRs that touch public surfaces.",
    },
    recent_runs: [
      run("ok", 23 * HOUR, "Drafted 2 doc updates from merged PRs (workspace browser, notification center)."),
      run("ok", 2 * DAY - HOUR, "No public-surface changes found; nothing to draft."),
      run("ok", 3 * DAY - HOUR, "Drafted 1 doc update (sandbox credential reuse)."),
    ],
  },
  {
    id: "routine-pr-triage",
    name: "pr-triage",
    description:
      "Label new pull requests by area, request reviewers from the CODEOWNERS map, and flag PRs without a linked issue.",
    enabled: true,
    status: "active",
    verification_status: "verified",
    trigger_type: "event",
    trigger_summary: "On github.pull_request.opened",
    action_type: "lightweight",
    run_count: 388,
    consecutive_failures: 0,
    next_fire_at: null,
    last_run_at: iso(48 * MINUTE),
    created_at: iso(160 * DAY),
    trigger: { type: "event", source: "github", event: "pull_request.opened" },
    action: {
      type: "lightweight",
      prompt: "Label the PR, assign reviewers, verify a linked issue exists.",
    },
    recent_runs: [
      run("ok", 48 * MINUTE, "PR #6843 labeled `webui`; reviewers assigned."),
      run("ok", 3 * HOUR + 10 * MINUTE, "PR #6842 labeled `engine`; flagged: no linked issue."),
      run("ok", 5 * HOUR, "PR #6841 labeled `docs` (release notes draft)."),
    ],
  },
  {
    id: "routine-metrics-rollup",
    name: "metrics-rollup",
    description:
      "Aggregate hourly usage metrics into daily rollups and publish them to the operator dashboard.",
    enabled: true,
    status: "active",
    verification_status: "unverified",
    trigger_type: "schedule",
    trigger_summary: "Every day at 00:15 UTC",
    action_type: "full_job",
    run_count: 2,
    consecutive_failures: 0,
    next_fire_at: isoIn(7 * HOUR + 15 * MINUTE),
    last_run_at: iso(28 * HOUR + 45 * MINUTE),
    created_at: iso(3 * DAY),
    trigger: { type: "cron", cron: "15 0 * * *", timezone: "UTC" },
    action: {
      type: "full_job",
      prompt: "Roll up hourly metrics into daily aggregates; publish to the dashboard.",
      timeout_secs: 900,
    },
    recent_runs: [
      run("ok", 28 * HOUR + 45 * MINUTE, "Rolled up 24 hourly buckets; dashboard updated."),
      run("ok", 2 * DAY + 4 * HOUR + 45 * MINUTE, "First run: backfilled 48 hourly buckets."),
    ],
  },
  {
    id: "routine-inbox-digest",
    name: "inbox-digest",
    description:
      "Morning summary of unread support tickets and mentions, delivered to the operator before standup.",
    enabled: false,
    status: "disabled",
    verification_status: "verified",
    trigger_type: "schedule",
    trigger_summary: "Weekdays at 07:30 UTC",
    action_type: "lightweight",
    run_count: 61,
    consecutive_failures: 0,
    next_fire_at: null,
    last_run_at: iso(6 * DAY),
    created_at: iso(110 * DAY),
    trigger: { type: "cron", cron: "30 7 * * 1-5", timezone: "UTC" },
    action: {
      type: "lightweight",
      prompt: "Summarize unread tickets and mentions into a morning digest.",
    },
    recent_runs: [
      run("ok", 6 * DAY, "Digest sent: 4 unread tickets, 2 mentions."),
      run("ok", 7 * DAY, "Digest sent: 9 unread tickets, 1 mention."),
    ],
  },
  {
    id: "routine-dep-audit",
    name: "dep-audit",
    description:
      "Weekly dependency audit across Cargo and pnpm lockfiles; files issues for new advisories not on the allowlist.",
    enabled: true,
    status: "active",
    verification_status: "verified",
    trigger_type: "schedule",
    trigger_summary: "Mondays at 06:00 UTC",
    action_type: "full_job",
    run_count: 31,
    consecutive_failures: 0,
    next_fire_at: isoIn(4 * DAY + 19 * HOUR),
    last_run_at: iso(2 * DAY + 6 * HOUR),
    created_at: iso(220 * DAY),
    trigger: { type: "cron", cron: "0 6 * * 1", timezone: "UTC" },
    action: {
      type: "full_job",
      prompt: "Run cargo audit + pnpm audit; file issues for new advisories.",
      timeout_secs: 1200,
    },
    recent_runs: [
      run("ok", 2 * DAY + 6 * HOUR, "No new advisories; 2 known, allowlisted."),
      run("ok", 9 * DAY + 6 * HOUR, "1 new advisory (RUSTSEC-2026-0141) — issue #6790 filed."),
    ],
  },
];

export function findRoutine(routineId: string): DemoRoutine | undefined {
  return routines.find((routine) => routine.id === routineId);
}

export function routinesSummary() {
  const summary = {
    total: routines.length,
    enabled: 0,
    disabled: 0,
    unverified: 0,
    failing: 0,
    runs_today: 0,
  };
  const dayStart = new Date();
  dayStart.setHours(0, 0, 0, 0);
  for (const routine of routines) {
    if (routine.enabled) summary.enabled += 1;
    else summary.disabled += 1;
    if (routine.verification_status === "unverified") summary.unverified += 1;
    if (routine.status === "failing") summary.failing += 1;
    if (
      routine.recent_runs.some(
        (entry) => new Date(entry.started_at).getTime() >= dayStart.getTime()
      )
    ) {
      summary.runs_today += 1;
    }
  }
  return summary;
}

export function triggerRoutine(routineId: string): boolean {
  const routine = findRoutine(routineId);
  if (!routine) return false;
  routine.recent_runs.unshift(run("running", 0, "Manual run queued from the Routines page."));
  routine.run_count += 1;
  routine.last_run_at = new Date().toISOString();
  return true;
}

export function toggleRoutine(routineId: string): boolean {
  const routine = findRoutine(routineId);
  if (!routine) return false;
  routine.enabled = !routine.enabled;
  routine.status = routine.enabled
    ? routine.consecutive_failures > 0
      ? "failing"
      : "active"
    : "disabled";
  routine.next_fire_at = routine.enabled ? isoIn(45 * MINUTE) : null;
  return true;
}

export function deleteRoutine(routineId: string): void {
  const index = routines.findIndex((routine) => routine.id === routineId);
  if (index >= 0) routines.splice(index, 1);
}
