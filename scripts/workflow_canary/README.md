# Workflow Canary

End-to-end canary lane for the multi-tool / multi-channel user
workflows defined in [issue #1044][issue]. Where `auth-live-canary`
covers credential / OAuth flows, this lane covers what happens
*after* the user is authenticated: cron-driven routines, chat-driven
tool dispatch, Telegram round-trips, Sheets writes, etc.

[issue]: https://github.com/nearai/ironclaw/issues/1044

## Lane structure

```
scripts/workflow_canary/
├── run_workflow_canary.py     # entrypoint — mirrors run_live_canary.py CLI
├── telegram_mock.py            # fake Telegram Bot API (single-port aiohttp)
├── routines.py                 # libSQL helpers (insert + backdate + poll)
├── scenarios/
│   ├── _common.py              # shared run_routine_probe()
│   ├── bug_logger.py           # Script 1
│   ├── calendar_prep.py        # Script 2
│   ├── hn_monitor.py           # Script 3
│   ├── periodic_reminder.py    # Script 4
│   └── crm_tracker.py          # Script 5
```

`scripts/live-canary/run.sh` dispatches `LANE=workflow-canary` here, and
`.github/workflows/live-canary.yml` has a matching `workflow-canary`
job in the live-canary matrix.

## What's covered today

Every scenario runs the same shape:

1. Insert a `Lightweight` cron routine directly into libSQL with
   `next_fire_at` backdated by 60 s. The routine's prompt embeds
   a `[CANARY-WORKFLOW-<key>]` sentinel that the mock LLM matches
   to emit a deterministic `http` tool call to
   `api.telegram.org/bot.../sendMessage`.
2. Wait for the cron ticker (configured to 2 s in
   `run_workflow_canary.py`) to pick it up.
3. The mock LLM (`tests/e2e/mock_llm.py`) responds with the http
   tool call; the routine engine dispatches it.
4. The http tool's request is rewritten by
   `IRONCLAW_TEST_HTTP_REMAP=api.telegram.org=<mock>` (carried
   through to the routine's `JobContext` via the engine's
   `http_interceptor` field — see fix in `src/agent/routine_engine.rs`).
5. Mock Telegram captures the sendMessage on
   `/__mock/sent_messages`; the scenario asserts the per-scenario
   ack text (`[canary-workflow:<key>] ack`) matches.

This catches regressions in:
- `RoutineEngine::spawn_cron_ticker` and the `check_cron_triggers` loop
- `RoutineAction::Lightweight` execution path
- Routine state machine (`routines.next_fire_at` → `routine_runs.status`)
- DB serialization of `action_config` / `trigger_config`
- Mock LLM `TOOL_CALL_PATTERNS` matching the canary sentinel
- http tool dispatch from a routine action
- `IRONCLAW_TEST_HTTP_REMAP` interceptor propagation through
  the routine engine's `JobContext` (regression-tested by the fact
  that absent this propagation, the http call goes to real
  api.telegram.org and 401s on the fake bot token, leaving
  mock_telegram empty — exactly what we caught and fixed)
- Mock Telegram bot recording the inbound sendMessage shape

What's not covered yet (per-scenario follow-ups):
- Telegram channel install + bot-token seed via `/api/extensions/telegram/setup`
  (the current canary uses the http tool directly, bypassing the
  channel install path)
- Mock Sheets write semantics (`values:append`, header-row creation)
- Mock Google Calendar reads
- Mock Hacker News HTTP scrape
- LLM-driven email classification (for the CRM tracker, would need
  per-email-shape canned responses in mock_llm.py)
- Cross-scenario UI flows (routine "Run now" button, manual-trigger
  from Telegram, schedule update via NL chat, disable/enable/delete)

## How to run locally

Foundation (build + venv + Playwright skipped):

```bash
tests/e2e/.venv/bin/python scripts/workflow_canary/run_workflow_canary.py \
  --skip-build --skip-python-bootstrap
```

Output:

```
[workflow-canary] mock telegram listening at http://127.0.0.1:51306
[workflow-canary] === Script 1 — Telegram → Google Sheet Bug Logger ===
[workflow-canary] === Script 2 — Calendar Prep Assistant ===
[workflow-canary] === Script 3 — Hacker News Keyword Monitor ===
[workflow-canary] === Script 4 — Periodic Reminder via Telegram ===
[workflow-canary] === Script 5 — Email → CRM Inbound Tracker ===
[workflow-canary] all 5 probe(s) passed.
```

CLI matches `run_live_canary.py` so the same `scripts/live-canary/run.sh`
dispatcher drives both.

## Adding a new scenario

1. Drop a `scenarios/<name>.py` exporting an `async def run(*, stack,
   mock_telegram_url, output_dir, log_dir) -> list[ProbeResult]`.
2. For Phase 1A coverage, delegate to
   `scenarios._common.run_routine_probe()` — pass the script-specific
   `provider` / `mode` / `routine_name` / `prompt` and you're done.
3. Register in `SCENARIOS` in `run_workflow_canary.py`.
4. For Phase 1B side-effect verification, extend the scenario's `run`
   to drive the relevant API (e.g. `/api/extensions/telegram/setup`,
   reset the mock, fire the routine, then assert on
   `/__mock/sent_messages`). The `ProbeResult.details` dict is the
   right place to capture observed side effects for the artifact.

## Phase 1B follow-up plan

For each script, the next coverage layer is:

| Script | Phase 1B work |
|--------|---------------|
| 1 — Bug Logger | Telegram channel install + Sheets `values:append` mock; assert row appended after `bug:` message injection |
| 2 — Calendar Prep | Calendar `events:list` mock + web-search HTTP mock; assert Telegram briefing fires at lead-time |
| 3 — HN Monitor | HN `/newest` HTTP mock; assert "first immediate run" fires within budget + dedup across runs |
| 4 — Periodic Reminder | Telegram channel install + bot-token seed; assert mock `sendMessage` received the reminder text |
| 5 — CRM Tracker | Gmail seeded inbox + Sheets write; mock LLM canned responses for email classification; assert structured columns + dedup |

Each is an independent follow-up commit on top of this PR's foundation.
