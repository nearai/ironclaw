// DEMO filesystem fixtures for the read-only workspace browser
// (`/api/webchat/v2/fs/*`).
//
// Shapes follow `src/pages/workspace/lib/workspace-api.ts`:
//   mounts  -> { mounts: [{ mount, label }] }
//   list    -> { entries: [{ name, path, kind }] }   (path is mount-relative)
//   stat    -> { stat: { kind, mime_type, size_bytes } }
//   content -> raw bytes (served via the DemoResponse `text` field)
//
// `notes/release-v0.9.md` matches the chat fixture where the agent wrote the
// release-notes draft; `assets/dashboard.png` is oversized on purpose so the
// viewer takes the binary/download path instead of inlining an image.

import { dateStamp } from "./clock";

type FsNode = {
  path: string;
  kind: "file" | "directory";
  mime_type?: string;
  size_bytes?: number;
  content?: string;
};

const RELEASE_NOTES = `# IronClaw v0.9 release notes (draft)

_Drafted from the 41 PRs merged since the v0.8 tag. Grouped by area;
breaking changes are called out at the end. Tracking PR: #6841._

## Highlights

- **WebUI** — design system extracted into \`@ironclaw/ui\`; workspace file browser; notification center.
- **Engine** — turn-failure guidance; benchmark-mode system-prompt addendum.
- **Sandbox** — \`RuntimeKind::Sandbox\` lane with credential reuse.

## WebUI

- Extracted the design system into the \`@ironclaw/ui\` workspace package so
  surfaces share one component vocabulary (Buttons, Panels, StatusPills).
- New workspace file browser: browse the workspace and memory mounts,
  preview text and markdown inline, download binaries.
- Notification center with per-surface filters and quiet hours.
- Automations page: run-history summary chips and an active-hold pill that
  explains why a due trigger is being skipped.

## Engine

- Turn failures now carry operator guidance: the failing tool, the error
  family, and a suggested next step land in the thread timeline.
- Opt-in \`BENCHMARKING_MODE\` system-prompt addendum for unattended evals.

## Sandbox

- New \`RuntimeKind::Sandbox\` execution lane with credential reuse — gated
  credentials granted once per thread now flow to sandbox jobs without a
  second prompt.
- Job timeouts now record the stage that exhausted the budget (see the
  job-7f3a triage: a git dependency fetch stalled behind the egress proxy).

## Docs

- Component reference regenerated from prop tables.
- Install guide updated for the new frontend filter (see breaking changes).

## Breaking changes

- \`build.rs\` now installs the SPA with
  \`--filter ironclaw-webui-v2-frontend\`; custom build scripts that invoked
  the old package name must update.

## Upgrade notes

1. Rotate sandbox deploy tokens before enabling credential reuse.
2. Re-run \`corepack pnpm install\` after pulling — the \`@ironclaw/ui\`
   package is new in the workspace.
`;

const SANDBOX_TRIAGE_NOTE = `# Sandbox job-7f3a triage

**Symptom:** job timed out after 30m during the build stage.

**Root cause:** the build resolved \`ironclaw-vendored-parser\` over a git
remote that sits behind the egress proxy. The proxy drops idle connections,
the fetch retried silently (5 attempts), and the 1800s job budget ran out.

**Fixes considered:**

1. Pin the dependency to a vendored tarball (preferred — deterministic).
2. Raise \`IRONCLAW_JOB_TIMEOUT\` for the routine (masks the stall).

**Decision:** tarball pin. Patch staged in \`src/vendored.rs\` on the job's
workspace; PR to follow after the v0.9 notes land.
`;

const README = `# Atlas workspace

Working area for the Atlas release-engineering project.

- \`notes/\` — drafts the agent writes (release notes, triage notes).
- \`src/\` — helper scripts used by release missions.
- \`assets/\` — rendered charts and screenshots (binary, download-only).
- \`config.json\` — mission defaults (repo, tag range, notes path).
`;

const CONFIG_JSON = `{
  "repository": "nearai/ironclaw",
  "release": {
    "base_tag": "v0.8",
    "target": "v0.9",
    "notes_path": "notes/release-v0.9.md",
    "tracking_pr": 6841
  },
  "sandbox": {
    "job_timeout_secs": 1800,
    "image": "ironclaw-build:0.9"
  }
}
`;

const COLLECT_PRS_TS = `// Collect merged PRs since a tag and bucket them by area label.
// Used by the release-notes mission before drafting notes/release-v0.9.md.

type MergedPr = { number: number; title: string; labels: string[] };

const AREAS = ["webui", "engine", "sandbox", "docs"] as const;

export function bucketByArea(prs: MergedPr[]) {
  const buckets: Record<string, MergedPr[]> = {};
  for (const area of AREAS) buckets[area] = [];
  for (const pr of prs) {
    const area = AREAS.find((candidate) => pr.labels.includes(candidate)) || "docs";
    buckets[area].push(pr);
  }
  return buckets;
}
`;

const RENDER_NOTES_TS = `// Render bucketed PRs into the release-notes markdown skeleton.

import { bucketByArea } from "./collect-prs";

export function renderNotes(prs: Parameters<typeof bucketByArea>[0]) {
  const buckets = bucketByArea(prs);
  const sections = Object.entries(buckets)
    .filter(([, entries]) => entries.length > 0)
    .map(([area, entries]) => {
      const items = entries.map((pr) => \`- \${pr.title} (#\${pr.number})\`);
      return [\`## \${area}\`, ...items].join("\\n");
    });
  return sections.join("\\n\\n");
}
`;

const MEMORY_MD = `# Long-term memory

## Operator preferences

- Release notes grouped by area; breaking changes get their own section.
- Standup summaries lead with failures, then routine activity.
- Alerts route to Slack #ops once the extension is authorized.

## Environment facts

- Sandbox egress goes through a proxy that drops idle connections —
  prefer tarball pins over git dependencies for sandbox builds.
- The backup runner has ~40 GB of disk; snapshots average 8 GB.
`;

const FACTS_JSON = `{
  "facts": [
    { "key": "release.cadence", "value": "minor every 6 weeks", "confidence": 0.92 },
    { "key": "ops.alert_channel", "value": "#ops (pending Slack auth)", "confidence": 0.88 },
    { "key": "sandbox.proxy_idle_drop", "value": true, "confidence": 0.97 },
    { "key": "backup.runner_disk_gb", "value": 40, "confidence": 0.85 }
  ]
}
`;

function textSize(content: string): number {
  return new TextEncoder().encode(content).length;
}

function textFile(path: string, content: string, mime = "text/markdown"): FsNode {
  return { path, kind: "file", mime_type: mime, size_bytes: textSize(content), content };
}

function dir(path: string): FsNode {
  return { path, kind: "directory" };
}

const trees: Record<string, FsNode[]> = {
  workspace: [
    textFile("README.md", README),
    textFile("config.json", CONFIG_JSON, "application/json"),
    dir("notes"),
    textFile("notes/release-v0.9.md", RELEASE_NOTES),
    textFile("notes/sandbox-triage.md", SANDBOX_TRIAGE_NOTE),
    textFile(`notes/standup-${dateStamp(1)}.md`,
      `# Standup notes — ${dateStamp(1)}\n\n- 14 automation runs, 13 ok, 1 failed (nightly-backup: disk pressure).\n- docs-sync drafted 2 doc updates.\n- 41 tool invocations across 6 threads; no gates raised.\n`),
    dir("src"),
    textFile("src/collect-prs.ts", COLLECT_PRS_TS, "text/x-typescript"),
    textFile("src/render-notes.ts", RENDER_NOTES_TS, "text/x-typescript"),
    dir("assets"),
    {
      path: "assets/dashboard.png",
      kind: "file",
      mime_type: "image/png",
      // Above the viewer's 8 MB inline-image cap on purpose: this entry
      // exercises the binary/download presentation instead of an <img>.
      size_bytes: 12_582_912,
      content: "PNG render of the release burndown dashboard (demo placeholder).",
    },
  ],
  memory: [
    textFile("MEMORY.md", MEMORY_MD),
    textFile("facts.json", FACTS_JSON, "application/json"),
    dir("daily"),
    textFile(`daily/${dateStamp(1)}.md`,
      `# ${dateStamp(1)}\n\n- Investigated job-7f3a timeout; root cause: proxied git fetch.\n- nightly-backup failed (disk); freed 12 GB on the runner.\n`),
    textFile(`daily/${dateStamp(0)}.md`,
      `# ${dateStamp(0)}\n\n- Opened PR #6841 with the v0.9 release notes.\n- CI green on main as of this morning.\n`),
  ],
};

export const fsMounts = [
  { mount: "workspace", label: "Workspace" },
  { mount: "memory", label: "Memory" },
];

export function listFsEntries(mount: string, path: string) {
  const nodes = trees[mount] || [];
  const prefix = path ? `${path}/` : "";
  return nodes
    .filter((node) => {
      if (!node.path.startsWith(prefix)) return false;
      const rest = node.path.slice(prefix.length);
      return rest.length > 0 && !rest.includes("/");
    })
    .map((node) => ({
      name: node.path.slice(prefix.length),
      path: node.path,
      kind: node.kind,
    }));
}

export function statFsEntry(mount: string, path: string) {
  const node = (trees[mount] || []).find((candidate) => candidate.path === path);
  if (!node) return null;
  return {
    kind: node.kind,
    mime_type: node.mime_type || null,
    size_bytes: node.kind === "file" ? node.size_bytes || 0 : 0,
  };
}

export function readFsContent(mount: string, path: string) {
  const node = (trees[mount] || []).find(
    (candidate) => candidate.path === path && candidate.kind === "file"
  );
  if (!node) return null;
  return {
    content: node.content || "",
    contentType: node.mime_type || "text/plain; charset=utf-8",
  };
}
