// Core DEMO fixtures: auth/session, threads, chat timeline, message sends.
//
// The dataset is written as a small product tour: a handful of threads whose
// timelines show plain conversation, tool activity cards, and a failed tool,
// so the chat surface demonstrates every message shape it can render.

import type { DemoRoute } from "../types";
import { emitDemoThreadEvent } from "../streams";

const now = Date.now();
const MINUTE = 60_000;
const HOUR = 3_600_000;
const DAY = 86_400_000;

function iso(msAgo: number): string {
  return new Date(now - msAgo).toISOString();
}

let idCounter = 0;
function demoId(prefix: string): string {
  idCounter += 1;
  return `${prefix}-demo-${String(idCounter).padStart(4, "0")}`;
}

/* ── Session ───────────────────────────────────────────────────────── */

const SESSION = {
  tenant_id: "demo-tenant",
  user_id: "demo-operator",
  capabilities: { operator_webui_config: true },
  features: { reborn_projects: true, global_auto_approve: false },
  attachments: {
    accept: ["image/*", "text/*", "application/pdf", "application/json"],
    max_count: 6,
    max_file_bytes: 8 * 1024 * 1024,
    max_total_bytes: 24 * 1024 * 1024,
  },
};

/* ── Threads + timelines ───────────────────────────────────────────── */

type TimelineRecord = Record<string, unknown>;

function userMsg(content: string, msAgo: number, extra: TimelineRecord = {}): TimelineRecord {
  return {
    message_id: demoId("msg"),
    kind: "user",
    content,
    status: "finalized",
    created_at: iso(msAgo),
    updated_at: iso(msAgo),
    sequence: ++idCounter,
    ...extra,
  };
}

function assistantMsg(
  content: string,
  msAgo: number,
  extra: TimelineRecord = {}
): TimelineRecord {
  return {
    message_id: demoId("msg"),
    kind: "assistant",
    content,
    status: "finalized",
    created_at: iso(msAgo),
    updated_at: iso(msAgo),
    sequence: ++idCounter,
    ...extra,
  };
}

function toolCard(
  {
    title,
    subtitle,
    input,
    output,
    status = "completed",
    errorKind = null,
    msAgo,
    turnRunId,
  }: {
    title: string;
    subtitle?: string;
    input?: string;
    output?: string;
    status?: string;
    errorKind?: string | null;
    msAgo: number;
    turnRunId?: string;
  }
): TimelineRecord {
  const invocationId = demoId("inv");
  return {
    message_id: demoId("msg"),
    kind: "capability_display_preview",
    content: JSON.stringify({
      invocation_id: invocationId,
      capability_id: title,
      title,
      subtitle: subtitle || null,
      input_summary: input || null,
      output_preview: status === "completed" ? output || null : null,
      output_summary: status === "failed" ? output || null : null,
      status,
      error_kind: errorKind,
      activity_order: idCounter,
      updated_at: iso(msAgo),
      turn_run_id: turnRunId || null,
    }),
    status: "finalized",
    created_at: iso(msAgo),
    updated_at: iso(msAgo),
    sequence: ++idCounter,
    turn_run_id: turnRunId || null,
  };
}

const releaseRun = demoId("run");
const timelines = new Map<string, TimelineRecord[]>();
const threads: Record<string, unknown>[] = [];

function seedThread(
  {
    id,
    title,
    updatedMsAgo,
    projectId,
    records,
  }: {
    id: string;
    title: string;
    updatedMsAgo: number;
    projectId?: string;
    records: TimelineRecord[];
  }
) {
  threads.push({
    thread_id: id,
    title,
    created_at: iso(updatedMsAgo + 2 * HOUR),
    updated_at: iso(updatedMsAgo),
    ...(projectId ? { project_id: projectId } : {}),
  });
  timelines.set(id, records);
}

seedThread({
  id: "thread-release-notes",
  title: "Draft the v0.9 release notes",
  updatedMsAgo: 8 * MINUTE,
  projectId: "project-atlas",
  records: [
    userMsg(
      "Pull the merged PRs since the v0.8 tag and draft release notes for v0.9. Group by area and call out breaking changes.",
      52 * MINUTE
    ),
    toolCard({
      title: "github.list_merged_prs",
      subtitle: "nearai/ironclaw since v0.8",
      input: 'repo: "nearai/ironclaw", base: "main", since_tag: "v0.8"',
      output: "41 merged PRs across webui, engine, sandbox, and docs.",
      msAgo: 50 * MINUTE,
      turnRunId: releaseRun,
    }),
    toolCard({
      title: "workspace.write_file",
      subtitle: "notes/release-v0.9.md",
      input: 'path: "notes/release-v0.9.md" (4.1 KB)',
      output: "Wrote 4.1 KB to notes/release-v0.9.md",
      msAgo: 48 * MINUTE,
      turnRunId: releaseRun,
    }),
    assistantMsg(
      "Drafted the v0.9 release notes in `notes/release-v0.9.md`.\n\n**Highlights**\n\n- **WebUI**: design-system extraction into `@ironclaw/ui`, workspace file browser, notification center.\n- **Engine**: turn-failure guidance, benchmark mode addendum.\n- **Sandbox**: `RuntimeKind::Sandbox` lane with credential reuse.\n\n**Breaking changes**\n\n- `build.rs` now installs with `--filter ironclaw-webui-v2-frontend`.\n\nWant me to open a PR with the draft?",
      46 * MINUTE
    ),
    userMsg("Looks good — tighten the highlights to one line each and open the PR.", 12 * MINUTE),
    assistantMsg(
      "Tightened the highlights and opened PR #6841 with the release notes. CI is running; I'll flag anything that fails.",
      8 * MINUTE
    ),
  ],
});

seedThread({
  id: "thread-sandbox-triage",
  title: "Why did the sandbox job time out?",
  updatedMsAgo: 3 * HOUR,
  projectId: "project-atlas",
  records: [
    userMsg("Job job-7f3a timed out after 30m. Can you find out why?", 4 * HOUR),
    toolCard({
      title: "jobs.inspect",
      subtitle: "job-7f3a",
      input: 'job_id: "job-7f3a"',
      output: "State: timed_out after 1800s. Last transition: install -> build.",
      msAgo: 4 * HOUR + -2 * MINUTE,
    }),
    toolCard({
      title: "logs.query",
      subtitle: "job-7f3a build stage",
      input: 'run_id: "job-7f3a", level: "error", tail: true',
      status: "failed",
      errorKind: "timeout",
      output: "Log tail exceeded the 30s query budget while the runner was under load.",
      msAgo: 3 * HOUR + 55 * MINUTE,
    }),
    assistantMsg(
      "The build stage stalled resolving a git dependency over a proxy that drops idle connections — the fetch retried silently until the job budget ran out. Two fixes: pin the dependency to a tarball, or raise `IRONCLAW_JOB_TIMEOUT` for this routine. I'd do the tarball pin; want a patch?",
      3 * HOUR
    ),
  ],
});

seedThread({
  id: "thread-standup",
  title: "Summarize yesterday's activity",
  updatedMsAgo: 26 * HOUR,
  records: [
    userMsg("Give me a standup summary of what ran yesterday.", 27 * HOUR),
    assistantMsg(
      "**Yesterday at a glance**\n\n- 14 automation runs — 13 succeeded, 1 failed (`nightly-backup`, disk pressure on the runner).\n- 3 routines executed on schedule; `docs-sync` drafted 2 doc updates.\n- 41 tool invocations across 6 threads, no gates raised.\n\nThe backup failure is retried tonight; I freed 12 GB on the runner so it should pass.",
      26 * HOUR
    ),
  ],
});

seedThread({
  id: "thread-onboarding",
  title: "Wire up the Slack extension",
  updatedMsAgo: 2 * DAY,
  records: [
    userMsg("Help me connect Slack so alerts land in #ops.", 2 * DAY + HOUR),
    assistantMsg(
      "Slack is installed but not yet authorized. Open **Extensions → Channels → Slack** and click *Connect workspace* — I'll take it from there and route alerts to `#ops` once the OAuth handshake completes.",
      2 * DAY
    ),
  ],
});

/* ── Routes ────────────────────────────────────────────────────────── */

export const coreRoutes: DemoRoute[] = [
  {
    method: "GET",
    pattern: /^\/api\/webchat\/v2\/session$/,
    handle: () => ({ json: SESSION }),
  },
  {
    method: "GET",
    pattern: /^\/auth\/providers$/,
    handle: () => ({ json: { providers: [] } }),
  },
  {
    method: "POST",
    pattern: /^\/auth\/logout$/,
    handle: () => ({ json: {} }),
  },
  {
    method: "GET",
    pattern: /^\/api\/webchat\/v2\/threads$/,
    handle: (req) => {
      const projectId = req.url.searchParams.get("project_id");
      const needsApproval = req.url.searchParams.get("needs_approval") === "true";
      let records = threads;
      if (projectId) {
        records = records.filter((thread) => thread.project_id === projectId);
      }
      if (needsApproval) records = [];
      const sorted = [...records].sort((a, b) =>
        String(b.updated_at).localeCompare(String(a.updated_at))
      );
      return { json: { threads: sorted, next_cursor: null } };
    },
  },
  {
    method: "POST",
    pattern: /^\/api\/webchat\/v2\/threads$/,
    handle: (req) => {
      const requested = req.body?.requested_thread_id;
      const id = typeof requested === "string" && requested ? requested : demoId("thread");
      const record = {
        thread_id: id,
        title: "New conversation",
        created_at: new Date().toISOString(),
        updated_at: new Date().toISOString(),
        ...(typeof req.body?.project_id === "string"
          ? { project_id: req.body.project_id }
          : {}),
      };
      threads.unshift(record);
      timelines.set(id, []);
      return { json: { thread: record } };
    },
  },
  {
    method: "DELETE",
    pattern: /^\/api\/webchat\/v2\/threads\/([^/]+)$/,
    handle: (_req, match) => {
      const id = decodeURIComponent(match[1]);
      const index = threads.findIndex((thread) => thread.thread_id === id);
      if (index >= 0) threads.splice(index, 1);
      timelines.delete(id);
      return { json: {} };
    },
  },
  {
    method: "GET",
    pattern: /^\/api\/webchat\/v2\/threads\/([^/]+)\/timeline$/,
    handle: (_req, match) => {
      const id = decodeURIComponent(match[1]);
      return { json: { messages: timelines.get(id) || [], next_cursor: null } };
    },
  },
  {
    method: "POST",
    pattern: /^\/api\/webchat\/v2\/threads\/([^/]+)\/messages$/,
    handle: (req, match) => {
      const threadId = decodeURIComponent(match[1]);
      const content = typeof req.body?.content === "string" ? req.body.content : "";
      const runId = demoId("run");
      const messageId = demoId("msg");
      const records = timelines.get(threadId) || [];
      timelines.set(threadId, records);
      records.push({
        ...userMsg(content, 0),
        message_id: messageId,
      });

      const thread = threads.find((entry) => entry.thread_id === threadId);
      if (thread) thread.updated_at = new Date().toISOString();

      const replyText =
        "This staging build runs in **demo mode** — no agent backend is attached, " +
        "so I can't actually act on that. Everything you see (threads, runs, tool " +
        "cards, jobs, automations) is fixture data meant for walking through the UI.";

      // Play the run lifecycle over the (inert) SSE stream so the composer
      // shows the live accepted -> final-reply flow.
      setTimeout(() => {
        emitDemoThreadEvent(threadId, "accepted", {
          ack: { run_id: runId, thread_id: threadId, status: "running" },
        });
      }, 350);
      setTimeout(() => {
        records.push(
          assistantMsg(replyText, 0, { turn_run_id: runId })
        );
        emitDemoThreadEvent(threadId, "final_reply", {
          reply: {
            text: replyText,
            generated_at: new Date().toISOString(),
            turn_run_id: runId,
          },
        });
      }, 1400);

      return { json: { message_id: messageId, run_id: runId } };
    },
  },
  {
    method: "POST",
    pattern: /^\/api\/webchat\/v2\/threads\/([^/]+)\/runs\/([^/]+)\/cancel$/,
    handle: () => ({ json: {} }),
  },
];
