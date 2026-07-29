// Operator/caller log fixtures: ~80 records spanning levels and subsystems,
// correlated with the chat fixtures in routes/core.ts (thread-release-notes,
// thread-sandbox-triage) so drilling from chat into logs lines up.

import { MINUTE, iso } from "./helpers";

export type LogRecord = {
  id: string;
  timestamp: string;
  level: "trace" | "debug" | "info" | "warn" | "error";
  target: string;
  message: string;
  thread_id?: string;
  run_id?: string;
  turn_id?: string;
  tool_call_id?: string;
  tool_name?: string;
  source?: string;
};

const LEVEL_RANK: Record<string, number> = {
  trace: 0,
  debug: 1,
  info: 2,
  warn: 3,
  error: 4,
};

const RELEASE_THREAD = "thread-release-notes";
const RELEASE_RUN = "run-release-notes-7d41";
const TRIAGE_THREAD = "thread-sandbox-triage";
const TRIAGE_RUN = "run-sandbox-triage-1f8c";
const STANDUP_RUN = "run-standup-90aa";

let sequence = 0;
function record(
  minutesAgo: number,
  level: LogRecord["level"],
  target: string,
  message: string,
  extra: Partial<LogRecord> = {}
): LogRecord {
  sequence += 1;
  return {
    id: `log-${String(sequence).padStart(4, "0")}`,
    timestamp: iso(minutesAgo * MINUTE),
    level,
    target,
    message,
    source: "gateway",
    ...extra,
  };
}

const logRecords: LogRecord[] = [
  // ── Boot + steady-state gateway chatter (oldest) ─────────────────
  record(26 * 60, "info", "ironclaw::gateway", "Gateway listening on 0.0.0.0:8080 (tls terminated upstream)"),
  record(26 * 60, "info", "ironclaw::engine", "Engine v2 online; 11 tools registered, 5 extensions active"),
  record(26 * 60, "debug", "ironclaw::extensions::slack", "Slack socket-mode connection established (team T0189DEMO)"),
  record(26 * 60, "debug", "ironclaw::extensions::telegram", "Telegram webhook verified: https://gateway.demo.ironclaw.dev/hooks/telegram"),
  record(25 * 60 + 40, "info", "ironclaw::scheduler", "Loaded 3 routines: docs-sync, nightly-backup, standup-digest"),
  record(24 * 60, "info", "ironclaw::llm", "Active provider: anthropic (claude-sonnet-4-5)"),
  record(23 * 60, "warn", "ironclaw::llm", "Rate-limit headroom below 20% on anthropic; enabling request pacing"),
  record(22 * 60, "info", "ironclaw::memory", "Compacted 148 memory documents in 412 ms"),

  // ── Yesterday's nightly backup failure (standup summary refers to it) ─
  record(21 * 60, "info", "ironclaw::scheduler", "Routine nightly-backup started", { run_id: "run-nightly-backup-3311" }),
  record(21 * 60 - 4, "error", "ironclaw::jobs", "nightly-backup failed: disk pressure on runner-2 (94% used)", { run_id: "run-nightly-backup-3311" }),
  record(21 * 60 - 5, "warn", "ironclaw::jobs", "Retry scheduled for nightly-backup at 02:00 local", { run_id: "run-nightly-backup-3311" }),
  record(20 * 60, "info", "ironclaw::jobs", "Freed 12 GB on runner-2 (pruned stale sandbox layers)"),

  // ── Standup digest run ────────────────────────────────────────────
  record(19 * 60, "info", "ironclaw::engine::turn", "Run accepted", { thread_id: "thread-standup", run_id: STANDUP_RUN }),
  record(19 * 60 - 1, "debug", "ironclaw::engine::turn", "Prompt assembled: 6.2k tokens (3 memory hits)", { thread_id: "thread-standup", run_id: STANDUP_RUN }),
  record(19 * 60 - 2, "info", "ironclaw::engine::turn", "Final reply delivered in 7.4 s", { thread_id: "thread-standup", run_id: STANDUP_RUN }),

  // ── Sandbox triage thread (job-7f3a timeout) ─────────────────────
  record(4 * 60 + 5, "info", "ironclaw::engine::turn", "Run accepted", { thread_id: TRIAGE_THREAD, run_id: TRIAGE_RUN }),
  record(4 * 60 + 3, "info", "ironclaw::tools", "jobs.inspect completed in 320 ms", { thread_id: TRIAGE_THREAD, run_id: TRIAGE_RUN, tool_name: "jobs.inspect", tool_call_id: "call-jobs-inspect-01" }),
  record(4 * 60 + 1, "warn", "ironclaw::sandbox", "job-7f3a: build stage exceeded soft budget (1500 s of 1800 s)", { run_id: TRIAGE_RUN }),
  record(4 * 60, "error", "ironclaw::sandbox", "job-7f3a timed out after 1800 s in stage build", { run_id: TRIAGE_RUN }),
  record(3 * 60 + 56, "error", "ironclaw::tools", "logs.query failed: tail exceeded 30 s query budget (runner under load)", { thread_id: TRIAGE_THREAD, run_id: TRIAGE_RUN, tool_name: "logs.query", tool_call_id: "call-logs-query-02" }),
  record(3 * 60 + 55, "warn", "ironclaw::sandbox", "job-7f3a: git fetch retried 6x through proxy (idle connections dropped)", { run_id: TRIAGE_RUN }),
  record(3 * 60 + 52, "info", "ironclaw::engine::turn", "Final reply delivered in 41.8 s", { thread_id: TRIAGE_THREAD, run_id: TRIAGE_RUN }),

  // ── Release-notes thread ─────────────────────────────────────────
  record(52, "info", "ironclaw::engine::turn", "Run accepted", { thread_id: RELEASE_THREAD, run_id: RELEASE_RUN }),
  record(51, "debug", "ironclaw::engine::turn", "Prompt assembled: 9.8k tokens (release-notes skill active)", { thread_id: RELEASE_THREAD, run_id: RELEASE_RUN }),
  record(50, "info", "ironclaw::tools", "github.list_merged_prs returned 41 PRs in 1.9 s", { thread_id: RELEASE_THREAD, run_id: RELEASE_RUN, tool_name: "github.list_merged_prs", tool_call_id: "call-gh-prs-03" }),
  record(48, "info", "ironclaw::tools", "workspace.write_file wrote 4.1 KB to notes/release-v0.9.md", { thread_id: RELEASE_THREAD, run_id: RELEASE_RUN, tool_name: "workspace.write_file", tool_call_id: "call-write-04" }),
  record(46, "info", "ironclaw::engine::turn", "Final reply delivered in 92.1 s", { thread_id: RELEASE_THREAD, run_id: RELEASE_RUN }),
  record(12, "info", "ironclaw::engine::turn", "Run accepted", { thread_id: RELEASE_THREAD, run_id: "run-release-notes-8e02" }),
  record(9, "info", "ironclaw::tools", "github.create_pr opened PR #6841", { thread_id: RELEASE_THREAD, run_id: "run-release-notes-8e02", tool_name: "github.create_pr", tool_call_id: "call-gh-pr-05" }),
  record(8, "info", "ironclaw::engine::turn", "Final reply delivered in 18.6 s", { thread_id: RELEASE_THREAD, run_id: "run-release-notes-8e02" }),
];

// Steady-state filler: heartbeats, channel deliveries, occasional warns —
// interleaved across the last ~24 h so the console looks alive at any zoom.
const FILLER: [LogRecord["level"], string, string][] = [
  ["info", "ironclaw::heartbeat", "Heartbeat OK — queue depth 0, 2 active sessions"],
  ["debug", "ironclaw::gateway", "GET /api/webchat/v2/threads 200 in 12 ms"],
  ["info", "ironclaw::extensions::slack", "Delivered final reply to #ops (1 message)"],
  ["debug", "ironclaw::memory", "memory_search hit 3 documents in 38 ms"],
  ["info", "ironclaw::scheduler", "Routine docs-sync completed: 2 doc updates drafted"],
  ["debug", "ironclaw::extensions::telegram", "Long-poll cycle completed (0 updates)"],
  ["warn", "ironclaw::llm", "anthropic request retried once (429, backoff 800 ms)"],
  ["info", "ironclaw::gateway", "Session token refreshed for demo-operator"],
  ["debug", "ironclaw::sandbox", "Prewarmed sandbox image ironclaw/runtime:0.9 in 2.1 s"],
  ["info", "ironclaw::traces", "Trace Commons sync: 2 submissions scored, +1.85 credit"],
  ["warn", "ironclaw::extensions::postgres", "Setup incomplete: connection string not configured"],
  ["debug", "ironclaw::gateway", "GET /api/webchat/v2/notifications 200 in 9 ms"],
];

for (let index = 0; index < 52; index += 1) {
  const [level, target, message] = FILLER[index % FILLER.length];
  // Spread fillers between 5 minutes and ~23 hours ago, deterministically.
  const minutesAgo = 5 + index * 26 + (index % 5) * 3;
  logRecords.push(record(minutesAgo, level, target, message));
}

logRecords.sort((a, b) => a.timestamp.localeCompare(b.timestamp));

type LogQuery = {
  level?: string | null;
  target?: string | null;
  threadId?: string | null;
  runId?: string | null;
  turnId?: string | null;
  toolCallId?: string | null;
  toolName?: string | null;
  source?: string | null;
  limit?: number | null;
  tail?: boolean;
};

export function queryLogRecords(query: LogQuery): LogRecord[] {
  const minRank = query.level ? (LEVEL_RANK[query.level] ?? 0) : 0;
  const target = (query.target || "").toLowerCase();
  let entries = logRecords.filter((entry) => {
    if (LEVEL_RANK[entry.level] < minRank) return false;
    if (target && !entry.target.toLowerCase().includes(target)) return false;
    if (query.threadId && entry.thread_id !== query.threadId) return false;
    if (query.runId && entry.run_id !== query.runId) return false;
    if (query.turnId && entry.turn_id !== query.turnId) return false;
    if (query.toolCallId && entry.tool_call_id !== query.toolCallId) return false;
    if (query.toolName && entry.tool_name !== query.toolName) return false;
    if (query.source && entry.source !== query.source) return false;
    return true;
  });
  const limit = query.limit && query.limit > 0 ? query.limit : entries.length;
  if (entries.length > limit) {
    // `tail` (and the default) return the newest slice; the page renders
    // oldest → newest with auto-scroll pinned to the bottom.
    entries = entries.slice(entries.length - limit);
  }
  return entries;
}

export function logsResponse(query: LogQuery, source: string) {
  return {
    source,
    entries: queryLogRecords(query),
    next_cursor: null,
    tail_supported: true,
    follow_supported: false,
  };
}

