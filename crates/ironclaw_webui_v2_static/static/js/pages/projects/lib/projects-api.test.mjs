import assert from "node:assert/strict";
import test from "node:test";

import {
  addProjectMember,
  createProject,
  deleteProject,
  fetchMissionDetail,
  fetchProjectDetail,
  fetchProjectMembers,
  fetchProjectMissions,
  fetchProjectThreads,
  fetchProjectWidgets,
  fetchProjectsOverview,
  fireMission,
  pauseMission,
  removeProjectMember,
  resumeMission,
  updateProject,
  updateProjectMemberRole,
} from "./projects-api.js";

function installBrowserStubs({ responses, token = "projects-token" } = {}) {
  const calls = [];
  globalThis.window = { location: { origin: "https://app.test" } };
  globalThis.sessionStorage = {
    getItem: () => token,
    setItem: () => {},
    removeItem: () => {},
  };
  globalThis.fetch = async (path, options) => {
    calls.push({ path, options });
    const response = responses.shift();
    if (!response) throw new Error(`unexpected fetch: ${path}`);
    return response;
  };
  return calls;
}

function jsonResponse(body) {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

function noContentResponse() {
  return new Response(null, { status: 204 });
}

test("fetchProjectsOverview maps project records for page cards", async () => {
  const calls = installBrowserStubs({
    responses: [
      jsonResponse({
        projects: [
          {
            project_id: "project-1",
            name: "Alpha",
            description: "Plan launch",
            icon: "rocket",
            color: "#3366ff",
            state: "active",
            role: "owner",
            metadata: { goals: ["ship"], extra: true },
            created_at: "2026-01-01T00:00:00Z",
            updated_at: "2026-01-02T00:00:00Z",
          },
          {
            project_id: "project-2",
            name: "Archive",
            state: "archived",
            metadata: [],
          },
        ],
      }),
    ],
  });

  const result = await fetchProjectsOverview();

  assert.deepEqual(result.attention, []);
  assert.equal(result.projects[0].id, "project-1");
  assert.equal(result.projects[0].name, "Alpha");
  assert.deepEqual(result.projects[0].goals, ["ship"]);
  assert.equal(result.projects[0].health, "green");
  assert.deepEqual(result.projects[1].metadata, {});
  assert.deepEqual(result.projects[1].goals, []);
  assert.equal(result.projects[1].health, "muted");
  assert.equal(calls[0].path, "/api/webchat/v2/projects?limit=200");
  assert.equal(calls[0].options.headers.get("Authorization"), "Bearer projects-token");
});

test("fetchProjectDetail returns null without a project id", async () => {
  const calls = installBrowserStubs({ responses: [] });

  assert.equal(await fetchProjectDetail(""), null);
  assert.equal(calls.length, 0);
});

test("fetchProjectDetail reads the encoded v2 project detail route", async () => {
  const calls = installBrowserStubs({
    responses: [
      jsonResponse({
        project: {
          project_id: "project/1",
          name: "Alpha",
          state: "active",
          metadata: { goals: ["ship"] },
        },
      }),
    ],
  });

  const result = await fetchProjectDetail("project/1");

  assert.equal(result.id, "project/1");
  assert.equal(result.name, "Alpha");
  assert.deepEqual(result.goals, ["ship"]);
  assert.equal(calls[0].path, "/api/webchat/v2/projects/project%2F1");
  assert.equal(calls[0].options.headers.get("Authorization"), "Bearer projects-token");
});

test("project create, update, and delete send v2 project payloads", async () => {
  const calls = installBrowserStubs({
    responses: [
      jsonResponse({ project: { project_id: "created", name: "Created" } }),
      jsonResponse({ project: { project_id: "project/1", name: "Renamed" } }),
      noContentResponse(),
    ],
  });

  await createProject({
    name: "Created",
    description: "A project",
    metadata: { goals: ["qa"] },
  });
  await updateProject({
    projectId: "project/1",
    name: "Renamed",
    state: "archived",
  });
  await deleteProject("project/1");

  assert.equal(calls[0].path, "/api/webchat/v2/projects");
  assert.equal(calls[0].options.method, "POST");
  assert.deepEqual(JSON.parse(calls[0].options.body), {
    name: "Created",
    description: "A project",
    metadata: { goals: ["qa"] },
  });
  assert.equal(calls[1].path, "/api/webchat/v2/projects/project%2F1");
  assert.equal(calls[1].options.method, "POST");
  assert.deepEqual(JSON.parse(calls[1].options.body), {
    name: "Renamed",
    state: "archived",
  });
  assert.equal(calls[2].path, "/api/webchat/v2/projects/project%2F1");
  assert.equal(calls[2].options.method, "DELETE");

  await assert.rejects(deleteProject(""), /projectId is required/);
});

test("project membership helpers encode ids and fail closed on missing fields", async () => {
  const calls = installBrowserStubs({
    responses: [
      jsonResponse({ members: [] }),
      jsonResponse({ ok: true }),
      jsonResponse({ ok: true }),
      jsonResponse({ ok: true }),
    ],
  });

  await fetchProjectMembers("project/1");
  await addProjectMember("project/1", "user/a", "editor");
  await updateProjectMemberRole("project/1", "user/a", "viewer");
  await removeProjectMember("project/1", "user/a");

  assert.equal(calls[0].path, "/api/webchat/v2/projects/project%2F1/members");
  assert.equal(calls[1].path, "/api/webchat/v2/projects/project%2F1/members");
  assert.deepEqual(JSON.parse(calls[1].options.body), {
    user_id: "user/a",
    role: "editor",
  });
  assert.equal(calls[2].path, "/api/webchat/v2/projects/project%2F1/members/user%2Fa");
  assert.deepEqual(JSON.parse(calls[2].options.body), { role: "viewer" });
  assert.equal(calls[3].path, "/api/webchat/v2/projects/project%2F1/members/user%2Fa");
  assert.equal(calls[3].options.method, "DELETE");

  await assert.rejects(addProjectMember("", "user", "editor"), /projectId and userId/);
  await assert.rejects(addProjectMember("project", "user", ""), /role is required/);
  await assert.rejects(updateProjectMemberRole("project", "", "viewer"), /projectId and userId/);
  await assert.rejects(removeProjectMember("project", ""), /projectId and userId/);
});

test("fetchProjectMembers returns an empty list without a project id", async () => {
  const calls = installBrowserStubs({ responses: [] });

  assert.deepEqual(await fetchProjectMembers(""), { members: [] });
  assert.equal(calls.length, 0);
});

test("project mission and thread helpers remain TODO stubs without fetch", async () => {
  const calls = installBrowserStubs({ responses: [] });

  assert.deepEqual(await fetchProjectMissions("project-1"), {
    missions: [],
    todo: true,
  });
  assert.deepEqual(await fetchProjectThreads("project-1"), {
    threads: [],
    todo: true,
  });
  assert.deepEqual(await fetchProjectWidgets("project-1"), {
    widgets: [],
    todo: true,
  });
  assert.equal(await fetchMissionDetail("mission-1"), null);
  assert.deepEqual(await fireMission("mission-1"), {
    success: false,
    message: "TODO: requires v2 missions endpoint",
  });
  assert.deepEqual(await pauseMission("mission-1"), {
    success: false,
    message: "TODO: requires v2 missions endpoint",
  });
  assert.deepEqual(await resumeMission("mission-1"), {
    success: false,
    message: "TODO: requires v2 missions endpoint",
  });
  assert.equal(calls.length, 0);
});
