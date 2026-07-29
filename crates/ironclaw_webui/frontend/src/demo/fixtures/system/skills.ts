// Skills fixtures for the settings Skills tab: a mix of user-authored,
// learned, imported, system, and workspace skills so every badge/action the
// SkillCard can render is on screen.

import { DAY, HOUR, iso } from "./helpers";

export type SkillRecord = {
  name: string;
  description: string;
  trust_level: string;
  source_kind: "user" | "system" | "workspace";
  version?: string;
  keywords?: string[];
  usage_hint?: string;
  setup_hint?: string;
  can_edit: boolean;
  can_delete: boolean;
  auto_activate: boolean;
  has_requirements?: boolean;
  has_scripts?: boolean;
  install_source_url?: string;
  updated_at: string;
};

export const skills: SkillRecord[] = [
  {
    name: "release-notes",
    description: "Drafts grouped release notes from merged PRs since the last tag.",
    trust_level: "trusted",
    source_kind: "user",
    version: "1.2.0",
    keywords: ["release", "changelog", "notes"],
    usage_hint: "Mention a tag range (e.g. v0.8..v0.9) for tighter grouping.",
    can_edit: true,
    can_delete: true,
    auto_activate: true,
    updated_at: iso(3 * DAY),
  },
  {
    name: "pr-triage",
    description:
      "Learned from 14 review threads: labels, assigns, and drafts a first-pass review comment for incoming PRs.",
    trust_level: "learned",
    source_kind: "user",
    version: "0.3.1",
    keywords: ["pull request", "review", "triage"],
    can_edit: true,
    can_delete: true,
    auto_activate: false,
    updated_at: iso(26 * HOUR),
  },
  {
    name: "sandbox-debugging",
    description:
      "Learned playbook for stuck sandbox jobs: inspect the job, tail the failing stage, propose a pin or timeout fix.",
    trust_level: "learned",
    source_kind: "user",
    keywords: ["sandbox", "timeout", "job"],
    has_scripts: true,
    can_edit: true,
    can_delete: true,
    auto_activate: true,
    updated_at: iso(5 * HOUR),
  },
  {
    name: "sql-analyst",
    description: "Writes and sanity-checks analytical SQL against the warehouse schema.",
    trust_level: "trusted",
    source_kind: "user",
    version: "2.0.4",
    keywords: ["sql", "warehouse", "analytics"],
    has_requirements: true,
    install_source_url: "https://github.com/nearai/skill-sql-analyst",
    can_edit: true,
    can_delete: true,
    auto_activate: true,
    updated_at: iso(9 * DAY),
  },
  {
    name: "incident-response",
    description:
      "Bundled runbook: correlate alerts, open a tracking thread, and page the on-call rotation.",
    trust_level: "trusted",
    source_kind: "system",
    version: "1.0.0",
    keywords: ["incident", "alert", "on-call"],
    can_edit: false,
    can_delete: false,
    auto_activate: true,
    updated_at: iso(21 * DAY),
  },
  {
    name: "brand-voice",
    description: "Workspace style guide for customer-facing copy and announcements.",
    trust_level: "trusted",
    source_kind: "workspace",
    keywords: ["copy", "tone", "announcement"],
    setup_hint: "Applies to outbound Slack + email drafts only.",
    can_edit: false,
    can_delete: false,
    auto_activate: true,
    updated_at: iso(12 * DAY),
  },
];

export let autoActivateLearned = true;

export function setAutoActivateLearned(enabled: boolean) {
  autoActivateLearned = enabled;
}

const skillContent = new Map<string, string>([
  [
    "release-notes",
    "# release-notes\n\nDraft release notes from merged PRs since the previous tag.\n\n## Steps\n1. `github.list_merged_prs` since the last tag.\n2. Group by area (webui, engine, sandbox, docs).\n3. Call out breaking changes in their own section.\n4. Write the draft to `notes/release-<version>.md`.\n",
  ],
  [
    "pr-triage",
    "# pr-triage (learned)\n\nLearned from 14 review threads.\n\n- Label by touched paths (webui/, engine/, sandbox/).\n- Assign the area owner from CODEOWNERS.\n- Draft a first-pass review comment; never approve automatically.\n",
  ],
  [
    "sandbox-debugging",
    "# sandbox-debugging (learned)\n\n1. `jobs.inspect` the failing job.\n2. Tail the failing stage with `logs.query`.\n3. Prefer pinning flaky git dependencies to tarballs over raising timeouts.\n",
  ],
  [
    "sql-analyst",
    "# sql-analyst\n\nWrite warehouse SQL with explicit column lists, verify with `EXPLAIN`, and cap exploratory queries with `LIMIT 100`.\n",
  ],
]);

export function getSkillContent(name: string): string {
  return skillContent.get(name) || `# ${name}\n\n(No editable content for this skill.)\n`;
}

export function setSkillContent(name: string, content: string) {
  skillContent.set(name, content);
  const skill = skills.find((entry) => entry.name === name);
  if (skill) skill.updated_at = new Date().toISOString();
}

export function findSkill(name: string): SkillRecord | undefined {
  return skills.find((entry) => entry.name === name);
}

export function removeSkill(name: string): boolean {
  const index = skills.findIndex((entry) => entry.name === name);
  if (index < 0) return false;
  skills.splice(index, 1);
  skillContent.delete(name);
  return true;
}

export function installSkill(body: Record<string, unknown>) {
  const fromUrl = typeof body.url === "string" && body.url ? body.url : null;
  const name =
    typeof body.name === "string" && body.name
      ? body.name
      : fromUrl
        ? fromUrl.split("/").filter(Boolean).slice(-1)[0].replace(/\.md$/, "")
        : "new-skill";
  if (findSkill(name)) return name;
  skills.unshift({
    name,
    description:
      typeof body.description === "string" && body.description
        ? body.description
        : "Installed during this demo session.",
    trust_level: "trusted",
    source_kind: "user",
    keywords: [],
    can_edit: true,
    can_delete: true,
    auto_activate: true,
    ...(fromUrl ? { install_source_url: fromUrl } : {}),
    updated_at: new Date().toISOString(),
  });
  if (typeof body.content === "string" && body.content) {
    skillContent.set(name, body.content);
  }
  return name;
}
