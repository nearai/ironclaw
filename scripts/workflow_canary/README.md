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
├── run_workflow_canary.py     # entrypoint
├── telegram_mock.py            # fake Telegram Bot API
├── sheets_mock.py              # mock Google Sheets v4
├── calendar_mock.py            # mock Google Calendar v3
├── hn_mock.py                  # mock Hacker News /newest
├── gmail_mock.py               # mock Gmail v1
├── web_search_mock.py          # mock Brave Search v3
├── telegram_setup.py           # install + capability patch + pairing helpers
├── routines.py                 # libSQL helpers (insert + backdate + poll)
└── scenarios/
    ├── _common.py                              # shared run_routine_probe()
    ├── bug_logger.py                           # Script 1 — Sheet write
    ├── calendar_prep.py                        # Script 2 — Calendar → Telegram
    ├── hn_monitor.py                           # Script 3 — HN → Telegram
    ├── periodic_reminder.py                    # Script 4 — periodic reminder
    ├── crm_tracker.py                          # Script 5 — Gmail → Sheets CRM
    ├── manual_trigger.py                       # POST /api/routines/<id>/trigger
    ├── lifecycle.py                            # disable/enable/delete via API
    ├── dedup_cooldown.py                       # cooldown_secs back-to-back
    ├── nl_routine_create.py                    # NL → routine_create tool
    ├── nl_schedule_update.py                   # NL → routine_update tool
    ├── telegram_channel_install.py             # install + setup → Active
    ├── telegram_round_trip.py                  # webhook → agent → reply
    ├── routine_visibility_from_telegram.py     # paired user → list routines
    ├── manual_trigger_from_telegram.py         # paired user → trigger now
    ├── first_immediate_run.py                  # fire_immediately ≤ 10s
    ├── idempotent_disable_enable.py            # double-toggle is no-op
    ├── cron_timing_accuracy.py                 # next_fire_at ±10s
    └── log_assertions.py                       # gateway log scan (runs LAST)
```

`scripts/live-canary/run.sh` dispatches `LANE=workflow-canary` here, and
`.github/workflows/live-canary.yml` has a matching `workflow-canary`
job in the live-canary matrix.

## Mock surfaces

Every external Google / external service is replaced by a single-port
aiohttp mock in this directory. Mocks announce their port via
`MOCK_<NAME>_PORT=<n>` on stdout (caught by `wait_for_port_line`),
expose Bot-API-equivalent handlers under their canonical paths, and
provide `/__mock/...` test hooks for seeding / draining / resetting.

`run_workflow_canary.py` builds a comma-joined
`IRONCLAW_TEST_HTTP_REMAP` for the gateway env so outbound HTTP for
`api.telegram.org`, `sheets.googleapis.com`, `www.googleapis.com`,
`news.ycombinator.com`, `gmail.googleapis.com`, and
`api.search.brave.com` lands at the corresponding mock loopback
address.

## What's covered today

20 probes across 7 phases:

**Phase 1 (Sheets):** bug_logger asserts the routine fires AND a row
gets appended to mock_sheets with the correct timestamp / message /
source shape. Catches the canonical "expected a sequence" regression.

**Phase 2 (Calendar):** calendar_prep seeds a deterministic event,
asserts mock_calendar saw events.list AND mock_telegram received a
prep briefing referencing the seeded event title.

**Phase 3 (HN):** hn_monitor seeds two posts, asserts mock_hn
received the /newest fetch AND mock_telegram captured a summary
mentioning both posts.

**Phase 4 (CRM):** crm_tracker seeds 1 lead + 1 newsletter + 1
receipt, asserts exactly ONE row gets appended to mock_sheets (the
lead only) with all 6 expected columns.

**Phase 5 (Telegram):** telegram_channel_install runs the full
install + capability patch + setup flow and asserts the channel
reaches the installed/active state. telegram_round_trip posts an
inbound webhook and asserts mock_telegram receives an outbound
sendMessage with the actual chat_id (not 'default').
routine_visibility_from_telegram + manual_trigger_from_telegram
exercise the post-pairing chat path.

**Phase 6 (Stability):** first_immediate_run asserts a backdated
routine fires within 10s. log_assertions runs LAST and scans
`gateway.log` for known fail-criterion regex patterns
(`chat_id 'default'`, `parsed naive timestamp without timezone`,
`retry after None`, `expected a sequence`).

**Phase 7 (Timing):** cron_timing_accuracy sets `next_fire_at` to
"now + 5s" and asserts the actual fire happens within ±10s.
idempotent_disable_enable double-toggles enable/disable and asserts
both halves are no-ops.

## How to run locally

```bash
tests/e2e/.venv/bin/python scripts/workflow_canary/run_workflow_canary.py \
  --skip-build --skip-python-bootstrap
```

Run a single scenario:

```bash
tests/e2e/.venv/bin/python scripts/workflow_canary/run_workflow_canary.py \
  --skip-build --skip-python-bootstrap \
  --scenario telegram_round_trip
```

CLI matches `run_live_canary.py` so the same `scripts/live-canary/run.sh`
dispatcher drives both.

## Adding a new scenario

1. Drop a `scenarios/<name>.py` exporting an `async def run(*, stack,
   mock_telegram_url, mock_sheets_url=None, mock_calendar_url=None,
   mock_hn_url=None, mock_gmail_url=None, mock_web_search_url=None,
   output_dir, log_dir) -> list[ProbeResult]`.
2. For routine-only coverage, delegate to
   `scenarios._common.run_routine_probe()` — pass the script-specific
   `provider` / `mode` / `routine_name` / `prompt` and you're done.
3. For side-effect verification, drive the relevant API directly and
   read back from the appropriate mock's `/__mock/` endpoint. The
   `ProbeResult.details` dict is the right place to capture observed
   side effects for the artifact.
4. Register in `SCENARIOS` in `run_workflow_canary.py`. Order matters
   — `log_assertions` should always run last so it sees the full log
   surface.
5. For new mocks, add a `<name>_mock.py` in this directory, a
   `_spawn_mock_<name>` helper in the runner, and an entry in the
   comma-joined `IRONCLAW_TEST_HTTP_REMAP`.

## Deferred coverage

- **Real-provider variant** (`workflow-canary-live`) — same probes
  with real Gmail / Calendar / Sheets credentials. Separate lane.
- **Auth recovery** (token revocation → auth_required SSE event with
  valid auth_url) — requires real OAuth setup; covered by
  `auth-live-canary`.
- **UI flows** (Approval modal, Reconfigure flow, "Run now" button,
  Routines tab interactions) — Playwright concerns; belong in
  `auth-browser-consent` or a new `routine-ui-canary` lane.
- **App-level dedup across cron fires** — would need a feature on
  the agent side to track already-reported items; not implemented.
