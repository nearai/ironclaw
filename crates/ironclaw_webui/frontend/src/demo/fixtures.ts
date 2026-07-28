// Demo-mode fixtures. Static, realistic sample data for the hosted
// workspace demo (see mock-backend.ts). Shapes mirror the WebChat v2
// wire DTOs the SPA consumes; keep field names in sync with
// `ironclaw_product_workflow::webui_inbound` / `reborn_services::types`.

const NOW = Date.now();

function iso(minutesAgo: number) {
  return new Date(NOW - minutesAgo * 60_000).toISOString();
}

export const DEMO_SESSION = {
  tenant_id: "tenant-demo",
  user_id: "demo-user",
  capabilities: { operator_webui_config: true },
  features: { reborn_projects: false, global_auto_approve: false },
  attachments: {
    accept: [
      "image/png",
      ".png",
      "image/jpeg",
      ".jpeg",
      ".jpg",
      "application/pdf",
      ".pdf",
      "text/plain",
      ".txt",
    ],
    max_count: 10,
    max_file_bytes: 5_242_880,
    max_total_bytes: 10_485_760,
  },
};

const SCOPE = {
  tenant_id: "tenant-demo",
  agent_id: "agent-demo",
  owner_user_id: "demo-user",
};

export const DEMO_THREADS = [
  {
    thread_id: "thread-inbox-triage",
    title: "Inbox triage — label and summarize",
    created_by_actor_id: "demo-user",
    created_at: iso(60 * 26),
    updated_at: iso(12),
    scope: SCOPE,
  },
  {
    thread_id: "thread-daily-briefing",
    title: "Daily morning briefing",
    created_by_actor_id: "demo-user",
    created_at: iso(60 * 49),
    updated_at: iso(60 * 3),
    scope: SCOPE,
  },
  {
    thread_id: "thread-hn-monitor",
    title: "Hacker News keyword monitor",
    created_by_actor_id: "demo-user",
    created_at: iso(60 * 30),
    updated_at: iso(60 * 7),
    scope: SCOPE,
  },
  {
    thread_id: "thread-invoice-parser",
    title: "Invoice parsing into Sheets",
    created_by_actor_id: "demo-user",
    created_at: iso(60 * 96),
    updated_at: iso(60 * 20),
    scope: SCOPE,
  },
];

function toolPreview(order: number, capabilityId: string, title: string, summary: string) {
  return JSON.stringify({
    version: 1,
    invocation_id: `inv-${capabilityId}-${order}`,
    capability_id: capabilityId,
    status: "completed",
    title,
    activity_order: order,
    output_summary: summary,
  });
}

export const DEMO_TIMELINES: Record<string, Array<Record<string, unknown>>> = {
  "thread-inbox-triage": [
    {
      message_id: "msg-it-1",
      thread_id: "thread-inbox-triage",
      sequence: 1,
      kind: "user",
      status: "accepted",
      content:
        'Triage my inbox: label new emails as "Action", "FYI", or "Ignore", and summarize the Action ones for me.',
      created_at: iso(32),
      updated_at: iso(32),
      turn_run_id: "run-it-1",
    },
    {
      message_id: "msg-it-2",
      thread_id: "thread-inbox-triage",
      sequence: 2,
      kind: "capability_display_preview",
      status: "finalized",
      turn_run_id: "run-it-1",
      created_at: iso(31),
      updated_at: iso(31),
      content: toolPreview(1, "gmail.list_messages", "gmail.list_messages", "Fetched 18 new messages"),
    },
    {
      message_id: "msg-it-3",
      thread_id: "thread-inbox-triage",
      sequence: 3,
      kind: "capability_display_preview",
      status: "finalized",
      turn_run_id: "run-it-1",
      created_at: iso(30),
      updated_at: iso(30),
      content: toolPreview(2, "gmail.apply_label", "gmail.apply_label", "Labeled 18 messages (5 Action, 9 FYI, 4 Ignore)"),
    },
    {
      message_id: "msg-it-4",
      thread_id: "thread-inbox-triage",
      sequence: 4,
      kind: "assistant",
      status: "finalized",
      turn_run_id: "run-it-1",
      created_at: iso(29),
      updated_at: iso(29),
      content:
        "Done — 18 new emails triaged.\n\n**Action (5):**\n1. **Legal** — countersigned MSA needed by Thursday.\n2. **Priya (Design)** — review the workspace redesign Storybook before tomorrow's crit.\n3. **AWS** — payment method expires this month.\n4. **Recruiting** — two candidates waiting on scheduling confirmation.\n5. **Finance** — Q3 budget sheet needs your numbers by Friday.\n\nFYI (9) and Ignore (4) were labeled and archived. Want me to draft replies for any of the Action items?",
    },
    {
      message_id: "msg-it-5",
      thread_id: "thread-inbox-triage",
      sequence: 5,
      kind: "user",
      status: "accepted",
      content: "Draft a reply to Priya saying I'll review it tonight.",
      created_at: iso(13),
      updated_at: iso(13),
      turn_run_id: "run-it-2",
    },
    {
      message_id: "msg-it-6",
      thread_id: "thread-inbox-triage",
      sequence: 6,
      kind: "assistant",
      status: "finalized",
      turn_run_id: "run-it-2",
      created_at: iso(12),
      updated_at: iso(12),
      content:
        'Draft saved in Gmail:\n\n> Hi Priya — I\'ll go through the redesign Storybook tonight and leave comments inline. Expect notes before tomorrow\'s crit.\n\nIt\'s in your Drafts folder — say "send it" and I\'ll send it as-is.',
    },
  ],
  "thread-daily-briefing": [
    {
      message_id: "msg-db-1",
      thread_id: "thread-daily-briefing",
      sequence: 1,
      kind: "user",
      status: "accepted",
      content: "Every morning at 9am, send me a briefing with my calendar, important emails, and open tasks.",
      created_at: iso(60 * 49),
      updated_at: iso(60 * 49),
      turn_run_id: "run-db-1",
    },
    {
      message_id: "msg-db-2",
      thread_id: "thread-daily-briefing",
      sequence: 2,
      kind: "assistant",
      status: "finalized",
      turn_run_id: "run-db-1",
      created_at: iso(60 * 49 - 1),
      updated_at: iso(60 * 49 - 1),
      content:
        "Scheduled. I'll assemble a briefing every weekday at 9:00 (your timezone) from Google Calendar, Gmail, and your task list, and post it here. You can pause it any time from Automations.",
    },
    {
      message_id: "msg-db-3",
      thread_id: "thread-daily-briefing",
      sequence: 3,
      kind: "capability_display_preview",
      status: "finalized",
      turn_run_id: "run-db-2",
      created_at: iso(60 * 3 + 2),
      updated_at: iso(60 * 3 + 2),
      content: toolPreview(1, "google_calendar.list_events", "google_calendar.list_events", "4 events today"),
    },
    {
      message_id: "msg-db-4",
      thread_id: "thread-daily-briefing",
      sequence: 4,
      kind: "assistant",
      status: "finalized",
      turn_run_id: "run-db-2",
      created_at: iso(60 * 3),
      updated_at: iso(60 * 3),
      content:
        "**Morning briefing — Tuesday**\n\n**Calendar (4):** 10:00 design crit · 11:30 1:1 with Sam · 14:00 infra sync · 16:30 candidate call.\n\n**Email:** 2 Action items overnight (legal MSA, AWS billing). Nothing urgent from the team.\n\n**Tasks:** 3 open — budget numbers (Fri), Storybook review (today), draft launch notes.\n\nHave a good one — ping me if you want any meeting prepped.",
    },
  ],
  "thread-hn-monitor": [
    {
      message_id: "msg-hn-1",
      thread_id: "thread-hn-monitor",
      sequence: 1,
      kind: "user",
      status: "accepted",
      content: 'If "IronClaw" or "NEAR AI" appears on Hacker News, send a summary to me here.',
      created_at: iso(60 * 30),
      updated_at: iso(60 * 30),
      turn_run_id: "run-hn-1",
    },
    {
      message_id: "msg-hn-2",
      thread_id: "thread-hn-monitor",
      sequence: 2,
      kind: "assistant",
      status: "finalized",
      turn_run_id: "run-hn-1",
      created_at: iso(60 * 30 - 1),
      updated_at: iso(60 * 30 - 1),
      content:
        "Watching Hacker News for \"IronClaw\" and \"NEAR AI\" — I'll check every 15 minutes and summarize any hit here with the thread link and top comments.",
    },
    {
      message_id: "msg-hn-3",
      thread_id: "thread-hn-monitor",
      sequence: 3,
      kind: "capability_display_preview",
      status: "finalized",
      turn_run_id: "run-hn-2",
      created_at: iso(60 * 7 + 1),
      updated_at: iso(60 * 7 + 1),
      content: toolPreview(1, "builtin.web_search", "web_search", "1 new match on HN front page"),
    },
    {
      message_id: "msg-hn-4",
      thread_id: "thread-hn-monitor",
      sequence: 4,
      kind: "assistant",
      status: "finalized",
      turn_run_id: "run-hn-2",
      created_at: iso(60 * 7),
      updated_at: iso(60 * 7),
      content:
        "**HN mention:** \"IronClaw 1.0 — an open-source, security-first personal AI agent\" is on the front page (#8, 214 points, 96 comments).\n\nTop themes in comments: praise for the WASM tool sandbox, questions about self-hosting vs the hosted NEAR AI offering, and a long thread comparing it to other agent frameworks. Want a drafted response to the self-hosting question?",
    },
  ],
  "thread-invoice-parser": [
    {
      message_id: "msg-ip-1",
      thread_id: "thread-invoice-parser",
      sequence: 1,
      kind: "user",
      status: "accepted",
      content:
        'When I forward you a PDF invoice, extract the amount, date, and vendor into a Google Sheet called "Invoices".',
      created_at: iso(60 * 96),
      updated_at: iso(60 * 96),
      turn_run_id: "run-ip-1",
    },
    {
      message_id: "msg-ip-2",
      thread_id: "thread-invoice-parser",
      sequence: 2,
      kind: "assistant",
      status: "finalized",
      turn_run_id: "run-ip-1",
      created_at: iso(60 * 96 - 2),
      updated_at: iso(60 * 96 - 2),
      content:
        "Set up. Forward any invoice to your agent address and I'll parse it and append a row to the \"Invoices\" sheet (amount, date, vendor, source email link). I processed 3 invoices from your backlog already — total $4,812.40.",
    },
  ],
};

export const DEMO_AUTOMATIONS = {
  scheduler_enabled: true,
  automations: [
    {
      automation_id: "auto-daily-briefing",
      name: "Daily morning briefing",
      source: { type: "schedule", cron: "0 9 * * 1-5", timezone: "America/New_York" },
      state: "active",
      is_active: true,
      next_run_at: new Date(NOW + 16 * 3_600_000).toISOString(),
      last_run_at: iso(60 * 3),
      last_status: "ok",
      created_at: iso(60 * 49),
      recent_runs: [
        {
          run_id: "run-db-2",
          thread_id: "thread-daily-briefing",
          status: "ok",
          fire_slot: iso(60 * 3 + 4),
          submitted_at: iso(60 * 3 + 4),
          completed_at: iso(60 * 3),
        },
        {
          run_id: "run-db-prev",
          thread_id: "thread-daily-briefing",
          status: "ok",
          fire_slot: iso(60 * 27),
          submitted_at: iso(60 * 27),
          completed_at: iso(60 * 27 - 1),
        },
      ],
    },
    {
      automation_id: "auto-hn-monitor",
      name: "Hacker News keyword monitor",
      source: { type: "schedule", cron: "*/15 * * * *", timezone: "UTC" },
      state: "active",
      is_active: true,
      next_run_at: new Date(NOW + 9 * 60_000).toISOString(),
      last_run_at: iso(6),
      last_status: "ok",
      created_at: iso(60 * 30),
      recent_runs: [
        {
          run_id: "run-hn-2",
          thread_id: "thread-hn-monitor",
          status: "ok",
          fire_slot: iso(6),
          submitted_at: iso(6),
          completed_at: iso(5),
        },
      ],
    },
    {
      automation_id: "auto-health-watch",
      name: "Deployment health watcher",
      source: { type: "schedule", cron: "*/5 * * * *", timezone: "UTC" },
      state: "paused",
      is_active: false,
      last_run_at: iso(60 * 11),
      last_status: "error",
      created_at: iso(60 * 120),
      recent_runs: [
        {
          run_id: "run-hw-1",
          thread_id: "thread-hn-monitor",
          status: "error",
          fire_slot: iso(60 * 11),
          submitted_at: iso(60 * 11),
          completed_at: iso(60 * 11 - 1),
        },
      ],
    },
    {
      automation_id: "auto-quarterly-report",
      name: "Quarterly usage report",
      source: { type: "once", at: new Date(NOW + 5 * 86_400_000).toISOString(), timezone: "UTC" },
      state: "scheduled",
      is_active: false,
      next_run_at: new Date(NOW + 5 * 86_400_000).toISOString(),
      recent_runs: [],
    },
  ],
};

export const DEMO_EXTENSIONS = {
  extensions: [
    {
      package_ref: { kind: "extension", id: "gmail" },
      display_name: "Gmail",
      kind: "wasm_tool",
      description: "Read, label, draft, and send email on your behalf.",
      authenticated: true,
      active: true,
      tools: ["gmail.list_messages", "gmail.apply_label", "gmail.create_draft"],
      needs_setup: false,
      has_auth: true,
      activation_status: "active",
      version: "1.2.0",
      install_scope: "tenant",
    },
    {
      package_ref: { kind: "extension", id: "google-calendar" },
      display_name: "Google Calendar",
      kind: "wasm_tool",
      description: "Read events and schedule meetings.",
      authenticated: true,
      active: true,
      tools: ["google_calendar.list_events", "google_calendar.create_event"],
      needs_setup: false,
      has_auth: true,
      activation_status: "active",
      version: "1.1.3",
      install_scope: "tenant",
    },
    {
      package_ref: { kind: "extension", id: "slack" },
      display_name: "Slack",
      kind: "wasm_channel",
      description: "Chat with your agent from Slack and post updates to channels.",
      authenticated: false,
      active: false,
      tools: [],
      needs_setup: true,
      has_auth: true,
      activation_status: "installed",
      onboarding_state: "auth_required",
      onboarding: {
        credential_instructions: "Authorize the Slack workspace in the browser.",
        credential_next_step: "Complete OAuth",
        setup_url: null,
      },
      version: "0.9.4",
      install_scope: "tenant",
    },
    {
      package_ref: { kind: "extension", id: "telegram" },
      display_name: "Telegram",
      kind: "wasm_channel",
      description: "Pair your Telegram account to chat with the agent.",
      authenticated: false,
      active: false,
      tools: [],
      needs_setup: true,
      has_auth: false,
      activation_status: "installed",
      onboarding_state: "setup_required",
      version: "1.0.1",
      install_scope: "tenant",
    },
    {
      package_ref: { kind: "extension", id: "web-search" },
      display_name: "Web Search",
      kind: "first_party",
      description: "Search the web and summarize results.",
      authenticated: true,
      active: true,
      tools: ["builtin.web_search"],
      needs_setup: false,
      has_auth: false,
      activation_status: "active",
      version: "1.0.0",
      install_scope: "tenant",
    },
  ],
};

export const DEMO_REGISTRY = {
  entries: [
    {
      package_ref: { kind: "extension", id: "github" },
      display_name: "GitHub",
      kind: "mcp_server",
      description: "Issues, pull requests, and repo automation via MCP.",
      installed: false,
      keywords: ["git", "code", "mcp"],
      version: "0.4.0",
    },
    {
      package_ref: { kind: "extension", id: "notion" },
      display_name: "Notion",
      kind: "mcp_server",
      description: "Read and write Notion pages and databases.",
      installed: false,
      keywords: ["notes", "wiki"],
      version: "0.2.1",
    },
    {
      package_ref: { kind: "extension", id: "google-sheets" },
      display_name: "Google Sheets",
      kind: "wasm_tool",
      description: "Append rows and read ranges from spreadsheets.",
      installed: false,
      keywords: ["spreadsheet"],
      version: "0.8.0",
    },
    {
      package_ref: { kind: "extension", id: "discord" },
      display_name: "Discord",
      kind: "wasm_channel",
      description: "Chat with your agent from Discord.",
      installed: false,
      keywords: ["chat"],
      version: "0.6.2",
    },
  ],
};

export const DEMO_FS_MOUNTS = {
  mounts: [
    { mount: "memory", label: "Memory" },
    { mount: "workspace", label: "Workspace files" },
    { mount: "skills", label: "Skills" },
  ],
};

type FsEntry = { name: string; path: string; kind: "file" | "directory"; size_bytes?: number };

export const DEMO_FS_TREE: Record<string, Record<string, FsEntry[]>> = {
  memory: {
    "": [
      { name: "preferences.md", path: "preferences.md", kind: "file", size_bytes: 412 },
      { name: "people", path: "people", kind: "directory" },
      { name: "projects.md", path: "projects.md", kind: "file", size_bytes: 1180 },
    ],
    people: [
      { name: "priya.md", path: "people/priya.md", kind: "file", size_bytes: 224 },
      { name: "sam.md", path: "people/sam.md", kind: "file", size_bytes: 198 },
    ],
  },
  workspace: {
    "": [
      { name: "notes", path: "notes", kind: "directory" },
      { name: "reports", path: "reports", kind: "directory" },
      { name: "invoices.csv", path: "invoices.csv", kind: "file", size_bytes: 642 },
    ],
    notes: [
      { name: "todo.md", path: "notes/todo.md", kind: "file", size_bytes: 384 },
      { name: "launch-notes.md", path: "notes/launch-notes.md", kind: "file", size_bytes: 1520 },
    ],
    reports: [
      { name: "deploys.md", path: "reports/deploys.md", kind: "file", size_bytes: 903 },
    ],
  },
  skills: {
    "": [
      { name: "inbox-triage.skill.md", path: "inbox-triage.skill.md", kind: "file", size_bytes: 1044 },
      { name: "morning-briefing.skill.md", path: "morning-briefing.skill.md", kind: "file", size_bytes: 933 },
    ],
  },
};

export const DEMO_FS_CONTENT: Record<string, string> = {
  "memory/preferences.md":
    "# Preferences\n\n- Briefings at 9:00 America/New_York, weekdays only.\n- Prefers bullet summaries over prose.\n- Never auto-send email — always draft for review.\n- Quiet hours: 22:00-07:00 (no notifications).\n",
  "memory/projects.md":
    "# Active projects\n\n## Workspace redesign\nDesign-system application across the agent workspace. Storybook review pending.\n\n## Q3 budget\nNumbers due Friday. Sheet: Finance/Q3-budget.\n",
  "memory/people/priya.md": "# Priya\nDesign lead. Owns the design system and Storybook. Crit on Wednesdays.\n",
  "memory/people/sam.md": "# Sam\nEng manager. Weekly 1:1 Tuesdays 11:30.\n",
  "workspace/notes/todo.md":
    "# TODO\n\n- [ ] Review workspace redesign Storybook (tonight)\n- [ ] Budget numbers for Q3 sheet (Friday)\n- [ ] Draft launch notes\n",
  "workspace/notes/launch-notes.md":
    "# IronClaw 1.0 launch notes (draft)\n\nSecurity-first personal AI agent: WASM tool sandbox, egress allowlists,\nchannel pairing, and a redesigned workspace UI on the shared design system.\n",
  "workspace/reports/deploys.md":
    "# Deploys — yesterday\n\n1. api-gateway v2.14.1 — canary, then full rollout, no regressions.\n2. webui bundle refresh — design-system density pass.\n3. scheduler hotfix — duplicate fire-slot guard.\n",
  "workspace/invoices.csv":
    "date,vendor,amount\n2026-07-14,Vercel,$220.00\n2026-07-18,Figma,$144.00\n2026-07-21,AWS,$4448.40\n",
  "skills/inbox-triage.skill.md":
    "# Skill: inbox triage\n\nLabel new mail Action/FYI/Ignore; summarize Action items; never auto-send.\n",
  "skills/morning-briefing.skill.md":
    "# Skill: morning briefing\n\nAssemble calendar + email + tasks into one 9am summary message.\n",
};

const LOG_TARGETS = [
  "ironclaw::scheduler",
  "ironclaw::webchat",
  "ironclaw::extensions::gmail",
  "ironclaw::sandbox",
  "ironclaw::llm",
];

const LOG_LINES: Array<[string, string]> = [
  ["info", "run accepted for thread thread-inbox-triage"],
  ["debug", "capability gmail.list_messages resolved in 412ms"],
  ["info", "scheduler fired automation auto-hn-monitor (slot on time)"],
  ["warn", "outbound target slack-dm unavailable, falling back to webui"],
  ["info", "final reply delivered to thread thread-daily-briefing"],
  ["debug", "token budget for turn: 6_214 prompt / 512 completion"],
  ["error", "health check https://example.com/health returned 503"],
  ["info", "sandbox container recycled after idle timeout"],
  ["debug", "sse stream resumed from cursor before:118"],
  ["info", "extension slack awaiting oauth authorization"],
];

export const DEMO_LOGS = {
  source: "in_memory_tracing",
  entries: Array.from({ length: 40 }, (_, index) => {
    const [level, message] = LOG_LINES[index % LOG_LINES.length];
    return {
      id: String(200 - index),
      timestamp: new Date(NOW - index * 47_000).toISOString(),
      level,
      target: LOG_TARGETS[index % LOG_TARGETS.length],
      message,
      thread_id: index % 3 === 0 ? "thread-inbox-triage" : undefined,
      run_id: index % 3 === 0 ? "run-it-1" : undefined,
    };
  }),
  next_cursor: null,
  tail_supported: true,
  follow_supported: false,
};

export const DEMO_LLM_PROVIDERS = {
  providers: [
    {
      id: "nearai",
      description: "NEAR AI",
      adapter: "nearai",
      default_model: "qwen3-235b-instruct",
      base_url: null,
      builtin: true,
      active: true,
      active_model: "qwen3-235b-instruct",
      api_key_required: false,
      accepts_api_key: true,
      api_key_set: true,
      can_list_models: true,
    },
    {
      id: "anthropic",
      description: "Anthropic",
      adapter: "anthropic",
      default_model: "claude-sonnet-4-6",
      base_url: null,
      builtin: true,
      active: false,
      api_key_required: true,
      accepts_api_key: true,
      api_key_set: false,
      can_list_models: true,
    },
  ],
  active: { provider_id: "nearai", model: "qwen3-235b-instruct" },
};

export const DEMO_OUTBOUND_PREFERENCES = {
  final_reply_target: null,
  final_reply_target_status: "none_configured",
  default_modality: "text",
};

export const DEMO_OUTBOUND_TARGETS = {
  targets: [
    {
      target: {
        target_id: "webui",
        channel: "webui",
        display_name: "Web app",
        description: "Replies appear in this workspace",
      },
      capabilities: { final_replies: true, gate_prompts: true, auth_prompts: true },
    },
  ],
  next_cursor: null,
};

export const DEMO_CONNECTABLE_CHANNELS = {
  channels: [
    {
      channel: "telegram",
      display_name: "Telegram",
      strategy: "inbound_proof_code",
      action: {
        title: "Telegram account connection",
        instructions:
          "Message the Telegram bot to get a code, then paste it here. Codes expire in 10 minutes.",
        input_placeholder: "Enter Telegram pairing code...",
        submit_label: "Connect",
        success_message: "Telegram account connected.",
        error_message: "Invalid or expired Telegram pairing code.",
      },
      command_aliases: ["telegram"],
    },
  ],
};

export const DEMO_SKILLS = {
  skills: [
    {
      name: "inbox-triage",
      description: "Label new mail Action/FYI/Ignore and summarize Action items.",
      enabled: true,
      auto_activate: true,
    },
    {
      name: "morning-briefing",
      description: "Assemble calendar, email, and tasks into one 9am summary.",
      enabled: true,
      auto_activate: true,
    },
  ],
};

// Canned assistant replies for messages sent live in the demo.
export const DEMO_CANNED_REPLIES = [
  "On it. In this demo the backend is mocked, so I can't actually run tools — but in a real deployment I'd pick up this request, run the relevant capabilities in the sandbox, and report back here.",
  "Got it. This hosted preview is running against fixture data (no live agent), so treat this as a tour of the redesigned workspace: the sidebar, Automations, Extensions, Files, Logs, and Settings are all real UI on the shared design system.",
  "Noted! If this were a live agent I'd schedule that and confirm — here you can explore how runs, tool activity, and final replies render in the redesigned chat surface.",
];
