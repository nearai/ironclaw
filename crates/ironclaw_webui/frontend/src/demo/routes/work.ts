// DEMO fixtures for the "work" surfaces: projects, missions, jobs, routines,
// automations, the workspace file browser (/fs/*), and outbound delivery
// preferences.
//
// Fixture data lives in `../fixtures/work/*`; this module only maps
// (method, path) pairs onto that in-memory state. Mutations mutate the module
// state so pause/resume/rename/toggle/etc. are reflected on the next refetch.
// Route order matters where a literal segment ("summary") would otherwise be
// swallowed by an `:id` pattern — literals are registered first.

import type { DemoRoute } from "../types";
import {
  createProject,
  deleteProject,
  findProject,
  projectMembers,
  projects,
  updateProject,
} from "../fixtures/work/projects";
import {
  applyMissionAction,
  findMission,
  missionSummary,
  missionsForProject,
} from "../fixtures/work/missions";
import {
  cancelJob,
  findJob,
  jobEvents,
  jobs,
  jobsSummary,
  listJobFiles,
  readJobFile,
  restartJob,
} from "../fixtures/work/jobs";
import {
  deleteRoutine,
  findRoutine,
  routines,
  routinesSummary,
  toggleRoutine,
  triggerRoutine,
} from "../fixtures/work/routines";
import {
  deleteAutomation,
  listAutomations,
  pauseAutomation,
  renameAutomation,
  resumeAutomation,
} from "../fixtures/work/automations";
import {
  fsMounts,
  listFsEntries,
  readFsContent,
  statFsEntry,
} from "../fixtures/work/workspace-fs";
import {
  listDeliveryTargets,
  outboundPreferences,
  setFinalReplyTarget,
} from "../fixtures/work/outbound";

const NOT_FOUND = { status: 404, json: { error: "not_found", kind: "not_found" } };

function param(match: RegExpExecArray, index = 1): string {
  return decodeURIComponent(match[index]);
}

/* ── Projects ──────────────────────────────────────────────────────── */

const projectRoutes: DemoRoute[] = [
  {
    method: "GET",
    pattern: /^\/api\/webchat\/v2\/projects$/,
    handle: () => ({ json: { projects } }),
  },
  {
    method: "POST",
    pattern: /^\/api\/webchat\/v2\/projects$/,
    handle: (req) => ({ json: { project: createProject(req.body) } }),
  },
  {
    method: "GET",
    pattern: /^\/api\/webchat\/v2\/projects\/([^/]+)$/,
    handle: (_req, match) => {
      const project = findProject(param(match));
      return project ? { json: { project } } : NOT_FOUND;
    },
  },
  {
    method: "POST",
    pattern: /^\/api\/webchat\/v2\/projects\/([^/]+)$/,
    handle: (req, match) => {
      const project = updateProject(param(match), req.body);
      return project ? { json: { project } } : NOT_FOUND;
    },
  },
  {
    method: "DELETE",
    pattern: /^\/api\/webchat\/v2\/projects\/([^/]+)$/,
    handle: (_req, match) => {
      deleteProject(param(match));
      return { json: {} };
    },
  },
  {
    method: "GET",
    pattern: /^\/api\/webchat\/v2\/projects\/([^/]+)\/members$/,
    handle: (_req, match) => ({ json: { members: projectMembers(param(match)) } }),
  },
  {
    method: "POST",
    pattern: /^\/api\/webchat\/v2\/projects\/([^/]+)\/members$/,
    handle: (req, match) => {
      const members = projectMembers(param(match));
      const userId = typeof req.body?.user_id === "string" ? req.body.user_id : "";
      const role = typeof req.body?.role === "string" ? req.body.role : "viewer";
      if (userId && !members.some((member) => member.user_id === userId)) {
        members.push({
          user_id: userId,
          role: role as "owner" | "editor" | "viewer",
          status: "active",
          granted_by: "demo-operator",
          created_at: new Date().toISOString(),
          updated_at: new Date().toISOString(),
        });
      }
      return { json: { members } };
    },
  },
  {
    method: "POST",
    pattern: /^\/api\/webchat\/v2\/projects\/([^/]+)\/members\/([^/]+)$/,
    handle: (req, match) => {
      const members = projectMembers(param(match));
      const member = members.find((entry) => entry.user_id === param(match, 2));
      if (member && typeof req.body?.role === "string") {
        member.role = req.body.role as "owner" | "editor" | "viewer";
        member.updated_at = new Date().toISOString();
      }
      return { json: {} };
    },
  },
  {
    method: "DELETE",
    pattern: /^\/api\/webchat\/v2\/projects\/([^/]+)\/members\/([^/]+)$/,
    handle: (_req, match) => {
      const members = projectMembers(param(match));
      const index = members.findIndex((entry) => entry.user_id === param(match, 2));
      if (index >= 0) members.splice(index, 1);
      return { json: {} };
    },
  },
];

/* ── Missions ──────────────────────────────────────────────────────── */

const missionRoutes: DemoRoute[] = [
  {
    // The missions page fans out over a project overview keyed by `id`
    // (not the v2 `project_id`), then queries missions per project.
    method: "GET",
    pattern: /^\/api\/engine\/projects\/overview$/,
    handle: () => ({
      json: {
        projects: projects.map((project) => ({
          ...project,
          id: project.project_id,
          health: project.state === "active" ? "healthy" : "paused",
        })),
      },
    }),
  },
  {
    method: "GET",
    pattern: /^\/api\/engine\/missions\/summary$/,
    handle: () => ({ json: missionSummary() }),
  },
  {
    method: "GET",
    pattern: /^\/api\/engine\/missions$/,
    handle: (req) => ({
      json: { missions: missionsForProject(req.url.searchParams.get("project_id")) },
    }),
  },
  {
    method: "POST",
    pattern: /^\/api\/engine\/missions\/([^/]+)\/(pause|resume|fire)$/,
    handle: (_req, match) =>
      applyMissionAction(param(match), match[2]) ? { json: { success: true } } : NOT_FOUND,
  },
  {
    method: "GET",
    pattern: /^\/api\/engine\/missions\/([^/]+)$/,
    handle: (_req, match) => {
      const mission = findMission(param(match));
      return mission ? { json: { mission } } : { json: { mission: null } };
    },
  },
];

/* ── Jobs ──────────────────────────────────────────────────────────── */

const jobRoutes: DemoRoute[] = [
  {
    method: "GET",
    pattern: /^\/api\/jobs\/summary$/,
    handle: () => ({ json: jobsSummary() }),
  },
  {
    method: "GET",
    pattern: /^\/api\/jobs$/,
    handle: () => ({ json: { jobs, pagination: null } }),
  },
  {
    method: "GET",
    pattern: /^\/api\/jobs\/([^/]+)\/events$/,
    handle: (_req, match) => ({ json: { events: jobEvents(param(match)) } }),
  },
  {
    method: "GET",
    pattern: /^\/api\/jobs\/([^/]+)\/files\/list$/,
    handle: (req, match) => ({
      json: { entries: listJobFiles(param(match), req.url.searchParams.get("path") || "") },
    }),
  },
  {
    method: "GET",
    pattern: /^\/api\/jobs\/([^/]+)\/files\/read$/,
    handle: (req, match) => {
      const path = req.url.searchParams.get("path") || "";
      const content = readJobFile(param(match), path);
      return content === null ? NOT_FOUND : { json: { content, path } };
    },
  },
  {
    method: "POST",
    pattern: /^\/api\/jobs\/([^/]+)\/cancel$/,
    handle: (_req, match) =>
      cancelJob(param(match)) ? { json: { success: true } } : NOT_FOUND,
  },
  {
    method: "POST",
    pattern: /^\/api\/jobs\/([^/]+)\/restart$/,
    handle: (_req, match) => {
      const newJobId = restartJob(param(match));
      return newJobId ? { json: { success: true, new_job_id: newJobId } } : NOT_FOUND;
    },
  },
  {
    method: "POST",
    pattern: /^\/api\/jobs\/([^/]+)\/prompt$/,
    handle: () => ({ json: { success: true } }),
  },
  {
    method: "GET",
    pattern: /^\/api\/jobs\/([^/]+)$/,
    handle: (_req, match) => {
      const job = findJob(param(match));
      return job ? { json: job } : NOT_FOUND;
    },
  },
];

/* ── Routines ──────────────────────────────────────────────────────── */

const routineRoutes: DemoRoute[] = [
  {
    method: "GET",
    pattern: /^\/api\/routines\/summary$/,
    handle: () => ({ json: routinesSummary() }),
  },
  {
    method: "GET",
    pattern: /^\/api\/routines$/,
    handle: () => ({ json: { routines } }),
  },
  {
    method: "POST",
    pattern: /^\/api\/routines\/([^/]+)\/trigger$/,
    handle: (_req, match) =>
      triggerRoutine(param(match)) ? { json: { success: true } } : NOT_FOUND,
  },
  {
    method: "POST",
    pattern: /^\/api\/routines\/([^/]+)\/toggle$/,
    handle: (_req, match) =>
      toggleRoutine(param(match)) ? { json: { success: true } } : NOT_FOUND,
  },
  {
    method: "DELETE",
    pattern: /^\/api\/routines\/([^/]+)$/,
    handle: (_req, match) => {
      deleteRoutine(param(match));
      return { json: { success: true } };
    },
  },
  {
    method: "GET",
    pattern: /^\/api\/routines\/([^/]+)$/,
    handle: (_req, match) => {
      const routine = findRoutine(param(match));
      return routine ? { json: routine } : NOT_FOUND;
    },
  },
];

/* ── Automations ───────────────────────────────────────────────────── */

const automationRoutes: DemoRoute[] = [
  {
    method: "GET",
    pattern: /^\/api\/webchat\/v2\/automations$/,
    handle: (req) => ({
      json: listAutomations(req.url.searchParams.get("include_completed") === "true"),
    }),
  },
  {
    method: "POST",
    pattern: /^\/api\/webchat\/v2\/automations\/([^/]+)\/pause$/,
    handle: (_req, match) => {
      const automation = pauseAutomation(param(match));
      return automation ? { json: { automation } } : NOT_FOUND;
    },
  },
  {
    method: "POST",
    pattern: /^\/api\/webchat\/v2\/automations\/([^/]+)\/resume$/,
    handle: (_req, match) => {
      const automation = resumeAutomation(param(match));
      return automation ? { json: { automation } } : NOT_FOUND;
    },
  },
  {
    method: "POST",
    pattern: /^\/api\/webchat\/v2\/automations\/([^/]+)$/,
    handle: (req, match) => {
      const automation = renameAutomation(param(match), req.body?.name);
      return automation ? { json: { automation } } : NOT_FOUND;
    },
  },
  {
    method: "DELETE",
    pattern: /^\/api\/webchat\/v2\/automations\/([^/]+)$/,
    handle: (_req, match) => {
      deleteAutomation(param(match));
      return { json: {} };
    },
  },
];

/* ── Outbound delivery ─────────────────────────────────────────────── */

const outboundRoutes: DemoRoute[] = [
  {
    method: "GET",
    pattern: /^\/api\/webchat\/v2\/outbound\/preferences$/,
    handle: () => ({ json: outboundPreferences() }),
  },
  {
    method: "POST",
    pattern: /^\/api\/webchat\/v2\/outbound\/preferences$/,
    handle: (req) => ({ json: setFinalReplyTarget(req.body?.final_reply_target_id) }),
  },
  {
    method: "GET",
    pattern: /^\/api\/webchat\/v2\/outbound\/targets$/,
    handle: () => ({ json: listDeliveryTargets() }),
  },
];

/* ── Workspace file browser (/fs/*) ────────────────────────────────── */

const fsRoutes: DemoRoute[] = [
  {
    method: "GET",
    pattern: /^\/api\/webchat\/v2\/fs\/mounts$/,
    handle: () => ({ json: { mounts: fsMounts } }),
  },
  {
    method: "GET",
    pattern: /^\/api\/webchat\/v2\/fs\/list$/,
    handle: (req) => {
      const mount = req.url.searchParams.get("mount") || "";
      const path = req.url.searchParams.get("path") || "";
      return { json: { entries: listFsEntries(mount, path) } };
    },
  },
  {
    method: "GET",
    pattern: /^\/api\/webchat\/v2\/fs\/stat$/,
    handle: (req) => {
      const mount = req.url.searchParams.get("mount") || "";
      const path = req.url.searchParams.get("path") || "";
      const stat = statFsEntry(mount, path);
      return stat ? { json: { stat } } : NOT_FOUND;
    },
  },
  {
    method: "GET",
    pattern: /^\/api\/webchat\/v2\/fs\/content$/,
    handle: (req) => {
      const mount = req.url.searchParams.get("mount") || "";
      const path = req.url.searchParams.get("path") || "";
      const file = readFsContent(mount, path);
      if (!file) return NOT_FOUND;
      return { text: file.content, contentType: file.contentType };
    },
  },
];

export const workRoutes: DemoRoute[] = [
  ...projectRoutes,
  ...missionRoutes,
  ...jobRoutes,
  ...routineRoutes,
  ...automationRoutes,
  ...outboundRoutes,
  ...fsRoutes,
];
