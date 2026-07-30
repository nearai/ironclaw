// DEMO job fixtures for `/api/jobs/*`.
//
// Shapes follow the Jobs page consumers: the list is `{ jobs }`, the summary
// strip reads `{ total, pending, in_progress, completed, failed, stuck }`,
// detail is the bare job object (`useJobDetail` uses the response directly),
// events are `{ events: [{ event_type, data, created_at }] }`, and the files
// tab lists `{ entries: [{ name, path, is_dir }] }` / reads `{ content }`.
//
// "job-7f3a" is load-bearing: the chat thread "Why did the sandbox job time
// out?" (routes/core.ts) tells the story of a 30-minute timeout during the
// build stage, so its transitions, events, and files must match.

import { DAY, HOUR, MINUTE, iso } from "./clock";

type JobTransition = {
  from: string;
  to: string;
  timestamp: string;
  reason?: string;
};

export type DemoJob = {
  id: string;
  title: string;
  state: string;
  job_kind: "sandbox" | "agent";
  job_mode: string | null;
  description: string | null;
  created_at: string;
  started_at: string | null;
  completed_at: string | null;
  elapsed_secs: number | null;
  can_restart: boolean;
  can_prompt: boolean;
  project_dir: string | null;
  browse_url: string | null;
  transitions: JobTransition[];
};

type JobEvent = {
  id: string;
  event_type: string;
  data: Record<string, unknown>;
  created_at: string;
};

type JobFileNode = {
  path: string;
  is_dir: boolean;
  content?: string;
};

export const jobs: DemoJob[] = [
  {
    id: "job-7f3a",
    title: "Sandbox build: ironclaw v0.9 release candidate",
    state: "timed_out",
    job_kind: "sandbox",
    job_mode: "acp:workspace",
    description:
      "Build the v0.9 release candidate in a clean sandbox: `cargo build --release` plus the frontend bundle, then attach artifacts.\n\n**Outcome:** the build stage stalled resolving a git dependency behind the egress proxy and the 30m job budget ran out. See the triage thread for the tarball-pin fix.",
    created_at: iso(4 * HOUR + 18 * MINUTE),
    started_at: iso(4 * HOUR + 10 * MINUTE),
    completed_at: iso(3 * HOUR + 40 * MINUTE),
    elapsed_secs: 1800,
    can_restart: true,
    can_prompt: false,
    project_dir: "/sandbox/job-7f3a/project",
    browse_url: null,
    transitions: [
      {
        from: "pending",
        to: "in_progress",
        timestamp: iso(4 * HOUR + 10 * MINUTE),
        reason: "Sandbox worker claimed the job; install stage started",
      },
      {
        from: "in_progress",
        to: "timed_out",
        timestamp: iso(3 * HOUR + 40 * MINUTE),
        reason:
          "30m job budget exhausted during the build stage (git dependency fetch stalled behind the egress proxy)",
      },
    ],
  },
  {
    id: "job-91c2",
    title: "Generate API docs for @ironclaw/ui",
    state: "in_progress",
    job_kind: "sandbox",
    job_mode: "acp:workspace",
    description:
      "Extract prop tables and usage examples from `packages/ui/src` and render the component reference pages for the docs site.",
    created_at: iso(24 * MINUTE),
    started_at: iso(21 * MINUTE),
    completed_at: null,
    elapsed_secs: null,
    can_restart: false,
    can_prompt: true,
    project_dir: "/sandbox/job-91c2/project",
    browse_url: null,
    transitions: [
      {
        from: "pending",
        to: "in_progress",
        timestamp: iso(21 * MINUTE),
        reason: "Sandbox worker claimed the job",
      },
    ],
  },
  {
    id: "job-b04d",
    title: "Import v0.8 changelog into docs site",
    state: "completed",
    job_kind: "sandbox",
    job_mode: "acp:workspace",
    description:
      "Convert the v0.8 changelog to MDX, add anchors per area, and open a docs-site PR.",
    created_at: iso(7 * HOUR),
    started_at: iso(7 * HOUR - 2 * MINUTE),
    completed_at: iso(7 * HOUR - 2 * MINUTE - 342_000),
    elapsed_secs: 342,
    can_restart: false,
    can_prompt: false,
    project_dir: "/sandbox/job-b04d/project",
    browse_url: null,
    transitions: [
      {
        from: "pending",
        to: "in_progress",
        timestamp: iso(7 * HOUR - 2 * MINUTE),
        reason: "Sandbox worker claimed the job",
      },
      {
        from: "in_progress",
        to: "completed",
        timestamp: iso(7 * HOUR - 2 * MINUTE - 342_000),
        reason: "Exited 0; PR docs-site#212 opened",
      },
    ],
  },
  {
    id: "job-52aa",
    title: "Nightly dependency audit",
    state: "completed",
    job_kind: "agent",
    job_mode: null,
    description:
      "Run `cargo audit` and `pnpm audit`, dedupe advisories against the allowlist, and file issues for anything new.",
    created_at: iso(11 * HOUR),
    started_at: iso(11 * HOUR - MINUTE),
    completed_at: iso(11 * HOUR - MINUTE - 128_000),
    elapsed_secs: 128,
    can_restart: false,
    can_prompt: false,
    project_dir: null,
    browse_url: null,
    transitions: [
      {
        from: "pending",
        to: "in_progress",
        timestamp: iso(11 * HOUR - MINUTE),
      },
      {
        from: "in_progress",
        to: "completed",
        timestamp: iso(11 * HOUR - MINUTE - 128_000),
        reason: "No new advisories; 2 known, allowlisted",
      },
    ],
  },
  {
    id: "job-e6f1",
    title: "Publish docs preview deployment",
    state: "failed",
    job_kind: "sandbox",
    job_mode: "acp:workspace",
    description:
      "Build the docs site from the release-notes branch and publish a preview deployment for review.",
    created_at: iso(2 * HOUR + 12 * MINUTE),
    started_at: iso(2 * HOUR + 10 * MINUTE),
    completed_at: iso(2 * HOUR + 4 * MINUTE),
    elapsed_secs: 360,
    can_restart: true,
    can_prompt: false,
    project_dir: "/sandbox/job-e6f1/project",
    browse_url: null,
    transitions: [
      {
        from: "pending",
        to: "in_progress",
        timestamp: iso(2 * HOUR + 10 * MINUTE),
        reason: "Sandbox worker claimed the job",
      },
      {
        from: "in_progress",
        to: "failed",
        timestamp: iso(2 * HOUR + 4 * MINUTE),
        reason: "npm publish token expired (E401) — rotate the deploy credential",
      },
    ],
  },
  {
    id: "job-c3d9",
    title: "Rebuild workspace search index",
    state: "pending",
    job_kind: "agent",
    job_mode: null,
    description:
      "Re-embed workspace notes and memory files after the embedding model upgrade.",
    created_at: iso(6 * MINUTE),
    started_at: null,
    completed_at: null,
    elapsed_secs: null,
    can_restart: false,
    can_prompt: false,
    project_dir: null,
    browse_url: null,
    transitions: [],
  },
  {
    id: "job-a1b8",
    title: "Migrate memory embeddings to v3 schema",
    state: "stuck",
    job_kind: "agent",
    job_mode: null,
    description:
      "Batch-migrate stored embeddings to the v3 schema. The job checkpointed at 71% and has not advanced in over an hour — likely waiting on a lock held by the search-index rebuild.",
    created_at: iso(3 * HOUR + 30 * MINUTE),
    started_at: iso(3 * HOUR + 28 * MINUTE),
    completed_at: null,
    elapsed_secs: null,
    can_restart: false,
    can_prompt: true,
    project_dir: null,
    browse_url: null,
    transitions: [
      {
        from: "pending",
        to: "in_progress",
        timestamp: iso(3 * HOUR + 28 * MINUTE),
      },
      {
        from: "in_progress",
        to: "stuck",
        timestamp: iso(1 * HOUR + 12 * MINUTE),
        reason: "No checkpoint progress for 65m (last checkpoint: batch 71/100)",
      },
    ],
  },
  {
    id: "job-77de",
    title: "Backfill telemetry rollups (June)",
    state: "interrupted",
    job_kind: "sandbox",
    job_mode: "acp:workspace",
    description: "Recompute June telemetry rollups after the aggregation fix.",
    created_at: iso(1 * DAY + 3 * HOUR),
    started_at: iso(1 * DAY + 3 * HOUR - 2 * MINUTE),
    completed_at: iso(1 * DAY + 2 * HOUR),
    elapsed_secs: 3480,
    can_restart: true,
    can_prompt: false,
    project_dir: "/sandbox/job-77de/project",
    browse_url: null,
    transitions: [
      {
        from: "pending",
        to: "in_progress",
        timestamp: iso(1 * DAY + 3 * HOUR - 2 * MINUTE),
      },
      {
        from: "in_progress",
        to: "interrupted",
        timestamp: iso(1 * DAY + 2 * HOUR),
        reason: "Worker restarted during the nightly deploy window",
      },
    ],
  },
  {
    id: "job-4be7",
    title: "Smoke-test the v0.9 install script",
    state: "completed",
    job_kind: "sandbox",
    job_mode: "acp:workspace",
    description:
      "Run the install script on a clean container and assert the `--filter ironclaw-webui-v2-frontend` path works.",
    created_at: iso(26 * HOUR),
    started_at: iso(26 * HOUR - MINUTE),
    completed_at: iso(26 * HOUR - MINUTE - 204_000),
    elapsed_secs: 204,
    can_restart: false,
    can_prompt: false,
    project_dir: "/sandbox/job-4be7/project",
    browse_url: null,
    transitions: [
      {
        from: "pending",
        to: "in_progress",
        timestamp: iso(26 * HOUR - MINUTE),
      },
      {
        from: "in_progress",
        to: "completed",
        timestamp: iso(26 * HOUR - MINUTE - 204_000),
        reason: "Install verified on a clean container",
      },
    ],
  },
];

/* ── Events ────────────────────────────────────────────────────────── */

let eventCounter = 0;
function event(
  eventType: string,
  data: Record<string, unknown>,
  msAgo: number
): JobEvent {
  eventCounter += 1;
  return {
    id: `job-event-${String(eventCounter).padStart(4, "0")}`,
    event_type: eventType,
    data,
    created_at: iso(msAgo),
  };
}

const eventsByJob: Record<string, JobEvent[]> = {
  "job-7f3a": [
    event("status", { status: "Sandbox provisioned (image ironclaw-build:0.9)" }, 4 * HOUR + 10 * MINUTE),
    event(
      "tool_use",
      { tool_name: "shell.run", input: "cargo fetch --locked" },
      4 * HOUR + 9 * MINUTE
    ),
    event(
      "tool_result",
      {
        tool_name: "shell.run",
        output: "Fetched 412 crates in 96s. 1 git dependency pending: ironclaw-vendored-parser",
      },
      4 * HOUR + 7 * MINUTE
    ),
    event(
      "message",
      {
        role: "assistant",
        content:
          "Install stage done; entering the build stage. The git dependency ironclaw-vendored-parser is still resolving through the egress proxy.",
      },
      4 * HOUR + 6 * MINUTE
    ),
    event(
      "tool_use",
      { tool_name: "shell.run", input: "cargo build --release --workspace" },
      4 * HOUR + 5 * MINUTE
    ),
    event(
      "status",
      { status: "Build stage: git fetch retry 3/5 for ironclaw-vendored-parser (proxy idle-drop)" },
      3 * HOUR + 52 * MINUTE
    ),
    event(
      "result",
      {
        message:
          "Job timed out after 1800s. Last transition: install -> build. The dependency fetch retried silently until the job budget ran out.",
      },
      3 * HOUR + 40 * MINUTE
    ),
  ],
  "job-91c2": [
    event("status", { status: "Sandbox provisioned (image ironclaw-docs:latest)" }, 21 * MINUTE),
    event(
      "tool_use",
      { tool_name: "shell.run", input: "corepack pnpm docgen packages/ui/src" },
      19 * MINUTE
    ),
    event(
      "tool_result",
      { tool_name: "shell.run", output: "34 components scanned; 34 prop tables extracted" },
      12 * MINUTE
    ),
    event(
      "message",
      {
        role: "assistant",
        content:
          "Prop tables extracted for all 34 components. Rendering MDX pages now — Button, Card, and Badge get hand-written usage examples.",
      },
      9 * MINUTE
    ),
  ],
  "job-e6f1": [
    event("status", { status: "Building docs site from branch release-notes-v0.9" }, 2 * HOUR + 9 * MINUTE),
    event(
      "tool_result",
      { tool_name: "shell.run", output: "Build OK (2.4 MB). Publishing preview…" },
      2 * HOUR + 5 * MINUTE
    ),
    event(
      "result",
      { message: "npm publish failed: E401 Unauthorized — deploy token expired." },
      2 * HOUR + 4 * MINUTE
    ),
  ],
  "job-a1b8": [
    event("status", { status: "Checkpoint: batch 71/100 (218k embeddings migrated)" }, 1 * HOUR + 17 * MINUTE),
    event(
      "status",
      { status: "No progress for 65m — waiting on table lock held by search-index rebuild" },
      1 * HOUR + 12 * MINUTE
    ),
  ],
};

/* ── Files (sandbox project trees) ─────────────────────────────────── */

const filesByJob: Record<string, JobFileNode[]> = {
  "job-7f3a": [
    { path: "src", is_dir: true },
    {
      path: "src/main.rs",
      is_dir: false,
      content:
        'fn main() {\n    // v0.9 release-candidate entry point.\n    ironclaw::boot::run(ironclaw::boot::Profile::Release)\n        .expect("release boot");\n}\n',
    },
    {
      path: "src/vendored.rs",
      is_dir: false,
      content:
        "//! Pin point for the fix suggested in triage: replace the git\n//! dependency with a vendored tarball so the sandbox build cannot\n//! stall on a proxied git fetch.\n\npub const VENDORED_PARSER_TARBALL: &str =\n    \"https://artifacts.nearai.dev/ironclaw-vendored-parser-0.4.2.tar.gz\";\n",
    },
    {
      path: "Cargo.toml",
      is_dir: false,
      content:
        '[package]\nname = "ironclaw-rc"\nversion = "0.9.0-rc.1"\nedition = "2021"\n\n[dependencies]\n# TODO(triage thread): pin to the tarball — this git dep stalled the\n# sandbox build behind the egress proxy and burned the 30m job budget.\nironclaw-vendored-parser = { git = "https://git.internal/parsers.git", tag = "v0.4.1" }\n',
    },
    {
      path: "build.log",
      is_dir: false,
      content:
        "[00:00:04] install: fetching 412 crates (locked)\n[00:01:40] install: done\n[00:01:41] build: cargo build --release --workspace\n[00:02:12] build: waiting on git dep ironclaw-vendored-parser (attempt 1)\n[00:07:44] build: git fetch retry 2/5 (connection reset by proxy)\n[00:13:20] build: git fetch retry 3/5 (connection reset by proxy)\n[00:21:03] build: git fetch retry 4/5 (connection reset by proxy)\n[00:29:58] build: git fetch retry 5/5 (connection reset by proxy)\n[00:30:00] FATAL: job budget exhausted (1800s) during build stage\n",
    },
  ],
  "job-91c2": [
    { path: "docs", is_dir: true },
    {
      path: "docs/button.mdx",
      is_dir: false,
      content:
        "# Button\n\nPrimary action component from `@ironclaw/ui`.\n\n| Prop | Type | Default |\n| --- | --- | --- |\n| `variant` | `primary \\| secondary \\| ghost \\| danger` | `primary` |\n| `size` | `sm \\| md \\| icon-sm` | `md` |\n",
    },
    {
      path: "docgen.config.json",
      is_dir: false,
      content:
        '{\n  "source": "packages/ui/src",\n  "out": "docs/",\n  "examples": ["Button", "Card", "Badge"]\n}\n',
    },
  ],
};

/* ── Accessors + mutations ─────────────────────────────────────────── */

export function findJob(jobId: string): DemoJob | undefined {
  return jobs.find((job) => job.id === jobId);
}

export function jobsSummary() {
  const summary = {
    total: jobs.length,
    pending: 0,
    in_progress: 0,
    completed: 0,
    failed: 0,
    stuck: 0,
  };
  for (const job of jobs) {
    if (job.state === "pending") summary.pending += 1;
    else if (job.state === "in_progress") summary.in_progress += 1;
    else if (job.state === "completed") summary.completed += 1;
    else if (job.state === "failed") summary.failed += 1;
    else if (job.state === "stuck") summary.stuck += 1;
  }
  return summary;
}

export function jobEvents(jobId: string): JobEvent[] {
  return eventsByJob[jobId] || [];
}

export function listJobFiles(jobId: string, path: string) {
  const nodes = filesByJob[jobId] || [];
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
      is_dir: node.is_dir,
    }));
}

export function readJobFile(jobId: string, path: string): string | null {
  const node = (filesByJob[jobId] || []).find(
    (candidate) => candidate.path === path && !candidate.is_dir
  );
  return node ? node.content || "" : null;
}

export function cancelJob(jobId: string): boolean {
  const job = findJob(jobId);
  if (!job) return false;
  if (job.state !== "pending" && job.state !== "in_progress") return true;
  const previous = job.state;
  job.state = "cancelled";
  job.completed_at = new Date().toISOString();
  job.transitions.push({
    from: previous,
    to: "cancelled",
    timestamp: new Date().toISOString(),
    reason: "Cancelled by operator",
  });
  return true;
}

let restartCounter = 0;

export function restartJob(jobId: string): string | null {
  const source = findJob(jobId);
  if (!source) return null;
  restartCounter += 1;
  const newId = `job-r${String(restartCounter).padStart(3, "0")}`;
  jobs.unshift({
    ...source,
    id: newId,
    state: "pending",
    created_at: new Date().toISOString(),
    started_at: null,
    completed_at: null,
    elapsed_secs: null,
    can_restart: false,
    transitions: [],
  });
  if (filesByJob[jobId]) filesByJob[newId] = filesByJob[jobId];
  return newId;
}
