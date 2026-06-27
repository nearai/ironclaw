import assert from "node:assert/strict";
import test from "node:test";

import {
  activateAdminUser,
  createAdminUser,
  createUserToken,
  deleteAdminUser,
  fetchAdminUser,
  fetchAdminUsers,
  fetchUsage,
  fetchUsageSummary,
  suspendAdminUser,
  updateAdminUser,
} from "../pages/admin/lib/admin-api.js";
import {
  cancelJob,
  fetchJobDetail,
  fetchJobEvents,
  fetchJobFiles,
  fetchJobs,
  fetchJobsSummary,
  readJobFile,
  restartJob,
  sendJobPrompt,
} from "../pages/jobs/lib/jobs-api.js";
import {
  fetchMissionDetail,
  fetchMissions,
  fetchProjectsOverview,
  fireMission,
  pauseMission,
  resumeMission,
} from "../pages/missions/lib/missions-api.js";
import {
  deleteRoutine,
  fetchRoutineDetail,
  fetchRoutines,
  fetchRoutinesSummary,
  toggleRoutine,
  triggerRoutine,
} from "../pages/routines/lib/routines-api.js";

function forbidFetch() {
  let fetchCalled = false;
  globalThis.fetch = async () => {
    fetchCalled = true;
    throw new Error("stubbed hidden-route APIs must not call fetch");
  };
  return () => fetchCalled;
}

test("hidden jobs API adapter returns empty TODO data without legacy v1 fetches", async () => {
  const fetchCalled = forbidFetch();

  assert.deepEqual(await fetchJobs(), { jobs: [], pagination: null, todo: true });
  assert.deepEqual(await fetchJobsSummary(), {
    total: 0,
    active: 0,
    completed: 0,
    failed: 0,
    todo: true,
  });
  assert.equal(await fetchJobDetail("job-1"), null);
  assert.deepEqual(await fetchJobEvents("job-1"), { events: [], todo: true });
  assert.deepEqual(await fetchJobFiles("job-1", "/"), { entries: [], todo: true });
  assert.deepEqual(await readJobFile("job-1", "log.txt"), { content: "", todo: true });
  for (const action of [cancelJob, restartJob, sendJobPrompt]) {
    const result = await action("job-1", {});
    assert.equal(result.success, false);
    assert.match(result.message, /v2 jobs endpoint/);
  }
  assert.equal(fetchCalled(), false);
});

test("hidden routines API adapter returns empty TODO data without legacy v1 fetches", async () => {
  const fetchCalled = forbidFetch();

  assert.deepEqual(await fetchRoutines(), { routines: [], todo: true });
  assert.deepEqual(await fetchRoutinesSummary(), {
    total: 0,
    active: 0,
    paused: 0,
    todo: true,
  });
  assert.equal(await fetchRoutineDetail("routine-1"), null);
  for (const action of [triggerRoutine, toggleRoutine, deleteRoutine]) {
    const result = await action("routine-1");
    assert.equal(result.success, false);
    assert.match(result.message, /v2 routines endpoint/);
  }
  assert.equal(fetchCalled(), false);
});

test("hidden missions API adapter returns empty TODO data without legacy v1 fetches", async () => {
  const fetchCalled = forbidFetch();

  assert.deepEqual(await fetchProjectsOverview(), { projects: [], todo: true });
  assert.deepEqual(await fetchMissions({ projectId: "project-1" }), {
    missions: [],
    todo: true,
  });
  assert.equal(await fetchMissionDetail("mission-1"), null);
  for (const action of [fireMission, pauseMission, resumeMission]) {
    const result = await action("mission-1");
    assert.equal(result.success, false);
    assert.match(result.message, /v2 missions endpoint/);
  }
  assert.equal(fetchCalled(), false);
});

test("hidden admin API adapter returns empty TODO data without legacy v1 fetches", async () => {
  const fetchCalled = forbidFetch();

  assert.deepEqual(await fetchAdminUsers(), { users: [], total: 0, todo: true });
  assert.equal(await fetchAdminUser("user-1"), null);
  assert.deepEqual(await fetchUsage("day"), { entries: [], todo: true });
  assert.deepEqual(await fetchUsageSummary(), {
    total_users: 0,
    active_users: 0,
    suspended_users: 0,
    admin_users: 0,
    total_jobs: 0,
    llm_calls: 0,
    total_cost_usd: 0,
    active_jobs: 0,
    uptime_seconds: 0,
    recent_users: [],
    todo: true,
  });

  for (const action of [
    createAdminUser,
    updateAdminUser,
    deleteAdminUser,
    suspendAdminUser,
    activateAdminUser,
    createUserToken,
  ]) {
    const result = await action("user-1", {});
    assert.equal(result.success, false);
    assert.match(result.message, /v2 admin endpoint/);
  }
  assert.equal(fetchCalled(), false);
});
