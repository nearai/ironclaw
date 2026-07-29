// DEMO automation fixtures for `/api/webchat/v2/automations*`.
//
// Wire shapes mirror `RebornAutomationInfo` / `RebornAutomationRecentRunInfo`
// (ironclaw_product::reborn_services::types): source-discriminated
// schedule/once triggers, snake_case states, and bounded recent-run history.
// The chat standup thread claims "14 automation runs yesterday — 13 ok,
// 1 failed", so the run history includes exactly one error run in that
// window (the dependency audit).

import { DAY, HOUR, MINUTE, iso, isoIn } from "./clock";

type AutomationRun = {
  run_id: string;
  thread_id?: string;
  fire_slot: string;
  status: "ok" | "error" | "running";
  submitted_at: string;
  completed_at?: string;
};

type AutomationSource =
  | { type: "schedule"; cron: string; timezone: string }
  | { type: "once"; at: string; timezone: string };

export type DemoAutomation = {
  automation_id: string;
  name: string;
  source: AutomationSource;
  state: "active" | "scheduled" | "paused" | "disabled" | "inactive" | "completed";
  next_run_at: string | null;
  last_run_at: string | null;
  last_status: "ok" | "error" | "running" | null;
  recent_runs: AutomationRun[];
  is_active: boolean;
  created_at: string;
  active_hold?: {
    reason: "approval" | "auth" | "in_progress" | "other";
    since: string;
    elapsed_occurrences: number | null;
    elapsed_occurrences_capped: boolean;
  };
};

let runCounter = 0;
function run(
  status: AutomationRun["status"],
  firedMsAgo: number,
  { threadId, durationMs = 90_000 }: { threadId?: string; durationMs?: number } = {}
): AutomationRun {
  runCounter += 1;
  return {
    run_id: `auto-run-${String(runCounter).padStart(4, "0")}`,
    ...(threadId ? { thread_id: threadId } : {}),
    fire_slot: iso(firedMsAgo),
    status,
    submitted_at: iso(firedMsAgo - 2_000),
    ...(status === "running" ? {} : { completed_at: iso(firedMsAgo - durationMs) }),
  };
}

export const automations: DemoAutomation[] = [
  {
    automation_id: "auto-morning-brief",
    name: "Morning briefing",
    source: { type: "schedule", cron: "0 7 * * *", timezone: "America/Los_Angeles" },
    state: "scheduled",
    next_run_at: isoIn(20 * HOUR),
    last_run_at: iso(4 * HOUR),
    last_status: "ok",
    is_active: true,
    created_at: iso(80 * DAY),
    recent_runs: [
      run("ok", 4 * HOUR, { threadId: "thread-standup" }),
      run("ok", DAY + 4 * HOUR),
      run("ok", 2 * DAY + 4 * HOUR),
      run("ok", 3 * DAY + 4 * HOUR),
      run("ok", 4 * DAY + 4 * HOUR),
    ],
  },
  {
    automation_id: "auto-pr-digest",
    name: "Open PR digest",
    source: { type: "schedule", cron: "0 9 * * 1-5", timezone: "UTC" },
    state: "scheduled",
    next_run_at: isoIn(22 * HOUR),
    last_run_at: iso(2 * HOUR),
    last_status: "ok",
    is_active: true,
    created_at: iso(55 * DAY),
    recent_runs: [
      run("ok", 2 * HOUR),
      run("ok", DAY + 2 * HOUR),
      run("ok", 2 * DAY + 2 * HOUR),
      run("ok", 5 * DAY + 2 * HOUR),
    ],
  },
  {
    automation_id: "auto-dep-audit",
    name: "Dependency audit",
    source: { type: "schedule", cron: "0 6 * * 1", timezone: "UTC" },
    state: "active",
    next_run_at: isoIn(4 * DAY + 19 * HOUR),
    last_run_at: iso(26 * HOUR),
    last_status: "error",
    is_active: true,
    created_at: iso(120 * DAY),
    active_hold: {
      reason: "approval",
      since: iso(26 * HOUR),
      elapsed_occurrences: 1,
      elapsed_occurrences_capped: false,
    },
    recent_runs: [
      run("error", 26 * HOUR, { durationMs: 30_000 }),
      run("ok", 8 * DAY + 2 * HOUR),
      run("ok", 15 * DAY + 2 * HOUR),
      run("ok", 22 * DAY + 2 * HOUR),
    ],
  },
  {
    automation_id: "auto-log-compaction",
    name: "Log compaction",
    source: { type: "schedule", cron: "0 * * * *", timezone: "UTC" },
    state: "active",
    next_run_at: isoIn(38 * MINUTE),
    last_run_at: iso(HOUR + 22 * MINUTE),
    last_status: "ok",
    is_active: true,
    created_at: iso(200 * DAY),
    recent_runs: [
      run("running", 4 * MINUTE),
      run("ok", HOUR + 22 * MINUTE, { durationMs: 41_000 }),
      run("ok", 2 * HOUR + 22 * MINUTE, { durationMs: 39_000 }),
      run("ok", 3 * HOUR + 22 * MINUTE, { durationMs: 44_000 }),
      run("ok", 4 * HOUR + 22 * MINUTE, { durationMs: 40_000 }),
      run("ok", 5 * HOUR + 22 * MINUTE, { durationMs: 38_000 }),
    ],
  },
  {
    automation_id: "auto-release-reminder",
    name: "Release-cut reminder",
    source: { type: "once", at: isoIn(3 * DAY), timezone: "America/Los_Angeles" },
    state: "scheduled",
    next_run_at: isoIn(3 * DAY),
    last_run_at: null,
    last_status: null,
    is_active: true,
    created_at: iso(2 * DAY),
    recent_runs: [],
  },
  {
    automation_id: "auto-social-sweep",
    name: "Community mentions sweep",
    source: { type: "schedule", cron: "*/30 * * * *", timezone: "UTC" },
    state: "paused",
    next_run_at: isoIn(12 * MINUTE),
    last_run_at: iso(3 * DAY + 2 * HOUR),
    last_status: "ok",
    is_active: false,
    created_at: iso(60 * DAY),
    recent_runs: [
      run("ok", 3 * DAY + 2 * HOUR, { durationMs: 25_000 }),
      run("ok", 3 * DAY + 2 * HOUR + 30 * MINUTE, { durationMs: 27_000 }),
      run("ok", 3 * DAY + 3 * HOUR, { durationMs: 24_000 }),
    ],
  },
  {
    automation_id: "auto-v09-launch-email",
    name: "Send v0.9 launch email",
    source: { type: "once", at: iso(6 * DAY), timezone: "UTC" },
    state: "completed",
    next_run_at: null,
    last_run_at: iso(6 * DAY),
    last_status: "ok",
    is_active: false,
    created_at: iso(9 * DAY),
    recent_runs: [run("ok", 6 * DAY, { durationMs: 12_000 })],
  },
];

export function findAutomation(automationId: string): DemoAutomation | undefined {
  return automations.find((automation) => automation.automation_id === automationId);
}

export function listAutomations(includeCompleted: boolean) {
  const visible = includeCompleted
    ? automations
    : automations.filter((automation) => automation.state !== "completed");
  return { automations: visible, scheduler_enabled: true };
}

export function pauseAutomation(automationId: string): DemoAutomation | undefined {
  const automation = findAutomation(automationId);
  if (!automation) return undefined;
  automation.state = "paused";
  automation.is_active = false;
  return automation;
}

export function resumeAutomation(automationId: string): DemoAutomation | undefined {
  const automation = findAutomation(automationId);
  if (!automation) return undefined;
  automation.state = "scheduled";
  automation.is_active = true;
  if (!automation.next_run_at) automation.next_run_at = isoIn(30 * MINUTE);
  return automation;
}

export function renameAutomation(
  automationId: string,
  name: unknown
): DemoAutomation | undefined {
  const automation = findAutomation(automationId);
  if (!automation) return undefined;
  if (typeof name === "string" && name.trim()) automation.name = name.trim();
  return automation;
}

export function deleteAutomation(automationId: string): void {
  const index = automations.findIndex(
    (automation) => automation.automation_id === automationId
  );
  if (index >= 0) automations.splice(index, 1);
}
