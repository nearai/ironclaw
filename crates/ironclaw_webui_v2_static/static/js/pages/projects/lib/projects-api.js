// Project endpoints now call the real WebChat v2 `/api/webchat/v2/projects`
// surface (list/create/read/update/delete + membership ACL). Mission/thread/
// widget reads remain TODO stubs until the v2 missions/threads-by-project
// endpoints land — the page degrades to empty panels for those.

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
} from "../../../lib/api.js";

// Map a wire `RebornProjectInfo` to the shape the Projects page components
// expect. Mission/spend/gate metrics default to 0 in the components (`|| 0`),
// so they render cleanly before those endpoints exist. `goals` is read from the
// extensible `metadata` bag.
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
    health: project.state === "archived" ? "muted" : "green",
  };
}

export async function fetchProjectsOverview() {
  const response = await apiListProjects({ limit: 200 });
  const projects = (response?.projects || []).map(toPageProject);
  return { attention: [], projects };
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

// --- Still-stubbed: per-project missions/threads/widgets (need v2 endpoints) ---

export function fetchProjectMissions(_projectId) {
  return Promise.resolve({ missions: [], todo: true });
}
export function fetchProjectThreads(_projectId) {
  return Promise.resolve({ threads: [], todo: true });
}
export function fetchProjectWidgets(_projectId) {
  return Promise.resolve({ widgets: [], todo: true });
}
export function fetchMissionDetail(_missionId) {
  return Promise.resolve(null);
}
export function fetchThreadDetail(_threadId) {
  return Promise.resolve(null);
}
export function fireMission(_missionId) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 missions endpoint" });
}
export function pauseMission(_missionId) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 missions endpoint" });
}
export function resumeMission(_missionId) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 missions endpoint" });
}
