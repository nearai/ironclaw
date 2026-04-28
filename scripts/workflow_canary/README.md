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

## What's covered today (Phase 1A)

Every scenario runs the same shape:

1. Insert a `Lightweight` cron routine directly into libSQL with
   `next_fire_at` backdated by 60 s.
2. Wait for the cron ticker (configured to 2 s in
   `run_workflow_canary.py`) to pick it up.
3. The mock LLM (`tests/e2e/mock_llm.py`) responds to the routine's
   prompt; the engine completes the lightweight action.
4. Assert `routine_runs` has at least one row in a terminal status
   (`ok` / `attention` / `failed`); success requires `ok` or
   `attention`.

This catches regressions in:
- `RoutineEngine::spawn_cron_ticker` and the `check_cron_triggers`
  loop
- `RoutineAction::Lightweight` execution path
- Routine state machine (`routines.next_fire_at` → `routine_runs`
  status transitions)
- DB serialization of action_config / trigger_config
- Mock-LLM round-trip latency under cron-tick scheduling

What it deliberately does **not** cover yet:
- Telegram channel install + bot-token seed (per-scenario Phase 1B)
- Mock Sheets write semantics (`values:append`, header-row creation)
- Mock Google Calendar reads
- Mock Hacker News HTTP scrape
- LLM-driven email classification for the CRM tracker
- Cross-scenario verification (e.g., routine "Run now" button in the
  UI, manual-trigger from Telegram, schedule update via NL chat)

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
