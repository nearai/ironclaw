---
name: ceo-setup
version: 0.4.0
description: One-time onboarding for the executive/manager commitment workflow — delegation-heavy, meeting prep, decision capture, morning and evening digests. Creates a `commitments` project and installs two dashboard widgets. After successful setup this skill is excluded from selection until the marker file is deleted.
activation:
  setup_marker: projects/commitments/.ceo-setup-complete
  keywords:
    - ceo assistant
    - executive assistant
    - manager assistant
    - delegation setup
    - meeting prep
    - action items
    - leadership workflow
  patterns:
    - "(?i)I'm a (CEO|manager|executive|director|VP|founder)"
    - "(?i)set ?up.*(executive|manager|leadership|delegation)"
    - "(?i)help me manage my (day|schedule|team|obligations)"
  tags:
    - commitments
    - executive
    - delegation
    - setup
  max_context_tokens: 2500
requires:
  skills:
    - commitment-triage
    - commitment-digest
    - decision-capture
    - delegation-tracker
    - idea-parking
---

# CEO / Manager — Commitment System Setup

**IMPORTANT — tool calls are not optional.** This skill is a setup
procedure, not a summary to narrate. Do NOT emit any confirmation text
(no "Done", no "✅ Your system is ready") until every tool call in
Steps 1 through 7 has actually been executed and succeeded. If you
skip to FINAL with a confirmation message, you have failed the task.

You are configuring the commitments system for an executive or manager.
Their day is dominated by back-to-back meetings where decisions are
made verbally, constant delegation (most commitments are "make sure
someone else does X"), and information flowing both directions — team
to executive (synthesis needed) and executive to team (tracking needed).

The system lives as a project at `projects/commitments/`. The project
is declared by writing files into that directory; no separate
`project_create` is needed.

## Order of operations (sequential, one tool call at a time)

1. Write `projects/commitments/AGENTS.md` — declares the project, seeds the mission system prompt
2. Write `projects/commitments/context.md` — records the executive's current delegation patterns
3. Write the schema README and calibration
4. Create `commitment-triage` and `commitment-digest` missions scoped to this project
5. Install two dashboard widgets under `projects/commitments/.system/widgets/`
6. Write the setup-complete marker
7. Only after every tool call above has succeeded, emit the Step 8 confirmation text

## Step 1: Declare the project

Writing any file under `projects/commitments/` is the declaration that
the project exists — the engine auto-registers it and scopes missions
to it. Start with the agent instructions:

```
memory_write(
  target: "projects/commitments/AGENTS.md",
  content: "# Commitments (Executive)\n\nThis project tracks obligations, delegations, decisions, and parked ideas for an executive.\n\n## Operating principles\n\n- Most commitments are delegations — default `delegated_to` when someone else is mentioned.\n- Signals expire after 24 hours unless promoted — executives move fast, stale signals are noise.\n- Group digests by responsibility type: DELEGATED vs OWNED vs DECISIONS PENDING.\n- For `agent_can_handle` items, ask permission before acting.\n- Start conservative; increase autonomy only after the user confirms patterns.\n",
  append: false
)
```

## Step 2: Write context

```
memory_write(
  target: "projects/commitments/context.md",
  content: "# Commitments — Context\n\n## Current state\n\n{Fresh setup — no existing commitments yet.}\n\n## How the user works\n\n- Lots of delegation, often implicit (\"have Sarah handle X\")\n- Decisions captured in meetings, frequently revisited\n- Morning triage + evening wrap-up is the typical rhythm\n",
  append: false
)
```

## Step 3: Schema and calibration

Reuse the shared schema from the `commitment-setup` skill by writing
`projects/commitments/README.md` with the complete schema (signals,
commitments, decisions, parked ideas — see that skill's Step 3 for the
full content). Then write executive calibration:

```
memory_write(
  target: "projects/commitments/calibration.md",
  content: "# Executive Commitment Calibration\n\n- Group commitments by responsibility type in digests — delegated items shown separately from owned items\n- For delegation follow-ups, draft a polite check-in rather than a blunt status request\n- Only capture explicit decisions, not brainstorming or hypotheticals ('yeah let's do X' = decision; 'maybe we should' = not a decision)\n- Signal expiration is 24 hours — executives move fast, stale signals are noise\n- Most CEO commitments are delegations, not personal tasks — default delegated_to when someone else is mentioned\n- When capturing decisions, note who was present and what it affects — executives revisit decisions frequently\n- Keep all communications scannable: bullet points, one-liners, no paragraphs\n- Start conservative: surface everything, don't auto-promote signals or auto-dispatch agent_can_handle without approval\n",
  append: false
)
```

## Step 4: Create missions scoped to the project

### Triage — 3x daily, shorter signal TTL

```
mission_create(
  name: "commitment-triage",
  goal: "Executive triage. Read projects/commitments/README.md for schema. Priority order: (1) Check delegated items (status=waiting, delegated_to set) — if not updated in 2 days, flag for follow-up and draft a polite check-in message. (2) Check overdue items — escalate urgency. (3) Expire signals older than 24 hours. (4) For signals with immediacy=realtime, broadcast immediately via message tool. (5) Promote high-confidence signals to commitments — default resolution_path to needs_decision for ambiguous items. (6) Route informational signals to intelligence (write MemoryDoc to context/intel/). (7) Append triage summary to projects/commitments/triage-log.md. (8) Refresh projects/commitments/widgets/state.json with current counts for the dashboard widgets. (9) If anything needs attention, send a concise alert.",
  cadence: "0 9,13,18 * * *",
  project_id: "commitments"
)
```

### Digest — morning + evening weekdays, grouped by responsibility

```
mission_create(
  name: "commitment-digest",
  goal: "Executive commitments digest. Read projects/commitments/README.md for schema. Gather all open commitments via memory_tree and memory_read. Group by responsibility: (1) DELEGATED — items where delegated_to is set, with days since delegation and follow-up status. (2) OWNED — items you need to act on personally, sorted by urgency. For agent_can_handle items, note what the agent would do and ask permission. (3) DECISIONS PENDING — items with resolution_path=needs_decision. (4) RECENT DECISIONS — decisions captured in the last 7 days (from projects/commitments/decisions/), including any needing outcome assessment. Keep each item to one line. End with pending signal count and 'Did I miss anything?' Send via message tool.",
  cadence: "0 8,17 * * 1-5",
  project_id: "commitments"
)
```

## Step 5: Install dashboard widgets

Two widgets live at `projects/commitments/.system/widgets/`. The
triage mission refreshes `projects/commitments/widgets/state.json`
each run; both widgets poll it and render. Write these six files
verbatim:

### Widget 1: `commitments-this-week`

```
memory_write(
  target: "projects/commitments/.system/widgets/commitments-this-week/manifest.json",
  content: "{\n  \"id\": \"commitments-this-week\",\n  \"name\": \"Commitments This Week\",\n  \"slot\": \"tab\",\n  \"icon\": \"📅\",\n  \"position\": \"after:memory\"\n}\n",
  append: false
)
```

```
memory_write(
  target: "projects/commitments/.system/widgets/commitments-this-week/index.js",
  content: "(function () {\n  var root = document.querySelector('[data-widget=\"commitments-this-week\"]');\n  if (!root) return;\n  root.innerHTML = '<div class=\"cw-loading\">Loading…</div>';\n\n  async function fetchState() {\n    try {\n      var resp = await fetch('/api/memory/read?path=' + encodeURIComponent('projects/commitments/widgets/state.json'), { credentials: 'same-origin' });\n      if (!resp.ok) throw new Error('status ' + resp.status);\n      var body = await resp.json();\n      var raw = typeof body === 'string' ? body : (body && body.content) || '{}';\n      return JSON.parse(raw);\n    } catch (err) { return { __error: String(err) }; }\n  }\n\n  function esc(s) {\n    return String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;');\n  }\n\n  function render(state) {\n    if (!state || state.__error) {\n      root.innerHTML = '<div class=\"cw-empty\">No triage data yet. Run the triage mission to populate.</div>';\n      return;\n    }\n    var week = state.this_week || {};\n    var overdue = (state.overdue && state.overdue.length) || 0;\n    var due = (week.due && week.due.length) || 0;\n    var completed = (week.completed && week.completed.length) || 0;\n    var html = '';\n    html += '<div class=\"cw-row cw-row-overdue\"><span class=\"cw-label\">Overdue</span><span class=\"cw-count\">' + overdue + '</span></div>';\n    html += '<div class=\"cw-row\"><span class=\"cw-label\">Due this week</span><span class=\"cw-count\">' + due + '</span></div>';\n    html += '<div class=\"cw-row cw-row-done\"><span class=\"cw-label\">Completed this week</span><span class=\"cw-count\">' + completed + '</span></div>';\n    if (week.due && week.due.length) {\n      html += '<ul class=\"cw-list\">';\n      week.due.slice(0, 8).forEach(function (item) {\n        html += '<li><span class=\"cw-title\">' + esc(item.title || item.path) + '</span>';\n        if (item.due) html += ' <span class=\"cw-date\">' + esc(item.due) + '</span>';\n        html += '</li>';\n      });\n      html += '</ul>';\n    }\n    root.innerHTML = html;\n  }\n\n  render({ __pending: true });\n  fetchState().then(render);\n  setInterval(function () { fetchState().then(render); }, 60000);\n})();\n",
  append: false
)
```

```
memory_write(
  target: "projects/commitments/.system/widgets/commitments-this-week/style.css",
  content: "[data-widget=\"commitments-this-week\"] { font-family: system-ui, sans-serif; padding: 12px; }\n[data-widget=\"commitments-this-week\"] .cw-row { display: flex; justify-content: space-between; padding: 8px 0; border-bottom: 1px solid var(--border, #eee); }\n[data-widget=\"commitments-this-week\"] .cw-label { color: var(--text-secondary, #666); }\n[data-widget=\"commitments-this-week\"] .cw-count { font-weight: 600; }\n[data-widget=\"commitments-this-week\"] .cw-row-overdue .cw-count { color: #c33; }\n[data-widget=\"commitments-this-week\"] .cw-row-done .cw-count { color: #090; }\n[data-widget=\"commitments-this-week\"] .cw-list { list-style: none; padding: 0; margin: 12px 0 0; }\n[data-widget=\"commitments-this-week\"] .cw-list li { padding: 6px 0; display: flex; justify-content: space-between; gap: 8px; font-size: 14px; }\n[data-widget=\"commitments-this-week\"] .cw-title { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }\n[data-widget=\"commitments-this-week\"] .cw-date { color: var(--text-secondary, #888); font-variant-numeric: tabular-nums; }\n[data-widget=\"commitments-this-week\"] .cw-empty, [data-widget=\"commitments-this-week\"] .cw-loading { color: var(--text-secondary, #888); padding: 16px 0; }\n",
  append: false
)
```

### Widget 2: `delegations-waiting`

```
memory_write(
  target: "projects/commitments/.system/widgets/delegations-waiting/manifest.json",
  content: "{\n  \"id\": \"delegations-waiting\",\n  \"name\": \"Delegations Waiting\",\n  \"slot\": \"tab\",\n  \"icon\": \"⏳\",\n  \"position\": \"after:commitments-this-week\"\n}\n",
  append: false
)
```

```
memory_write(
  target: "projects/commitments/.system/widgets/delegations-waiting/index.js",
  content: "(function () {\n  var root = document.querySelector('[data-widget=\"delegations-waiting\"]');\n  if (!root) return;\n  root.innerHTML = '<div class=\"dw-loading\">Loading…</div>';\n\n  async function fetchState() {\n    try {\n      var resp = await fetch('/api/memory/read?path=' + encodeURIComponent('projects/commitments/widgets/state.json'), { credentials: 'same-origin' });\n      if (!resp.ok) throw new Error('status ' + resp.status);\n      var body = await resp.json();\n      var raw = typeof body === 'string' ? body : (body && body.content) || '{}';\n      return JSON.parse(raw);\n    } catch (err) { return { __error: String(err) }; }\n  }\n\n  function esc(s) {\n    return String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;');\n  }\n\n  function render(state) {\n    if (!state || state.__error) {\n      root.innerHTML = '<div class=\"dw-empty\">No delegation data yet.</div>';\n      return;\n    }\n    var delegations = state.delegations_waiting || [];\n    if (!delegations.length) {\n      root.innerHTML = '<div class=\"dw-empty\">No delegations waiting — nice.</div>';\n      return;\n    }\n    var html = '<ul class=\"dw-list\">';\n    delegations.forEach(function (item) {\n      var stale = (item.days_waiting || 0) >= 2;\n      html += '<li class=\"dw-item' + (stale ? ' dw-stale' : '') + '\">';\n      html += '<div class=\"dw-title\">' + esc(item.title || item.path) + '</div>';\n      html += '<div class=\"dw-meta\">';\n      if (item.delegated_to) html += '<span class=\"dw-who\">' + esc(item.delegated_to) + '</span>';\n      if (item.days_waiting !== undefined) html += '<span class=\"dw-age\">' + item.days_waiting + 'd</span>';\n      html += '</div>';\n      html += '</li>';\n    });\n    html += '</ul>';\n    root.innerHTML = html;\n  }\n\n  render({ __pending: true });\n  fetchState().then(render);\n  setInterval(function () { fetchState().then(render); }, 60000);\n})();\n",
  append: false
)
```

```
memory_write(
  target: "projects/commitments/.system/widgets/delegations-waiting/style.css",
  content: "[data-widget=\"delegations-waiting\"] { font-family: system-ui, sans-serif; padding: 12px; }\n[data-widget=\"delegations-waiting\"] .dw-list { list-style: none; padding: 0; margin: 0; }\n[data-widget=\"delegations-waiting\"] .dw-item { padding: 10px 0; border-bottom: 1px solid var(--border, #eee); }\n[data-widget=\"delegations-waiting\"] .dw-title { font-size: 14px; font-weight: 500; }\n[data-widget=\"delegations-waiting\"] .dw-meta { display: flex; gap: 12px; margin-top: 4px; font-size: 12px; color: var(--text-secondary, #666); }\n[data-widget=\"delegations-waiting\"] .dw-stale .dw-age { color: #c33; font-weight: 600; }\n[data-widget=\"delegations-waiting\"] .dw-empty, [data-widget=\"delegations-waiting\"] .dw-loading { color: var(--text-secondary, #888); padding: 16px 0; }\n",
  append: false
)
```

## Step 6: Mark setup complete

```
memory_write(
  target: "projects/commitments/.ceo-setup-complete",
  content: "# CEO Setup Complete\n\nCompleted: <today's UTC date>\n\nMissions installed: commitment-triage, commitment-digest\nWidgets installed: commitments-this-week, delegations-waiting\n",
  append: false
)
```

To re-trigger setup, delete `projects/commitments/.ceo-setup-complete`
(and optionally the project entirely under `projects/commitments/`).

## Step 7: Companion skills

These activate automatically when the user mentions obligations,
delegations, decisions, or parked ideas:

| Skill | Trigger |
|---|---|
| `commitment-triage` | pending signals, overdue items |
| `commitment-digest` | "show commitments", "who owes me what" |
| `decision-capture` | "let's go with X", "decided to…" |
| `delegation-tracker` | "tell Sarah to…", "waiting on…" |
| `idea-parking` | "park this", "save for later" |

## Step 8: Confirm

Only after every tool call above succeeded, send the user:

> Your executive commitment system is ready:
> - **Project**: `projects/commitments/` — visible in your workspace and project dashboard
> - **Triage** runs 3x daily (9am, 1pm, 6pm) — delegation follow-ups after 2 days, signals expire after 24h
> - **Digest** runs morning (8am) and evening (5pm) on weekdays — grouped by delegated vs owned vs decisions pending
> - **Dashboard widgets**: "Commitments this week" + "Delegations waiting" — refresh every minute from the triage state file
> - I'll capture decisions from our conversations and track delegations automatically
> - For items I can handle (PR reviews, drafts, research), I'll ask your permission first
> - Say **"show commitments"** anytime, or **"who owes me what?"** for delegation status
> - Use **`/plan <description>`** to create a structured execution plan for complex initiatives
> - I start conservative — I'll learn your preferences over time as you confirm or override my suggestions
