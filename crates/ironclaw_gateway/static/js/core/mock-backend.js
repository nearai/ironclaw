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
    { name: 'telegram', label: 'Telegram', blurb: 'Chat with your agent from your phone, on the go.' },
    { name: 'slack', label: 'Slack', blurb: 'Bring your agent into the place your team already works.' },
    { name: 'discord', label: 'Discord', blurb: 'Run your agent inside your community server.' },
    { name: 'whatsapp', label: 'WhatsApp', blurb: 'Message your agent like any other contact.' },
  ],

  // MOCK: Curated integration catalog for the first-class Integrations
  // surface. `id`s match extension registry names where one exists and the
  // `integrations` ids used by use cases above / the marketing-site handoff
  // (`?integrations=gmail,slack`). Entries with no registry match render as
  // "ask the agent to connect it" cards.
  integrationCatalog: [
    { id: 'gmail', label: 'Gmail', glyph: '\u2709', blurb: 'Read, triage, label, and draft email on your behalf.' },
    { id: 'google_calendar', label: 'Google Calendar', glyph: '\u29D7', blurb: 'See your schedule, prep you for meetings, and block time.' },
    { id: 'google_sheets', label: 'Google Sheets', glyph: '\u2261', blurb: 'Append rows, build reports, and keep trackers up to date.' },
    { id: 'slack', label: 'Slack', glyph: '\u2318', blurb: 'Chat with your agent and post updates where your team works.' },
    { id: 'telegram', label: 'Telegram', glyph: '\u2708', blurb: 'Message your agent from your phone, anywhere.' },
    { id: 'discord', label: 'Discord', glyph: '\u2756', blurb: 'Run your agent inside your community server.' },
    { id: 'whatsapp', label: 'WhatsApp', glyph: '\u260E', blurb: 'Talk to your agent like any other contact.' },
    { id: 'github', label: 'GitHub', glyph: '\u2387', blurb: 'Watch repos, summarize releases, and track issues.' },
    { id: 'linear', label: 'Linear', glyph: '\u2713', blurb: 'Create and update tickets straight from chat.' },
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
    // Skills (implied: /api/skills, /api/skills/search)
    skills: [
      { name: 'summarize', description: 'Summarize long content into key points.', enabled: true, source: 'builtin' },
      { name: 'web-research', description: 'Search the web and synthesize findings with citations.', enabled: true, source: 'builtin' },
      { name: 'daily-briefing', description: 'Compose a morning briefing from calendar, email, and tasks.', enabled: true, source: 'clawhub' },
    ],
  };

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
    { name: 'github', display_name: 'GitHub', kind: 'wasm_tool', description: 'Watch repos, releases, and issues.' },
    { name: 'linear', display_name: 'Linear', kind: 'wasm_tool', description: 'Create and update tickets from chat.' },
    { name: 'http', display_name: 'HTTP', kind: 'native', description: 'Fetch URLs and call APIs.' },
    { name: 'browser', display_name: 'Browser', kind: 'mcp_server', description: 'Drive a headless browser for research.' },
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
      return inboxTriageScenario(usedCase('inbox-triage'));
    }
    if (/briefing/.test(text)) {
      return briefingScenario(usedCase('daily-briefing'));
    }
    if (/hacker news|keyword|appears on/.test(text) && /send|watch|summar/.test(text)) {
      return keywordMonitorScenario(usedCase('keyword-monitor'));
    }
    if (/ping http|health|non-200|watch.*endpoint/.test(text)) {
      return deployWatcherScenario(usedCase('deploy-watcher'));
    }
    if (/watch the .*github|new releases/.test(text)) {
      return releaseTrackerScenario(usedCase('release-tracker'));
    }
    if (/summarize the top stories/.test(text)) {
      return hnSummaryScenario();
    }
    return genericScenario(content);
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

  function genericScenario(content) {
    var quoted = String(content || '').slice(0, 120);
    return [
      { t: 400, thinking: 'Thinking' },
      { t: 1200, respond:
        'Got it — here\'s how I\'d approach *"' + quoted + '"*:\n\n'
        + 'In this demo I run on scripted data, so I can\'t execute that one for real. In a live workspace I would plan the steps, '
        + 'pick the right tools or integrations, and either do it once or set it up as a repeating task under **Tasks**.\n\n'
        + 'Some things the demo *does* fully walk through:' },
      { t: 400, suggest: [
        'Set up a daily 9am briefing with my calendar and inbox',
        'If "IronClaw" appears on Hacker News, send a summary to me here',
        'Triage my inbox: label new emails as "Action", "FYI", or "Ignore", and summarize the Action ones for me.',
        'What can you do for me?',
      ] },
    ];
  }

  // Scenario player: walks the step list on timers, emitting SSE events the
  // way the real agent loop does (thinking → tools → streamed response →
  // suggestions → Done).
  function runScenario(threadId, content) {
    var steps = scenarioForMessage(content);
    var turn = { user_input: content, response: null, tool_calls: [] };
    getTurns(threadId).push(turn);

    var thread = findThread(threadId);
    if (thread) {
      if (!thread.title) thread.title = String(content).slice(0, 60);
      thread.updated_at = iso(0);
    }

    var delay = 0;
    var finalResponse = '';
    steps.forEach(function(step) {
      delay += step.t || 500;

      if (step.thinking) {
        setTimeout(function() {
          emit('thinking', { thread_id: threadId, message: step.thinking });
        }, delay);
      }

      if (step.tool) {
        var callId = uid('call');
        (function(tool, id, at) {
          setTimeout(function() {
            emit('tool_started', { thread_id: threadId, name: tool.name, call_id: id });
          }, at);
          var toolMs = 600 + Math.random() * 700;
          setTimeout(function() {
            emit('tool_completed', {
              thread_id: threadId, name: tool.name, call_id: id,
              success: true, duration_ms: Math.round(toolMs),
            });
            emit('tool_result', {
              thread_id: threadId, name: tool.name, call_id: id,
              preview: tool.preview || '',
            });
          }, at + toolMs);
          turn.tool_calls.push({ name: tool.name, success: true });
        })(step.tool, callId, delay);
        delay += 900;
      }

      if (step.connect) {
        (function(name, at) {
          setTimeout(function() { installExtension(name); }, at);
        })(step.connect, delay);
      }

      if (step.mission) {
        (function(spec, at) {
          setTimeout(function() { createMission(spec); }, at);
        })(step.mission, delay);
      }

      if (step.respond) {
        (function(text, at) {
          // Stream in word chunks like the real backend does.
          var words = text.split(/(\s+)/);
          var chunkSize = 6;
          var streamedAt = at;
          for (var i = 0; i < words.length; i += chunkSize) {
            var chunk = words.slice(i, i + chunkSize).join('');
            streamedAt += 40;
            (function(c, t) {
              setTimeout(function() {
                emit('stream_chunk', { thread_id: threadId, content: c });
              }, t);
            })(chunk, streamedAt);
          }
          setTimeout(function() {
            turn.response = text;
            emit('response', { thread_id: threadId, content: text });
            emit('status', { thread_id: threadId, message: 'Done' });
          }, streamedAt + 120);
        })(step.respond, delay);
        // Advance the step clock past the streaming window so later steps
        // (e.g. suggestion chips) land after the final response event.
        delay += 40 * Math.ceil(step.respond.split(/(\s+)/).length / 6) + 300;
      }

      if (step.suggest) {
        (function(suggestions, at) {
          setTimeout(function() {
            emit('suggestions', { thread_id: threadId, suggestions: suggestions });
          }, at);
        })(step.suggest, delay);
      }
    });
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
    if (path === '/auth/providers') return { providers: [] };
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
      return { results: [], skills: [] };
    }
    if (path === '/api/skills/install') return { success: true };
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

    // ---- memory ----
    if (path.indexOf('/api/memory/list') === 0) {
      return {
        entries: [
          { name: 'profile.md', path: 'profile.md', is_dir: false },
          { name: 'preferences.md', path: 'preferences.md', is_dir: false },
        ],
      };
    }
    if (path.indexOf('/api/memory/read') === 0) {
      return { content: '# Demo memory\n\nThis workspace runs on mock data — memory writes are not persisted.', path: 'profile.md' };
    }
    if (path === '/api/memory/search') return { results: [] };
    if (path === '/api/memory/write') return { success: true };

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
