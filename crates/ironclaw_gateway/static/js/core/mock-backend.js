// ============================================================================
// IronClaw NUX — SINGLE mock file: curated content + demo mock backend.
// ============================================================================
//
// MOCK: everything in this file is mock/curated product content, kept in ONE
// place so backend engineers can find every mocked surface and re-implement
// it where it belongs in the real stack. It has three sections:
//
//   1. NUX_DATA — editorial/curated content (use-case gallery, starter
//      prompts, channel/integration catalogs, discovery categories).
//      Real backend home: a CMS table or a static config served by the
//      gateway (e.g. GET /api/nux/content), so marketing can edit copy
//      without a frontend deploy.
//
//   2. NUX_BILLING — mock billing/credits plan content and tuning constants
//      consumed by js/core/billing.js. Real backend home: a billing service
//      (plans: GET /api/billing/plans; balance/usage: GET /api/billing/usage;
//      upgrade: POST /api/billing/subscribe). Client-side "burn per message"
//      becomes server-side metering on the LLM cost tracker.
//
//   3. DEMO MOCK BACKEND — an in-browser implementation of the gateway API
//      surface (fetch + SSE interception) so the whole SPA runs end-to-end
//      with NO server. Only active when `window.__IRONCLAW_DEMO__` is truthy
//      (set by the Vercel demo build, see demo/) or `?demo=1` is in the URL.
//      In a real gateway deployment this section is inert dead code; it
//      documents — endpoint by endpoint — the API contract the frontend
//      expects. Real backend home: each handler maps 1:1 onto an existing
//      gateway route (see `implied API surface` notes inline).
//
// Sources: "IronClaw — Use Cases to test" doc + Achal/Sergey product
// direction call (2026-06-05).
// ============================================================================

// ============================================================================
// SECTION 1 — Curated NUX content (was js/core/mock-data.js)
// ============================================================================

window.NUX_DATA = {
  // MOCK: Use-case gallery. `prompt` seeds the chat input; `integrations`
  // reference extension/channel registry names so setup can deep-link into
  // the real install/configure flows when a matching entry exists.
  useCases: [
    {
      id: 'inbox-triage',
      glyph: '\u2709', // ✉
      title: 'Inbox triage',
      description: 'Read, prioritize, and summarize email. Label inbound as Action, FYI, or Ignore and draft replies for the ones that matter.',
      category: 'communication',
      integrations: ['gmail'],
      prompt: 'Triage my inbox: label new emails as "Action", "FYI", or "Ignore", and summarize the Action ones for me.',
    },
    {
      id: 'daily-briefing',
      glyph: '\u2600', // ☀
      title: 'Daily morning briefing',
      description: 'A concise daily summary of your calendar, email, tasks, and key signals — delivered wherever you are.',
      category: 'productivity',
      integrations: ['gmail', 'google_calendar', 'telegram'],
      prompt: 'Every morning at 9am, send me a briefing with my calendar, important emails, and open tasks.',
    },
    {
      id: 'meeting-prep',
      glyph: '\u29D7', // ⧗
      title: 'Meeting prep assistant',
      description: '10 minutes before each meeting, get a brief on the company, attendees, and recent news.',
      category: 'productivity',
      integrations: ['google_calendar'],
      prompt: '10 minutes before each meeting on my calendar, send me a summary of the company and recent news about the attendees.',
    },
    {
      id: 'team-chat-ops',
      glyph: '\u2318', // ⌘
      title: 'Team chat operations',
      description: 'Use Slack or Telegram as your control layer — send updates, triage messages, and coordinate work from chat.',
      category: 'communication',
      integrations: ['slack', 'telegram'],
      prompt: 'Connect to Slack so I can message you from there, and post a summary of what you can do to my DMs.',
    },
    {
      id: 'keyword-monitor',
      glyph: '\u2317', // ⌗
      title: 'Keyword monitor',
      description: 'Watch Hacker News, Twitter, or the web for mentions of your product and get a summary the moment it appears.',
      category: 'monitoring',
      integrations: ['slack', 'telegram'],
      prompt: 'If "IronClaw" or "NEAR AI" appears on Hacker News, send a summary to me here.',
    },
    {
      id: 'deploy-watcher',
      glyph: '\u23F1', // ⏱
      title: 'Deployment health watcher',
      description: 'Ping an endpoint every 5 minutes and get an alert in chat if it returns anything but a 200.',
      category: 'monitoring',
      integrations: ['telegram'],
      prompt: 'Ping https://example.com/health every 5 minutes and alert me if it returns a non-200 status.',
    },
    {
      id: 'release-tracker',
      glyph: '\u2387', // ⎇
      title: 'Release tracker',
      description: 'Watch a GitHub repo and get new releases summarized into your channel of choice.',
      category: 'developer',
      integrations: ['github', 'telegram'],
      prompt: 'Watch the nearai/ironclaw GitHub repo and summarize new releases for me when they ship.',
    },
    {
      id: 'task-delegation',
      glyph: '\u2713', // ✓
      title: 'Task capture & delegation',
      description: 'Turn messages and emails into structured tasks with assignments and tracking — "create task: …" from anywhere.',
      category: 'productivity',
      integrations: ['slack', 'linear'],
      prompt: 'When I DM you "create task: ..." open a ticket with that description and confirm back with the link.',
    },
    {
      id: 'invoice-parser',
      glyph: '\u2756', // ❖
      title: 'Invoice parser',
      description: 'Forward a PDF invoice and the amount, date, and vendor land in a spreadsheet automatically.',
      category: 'automation',
      integrations: ['gmail', 'google_sheets'],
      prompt: 'When I forward you a PDF invoice, extract the amount, date, and vendor into a Google Sheet called "Invoices".',
    },
    {
      id: 'kpi-reporter',
      glyph: '\u2261', // ≡
      title: 'Daily KPI reporter',
      description: 'Pull simple metrics from a CSV or API and post a formatted dashboard to your team channel daily.',
      category: 'automation',
      integrations: ['slack'],
      prompt: 'Every weekday at 5pm, pull our KPI numbers and post a formatted summary to Slack.',
    },
  ],

  // MOCK: Use-case categories for filtering — ids must match `category` above.
  useCaseCategories: [
    { id: 'all', label: 'All' },
    { id: 'communication', label: 'Communication' },
    { id: 'productivity', label: 'Productivity' },
    { id: 'monitoring', label: 'Monitoring' },
    { id: 'developer', label: 'Developer' },
    { id: 'automation', label: 'Automation' },
  ],

  // MOCK: Starter prompts for the chat empty state. Unlike the old generic
  // chips ("Run a tool", "System status"), each demonstrates a real
  // capability and is phrased as something a non-technical user would say.
  starterPrompts: [
    'What can you do for me?',
    'Connect to Telegram so I can message you from my phone',
    'Summarize the top stories on Hacker News right now',
    'Set up a daily 9am briefing with my calendar and inbox',
    'Build me a tool that checks the weather every morning',
    'Watch a website and tell me when it changes',
  ],

  // MOCK: Channels to highlight in the guided setup. `name` must match the
  // extension registry name so install/configure uses the real flow.
  setupChannels: [
    { name: 'telegram', icon: '/icons/integrations/telegram.png', label: 'Telegram', blurb: 'Chat with your agent from your phone, on the go.' },
    { name: 'slack', icon: '/icons/integrations/slack.png', label: 'Slack', blurb: 'Bring your agent into the place your team already works.' },
    { name: 'discord', icon: '/icons/integrations/discord.png', label: 'Discord', blurb: 'Run your agent inside your community server.' },
    { name: 'whatsapp', icon: '/icons/integrations/whatsapp.png', label: 'WhatsApp', blurb: 'Message your agent like any other contact.' },
  ],

  // MOCK: Curated integration catalog for the first-class Integrations
  // surface. `id`s match extension registry names where one exists and the
  // `integrations` ids used by use cases above / the marketing-site handoff
  // (`?integrations=gmail,slack`). Entries with no registry match render as
  // "ask the agent to connect it" cards.
  integrationCatalog: [
    { id: 'gmail', icon: '/icons/integrations/gmail.png', label: 'Gmail', glyph: '\u2709', blurb: 'Read, triage, label, and draft email on your behalf.' },
    { id: 'google_calendar', icon: '/icons/integrations/google_calendar.png', label: 'Google Calendar', glyph: '\u29D7', blurb: 'See your schedule, prep you for meetings, and block time.' },
    { id: 'google_sheets', icon: '/icons/integrations/google_sheets.png', label: 'Google Sheets', glyph: '\u2261', blurb: 'Append rows, build reports, and keep trackers up to date.' },
    { id: 'google_drive', icon: '/icons/integrations/google_drive.png', label: 'Google Drive', glyph: '\u25B3', blurb: 'Files found, organized, and summarized on demand.' },
    { id: 'google_docs', icon: '/icons/integrations/google_docs.png', label: 'Google Docs', glyph: '\u2630', blurb: 'Drafts, edits, and summaries straight into Docs.' },
    { id: 'notion', icon: '/icons/integrations/notion.png', label: 'Notion', glyph: '\u25A4', blurb: 'Docs and databases your agent can read and update.' },
    { id: 'slack', icon: '/icons/integrations/slack.png', label: 'Slack', glyph: '\u2318', blurb: 'Chat with your agent and post updates where your team works.' },
    { id: 'telegram', icon: '/icons/integrations/telegram.png', label: 'Telegram', glyph: '\u2708', blurb: 'Message your agent from your phone, anywhere.' },
    { id: 'discord', icon: '/icons/integrations/discord.png', label: 'Discord', glyph: '\u2756', blurb: 'Run your agent inside your community server.' },
    { id: 'whatsapp', icon: '/icons/integrations/whatsapp.png', label: 'WhatsApp', glyph: '\u260E', blurb: 'Talk to your agent like any other contact.' },
    { id: 'github', icon: '/icons/integrations/github.png', label: 'GitHub', glyph: '\u2387', blurb: 'Watch repos, summarize releases, and track issues.' },
    { id: 'linear', icon: '/icons/integrations/linear.png', label: 'Linear', glyph: '\u2713', blurb: 'Create and update tickets straight from chat.' },
  ],

  // MOCK: Project starter templates for the Projects null state. Clicking
  // one pre-fills chat with `prompt` — the agent then creates the project
  // (implied: POST /api/engine/projects). Curated editorial content.
  projectStarters: [
    {
      id: 'competitive-intel',
      name: 'Competitive intel',
      blurb: 'Track competitor launches, pricing changes, and press — get a weekly digest.',
      prompt: 'Create a project called "Competitive intel". Watch our main competitors for launches, pricing changes, and press coverage, and post me a digest every Friday.',
    },
    {
      id: 'growth-reporting',
      name: 'Growth reporting',
      blurb: 'Weekly KPI reports assembled from your sheets and posted to the team.',
      prompt: 'Create a project called "Growth reporting". Every Monday, pull our KPI numbers from Google Sheets and post a formatted report to Slack.',
    },
    {
      id: 'personal-ops',
      name: 'Personal ops',
      blurb: 'Inbox triage, meeting prep, and reminders — your chief-of-staff workspace.',
      prompt: 'Create a project called "Personal ops" that handles my inbox triage, meeting prep, and daily reminders in one place.',
    },
  ],

  // MOCK: Unified discovery categories. `kinds` map onto real registry entry
  // kinds (`/api/extensions/registry`) plus the virtual `skill` kind backed
  // by `/api/skills` + `/api/skills/search`.
  discoverCategories: [
    { id: 'all', label: 'All', kinds: null },
    { id: 'channels', label: 'Channels', kinds: ['wasm_channel', 'channel', 'channel_relay'] },
    { id: 'tools', label: 'Tools', kinds: ['wasm_tool', 'tool', 'native'] },
    { id: 'mcp', label: 'MCP servers', kinds: ['mcp_server'] },
    { id: 'skills', label: 'Skills', kinds: ['skill'] },
  ],
};

// ============================================================================
// SECTION 2 — Mock billing/credits content (consumed by js/core/billing.js)
// ============================================================================
//
// MOCK: three-tier plan cards (mirrors the marketing site's pricing step)
// plus the tuning constants for the client-side credit simulation.
// Implied backend API surface:
//   GET  /api/billing/plans      -> { plans: [{id, name, price, period, credits, blurb, recommended}] }
//   GET  /api/billing/usage      -> { total_usd, used_usd, by_day: {date: usd} }
//   POST /api/billing/subscribe  -> { plan_id }
//   POST /api/billing/credits    -> top-up
// Today the state lives in localStorage (see billing.js) and usage burn is
// simulated per message; a real backend meters usage on the LLM cost tracker.

window.NUX_BILLING = {
  freeCreditsUsd: 5.0,
  lowBalanceThresholdUsd: 1.0,
  reminderAfterMessages: 3,
  plans: [
    { id: 'starter', name: 'Starter', price: '$9', period: '/mo', credits: '$10 credits/mo', blurb: 'For trying out a personal agent on light tasks.' },
    { id: 'basic', name: 'Basic', price: '$29', period: '/mo', credits: '$35 credits/mo', blurb: 'Daily briefings, monitoring, and automations.', recommended: true },
    { id: 'proplus', name: 'Pro+', price: '$99', period: '/mo', credits: '$140 credits/mo', blurb: 'Heavy multi-task workloads and team usage.' },
  ],
};

// ============================================================================
// SECTION 3 — Demo mock backend (fetch + SSE interception)
// ============================================================================
//
// Active ONLY in demo mode. Implements just enough of the gateway HTTP API
// and the `/api/chat/events` SSE stream for the SPA to run a full
// intent-URL → onboarding → agentic chat → task/automation-creation
// walkthrough on canned data. Every handler below documents the implied
// real endpoint next to it.

(function initIronclawDemoBackend() {
  'use strict';

  var demoRequested = false;
  try {
    var search = new URLSearchParams(window.location.search);
    if (search.get('demo') === '1') {
      sessionStorage.setItem('ironclaw_demo_mode', '1');
    }
    demoRequested = window.__IRONCLAW_DEMO__ === true
      || sessionStorage.getItem('ironclaw_demo_mode') === '1';
  } catch (e) {
    demoRequested = window.__IRONCLAW_DEMO__ === true;
  }
  if (!demoRequested) return;
  window.__IRONCLAW_DEMO__ = true;

  // --------------------------------------------------------------------
  // Demo state (in-memory; reset on reload)
  // --------------------------------------------------------------------

  var now = Date.now();
  function iso(msAgo) { return new Date(now - (msAgo || 0)).toISOString(); }
  function uid(prefix) {
    return prefix + '-' + Math.random().toString(36).slice(2, 10);
  }

  var state = {
    // Chat threads (implied: GET /api/chat/threads, POST /api/chat/thread/new)
    threads: [
      { id: 'thread-demo-1', title: null, channel: 'gateway', state: 'Idle', updated_at: iso(0) },
    ],
    // Turns per thread (implied: GET /api/chat/history?thread_id=)
    turnsByThread: {},
    // Engine missions == Tasks kanban (implied: /api/engine/missions*)
    missions: [],
    // Engine threads == per-mission runs (implied: /api/engine/threads*)
    engineThreads: [],
    // Installed extensions (implied: /api/extensions, /api/extensions/install)
    extensions: [],
    // Installed skills (implied: GET /api/skills -> { skills: [{ name,
    // version, trust: 'Trusted'|'Installed', description, keywords,
    // usage_hint, install_source_url }] })
    skills: [
      {
        name: 'summarize',
        icon: 'list',
        version: '1.2.0',
        trust: 'Trusted',
        description: 'Condense long content — pages, threads, transcripts — into clean key points.',
        keywords: ['summarize', 'tl;dr', 'recap'],
        usage_hint: 'Try: "/summarize this thread" or paste a link.',
      },
      {
        name: 'web-research',
        icon: 'globe',
        version: '2.0.1',
        trust: 'Trusted',
        description: 'Search the web, cross-check sources, and synthesize findings with citations.',
        keywords: ['research', 'search', 'sources'],
        usage_hint: 'Try: "research the best CI providers for a Rust monorepo".',
      },
      {
        name: 'daily-briefing',
        icon: 'sunrise',
        version: '0.9.4',
        trust: 'Installed',
        description: 'Compose a morning briefing from calendar, email, and open tasks.',
        keywords: ['briefing', 'morning', 'digest'],
        install_source_url: 'https://clawhub.ai/skills/nearai/daily-briefing',
      },
    ],
  };

  // ClawHub catalog backing Skills discovery (implied: POST
  // /api/skills/search { query } -> { catalog: [{ slug, name, description,
  // owner, version, stars, downloads, updatedAt, installed }], installed:
  // [...] }). An empty query returns the featured shelf.
  var SKILL_CATALOG = [
    { slug: 'nearai/inbox-zero', name: 'inbox-zero', icon: 'inbox', version: '1.4.2', owner: 'nearai', stars: 412, downloads: 12800, updatedAt: Date.now() - 3 * 86400000, description: 'Triage email into Action / FYI / Ignore, draft replies, and keep the inbox at zero.' },
      { slug: 'nearai/meeting-prep', name: 'meeting-prep', icon: 'calendar-clock', version: '2.1.0', owner: 'nearai', stars: 356, downloads: 9400, updatedAt: Date.now() - 6 * 86400000, description: 'Brief you on attendees, company context, and recent news before every meeting.' },
    { slug: 'clawhub/changelog-writer', name: 'changelog-writer', icon: 'file-diff', version: '0.8.0', owner: 'clawhub', stars: 288, downloads: 7600, updatedAt: Date.now() - 12 * 86400000, description: 'Turn merged PRs into a crisp weekly changelog, grouped by feature area.' },
    { slug: 'community/kpi-digest', name: 'kpi-digest', icon: 'bar-chart', version: '1.0.3', owner: 'community', stars: 190, downloads: 5100, updatedAt: Date.now() - 20 * 86400000, description: 'Pull metrics from Sheets or an API and post a formatted digest to your channel.' },
    { slug: 'community/pr-review-buddy', name: 'pr-review-buddy', icon: 'git-pull-request', version: '0.6.1', owner: 'community', stars: 173, downloads: 4300, updatedAt: Date.now() - 8 * 86400000, description: 'First-pass review notes on open pull requests: risky diffs, missing tests, nits.' },
    { slug: 'nearai/site-monitor', name: 'site-monitor', icon: 'radar', version: '1.1.0', owner: 'nearai', stars: 240, downloads: 6900, updatedAt: Date.now() - 15 * 86400000, description: 'Watch a page or endpoint and alert with a diff summary the moment it changes.' },
  ];

  // Registry entries backing Discover / the setup wizard / Integrations.
  // Implied: GET /api/extensions/registry -> { entries: [...] }
  var REGISTRY = [
    { name: 'telegram', display_name: 'Telegram', kind: 'wasm_channel', description: 'Chat with your agent from your phone.' },
    { name: 'slack', display_name: 'Slack', kind: 'wasm_channel', description: 'Bring your agent into your team workspace.' },
    { name: 'discord', display_name: 'Discord', kind: 'wasm_channel', description: 'Run your agent inside your community server.' },
    { name: 'whatsapp', display_name: 'WhatsApp', kind: 'wasm_channel', description: 'Message your agent like any other contact.' },
    { name: 'gmail', display_name: 'Gmail', kind: 'wasm_tool', description: 'Read, triage, label, and draft email.' },
    { name: 'google_calendar', display_name: 'Google Calendar', kind: 'wasm_tool', description: 'Schedule awareness and meeting prep.' },
    { name: 'google_sheets', display_name: 'Google Sheets', kind: 'wasm_tool', description: 'Append rows and build reports.' },
    { name: 'google_drive', display_name: 'Google Drive', kind: 'wasm_tool', description: 'Find, organize, and summarize files.' },
    { name: 'google_docs', display_name: 'Google Docs', kind: 'wasm_tool', description: 'Draft and edit documents.' },
    { name: 'notion', display_name: 'Notion', kind: 'mcp_server', description: 'Read and update docs and databases.' },
    { name: 'github', display_name: 'GitHub', kind: 'wasm_tool', description: 'Watch repos, releases, and issues.' },
    { name: 'linear', display_name: 'Linear', kind: 'wasm_tool', description: 'Create and update tickets from chat.' },
    { name: 'http', display_name: 'HTTP', kind: 'native', lucideIcon: 'globe', description: 'Fetch URLs and call APIs.' },
    { name: 'browser', display_name: 'Browser', kind: 'mcp_server', lucideIcon: 'app-window', description: 'Drive a headless browser for research.' },
  ];

  function getTurns(threadId) {
    if (!state.turnsByThread[threadId]) state.turnsByThread[threadId] = [];
    return state.turnsByThread[threadId];
  }

  function findThread(threadId) {
    for (var i = 0; i < state.threads.length; i++) {
      if (state.threads[i].id === threadId) return state.threads[i];
    }
    return null;
  }

  function installExtension(name, kind) {
    for (var i = 0; i < state.extensions.length; i++) {
      if (state.extensions[i].name === name) {
        state.extensions[i].onboarding_state = 'active';
        return state.extensions[i];
      }
    }
    var reg = null;
    for (var j = 0; j < REGISTRY.length; j++) {
      if (REGISTRY[j].name === name) { reg = REGISTRY[j]; break; }
    }
    var ext = {
      name: name,
      display_name: (reg && reg.display_name) || name,
      kind: kind || (reg && reg.kind) || 'wasm_tool',
      onboarding_state: 'active',
      activation_status: 'active',
      enabled: true,
    };
    state.extensions.push(ext);
    return ext;
  }

  // Integrations "connected" during the mocked marketing-site onboarding
  // (?integrations=gmail,slack) surface as installed+active extensions so
  // every surface (wizard, Integrations, scenario preconditions) agrees.
  // Seeded lazily on the first API call: landing.js (which parses the URL
  // params into sessionStorage) runs after this module, but before any
  // fetch resolves.
  var handoffSeeded = false;
  function seedHandoffIntegrations() {
    if (handoffSeeded) return;
    handoffSeeded = true;
    try {
      var handoffRaw = sessionStorage.getItem('ironclaw_nux_connected_integrations');
      var handoffIds = handoffRaw ? JSON.parse(handoffRaw) : [];
      if (Array.isArray(handoffIds)) {
        handoffIds.forEach(function(id) { installExtension(id); });
      }
    } catch (e) { /* no handoff state */ }
  }

  function isConnected(name) {
    for (var i = 0; i < state.extensions.length; i++) {
      if (state.extensions[i].name === name
          && state.extensions[i].onboarding_state === 'active') return true;
    }
    return false;
  }

  // Workspace files backing the Workspace editor (implied: the gateway's
  // memory store, ~/.ironclaw/workspace). In-memory; writes persist for
  // the session so the editor round-trips.
  var MEMORY_FILES = {
    'README.md': '# Workspace\n\nThis is your agent\u2019s working memory \u2014 notes it keeps, preferences it learns, and files it produces while working for you.\n\n- **profile.md** \u2014 who you are, how you like things done\n- **preferences.md** \u2014 standing instructions the agent honors\n- **notes/** \u2014 running notes from tasks and conversations\n- **reports/** \u2014 artifacts the agent generates on a schedule\n\nEdit anything here \u2014 the agent reads this on every turn.',
    'profile.md': '# Profile\n\n- **Name:** Demo User\n- **Timezone:** America/Los_Angeles\n- **Work hours:** 9:00\u201318:00, no meetings before 10:00\n- **Team:** Priya (design), Marcus (infra), Dana (growth)\n\n## Communication\n\n- Keep summaries under five bullets.\n- Prefer direct answers over hedging.',
    'preferences.md': '# Standing preferences\n\n1. Deliver the morning briefing at **9:00** sharp.\n2. Never auto-send email \u2014 draft and ask.\n3. Alert me on Telegram for anything urgent after hours.\n4. Weekly spend cap: **$20** without checking in.',
    'notes/meetings.md': '# Meeting notes\n\n## 2026-07-01 \u2014 Growth sync\n\n- Signups +12% WoW; activation flat.\n- Dana to run onboarding experiment; agent to report daily.\n\n## 2026-06-28 \u2014 Infra review\n\n- Migration to the new queue done; watch p99 this week.',
    'notes/ideas.md': '# Ideas backlog\n\n- Auto-label GitHub issues by component.\n- Weekly "what changed" digest for the whole workspace.\n- Invoice parser \u2192 Sheets pipeline (waiting on Sheets access).',
    'reports/2026-07-01-briefing.md': '# Morning briefing \u2014 July 1\n\n**Calendar:** 2 meetings (Growth sync 10:00, 1:1 with Priya 14:00)\n\n**Email:** 3 actionable \u2014 MSA redlines (Maya), Stripe invoice, recruiting slot\n\n**Tasks:** keyword monitor quiet; deploy watcher all green.',
    'data/kpis.csv': 'week,signups,activation,revenue\n2026-06-08,412,0.38,8210\n2026-06-15,455,0.39,8630\n2026-06-22,491,0.37,9120\n2026-06-29,548,0.41,9880',
  };

  // Directory listing over the flat MEMORY_FILES map.
  function memoryList(dirPath) {
    var prefix = dirPath ? dirPath.replace(/\/$/, '') + '/' : '';
    var dirs = {};
    var files = [];
    Object.keys(MEMORY_FILES).forEach(function(p) {
      if (prefix && p.indexOf(prefix) !== 0) return;
      var rest = p.slice(prefix.length);
      if (!prefix && rest.indexOf('/') !== -1) {
        dirs[rest.split('/')[0]] = true;
        return;
      }
      if (prefix && rest.indexOf('/') !== -1) {
        dirs[rest.split('/')[0]] = true;
        return;
      }
      files.push({ name: rest, path: p, is_dir: false });
    });
    var entries = Object.keys(dirs).sort().map(function(d) {
      return { name: d, path: prefix + d, is_dir: true };
    });
    files.sort(function(a, b) { return a.name.localeCompare(b.name); });
    return entries.concat(files);
  }

  // Human-friendly thread title from the first message (implied: the real
  // backend titles threads via a cheap LLM pass after the first turn; the
  // mock approximates it deterministically). Collapses whitespace, strips
  // wrapping quotes and trailing punctuation, cuts at a word boundary, and
  // capitalizes — "every morning at 9am, send…" -> "Every morning at 9am".
  function deriveThreadTitle(content) {
    var text = String(content || '')
      .replace(/\s+/g, ' ')
      .replace(/^["'\u201c\u2018]+|["'\u201d\u2019]+$/g, '')
      .trim();
    if (!text) return null;
    // First clause reads better than a mid-sentence cut.
    var clause = text.split(/(?<=[.!?])\s/)[0];
    if (clause.length > 48) {
      var cut = clause.slice(0, 48);
      var lastSpace = cut.lastIndexOf(' ');
      clause = (lastSpace > 24 ? cut.slice(0, lastSpace) : cut) + '\u2026';
    }
    clause = clause.replace(/[,;:.\s]+$/, '');
    return clause.charAt(0).toUpperCase() + clause.slice(1);
  }

  function createMission(spec) {
    var mission = {
      id: uid('mission'),
      name: spec.name,
      goal: spec.goal,
      status: 'Active',
      cadence_type: spec.cadenceType || 'cron',
      cadence_description: spec.cadence,
      thread_count: 1,
      created_at: iso(0),
      updated_at: iso(0),
      threads: [],
    };
    var run = {
      id: uid('ethread'),
      mission_id: mission.id,
      title: spec.firstRunTitle || (spec.name + ' — first run'),
      goal: spec.firstRunTitle || (spec.name + ' — first run'),
      state: 'Done',
      created_at: iso(20000),
      updated_at: iso(2000),
      completed_at: iso(2000),
      total_cost_usd: 0.03,
    };
    mission.threads.push(run);
    state.missions.push(mission);
    state.engineThreads.push(run);
    return mission;
  }

  // --------------------------------------------------------------------
  // Mock SSE (implied: GET /api/chat/events — typed SSE stream, see
  // src/channels/web/types.rs `SseEvent`)
  // --------------------------------------------------------------------

  var chatEventSource = null;
  var sseEventCounter = 0;

  // Seeded gateway log stream (implied: GET /api/logs/events, `log` SSE
  // events shaped { timestamp, level, target, message }). A believable
  // backlog plays immediately; a slow trickle keeps the view alive.
  var LOG_SEED = [
    ['INFO',  'gateway::server',      'listening on 0.0.0.0:3000 (tls off, behind proxy)'],
    ['INFO',  'gateway::auth',        'session established for user demo-user (oauth: google)'],
    ['DEBUG', 'engine::scheduler',    'cron tick — 1 mission due within the next hour'],
    ['INFO',  'engine::mission',      'mission daily-briefing: run started (trigger: cron 0 9 * * *)'],
    ['DEBUG', 'tools::gmail',         'list_messages: 12 new since 2026-07-01T18:00:00Z'],
    ['INFO',  'tools::gmail',         'apply_labels: 3x Action, 5x FYI, 4x Ignore'],
    ['DEBUG', 'llm::router',          'claw-demo-1 selected (ctx 12.4k tokens, est $0.0042)'],
    ['INFO',  'engine::mission',      'mission daily-briefing: run finished in 18.2s ($0.03)'],
    ['WARN',  'tools::http',          'GET https://example.com/health took 2140ms (budget 2000ms)'],
    ['INFO',  'gateway::sse',         'client connected (1 active stream)'],
    ['DEBUG', 'workspace::memory',    'read profile.md (412 bytes) for turn context'],
    ['INFO',  'channels::telegram',   'pairing code issued for @demo_user'],
    ['ERROR', 'tools::http',          'GET https://api.example.com/v2/usage -> 503 Service Unavailable (retry 1/3 in 2s)'],
    ['INFO',  'tools::http',          'GET https://api.example.com/v2/usage -> 200 on retry (312ms)'],
    ['DEBUG', 'engine::gate',         'no approval gates pending for thread thread-demo-1'],
    ['INFO',  'skills::registry',     'refreshed ClawHub index (6 entries, 210ms)'],
    ['WARN',  'llm::costs',           'daily spend $0.42 approaching soft cap $0.50'],
    ['DEBUG', 'gateway::static',      'served app.js (etag hit, 0ms)'],
    ['INFO',  'engine::monitor',      'keyword-monitor: scan complete, 0 new mentions'],
    ['DEBUG', 'db::libsql',           'checkpoint: wal 128kb -> main (4ms)'],
  ];
  var LOG_TRICKLE = [
    ['DEBUG', 'engine::scheduler',    'cron tick — nothing due'],
    ['INFO',  'engine::monitor',      'keyword-monitor: scan complete, 0 new mentions'],
    ['DEBUG', 'gateway::sse',         'keepalive ping (stream healthy)'],
    ['INFO',  'tools::http',          'GET https://example.com/health -> 200 OK (134ms)'],
    ['DEBUG', 'workspace::memory',    'context assembled in 6ms (3 files, 2.1kb)'],
  ];
  var logEventSourceMock = null;
  var logTrickleTimer = null;
  // The SPA reconnects the log stream on tab switches; the DOM keeps the
  // already-rendered rows, so replay the backlog only once per page load.
  var logSeedPlayed = false;

  function emitLogEntry(target, level, message, msAgo) {
    if (!logEventSourceMock || logEventSourceMock.readyState === 2) return;
    logEventSourceMock._emit('log', {
      timestamp: iso(msAgo || 0),
      level: level,
      target: target,
      message: message,
    });
  }

  function startLogStream(source) {
    logEventSourceMock = source;
    // Backlog: oldest first so the newest lands on top of the view.
    if (!logSeedPlayed) {
      logSeedPlayed = true;
      var span = 30 * 60000;
      LOG_SEED.forEach(function(row, i) {
        var age = span - Math.round((i / LOG_SEED.length) * span);
        setTimeout(function() { emitLogEntry(row[1], row[0], row[2], age); }, 40 + i * 12);
      });
    }
    if (logTrickleTimer) clearInterval(logTrickleTimer);
    logTrickleTimer = setInterval(function() {
      var row = LOG_TRICKLE[Math.floor(Math.random() * LOG_TRICKLE.length)];
      emitLogEntry(row[1], row[0], row[2], 0);
    }, 4000 + Math.random() * 4000);
  }

  function MockEventSource(url) {
    this.url = String(url || '');
    this.readyState = 0;
    this.onopen = null;
    this.onerror = null;
    this.onmessage = null;
    this._listeners = {};
    var self = this;
    if (this.url.indexOf('/api/chat/events') === 0) {
      chatEventSource = this;
    }
    if (this.url.indexOf('/api/logs/events') === 0) {
      setTimeout(function() { startLogStream(self); }, 60);
    }
    setTimeout(function() {
      if (self.readyState === 2) return;
      self.readyState = 1;
      if (typeof self.onopen === 'function') self.onopen({});
    }, 30);
  }
  MockEventSource.prototype.addEventListener = function(type, fn) {
    if (!this._listeners[type]) this._listeners[type] = [];
    this._listeners[type].push(fn);
  };
  MockEventSource.prototype.removeEventListener = function(type, fn) {
    var arr = this._listeners[type] || [];
    var idx = arr.indexOf(fn);
    if (idx !== -1) arr.splice(idx, 1);
  };
  MockEventSource.prototype.close = function() {
    this.readyState = 2;
    if (chatEventSource === this) chatEventSource = null;
    if (logEventSourceMock === this) {
      logEventSourceMock = null;
      if (logTrickleTimer) { clearInterval(logTrickleTimer); logTrickleTimer = null; }
    }
  };
  MockEventSource.prototype._emit = function(type, data) {
    var event = {
      data: JSON.stringify(data),
      lastEventId: String(++sseEventCounter),
      type: type,
    };
    var arr = this._listeners[type] || [];
    for (var i = 0; i < arr.length; i++) {
      try { arr[i](event); } catch (e) { console.error('[demo sse]', type, e); }
    }
  };

  window.EventSource = MockEventSource;

  function emit(type, data) {
    if (chatEventSource) chatEventSource._emit(type, data);
  }

  // --------------------------------------------------------------------
  // Onboarding flows (ported verbatim from the /start prototype:
  // private-assistant achal/chat-first — flows.ts + proactive.ts). Each
  // use-case chip resolves to a flow: connect pitch → connect card →
  // cascade chips → "reading your world" stats → 3 draft-automation
  // proposal cards (Approve creates a real mission on the Tasks board).
  // Implied backend: these are the scripted event sequences the engine v2
  // agent loop produces; cards ride a `flow_card` SSE event and actions
  // land on POST /api/flows/action { action, provider|proposal }.
  // --------------------------------------------------------------------

  // `runsWhen` + `uses` feed the proposal-detail modal (trigger line + the
  // automation spec preview) — mirrors the /start prototype's automation view.
  var FLOW_PROPOSALS = {
    'digest': { icon: 'bell', title: 'A morning digest of what you actually care about', body: 'I read your recent threads and grouped everything into one brief \u2014 the launch, hiring, and anything time-sensitive.', details: ['3 new replies that need you today', '2 threads waiting on a decision', '1 invoice to approve before Friday'], suggestLine: "I'll send it every morning at 8:00am once you approve.", runsWhen: 'schedule \u00b7 every weekday at 08:00', uses: ['gmail', 'google_calendar'], mission: { name: 'Morning digest', cadence: 'Every weekday, 8:00', cadenceType: 'cron' } },
    'triage': { icon: 'inbox', title: 'Inbox triage + drafted replies', body: 'I sorted your unread mail into Action / FYI / Ignore and drafted replies for the ones that matter.', details: ['9 labeled Action, 24 FYI, 41 Ignore', '5 replies drafted in your voice', '2 newsletters auto-archived'], suggestLine: "Approve and I'll keep your inbox triaged automatically.", runsWhen: 'event \u00b7 on new email', uses: ['gmail'], mission: { name: 'Inbox triage', cadence: 'On new email', cadenceType: 'event' } },
    'meeting-prep': { icon: 'calendar-check', title: 'Briefs before every meeting', body: 'From your calendar, I prepared a one-pager for each upcoming meeting \u2014 company, attendees, and recent context.', details: ['Prepped your next 4 meetings', 'Flagged 1 conflict to resolve', 'Found a 30-min slot for the Dana sync'], suggestLine: "Approve and I'll deliver each brief 10 minutes ahead.", runsWhen: 'schedule \u00b7 10 min before each meeting', uses: ['google_calendar', 'gmail'], mission: { name: 'Meeting briefs', cadence: '10 min before meetings', cadenceType: 'event' } },
    'people': { icon: 'users', title: "Who to talk to \u2014 and I've already held the time", body: "I read your docs and call transcripts. Here's who to sync with this week \u2014 I drafted the invites and blocked the slots \u2014 plus the calls only you can make.", details: ['Priya \u2014 Series A term sheet (today)', 'Marcus \u2014 close out the eng hiring loop', 'Dana \u2014 finalize the launch date & GTM', 'Decision needed: approve the 3 pricing tiers'], suggestLine: '3 invites drafted on your calendar \u2014 approve to send.', runsWhen: 'schedule \u00b7 weekly on Monday at 09:00', uses: ['google_calendar', 'gmail', 'notion'], mission: { name: 'Weekly sync scheduler', cadence: 'Every Monday, 8:00', cadenceType: 'cron' } },
    'twitter': { icon: 'trending-up', title: "You're growing on X \u2014 I got a head start", body: "I studied your voice and what's been landing, then put together a week of content and engagement.", details: ['Drafted 3 tweets in your voice', 'Replied to 2 mentions about you', 'Lined up 5 relevant accounts to follow'], suggestLine: 'All drafted \u2014 approve any to go live. Nothing posts without you.', runsWhen: 'manual \u00b7 on your approval', uses: [], mission: { name: 'X growth drafts', cadence: 'Daily', cadenceType: 'cron' } },
    'monitor': { icon: 'radar', title: 'Keep an eye on what matters', body: 'I can watch Hacker News, your repos, or your endpoints and ping you the moment something happens.', details: ['Watching mentions of your product', 'Alerting on failed deploys', 'Daily summary of new releases'], suggestLine: "Approve any and I'll start watching.", runsWhen: 'schedule \u00b7 every 5 minutes', uses: ['slack', 'telegram', 'github'], mission: { name: 'Keyword monitor', cadence: 'Every 10 minutes', cadenceType: 'cron' } },
    'gh-releases': { icon: 'git-branch', title: 'Release notes, summarized', body: 'I watch your repos and turn each new release into a clean summary for your channel.', details: ['nearai/ironclaw v0.31 \u2014 3 days ago', '12 PRs merged since the last release', '1 breaking change flagged'], suggestLine: "Approve and I'll summarize each release as it ships.", runsWhen: 'event \u00b7 on new release', uses: ['github'], mission: { name: 'Release tracker', cadence: 'On new release', cadenceType: 'event' } },
    'gh-reviews': { icon: 'git-pull-request', title: 'Your review queue, triaged', body: "PRs waiting on you, ranked by what's blocking the team.", details: ['4 PRs awaiting your review', '1 is blocking the launch branch', '2 have been stale for >3 days'], suggestLine: "Approve and I'll keep your review queue triaged.", runsWhen: 'schedule \u00b7 daily at 09:00', uses: ['github'], mission: { name: 'Review queue triage', cadence: 'Every day, 9:00', cadenceType: 'cron' } },
    'gh-ci': { icon: 'bug', title: 'CI watch + failure digests', body: 'When CI breaks on main, I find the failing job and the suspect commit before you ask.', details: ['main: green right now', 'Flaky test flagged: gateway_spec', 'Last failure traced to a config change'], suggestLine: "Approve and I'll watch CI and digest failures.", runsWhen: 'event \u00b7 on CI failure (branch: main)', uses: ['github'], mission: { name: 'CI watch', cadence: 'On CI failure', cadenceType: 'event' } },
    'slack-triage': { icon: 'messages-square', title: 'Mentions + threads, triaged', body: 'I sort your mentions into what needs you now vs. later, and draft replies.', details: ['17 mentions \u2014 5 need you today', '3 threads awaiting a decision', 'Drafted 4 replies in your voice'], suggestLine: "Approve and I'll keep your mentions triaged.", runsWhen: 'event \u00b7 on mention', uses: ['slack'], mission: { name: 'Slack mention triage', cadence: 'Continuous', cadenceType: 'event' } },
    'slack-digest': { icon: 'newspaper', title: 'A daily team digest', body: 'One post each morning summarizing what moved across your channels.', details: ['#launch: GTM doc finalized', '#eng: 2 incidents resolved', '#sales: 3 new deals in'], suggestLine: "Approve and I'll post a daily digest at 9am.", runsWhen: 'schedule \u00b7 daily at 09:00', uses: ['slack'], mission: { name: 'Team digest', cadence: 'Every day, 9:00', cadenceType: 'cron' } },
    'slack-tasks': { icon: 'check-square', title: 'Turn messages into tasks', body: 'When someone says "create task: \u2026" I open a ticket and confirm back with the link.', details: ['6 tasks captured this week', 'Routed to the right project', 'Nudged 2 that went overdue'], suggestLine: "Approve and I'll turn requests into tracked tasks.", runsWhen: 'event \u00b7 on message matching "create task:"', uses: ['slack', 'linear'], mission: { name: 'Task capture', cadence: 'On matching message', cadenceType: 'event' } },
    'tg-health': { icon: 'activity', title: 'Deployment health watch', body: 'I ping your endpoints and alert you on Telegram the moment anything returns non-200.', details: ['Watching 3 endpoints', 'Checking every 5 minutes', 'Alerts routed to Telegram'], suggestLine: "Approve and I'll watch your endpoints 24/7.", runsWhen: 'schedule \u00b7 every 5 minutes', uses: ['telegram'], mission: { name: 'Deployment health watch', cadence: 'Every 5 minutes', cadenceType: 'cron' } },
    'tg-incident': { icon: 'alert-triangle', title: 'Incident first-responder', body: 'When something breaks, I gather the logs and the suspect change and ping you with a one-line summary.', details: ['Hooks into your deploys', 'Pulls logs + recent commits', 'One-line summary to Telegram'], suggestLine: "Approve and I'll be your incident first-responder.", runsWhen: 'event \u00b7 on deploy failure', uses: ['telegram', 'github'], mission: { name: 'Incident first-responder', cadence: 'On deploy failure', cadenceType: 'event' } },
  };

  var NUX_FLOWS = {
    gmail: {
      lead: 'gmail',
      leadLabel: 'Gmail',
      cascade: [
        { id: 'gmail', label: 'Gmail' },
        { id: 'google_calendar', label: 'Google Calendar' },
        { id: 'google_drive', label: 'Google Drive' },
        { id: 'google_docs', label: 'Google Docs' },
        { id: 'notion', label: 'Notion', via: 'via Google' },
        { id: 'slack', label: 'Slack', via: 'via Google' },
      ],
      cascadeLabel: 'Connecting your stack \u00b7 one sign-in',
      connectCopy: "Love it \u2014 that's exactly the kind of thing I take off your plate. The fastest way to make it real: connect Gmail. From that one sign-in I can reach the rest of your stack \u2014 Calendar, Drive, Notion, Slack \u2014 and get to work. Your credentials stay in an encrypted vault I never see.",
      connectedCopy: 'Gmail connected \u2014 and your private agent is live on free credits. Encrypted enclave, credentials sealed in a vault the model never sees.',
      cascadeCopy: "Here's the trick: that one Google sign-in is all I need. I'm using it to connect the rest of your stack right now \u2014 no other logins, no setup.",
      readingCopy: "Now I'm reading everything you've just given me access to \u2014",
      readingStats: [
        { value: '1,284', label: 'emails' },
        { value: '37', label: 'Notion docs' },
        { value: '12', label: 'transcripts' },
      ],
      learned: ['Fundraising (Series A)', 'Hiring \u2014 eng loop', 'Launch + GTM', 'Top people: Priya, Marcus, Dana', 'Growing on X'],
      proposals: ['digest', 'people', 'twitter'],
    },
    github: {
      lead: 'github',
      leadLabel: 'GitHub',
      cascade: [
        { id: 'github', label: 'GitHub' },
        { id: 'telegram', label: 'Telegram' },
      ],
      cascadeLabel: 'Wiring up your repos + where to reach you',
      connectCopy: "On it \u2014 that's a great first one. Connect GitHub and I'll watch your repos, PRs, and releases and get to work. Read-only where it counts, and your token stays in an encrypted vault the model never sees.",
      connectedCopy: 'GitHub connected \u2014 your private agent is live on free credits, token sealed in the vault.',
      cascadeCopy: "Connected. I'm pulling in your repos and wiring up where to reach you \u2014",
      readingCopy: 'Scanning your repos and recent activity \u2014',
      readingStats: [
        { value: '14', label: 'repos' },
        { value: '23', label: 'open PRs' },
        { value: '188', label: 'CI runs' },
      ],
      learned: ['Active: nearai/ironclaw', 'Release cadence: weekly', 'Hot area: gateway', 'Reviewers: you, Marcus', 'CI flaky on main'],
      proposals: ['gh-releases', 'gh-reviews', 'gh-ci'],
    },
    slack: {
      lead: 'slack',
      leadLabel: 'Slack',
      cascade: [
        { id: 'slack', label: 'Slack' },
        { id: 'linear', label: 'Linear' },
      ],
      cascadeLabel: 'Plugging into your channels + task tracker',
      connectCopy: "Love it. Connect Slack and I'll plug into your channels and DMs and run your team ops from there. Your credentials stay sealed in an encrypted vault the model never sees.",
      connectedCopy: 'Slack connected \u2014 your private agent is live on free credits, credentials sealed in the vault.',
      cascadeCopy: 'Connected. Plugging into your channels and your task tracker \u2014',
      readingCopy: 'Reading your channels and recent threads \u2014',
      readingStats: [
        { value: '28', label: 'channels' },
        { value: '1,142', label: 'messages today' },
        { value: '17', label: 'mentions' },
      ],
      learned: ['Busiest: #launch', '17 mentions today', 'Recurring: standup, bugs', 'Collaborators: Priya, Dana', '2 decisions pending'],
      proposals: ['slack-triage', 'slack-digest', 'slack-tasks'],
    },
    telegram: {
      lead: 'telegram',
      leadLabel: 'Telegram',
      cascade: [
        { id: 'telegram', label: 'Telegram' },
        { id: 'github', label: 'GitHub' },
      ],
      cascadeLabel: 'Setting up your watch + where to reach you',
      connectCopy: "On it. Connect Telegram and I'll reach you there and keep watch on what you care about \u2014 your credentials stay sealed in an encrypted vault.",
      connectedCopy: 'Telegram connected \u2014 your private agent is live on free credits.',
      cascadeCopy: 'Connected. Setting up your watch and where to reach you \u2014',
      readingCopy: 'Setting up your monitors \u2014',
      readingStats: [
        { value: '3', label: 'endpoints' },
        { value: '288', label: 'checks/day' },
        { value: '0', label: 'alerts' },
      ],
      learned: ['Health endpoint found', 'Alert via Telegram', 'Baseline: 200 OK', 'Check every 5 min', 'On-call: you'],
      proposals: ['tg-health', 'monitor', 'tg-incident'],
    },
  };

  // Hover blurbs for cascade chips (one line on what the agent can do with
  // each tool — mirrors the prototype's tooltip copy).
  var CASCADE_BLURBS = {
    gmail: 'Read, send, and manage email \u2014 without the model ever seeing your credentials.',
    google_calendar: 'Schedules, conflicts, and prep \u2014 handled before you ask.',
    google_drive: 'Files found, organized, and summarized on demand.',
    google_docs: 'Drafts, edits, and summaries straight into Docs.',
    notion: 'Docs and databases your agent can read and update.',
    slack: 'Talk to your agent where your team already works.',
    telegram: 'Your agent in your pocket \u2014 on your phone, on the go.',
    github: 'Repos, issues, PRs, and releases \u2014 watched and summarized.',
    linear: 'Issues filed, triaged, and tracked from chat.',
  };

  // Map a lead integration id onto a flow bucket (mirrors the prototype).
  function flowForLead(lead) {
    if (lead === 'github') return NUX_FLOWS.github;
    if (lead === 'slack' || lead === 'linear' || lead === 'discord') return NUX_FLOWS.slack;
    if (lead === 'telegram') return NUX_FLOWS.telegram;
    return NUX_FLOWS.gmail;
  }

  // Naive keyword inference: free-typed prompt -> lead integration id
  // (ported from the /start prototype's inferIntegrationsFromPrompt; first
  // hit wins, gmail is the default lead).
  function inferLeadFromPrompt(text) {
    var p = String(text || '').toLowerCase();
    if (/(email|inbox|gmail|invoice)/.test(p)) return 'gmail';
    if (/(calendar|meeting|schedule|invite)/.test(p)) return 'gmail';
    if (/(github|repo\b|pull request|\bpr\b|release|commit|\bci\b)/.test(p)) return 'github';
    if (/slack/.test(p)) return 'slack';
    if (/(linear|ticket)/.test(p)) return 'slack';
    if (/(telegram|phone|alert|ping|monitor|watch|endpoint|deploy)/.test(p)) return 'telegram';
    return 'gmail';
  }

  // True once any flow-lead integration is connected — the moment the story
  // shifts from "get connected" to "the agent is on the job".
  function anyLeadConnected() {
    return isConnected('gmail') || isConnected('github')
      || isConnected('slack') || isConnected('telegram');
  }

  // --------------------------------------------------------------------
  // Scripted agent scenarios
  // --------------------------------------------------------------------
  //
  // Each scenario is a timed script of SSE events + state side effects,
  // simulating what the real agent loop (engine v2) produces for the
  // matching intent. Steps: thinking | tool | respond | suggest | mission |
  // connect. `respond` streams the markdown, records the turn, and emits
  // the terminal `response` + `status: Done` pair.

  function scenarioForMessage(content) {
    var text = String(content || '').toLowerCase();

    function usedCase(id) {
      var cases = window.NUX_DATA.useCases;
      for (var i = 0; i < cases.length; i++) {
        if (cases[i].id === id) return cases[i];
      }
      return null;
    }

    // Use-case prompts (the suggested chips) run the full onboarding flow
    // ported from the /start prototype: connect → cascade → reading →
    // draft-automation proposals.
    var cases = window.NUX_DATA.useCases || [];
    for (var uc = 0; uc < cases.length; uc++) {
      var normalized = cases[uc].prompt.toLowerCase().replace(/\s+/g, ' ').trim();
      if (text.replace(/\s+/g, ' ').trim() === normalized) {
        return flowScenario(flowForLead((cases[uc].integrations || [])[0] || 'gmail'));
      }
    }

    // Slash commands and capability question.
    if (/what can you do|what do you do|help me get started|capabilities/.test(text)) {
      return capabilityScenario();
    }
    // Connect a channel/integration by name.
    var connectMatch = text.match(/connect (?:to |me to )?(telegram|slack|discord|whatsapp|gmail|github|linear|google calendar|google sheets)/);
    if (connectMatch) {
      return connectScenario(connectMatch[1].replace(' ', '_'));
    }
    if (/triage my inbox|label new emails/.test(text)) {
      return flowScenario(NUX_FLOWS.gmail);
    }
    if (/briefing/.test(text)) {
      return flowScenario(NUX_FLOWS.gmail);
    }
    if (/hacker news|keyword|appears on/.test(text) && /send|watch|summar/.test(text)) {
      return keywordMonitorScenario(usedCase('keyword-monitor'));
    }
    if (/ping http|\bhealth\b|non-200|watch.*endpoint/.test(text)) {
      return deployWatcherScenario(usedCase('deploy-watcher'));
    }
    if (/watch the .*github|new releases/.test(text)) {
      return releaseTrackerScenario(usedCase('release-tracker'));
    }
    if (/summarize the top stories/.test(text)) {
      return hnSummaryScenario();
    }
    // Free-typed asks (prototype parity): before anything is connected, any
    // request routes into the matching connect→cascade→draft flow; once the
    // stack is connected, acknowledge and "get to work" instead.
    if (!anyLeadConnected()) {
      return flowScenario(flowForLead(inferLeadFromPrompt(text)));
    }
    return ackScenario(content);
  }

  // The signature prototype flow, expressed as engine steps. `say` posts a
  // full agent bubble (with a typing beat before it); `card` emits a
  // flow_card; `pause` stops the script until POST /api/flows/action
  // resumes it (e.g. the user clicks Connect on the connect card).
  function flowScenario(flow) {
    var alreadyConnected = isConnected(flow.lead);
    var steps = [{ t: 400, thinking: 'Thinking' }];

    if (!alreadyConnected) {
      steps.push({ t: 900, say: flow.connectCopy });
      steps.push({ t: 350, card: {
        kind: 'connect',
        provider: flow.lead,
        title: 'Connect ' + flow.leadLabel,
        caption: 'One sign-in connects your whole stack \u2014 credentials stay encrypted',
        icon: '/icons/integrations/' + flow.lead + '.png',
      } });
      steps.push({ pause: 'connect:' + flow.lead });
      steps.push({ t: 500, say: flow.connectedCopy });
    } else {
      steps.push({ t: 800, say: flow.leadLabel + ' is already connected \u2014 going straight to work.' });
    }

    steps.push({ t: 600, say: flow.cascadeCopy });
    steps.push({ t: 300, card: {
      kind: 'cascade',
      label: flow.cascadeLabel,
      chips: flow.cascade.map(function(c) {
        return {
          id: c.id, label: c.label, via: c.via || null,
          icon: '/icons/integrations/' + c.id + '.png',
          blurb: CASCADE_BLURBS[c.id] || null,
        };
      }),
    } });
    steps.push({ t: 1600, connectMany: flow.cascade.map(function(c) { return c.id; }) });
    steps.push({ t: 700, say: flow.readingCopy });
    steps.push({ t: 300, card: { kind: 'reading', stats: flow.readingStats, learned: flow.learned } });
    steps.push({ t: 1800, say: "Done. In your first hour \u2014 off that one " + flow.leadLabel + " connection \u2014 here's what I've drafted for you. Approve each one and I'll make it live." });
    flow.proposals.forEach(function(pid) {
      var p = FLOW_PROPOSALS[pid];
      steps.push({ t: 900, card: {
        kind: 'proposal',
        proposal: pid,
        icon: p.icon,
        title: p.title,
        body: p.body,
        details: p.details,
        suggestLine: p.suggestLine,
        runsWhen: p.runsWhen,
        uses: p.uses,
      } });
    });
    steps.push({ t: 900, say: "That's your first hour \u2014 done. You're on free credits, which cover today. Approve any draft above and it becomes a live task under **Tasks**." });
    // Land the moment (sparkle flourish rides on the upgrade card), then
    // surface the plan-upgrade path — the last beat of the scripted journey.
    steps.push({ t: 600, say: 'To keep all of this running, pick a plan whenever you\u2019re ready.' });
    steps.push({ t: 300, card: { kind: 'upgrade' } });
    steps.push({ t: 300, suggest: ['Show my tasks', 'What else can you take off my plate?'] });
    return steps;
  }

  // Post-connect acknowledgement for free-typed asks (prototype parity).
  function ackScenario(content) {
    var quoted = String(content || '').slice(0, 120);
    return [
      { t: 400, thinking: 'Thinking' },
      { t: 900, say: 'On it \u2014 I\u2019ll run "' + quoted + '" and post the results right here, then keep it running automatically.' },
      { t: 300, suggest: ['Show my tasks', 'What else can you take off my plate?'] },
    ];
  }

  function capabilityScenario() {
    return [
      { t: 350, thinking: 'Thinking' },
      { t: 1100, respond:
        'I\'m your personal agent — I chat here, but I can also work in the background. A few things I can do right now:\n\n'
        + '- **Automations** — "Every morning at 9am, send me a briefing." I\'ll create a scheduled task you can see under **Tasks**.\n'
        + '- **Monitoring** — watch Hacker News, a website, an endpoint, or a GitHub repo and alert you the moment something changes.\n'
        + '- **Integrations** — connect Gmail, Slack, Telegram, Google Calendar and more under **Integrations**, then ask me to use them.\n'
        + '- **Research & summaries** — "Summarize the top stories on Hacker News right now."\n\n'
        + 'Try one of the suggestions below, or just describe what you want in your own words.' },
      { t: 400, suggest: [
        'Set up a daily 9am briefing with my calendar and inbox',
        'If "IronClaw" appears on Hacker News, send a summary to me here',
        'Connect to Telegram so I can message you from my phone',
      ] },
    ];
  }

  function connectScenario(name) {
    var reg = null;
    for (var i = 0; i < REGISTRY.length; i++) {
      if (REGISTRY[i].name === name) { reg = REGISTRY[i]; break; }
    }
    var label = (reg && reg.display_name) || name;
    return [
      { t: 300, thinking: 'Setting up ' + label },
      { t: 900, tool: { name: 'extension_install', preview: 'Installed ' + label + ' from the registry and activated it for this workspace.' } },
      { t: 700, connect: name },
      { t: 500, respond:
        '**' + label + ' is connected.** \u2713\n\n'
        + 'You can see and manage it under **Integrations** in the sidebar. '
        + (reg && reg.kind === 'wasm_channel'
          ? 'From now on you can message me from ' + label + ' and I\'ll answer there too — same memory, same tasks.'
          : 'I can now use ' + label + ' when a task needs it — just ask in plain language.') },
      { t: 400, suggest: [
        'Set up a daily 9am briefing with my calendar and inbox',
        'What can you do for me?',
      ] },
    ];
  }

  function inboxTriageScenario(useCase) {
    if (!isConnected('gmail')) {
      return [
        { t: 350, thinking: 'Checking connected integrations' },
        { t: 900, respond:
          'I can do that as soon as **Gmail** is connected — I don\'t have access to your inbox yet.\n\n'
          + 'Open **Integrations** in the sidebar and connect Gmail (takes ~30 seconds), or just tell me "connect Gmail" and I\'ll set it up. '
          + 'Once it\'s connected I\'ll label new email as **Action**, **FYI**, or **Ignore** and summarize the Action ones for you.' },
        { t: 400, suggest: ['Connect Gmail', 'What can you do for me?'] },
      ];
    }
    return [
      { t: 350, thinking: 'Reading your inbox' },
      { t: 1300, tool: { name: 'gmail_list_messages', preview: '12 new messages since yesterday 18:00. 3 look actionable, 5 informational, 4 promotional.' } },
      { t: 1500, tool: { name: 'gmail_apply_labels', preview: 'Applied labels: 3\u00d7 Action, 5\u00d7 FYI, 4\u00d7 Ignore.' } },
      { t: 600, mission: {
        name: 'Inbox triage',
        goal: (useCase && useCase.prompt) || 'Label inbound email as Action / FYI / Ignore and summarize the actionable ones.',
        cadence: 'On new email',
        cadenceType: 'event',
        firstRunTitle: 'Triage 12 new messages',
      } },
      { t: 700, respond:
        'Done — I triaged **12 new messages** and set up ongoing triage for anything new. Here are your **Action** items:\n\n'
        + '1. **Maya (Legal)** — the MSA redlines are back; she needs your sign-off by Thursday.\n'
        + '2. **Stripe** — your March invoice failed to charge; card needs updating.\n'
        + '3. **Jordan (Recruiting)** — final-round candidate available Tue/Wed, asking which slot to book.\n\n'
        + 'The other 9 are labeled **FYI** (5) and **Ignore** (4). I\'ve created an **Inbox triage** task that runs on every new email — see it under **Tasks**.' },
      { t: 400, suggest: ['Draft a reply to Maya', 'Show my tasks', 'Set up a daily 9am briefing'] },
    ];
  }

  function briefingScenario(useCase) {
    return [
      { t: 350, thinking: 'Planning the automation' },
      { t: 1200, tool: { name: 'schedule_create', preview: 'Cron created: daily 09:00 local (' + (Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC') + '). Trigger wired to briefing composer.' } },
      { t: 1400, tool: { name: 'briefing_compose', preview: 'Dry run OK — calendar (2 events), inbox (3 actionable), open tasks (2).' } },
      { t: 600, mission: {
        name: 'Daily morning briefing',
        goal: (useCase && useCase.prompt) || 'Every morning at 9am, deliver a briefing with calendar, important email, and open tasks.',
        cadence: 'Every day, 9:00',
        cadenceType: 'cron',
        firstRunTitle: 'Dry-run: compose today\'s briefing',
      } },
      { t: 800, respond:
        'Your **daily briefing is live** — every morning at **9:00** I\'ll send you:\n\n'
        + '- **Calendar** — today\'s meetings with prep notes\n'
        + '- **Email** — only what needs action, summarized\n'
        + '- **Tasks** — what\'s open, what\'s blocked, what finished overnight\n\n'
        + 'I ran a dry run just now: you have **2 meetings** today, **3 actionable emails**, and **2 open tasks**. '
        + 'The automation lives under **Tasks** \u2192 *Daily morning briefing* — pause it, change the time, or tell me "make the briefing 8am" anytime.\n\n'
        + '*Tip: connect Telegram and I\'ll deliver it to your phone instead.*' },
      { t: 400, suggest: ['Show my tasks', 'Connect to Telegram', 'Make the briefing 8am'] },
    ];
  }

  function keywordMonitorScenario(useCase) {
    return [
      { t: 350, thinking: 'Setting up the monitor' },
      { t: 1300, tool: { name: 'http_fetch', preview: 'GET https://hn.algolia.com/api/v1/search?query=%22IronClaw%22 \u2192 200 (2 hits, both older than 7 days)' } },
      { t: 1200, tool: { name: 'monitor_create', preview: 'Watching Hacker News for "IronClaw", "NEAR AI" — checks every 10 minutes, notifies this thread on match.' } },
      { t: 600, mission: {
        name: 'Keyword monitor',
        goal: (useCase && useCase.prompt) || 'Watch Hacker News for "IronClaw" or "NEAR AI" and summarize new mentions here.',
        cadence: 'Every 10 minutes',
        cadenceType: 'cron',
        firstRunTitle: 'Baseline scan: 2 historical mentions',
      } },
      { t: 700, respond:
        '**Monitor armed.** I\'m watching Hacker News for **"IronClaw"** and **"NEAR AI"**, checking every 10 minutes.\n\n'
        + 'Baseline scan found 2 historical mentions (both >7 days old) — I\'ll only ping you about *new* ones, with a summary and a link. '
        + 'The monitor runs as a task — see **Tasks** \u2192 *Keyword monitor* to pause or tune it.' },
      { t: 400, suggest: ['Show my tasks', 'Also watch Reddit', 'Send alerts to Slack instead'] },
    ];
  }

  function deployWatcherScenario(useCase) {
    return [
      { t: 350, thinking: 'Checking the endpoint' },
      { t: 1200, tool: { name: 'http_fetch', preview: 'GET https://example.com/health \u2192 200 OK (134 ms)' } },
      { t: 1100, tool: { name: 'monitor_create', preview: 'Health check scheduled every 5 minutes; alert on non-200 or timeout >5s.' } },
      { t: 600, mission: {
        name: 'Deployment health watcher',
        goal: (useCase && useCase.prompt) || 'Ping https://example.com/health every 5 minutes and alert on non-200.',
        cadence: 'Every 5 minutes',
        cadenceType: 'cron',
        firstRunTitle: 'First check: 200 OK in 134 ms',
      } },
      { t: 700, respond:
        '**Watching it.** `https://example.com/health` responded **200 OK in 134 ms** just now.\n\n'
        + 'I\'ll ping it every **5 minutes** and alert you here the moment it returns anything but a 200 (or takes longer than 5s). '
        + 'Manage it under **Tasks** \u2192 *Deployment health watcher*.' },
      { t: 400, suggest: ['Show my tasks', 'Also alert me on Telegram', 'Check it hourly instead'] },
    ];
  }

  function releaseTrackerScenario(useCase) {
    return [
      { t: 350, thinking: 'Looking up the repo' },
      { t: 1200, tool: { name: 'github_releases', preview: 'nearai/ironclaw: latest release v0.9.2 (3 days ago). Subscribed to new releases.' } },
      { t: 600, mission: {
        name: 'Release tracker',
        goal: (useCase && useCase.prompt) || 'Watch nearai/ironclaw and summarize new releases.',
        cadence: 'On new release',
        cadenceType: 'event',
        firstRunTitle: 'Baseline: v0.9.2 is latest',
      } },
      { t: 700, respond:
        '**Tracking nearai/ironclaw releases.** Latest is **v0.9.2**, shipped 3 days ago.\n\n'
        + 'When a new release lands I\'ll summarize what changed — highlights, breaking changes, upgrade notes — right here. '
        + 'It runs as a task under **Tasks** \u2192 *Release tracker*.' },
      { t: 400, suggest: ['Show my tasks', 'Summarize v0.9.2 for me'] },
    ];
  }

  function hnSummaryScenario() {
    return [
      { t: 350, thinking: 'Fetching Hacker News' },
      { t: 1400, tool: { name: 'http_fetch', preview: 'GET https://hacker-news.firebaseio.com/v0/topstories.json \u2192 200 (fetched top 5 stories + comments)' } },
      { t: 800, respond:
        'Top of Hacker News right now:\n\n'
        + '1. **Open-source agent runtimes are converging** — long thread comparing sandboxing approaches; the debate is WASM vs microVMs.\n'
        + '2. **Show HN: I automated my whole inbox** — solo dev, 400 comments, mostly "how do I trust it?" (fair).\n'
        + '3. **Post-quantum TLS lands in mainline curl** — infra folks celebrating, everyone else confused.\n'
        + '4. **The return of personal software** — essay arguing agents make bespoke tools viable again.\n'
        + '5. **Ask HN: What replaced your standup?** — spoiler: an agent posting summaries to Slack.\n\n'
        + 'Want me to watch any of these topics and alert you on new developments?' },
      { t: 400, suggest: ['Watch HN for "agent runtimes"', 'Set up a daily 9am briefing'] },
    ];
  }

  // Scenario player: a sequential step walker emitting SSE events the way
  // the real agent loop does. Supports pausing (connect gates) and resuming
  // via POST /api/flows/action.
  var _pendingFlowResume = null; // { threadId, turn, steps, key }

  function runScenario(threadId, content) {
    var steps = scenarioForMessage(content);
    var turn = { user_input: content, response: null, tool_calls: [] };
    getTurns(threadId).push(turn);

    var thread = findThread(threadId);
    if (thread) {
      if (!thread.title) thread.title = deriveThreadTitle(content);
      thread.updated_at = iso(0);
    }

    playSteps(threadId, turn, steps);
  }

  function playSteps(threadId, turn, steps) {
    var idx = 0;

    function finishTurn() {
      emit('status', { thread_id: threadId, message: 'Done' });
    }

    function next() {
      if (idx >= steps.length) { finishTurn(); return; }
      var step = steps[idx++];
      if (step.pause) {
        // Stop here; the flow action handler resumes the remainder. Input
        // re-enables so the user can keep talking while the card waits.
        _pendingFlowResume = { threadId: threadId, turn: turn, steps: steps.slice(idx), key: step.pause };
        finishTurn();
        return;
      }
      // Typing beat before each agent bubble, like the prototype.
      if (step.say || step.respond) {
        emit('thinking', { thread_id: threadId, message: 'Thinking' });
      }
      setTimeout(function() { apply(step); }, step.t || 400);
    }

    function apply(step) {
      if (step.thinking) {
        emit('thinking', { thread_id: threadId, message: step.thinking });
        next();
        return;
      }
      if (step.tool) {
        var callId = uid('call');
        emit('tool_started', { thread_id: threadId, name: step.tool.name, call_id: callId });
        var toolMs = 600 + Math.random() * 700;
        turn.tool_calls.push({ name: step.tool.name, success: true });
        setTimeout(function() {
          emit('tool_completed', {
            thread_id: threadId, name: step.tool.name, call_id: callId,
            success: true, duration_ms: Math.round(toolMs),
          });
          emit('tool_result', {
            thread_id: threadId, name: step.tool.name, call_id: callId,
            preview: step.tool.preview || '',
          });
          next();
        }, toolMs);
        return;
      }
      if (step.connect) { installExtension(step.connect); next(); return; }
      if (step.connectMany) {
        step.connectMany.forEach(function(id) { installExtension(id); });
        next();
        return;
      }
      if (step.mission) { createMission(step.mission); next(); return; }
      if (step.card) {
        emit('flow_card', { thread_id: threadId, card: step.card });
        next();
        return;
      }
      if (step.say) {
        // Smooth character streaming for scripted agent bubbles (prototype
        // parity): chunks ride the real `stream_chunk` event, and the final
        // `response` event closes the bubble exactly like the live engine.
        var sayText = step.say;
        var sayAt = 0;
        for (var s = 0; s < sayText.length; s += 3) {
          sayAt += 18;
          (function(chunk, delay) {
            setTimeout(function() {
              emit('stream_chunk', { thread_id: threadId, content: chunk });
            }, delay);
          })(sayText.slice(s, s + 3), sayAt);
        }
        setTimeout(function() {
          turn.response = (turn.response ? turn.response + '\n\n' : '') + sayText;
          emit('response', { thread_id: threadId, content: sayText });
          next();
        }, sayAt + 120);
        return;
      }
      if (step.respond) {
        // Streamed final response (single-bubble scenarios).
        var text = step.respond;
        var words = text.split(/(\s+)/);
        var chunkSize = 6;
        var at = 0;
        for (var i = 0; i < words.length; i += chunkSize) {
          at += 40;
          (function(c, t) {
            setTimeout(function() {
              emit('stream_chunk', { thread_id: threadId, content: c });
            }, t);
          })(words.slice(i, i + chunkSize).join(''), at);
        }
        setTimeout(function() {
          turn.response = (turn.response ? turn.response + '\n\n' : '') + text;
          emit('response', { thread_id: threadId, content: text });
          next();
        }, at + 150);
        return;
      }
      if (step.suggest) {
        emit('suggestions', { thread_id: threadId, suggestions: step.suggest });
        next();
        return;
      }
      next();
    }

    next();
  }

  // Resume a paused flow (connect card clicked) or apply a proposal action.
  // Implied: POST /api/flows/action { action: 'connect'|'approve'|'dismiss',
  //          provider?, proposal? } -> { success, mission_id? }
  function handleFlowAction(body) {
    var action = (body && body.action) || '';
    if (action === 'connect' && body.provider) {
      installExtension(body.provider);
      if (_pendingFlowResume && _pendingFlowResume.key === 'connect:' + body.provider) {
        var resume = _pendingFlowResume;
        _pendingFlowResume = null;
        setTimeout(function() {
          playSteps(resume.threadId, resume.turn, resume.steps);
        }, 500);
      }
      return { success: true };
    }
    if (action === 'approve' && body.proposal && FLOW_PROPOSALS[body.proposal]) {
      var p = FLOW_PROPOSALS[body.proposal];
      var mission = createMission({
        name: p.mission.name,
        goal: p.body,
        cadence: p.mission.cadence,
        cadenceType: p.mission.cadenceType,
        firstRunTitle: p.details[0] || (p.mission.name + ' \u2014 first run'),
      });
      return { success: true, mission_id: mission.id };
    }
    if (action === 'dismiss') return { success: true };
    // Plan picked from the in-flow upgrade card: confirm it in the thread
    // the way the agent would (streamed bubble appended to the last turn).
    if (action === 'upgrade' && body.plan) {
      var plan = null;
      for (var bp = 0; bp < window.NUX_BILLING.plans.length; bp++) {
        if (window.NUX_BILLING.plans[bp].id === body.plan) { plan = window.NUX_BILLING.plans[bp]; break; }
      }
      var confirmThreadId = body.thread_id || (state.threads[0] && state.threads[0].id);
      var turns = getTurns(confirmThreadId);
      var confirmTurn = turns[turns.length - 1];
      if (!confirmTurn) {
        confirmTurn = { user_input: null, response: null, tool_calls: [] };
        turns.push(confirmTurn);
      }
      playSteps(confirmThreadId, confirmTurn, [
        { t: 400, say: 'You\u2019re on the ' + ((plan && plan.name) || body.plan) + ' plan now \u2014 everything I set up stays running, with room to grow. Welcome aboard.' },
      ]);
      return { success: true };
    }
    return { success: false, message: 'unknown flow action' };
  }

  // --------------------------------------------------------------------
  // Mock HTTP layer (fetch interception)
  // --------------------------------------------------------------------
  //
  // Implied API surface — each route below exists (or should exist) on the
  // real gateway; the mock returns the smallest response the SPA render
  // paths consume.

  function jsonResponse(obj, status) {
    return new Response(JSON.stringify(obj), {
      status: status || 200,
      headers: { 'Content-Type': 'application/json' },
    });
  }

  function handleRoute(path, method, body, authed) {
    seedHandoffIntegrations();
    // ---- auth ----
    // OAuth-style entry: the demo advertises sign-in providers so the token
    // form never surfaces as the primary path (it stays behind the "or use
    // a token" divider). Clicking a provider resolves locally (see
    // demoOAuthSignIn in init-auth.js).
    // Implied: GET /auth/providers -> { providers: [..], near_network }
    //          GET /auth/login/<provider> (redirect), POST /auth/near/verify
    if (path === '/auth/providers') {
      return { providers: ['google', 'github', 'near'], near_network: 'mainnet' };
    }
    if (path === '/auth/logout') return {};

    // The unauthenticated /api/gateway/status probe in autoAuth() detects
    // OIDC reverse-proxy auth. The demo must NOT pretend to be one — the
    // pre-auth landing (use-case gallery + carried-intent banner) is part
    // of the walkthrough. Any non-empty token is accepted (the marketing
    // handoff appends ?token=demo).
    if (path === '/api/gateway/status' && !authed) {
      return { __status: 401, body: { error: 'unauthorized' } };
    }

    // ---- profile / status ----
    if (path === '/api/profile') {
      return {
        id: 'demo-user', display_name: 'Demo User',
        email: 'demo@ironclaw.near.ai', role: 'admin', avatar_url: null,
      };
    }
    if (path === '/api/gateway/status') {
      return {
        engine_v2_enabled: true,
        version: 'demo', commit_hash: null,
        llm_model: 'claw-demo-1', llm_backend: 'mock',
        sse_connections: 1, ws_connections: 0,
        uptime_secs: Math.round((Date.now() - now) / 1000) + 4210,
        daily_cost: '0.42', actions_this_hour: 7,
        restart_enabled: false,
        model_usage: [
          { model: 'claw-demo-1', input_tokens: 48200, output_tokens: 9120, cost: '0.31' },
        ],
      };
    }

    // ---- chat ----
    if (path === '/api/chat/threads') {
      return { threads: state.threads, active_thread: null };
    }
    if (path === '/api/chat/thread/new') {
      var newThread = {
        id: uid('thread'), title: null, channel: 'gateway',
        state: 'Idle', updated_at: iso(0),
      };
      state.threads.unshift(newThread);
      return { id: newThread.id };
    }
    // Rename a thread (implied: PATCH /api/chat/threads/{id} { title }).
    var threadPatch = path.match(/^\/api\/chat\/threads\/([^/?]+)$/);
    if (threadPatch && (method === 'PATCH' || method === 'POST')) {
      var renamed = findThread(decodeURIComponent(threadPatch[1]));
      if (renamed && body && body.title) {
        renamed.title = String(body.title).slice(0, 80);
        renamed.updated_at = iso(0);
      }
      return { success: true };
    }
    if (path.indexOf('/api/chat/history') === 0) {
      var params = new URLSearchParams(path.split('?')[1] || '');
      var threadId = params.get('thread_id') || (state.threads[0] && state.threads[0].id);
      return { turns: getTurns(threadId), channel: 'gateway', in_progress: null };
    }
    if (path === '/api/chat/send') {
      var threadId2 = (body && body.thread_id) || (state.threads[0] && state.threads[0].id);
      runScenario(threadId2, (body && body.content) || '');
      return { success: true };
    }
    if (path === '/api/chat/auth-token' || path === '/api/chat/gate/resolve'
        || path === '/api/chat/auth-cancel') {
      return { success: true };
    }

    // ---- onboarding flow-card actions ----
    if (path === '/api/flows/action') {
      return handleFlowAction(body);
    }

    // ---- extensions / registry ----
    if (path === '/api/extensions/registry') {
      var installedNames = {};
      state.extensions.forEach(function(e) { installedNames[e.name] = true; });
      return {
        entries: REGISTRY.map(function(r) {
          return {
            name: r.name, display_name: r.display_name, kind: r.kind,
            description: r.description, installed: !!installedNames[r.name],
          };
        }),
      };
    }
    if (path === '/api/extensions/install') {
      installExtension(body && body.name, body && body.kind);
      return { success: true };
    }
    // Disconnect from the Integrations surface (implied: POST
    // /api/extensions/uninstall { name } or DELETE /api/extensions/{name}).
    if (path === '/api/extensions/uninstall') {
      var uninstallName = body && body.name;
      state.extensions = state.extensions.filter(function(e) { return e.name !== uninstallName; });
      // Drop it from the onboarding-handoff seed too so a reload doesn't
      // resurrect the connection.
      try {
        var seededRaw = sessionStorage.getItem('ironclaw_nux_connected_integrations');
        var seededIds = seededRaw ? JSON.parse(seededRaw) : [];
        if (Array.isArray(seededIds)) {
          sessionStorage.setItem('ironclaw_nux_connected_integrations',
            JSON.stringify(seededIds.filter(function(id) { return id !== uninstallName; })));
        }
      } catch (e) { /* no handoff state */ }
      return { success: true };
    }
    if (path === '/api/extensions') {
      return { extensions: state.extensions };
    }
    var setupMatch = path.match(/^\/api\/extensions\/([^/]+)\/setup$/);
    if (setupMatch) {
      // No credentials needed in the demo — configure modal short-circuits
      // to a "nothing to configure" toast and the wizard shows Connected.
      return { secrets: [], fields: [], interactive_login: null, onboarding: null };
    }
    if (path.indexOf('/api/extensions/') === 0) {
      return { success: true };
    }

    // ---- skills ----
    if (path === '/api/skills/search') {
      var q = String((body && body.query) || '').trim().toLowerCase();
      var installedNames = {};
      state.skills.forEach(function(s) { installedNames[s.name] = true; });
      var catalog = SKILL_CATALOG.filter(function(entry) {
        if (!q) return true;
        return (entry.name + ' ' + entry.slug + ' ' + entry.description).toLowerCase().indexOf(q) !== -1;
      }).map(function(entry) {
        var copy = {};
        for (var key in entry) copy[key] = entry[key];
        copy.installed = !!installedNames[entry.name];
        return copy;
      });
      var installed = state.skills.filter(function(s) {
        if (!q) return false;
        return (s.name + ' ' + (s.description || '')).toLowerCase().indexOf(q) !== -1;
      });
      return { catalog: catalog, installed: installed };
    }
    if (path === '/api/skills/install') {
      var skillName = (body && (body.name || body.slug)) || 'skill';
      var short = skillName.indexOf('/') >= 0 ? skillName.split('/').pop() : skillName;
      var known = null;
      for (var sc = 0; sc < SKILL_CATALOG.length; sc++) {
        if (SKILL_CATALOG[sc].name === short || SKILL_CATALOG[sc].slug === skillName) {
          known = SKILL_CATALOG[sc];
          break;
        }
      }
      var already = state.skills.some(function(s) { return s.name === short; });
      if (!already) {
        state.skills.push({
          name: short,
          icon: (known && known.icon) || 'sparkles',
          version: (known && known.version) || '1.0.0',
          trust: 'Installed',
          description: (known && known.description) || 'Installed from ' + ((body && body.url) || 'ClawHub') + '.',
          keywords: [short],
          install_source_url: (body && body.url) || (known ? 'https://clawhub.ai/skills/' + known.slug : null),
        });
      }
      return { success: true };
    }
    var skillDelete = path.match(/^\/api\/skills\/([^/?]+)$/);
    if (skillDelete && method === 'DELETE') {
      state.skills = state.skills.filter(function(s) { return s.name !== decodeURIComponent(skillDelete[1]); });
      return { success: true };
    }
    if (path.indexOf('/api/skills') === 0) {
      return { skills: state.skills };
    }

    // ---- engine v2: missions (Tasks kanban) ----
    if (path === '/api/engine/missions/summary') {
      var counts = { total: state.missions.length, active: 0, paused: 0, completed: 0, failed: 0 };
      state.missions.forEach(function(m) {
        var key = String(m.status || '').toLowerCase();
        if (counts[key] !== undefined) counts[key]++;
      });
      return counts;
    }
    var missionAction = path.match(/^\/api\/engine\/missions\/([^/]+)\/(pause|resume|fire)$/);
    if (missionAction) {
      for (var mi = 0; mi < state.missions.length; mi++) {
        if (state.missions[mi].id === missionAction[1]) {
          if (missionAction[2] === 'pause') state.missions[mi].status = 'Paused';
          if (missionAction[2] === 'resume') state.missions[mi].status = 'Active';
        }
      }
      return { success: true };
    }
    var missionDetail = path.match(/^\/api\/engine\/missions\/([^/?]+)$/);
    if (missionDetail) {
      for (var md = 0; md < state.missions.length; md++) {
        if (state.missions[md].id === missionDetail[1]) {
          return { mission: state.missions[md] };
        }
      }
      return { mission: null };
    }
    if (path.indexOf('/api/engine/missions') === 0) {
      return { missions: state.missions };
    }

    // ---- engine v2: threads + projects ----
    var engineThreadDetail = path.match(/^\/api\/engine\/threads\/([^/?]+)$/);
    if (engineThreadDetail) {
      for (var et = 0; et < state.engineThreads.length; et++) {
        if (state.engineThreads[et].id === engineThreadDetail[1]) {
          return { thread: state.engineThreads[et] };
        }
      }
      return { thread: null };
    }
    if (path.indexOf('/api/engine/threads') === 0) {
      return { threads: state.engineThreads };
    }
    if (path === '/api/engine/projects/overview') {
      return {
        projects: [{
          id: 'project-default', name: 'default', description: '',
          health: 'green',
          active_missions: state.missions.filter(function(m) { return m.status === 'Active'; }).length,
          threads_today: state.engineThreads.length,
          cost_today_usd: 0.42,
          last_activity: iso(0),
        }],
        attention: [],
      };
    }
    if (path.indexOf('/api/engine/projects/') === 0) {
      return { widgets: [] };
    }

    // ---- jobs (sandbox) ----
    if (path === '/api/jobs/summary') {
      return { total: 0, in_progress: 0, completed: 0, failed: 0 };
    }
    if (path.indexOf('/api/jobs') === 0) {
      return { jobs: [] };
    }

    // ---- routines (engine v1 — hidden when engine_v2_enabled) ----
    if (path === '/api/routines/summary') return { total: 0 };
    if (path.indexOf('/api/routines') === 0) return { routines: [] };

    // ---- memory / workspace files ----
    // Implied: GET /api/memory/list?path= -> { entries: [{name, path, is_dir}] }
    //          GET /api/memory/read?path= -> { content, path }
    //          POST /api/memory/write { path, content }
    //          POST /api/memory/search { query, limit } -> { results: [{path, content}] }
    if (path.indexOf('/api/memory/list') === 0) {
      var listParams = new URLSearchParams(path.split('?')[1] || '');
      var dirPath = listParams.get('path') || '';
      return { entries: memoryList(dirPath) };
    }
    if (path.indexOf('/api/memory/read') === 0) {
      var readParams = new URLSearchParams(path.split('?')[1] || '');
      var filePath = readParams.get('path') || '';
      var content = MEMORY_FILES[filePath];
      if (content === undefined) {
        return { __status: 404, body: { error: 'no such file: ' + filePath } };
      }
      return { content: content, path: filePath };
    }
    if (path === '/api/memory/search') {
      var memQuery = String((body && body.query) || '').toLowerCase();
      var results = [];
      if (memQuery) {
        Object.keys(MEMORY_FILES).forEach(function(p) {
          if ((p + ' ' + MEMORY_FILES[p]).toLowerCase().indexOf(memQuery) !== -1) {
            results.push({ path: p, content: MEMORY_FILES[p] });
          }
        });
      }
      return { results: results.slice(0, (body && body.limit) || 20) };
    }
    if (path === '/api/memory/write') {
      if (body && body.path) MEMORY_FILES[body.path] = String(body.content || '');
      return { success: true };
    }

    // ---- settings / tokens / llm / logs / traces ----
    if (path === '/api/settings/tools') return { tools: [] };
    if (path === '/api/settings/export') return { settings: {} };
    if (path.indexOf('/api/settings') === 0) return { success: true };
    if (path.indexOf('/api/tokens') === 0) {
      return method === 'GET' ? { tokens: [] } : { success: true };
    }
    if (path === '/api/llm/providers') return { providers: [] };
    if (path === '/api/llm/list_models') return { models: [] };
    if (path.indexOf('/api/llm') === 0) return { success: true };
    if (path.indexOf('/api/logs/level') === 0) return { level: 'info' };
    if (path.indexOf('/api/traces') === 0) return { credits: [], pending: 0, final: 0 };
    if (path.indexOf('/api/admin') === 0) return { users: [] };
    if (path.indexOf('/api/pairing') === 0) return { success: true };

    // Catch-all: succeed quietly so optional surfaces degrade gracefully.
    return {};
  }

  var realFetch = window.fetch.bind(window);
  window.fetch = function(input, options) {
    var url = typeof input === 'string' ? input : (input && input.url) || '';
    var isApiPath = url.indexOf('/api/') === 0 || url.indexOf('/auth/') === 0;
    if (!isApiPath) return realFetch(input, options);

    var opts = options || {};
    var method = (opts.method || 'GET').toUpperCase();
    var body = null;
    if (typeof opts.body === 'string') {
      try { body = JSON.parse(opts.body); } catch (e) { body = null; }
    }
    var headers = opts.headers || {};
    var authed = false;
    try {
      authed = !!(headers.Authorization || headers.authorization
        || sessionStorage.getItem('ironclaw_token'));
    } catch (e) { authed = !!(headers.Authorization || headers.authorization); }
    var path = url;
    return new Promise(function(resolve) {
      // Small latency so spinners/skeletons render like a real deployment.
      setTimeout(function() {
        try {
          var result = handleRoute(path, method, body, authed);
          if (result && result.__status) {
            resolve(jsonResponse(result.body || {}, result.__status));
          } else {
            resolve(jsonResponse(result));
          }
        } catch (err) {
          console.error('[demo api]', path, err);
          resolve(jsonResponse({ error: String(err && err.message) }, 500));
        }
      }, 80 + Math.random() * 120);
    });
  };

  console.info('[ironclaw] demo mode active — all /api and /auth requests are mocked in-browser (js/core/mock-backend.js)');
})();
