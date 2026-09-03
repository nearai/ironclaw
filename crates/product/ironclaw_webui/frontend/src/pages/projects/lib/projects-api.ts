// Project endpoints now call the real WebChat v2 `/api/webchat/v2/projects`
// surface (list/create/read/update/delete + membership ACL) plus the v2
// project-filtered thread list. Widget reads remain a TODO stub until that v2
// project child endpoint lands.

import {
  addProjectMember as apiAddProjectMember,
  createProject as apiCreateProject,
  deleteProject as apiDeleteProject,
  getProject as apiGetProject,
  listProjectMembers as apiListProjectMembers,
  listProjects as apiListProjects,
  removeProjectMember as apiRemoveProjectMember,
  updateProject as apiUpdateProject,
  updateProjectMemberRole as apiUpdateProjectMemberRole,
  listThreads as apiListThreads,
} from "../../../lib/api";

// Fetch the largest supported card page. Lifecycle totals are independent
// authoritative fields in the list response.
const PROJECTS_OVERVIEW_LIMIT = 500;

function recordArrayField(response, field, responseName) {
  const value = response?.[field];
  if (
    !Array.isArray(value) ||
    !value.every(
      (entry) => typeof entry === "object" && entry !== null && !Array.isArray(entry),
    )
  ) {
    throw new TypeError(`invalid ${responseName} response`);
  }
  return value;
}

function numberField(response, field, responseName) {
  const value = response?.[field];
  if (typeof value !== "number") {
    throw new TypeError(`invalid ${responseName} response`);
  }
  return value;
}

function requiredStringField(record, field, responseName) {
  const value = record?.[field];
  if (typeof value !== "string") {
    throw new TypeError(`invalid ${responseName} response`);
  }
  return value;
}

function optionalStringField(record, field, responseName) {
  const value = record?.[field];
  if (value !== undefined && value !== null && typeof value !== "string") {
    throw new TypeError(`invalid ${responseName} response`);
  }
  return value ?? null;
}

// Map a wire `RebornProjectInfo` to the shape the Projects page components
// expect. `goals` is read from the extensible `metadata` bag.
function toPageProject(project) {
  if (typeof project !== "object" || project === null || Array.isArray(project)) {
    throw new TypeError("invalid project response");
  }
  const projectId = requiredStringField(project, "project_id", "project");
  const name = requiredStringField(project, "name", "project");
  const description = requiredStringField(project, "description", "project");
  const createdAt = requiredStringField(project, "created_at", "project");
  const updatedAt = requiredStringField(project, "updated_at", "project");
  if (!["active", "archived"].includes(project.state)) {
    throw new TypeError("invalid project response");
  }
  if (!["owner", "editor", "viewer"].includes(project.role)) {
    throw new TypeError("invalid project response");
  }
  // The server constrains `metadata` to a JSON object or null
  // (`ProjectRecord::validate`), but guard against arrays defensively
  // (`typeof [] === "object"`) so the page always treats it as an object bag.
  const metadata =
    project.metadata &&
    typeof project.metadata === "object" &&
    !Array.isArray(project.metadata)
      ? project.metadata
      : {};
  return {
    id: projectId,
    name,
    description,
    goals: Array.isArray(metadata.goals) ? metadata.goals : [],
    icon: optionalStringField(project, "icon", "project"),
    color: optionalStringField(project, "color", "project"),
    state: project.state,
    role: project.role,
    metadata,
    created_at: createdAt,
    updated_at: updatedAt,
  };
}

function toPageThread(thread) {
  if (typeof thread !== "object" || thread === null || Array.isArray(thread)) {
    throw new TypeError("invalid project thread response");
  }
  const threadId = requiredStringField(thread, "thread_id", "project thread");
  optionalStringField(thread, "title", "project thread");
  optionalStringField(thread, "updated_at", "project thread");
  return {
    ...thread,
    id: threadId,
    state: thread.state || null,
    turn_count: thread.turn_count || 0,
    updated_at: thread.updated_at || null,
  };
}

export async function fetchProjectsOverview() {
  const response = await apiListProjects({ limit: PROJECTS_OVERVIEW_LIMIT });
  const projects = recordArrayField(response, "projects", "project list").map(
    toPageProject,
  );
  return {
    projects,
    lifecycleCounts: {
      total: numberField(response, "total_projects", "project list"),
      active: numberField(response, "active_projects", "project list"),
      archived: numberField(response, "archived_projects", "project list"),
    },
  };
}

export async function fetchProjectDetail(projectId) {
  if (!projectId) return null;
  const response = await apiGetProject({ projectId });
  return toPageProject(response?.project);
}

export async function createProject(input) {
  const response = await apiCreateProject(input);
  return toPageProject(response?.project);
}

export async function updateProject(input) {
  const response = await apiUpdateProject(input);
  return toPageProject(response?.project);
}

export function deleteProject(projectId) {
  return apiDeleteProject({ projectId });
}

export async function fetchProjectMembers(projectId) {
  if (!projectId) return { members: [] };
  return apiListProjectMembers({ projectId });
}

export function addProjectMember(projectId, userId, role) {
  return apiAddProjectMember({ projectId, userId, role });
}

export function updateProjectMemberRole(projectId, userId, role) {
  return apiUpdateProjectMemberRole({ projectId, userId, role });
}

export function removeProjectMember(projectId, userId) {
  return apiRemoveProjectMember({ projectId, userId });
}

export async function fetchProjectThreads(projectId) {
  if (!projectId) return { threads: [] };
  const response = await apiListThreads({ projectId, limit: 200 });
  return {
    threads: recordArrayField(response, "threads", "project threads")
      .map(toPageThread)
      .filter(Boolean),
    next_cursor: response?.next_cursor || null,
  };
}
export function fetchProjectWidgets(_projectId) {
  return Promise.resolve({ widgets: [], todo: true });
}
export function fetchThreadDetail(_threadId) {
  return Promise.resolve(null);
}
