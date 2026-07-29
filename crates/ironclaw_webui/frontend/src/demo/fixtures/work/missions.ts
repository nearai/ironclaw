// DEMO mission fixtures for `/api/engine/missions*`.
//
// Field names follow what the Missions page consumes (`useMissions`,
// `mission-detail-panel`, `missions-presenters`): `status` uses the
// capitalized Active/Paused/Completed/Failed vocabulary, list responses are
// `{ missions }`, detail is `{ mission }`. Missions carry a `project_id` so
// the per-project list filter works.

import { DAY, HOUR, MINUTE, iso, isoIn } from "./clock";

type MissionThread = {
  id: string;
  title: string;
  goal?: string;
  state: "Running" | "Completed" | "Failed" | "Done";
};

export type DemoMission = {
  id: string;
  project_id: string;
  name: string;
  status: "Active" | "Paused" | "Completed" | "Failed";
  goal: string;
  cadence_type: string;
  cadence_description: string | null;
  thread_count: number;
  threads_today: number;
  max_threads_per_day: number | null;
  next_fire_at: string | null;
  created_at: string;
  updated_at: string;
  current_focus?: string;
  success_criteria?: string;
  threads: MissionThread[];
};

export const missions: DemoMission[] = [
  {
    id: "mission-release-notes",
    project_id: "project-atlas",
    name: "v0.9 release notes",
    status: "Active",
    goal:
      "Keep `notes/release-v0.9.md` current as PRs merge: group changes by area, flag breaking changes, and open the release-notes PR when the milestone closes.",
    cadence_type: "cron",
    cadence_description: "Weekdays at 09:00 UTC",
    thread_count: 6,
    threads_today: 2,
    max_threads_per_day: 8,
    next_fire_at: isoIn(16 * HOUR),
    created_at: iso(20 * DAY),
    updated_at: iso(8 * MINUTE),
    current_focus:
      "PR #6841 (release notes draft) is open with CI running. Watching for the last two milestone PRs to merge.",
    success_criteria:
      "- Release notes cover every merged PR since the v0.8 tag\n- Breaking changes called out in their own section\n- PR approved before the release branch cut",
    threads: [
      {
        id: "thread-release-notes",
        title: "Draft the v0.9 release notes",
        state: "Completed",
      },
      {
        id: "thread-mission-relnotes-2",
        title: "Sweep merged PRs since yesterday",
        state: "Running",
      },
    ],
  },
  {
    id: "mission-ci-sentinel",
    project_id: "project-atlas",
    name: "CI sentinel",
    status: "Active",
    goal:
      "Watch CI on `main`. When a required check fails twice in a row, bisect the failing commit range and open a triage thread with the suspect diff.",
    cadence_type: "cron",
    cadence_description: "Every 30 minutes",
    thread_count: 11,
    threads_today: 3,
    max_threads_per_day: 12,
    next_fire_at: isoIn(18 * MINUTE),
    created_at: iso(38 * DAY),
    updated_at: iso(42 * MINUTE),
    current_focus: "main is green. Last flake: `webui_v2_handlers_contract` 6h ago (auto-retried).",
    threads: [
      {
        id: "thread-sandbox-triage",
        title: "Why did the sandbox job time out?",
        state: "Completed",
      },
      {
        id: "thread-mission-ci-2",
        title: "Bisect: flaky contract test on main",
        state: "Completed",
      },
    ],
  },
  {
    id: "mission-flaky-tests",
    project_id: "project-atlas",
    name: "Flaky test quarantine",
    status: "Paused",
    goal:
      "Track tests that fail then pass on retry. After three flakes in a week, quarantine the test and open an issue with the failure fingerprints.",
    cadence_type: "manual",
    cadence_description: null,
    thread_count: 4,
    threads_today: 0,
    max_threads_per_day: null,
    next_fire_at: null,
    created_at: iso(25 * DAY),
    updated_at: iso(3 * DAY),
    current_focus: "Paused during the v0.9 stabilization window — resume after the release cut.",
    threads: [
      {
        id: "thread-mission-flaky-1",
        title: "Quarantine: sandbox credential reuse test",
        state: "Completed",
      },
    ],
  },
  {
    id: "mission-v08-postmortem",
    project_id: "project-atlas",
    name: "v0.8 release postmortem",
    status: "Completed",
    goal:
      "Collect timeline, incidents, and action items from the v0.8 release into a postmortem doc, then file the action items as issues.",
    cadence_type: "manual",
    cadence_description: null,
    thread_count: 3,
    threads_today: 0,
    max_threads_per_day: null,
    next_fire_at: null,
    created_at: iso(34 * DAY),
    updated_at: iso(19 * DAY),
    success_criteria: "Postmortem doc reviewed; all 5 action items filed and assigned.",
    threads: [
      {
        id: "thread-mission-pm-1",
        title: "Draft v0.8 postmortem timeline",
        state: "Completed",
      },
    ],
  },
  {
    id: "mission-ticket-triage",
    project_id: "project-nimbus",
    name: "Ticket triage",
    status: "Active",
    goal:
      "Label every new support ticket within 15 minutes, draft a first reply for the queue, and page the on-call for anything with a crash signature.",
    cadence_type: "event",
    cadence_description: "On new ticket",
    thread_count: 27,
    threads_today: 5,
    max_threads_per_day: 40,
    next_fire_at: null,
    created_at: iso(29 * DAY),
    updated_at: iso(35 * MINUTE),
    current_focus: "Queue is clear. 5 tickets triaged today, median time-to-label 4m.",
    threads: [
      {
        id: "thread-mission-triage-1",
        title: "Ticket #4821: login loop after OAuth",
        state: "Running",
      },
      {
        id: "thread-mission-triage-2",
        title: "Ticket #4817: export CSV encoding",
        state: "Completed",
      },
    ],
  },
  {
    id: "mission-weekly-digest",
    project_id: "project-nimbus",
    name: "Weekly support digest",
    status: "Failed",
    goal:
      "Every Friday, summarize ticket volume, top issues, and notable escalations into an email digest for the support channel.",
    cadence_type: "cron",
    cadence_description: "Fridays at 16:00 UTC",
    thread_count: 8,
    threads_today: 0,
    max_threads_per_day: 2,
    next_fire_at: isoIn(2 * DAY + 5 * HOUR),
    created_at: iso(27 * DAY),
    updated_at: iso(3 * DAY + 2 * HOUR),
    current_focus:
      "Last run failed: the email provider rate-limited the digest send. Needs a retry with batched recipients.",
    threads: [
      {
        id: "thread-mission-digest-1",
        title: "Weekly digest — send failed (rate limit)",
        state: "Failed",
      },
    ],
  },
];

export function missionsForProject(projectId: string | null): DemoMission[] {
  if (!projectId) return missions;
  return missions.filter((mission) => mission.project_id === projectId);
}

export function findMission(missionId: string): DemoMission | undefined {
  return missions.find((mission) => mission.id === missionId);
}

export function missionSummary() {
  const counts = { total: missions.length, active: 0, paused: 0, completed: 0, failed: 0 };
  for (const mission of missions) {
    if (mission.status === "Active") counts.active += 1;
    else if (mission.status === "Paused") counts.paused += 1;
    else if (mission.status === "Completed") counts.completed += 1;
    else if (mission.status === "Failed") counts.failed += 1;
  }
  return counts;
}

export function applyMissionAction(missionId: string, action: string): boolean {
  const mission = findMission(missionId);
  if (!mission) return false;
  const now = new Date().toISOString();
  if (action === "pause") {
    mission.status = "Paused";
    mission.next_fire_at = null;
  } else if (action === "resume") {
    mission.status = "Active";
    mission.next_fire_at = isoIn(30 * MINUTE);
  } else if (action === "fire") {
    mission.thread_count += 1;
    mission.threads_today += 1;
    mission.threads.unshift({
      id: `thread-mission-fire-${Date.now()}`,
      title: `${mission.name} — manual run`,
      state: "Running",
    });
    if (mission.status !== "Active" && mission.status !== "Paused") {
      mission.status = "Active";
    }
  }
  mission.updated_at = now;
  return true;
}
