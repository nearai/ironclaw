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

// Map a wire `RebornProjectInfo` to the shape the Projects page components
// expect. `goals` is read from the extensible `metadata` bag.
function toPageProject(project) {
  if (!project) return null;
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
    id: project.project_id,
    name: project.name,
    description: project.description,
    goals: Array.isArray(metadata.goals) ? metadata.goals : [],
    icon: project.icon || null,
    color: project.color || null,
    state: project.state,
    role: project.role,
    metadata,
    created_at: project.created_at,
    updated_at: project.updated_at,
  };
}

function toPageThread(thread) {
  if (!thread) return null;
  return {
    ...thread,
    id: thread.thread_id,
    state: thread.state || null,
    turn_count: thread.turn_count || 0,
    updated_at: thread.updated_at || null,
  };
}

export async function fetchProjectsOverview() {
  const response = await apiListProjects({ limit: PROJECTS_OVERVIEW_LIMIT });
  const projects = (response?.projects || []).map(toPageProject);
  return {
    projects,
    lifecycleCounts: {
      total: response?.total_projects ?? 0,
      active: response?.active_projects ?? 0,
      archived: response?.archived_projects ?? 0,
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
    threads: (response?.threads || []).map(toPageThread).filter(Boolean),
    next_cursor: response?.next_cursor || null,
  };
}
export function fetchProjectWidgets(_projectId) {
  return Promise.resolve({ widgets: [], todo: true });
}
export function fetchThreadDetail(_threadId) {
  return Promise.resolve(null);
}
