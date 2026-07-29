// DEMO projects + membership ACL fixtures.
//
// Wire shapes mirror `RebornProjectInfo` / `RebornProjectMemberInfo` from
// `ironclaw_product::reborn_services::projects`. "project-atlas" must exist:
// the chat threads seeded in `routes/core.ts` carry `project_id:
// "project-atlas"`, and a project named "default" renders the General
// workspace card on the Projects page.

import { DAY, HOUR, MINUTE, iso } from "./clock";

export type DemoProject = {
  project_id: string;
  name: string;
  description: string;
  icon: string | null;
  color: string | null;
  metadata: Record<string, unknown>;
  state: "active" | "archived";
  role: "owner" | "editor" | "viewer";
  created_at: string;
  updated_at: string;
};

export type DemoProjectMember = {
  user_id: string;
  role: "owner" | "editor" | "viewer";
  status: "active" | "revoked";
  granted_by: string;
  created_at: string;
  updated_at: string;
};

export const projects: DemoProject[] = [
  {
    project_id: "project-default",
    name: "default",
    description: "General workspace for one-off conversations and quick asks.",
    icon: null,
    color: null,
    metadata: {},
    state: "active",
    role: "owner",
    created_at: iso(90 * DAY),
    updated_at: iso(26 * HOUR),
  },
  {
    project_id: "project-atlas",
    name: "Atlas",
    description:
      "Release engineering for nearai/ironclaw: changelogs, release notes, CI health, and sandbox build triage.",
    icon: "rocket",
    color: "#6366f1",
    metadata: {
      goals: ["Ship v0.9 on schedule", "Zero red CI on main", "Automate release notes"],
      repository: "nearai/ironclaw",
    },
    state: "active",
    role: "owner",
    created_at: iso(45 * DAY),
    updated_at: iso(8 * MINUTE),
  },
  {
    project_id: "project-nimbus",
    name: "Nimbus",
    description:
      "Support inbox copilot: triages incoming tickets, drafts replies, and escalates anything with a crash signature.",
    icon: "inbox",
    color: "#0ea5e9",
    metadata: {
      goals: ["First response under 4h", "Escalate crashes within 30m"],
    },
    state: "active",
    role: "editor",
    created_at: iso(30 * DAY),
    updated_at: iso(5 * HOUR),
  },
  {
    project_id: "project-orion",
    name: "Orion",
    description:
      "Growth experiments archive — Q2 landing-page and onboarding tests. Kept for reference; no active missions.",
    icon: "telescope",
    color: "#a855f7",
    metadata: { goals: ["Archive of Q2 experiments"] },
    state: "archived",
    role: "owner",
    created_at: iso(120 * DAY),
    updated_at: iso(21 * DAY),
  },
];

const membersByProject: Record<string, DemoProjectMember[]> = {
  "project-default": [
    member("demo-operator", "owner", "demo-operator", 90 * DAY),
  ],
  "project-atlas": [
    member("demo-operator", "owner", "demo-operator", 45 * DAY),
    member("mira.chen", "editor", "demo-operator", 40 * DAY),
    member("sam.okafor", "editor", "demo-operator", 33 * DAY),
    member("priya.nair", "viewer", "mira.chen", 12 * DAY),
  ],
  "project-nimbus": [
    member("lena.fischer", "owner", "lena.fischer", 30 * DAY),
    member("demo-operator", "editor", "lena.fischer", 28 * DAY),
    member("sam.okafor", "viewer", "lena.fischer", 9 * DAY),
  ],
  "project-orion": [
    member("demo-operator", "owner", "demo-operator", 120 * DAY),
  ],
};

function member(
  userId: string,
  role: DemoProjectMember["role"],
  grantedBy: string,
  createdMsAgo: number
): DemoProjectMember {
  return {
    user_id: userId,
    role,
    status: "active",
    granted_by: grantedBy,
    created_at: iso(createdMsAgo),
    updated_at: iso(createdMsAgo),
  };
}

export function findProject(projectId: string): DemoProject | undefined {
  return projects.find((project) => project.project_id === projectId);
}

export function projectMembers(projectId: string): DemoProjectMember[] {
  const existing = membersByProject[projectId];
  if (existing) return existing;
  const created: DemoProjectMember[] = [];
  membersByProject[projectId] = created;
  return created;
}

let createdProjectCounter = 0;

export function createProject(body: Record<string, unknown> | null): DemoProject {
  createdProjectCounter += 1;
  const name =
    typeof body?.name === "string" && body.name ? body.name : `New project ${createdProjectCounter}`;
  const project: DemoProject = {
    project_id: `project-demo-${String(createdProjectCounter).padStart(3, "0")}`,
    name,
    description: typeof body?.description === "string" ? body.description : "",
    icon: typeof body?.icon === "string" ? body.icon : null,
    color: typeof body?.color === "string" ? body.color : null,
    metadata:
      body?.metadata && typeof body.metadata === "object" && !Array.isArray(body.metadata)
        ? (body.metadata as Record<string, unknown>)
        : {},
    state: "active",
    role: "owner",
    created_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
  };
  projects.push(project);
  membersByProject[project.project_id] = [
    member("demo-operator", "owner", "demo-operator", 0),
  ];
  return project;
}

export function updateProject(
  projectId: string,
  body: Record<string, unknown> | null
): DemoProject | undefined {
  const project = findProject(projectId);
  if (!project) return undefined;
  if (typeof body?.name === "string" && body.name) project.name = body.name;
  if (typeof body?.description === "string") project.description = body.description;
  if (typeof body?.icon === "string") project.icon = body.icon;
  if (typeof body?.color === "string") project.color = body.color;
  if (body?.metadata && typeof body.metadata === "object" && !Array.isArray(body.metadata)) {
    project.metadata = body.metadata as Record<string, unknown>;
  }
  if (body?.state === "active" || body?.state === "archived") project.state = body.state;
  project.updated_at = new Date().toISOString();
  return project;
}

export function deleteProject(projectId: string): void {
  const index = projects.findIndex((project) => project.project_id === projectId);
  if (index >= 0) projects.splice(index, 1);
  delete membersByProject[projectId];
}
