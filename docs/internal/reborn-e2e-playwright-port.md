# Reborn Playwright E2E Port Worklog

Branch: `reborn-port-legacy-e2e-playwright`

Objective: functionally port the legacy `tests/e2e` Playwright coverage from
the legacy `ironclaw` web gateway to the standalone `ironclaw-reborn serve`
WebUI v2 surface, adjusting assertions for Reborn UI/API shape and fixing real
issues discovered during the migration.

## Current Test Surface

Legacy/general Playwright suite:

- Primary binary: `target/debug/ironclaw`
- UI entry: `/?token=<AUTH_TOKEN>`
- API family: `/api/chat/*` plus legacy gateway endpoints
- Shared selectors: `helpers.SEL`
- Approximate source inventory at migration start: 48 legacy/general files,
  about 405 `test_*` definitions.

Reborn WebUI v2:

- Primary binary: `target/debug/ironclaw-reborn`
- UI entry: `/v2/?token=<REBORN_V2_AUTH_TOKEN>`
- API family: `/api/webchat/v2/*`
- Shared selectors: `helpers.SEL_V2`
- Dedicated CI before this work: `test_reborn_webui_v2_smoke.py` only.

## Porting Rules

- Port behavior intent, not legacy DOM structure.
- Use real `ironclaw-reborn serve` for Reborn browser tests.
- Keep Reborn selectors in `SEL_V2` or explicit accessible roles when the UI
  already has stable labels.
- Prefer shared Reborn harness fixtures over importing fixtures from another
  scenario file.
- Record every mismatch as one of:
  - ported behavior;
  - Reborn product gap;
  - legacy-only behavior intentionally not ported;
  - test harness gap fixed in this branch.

## Progress

### Step 1: Shared Reborn Playwright Harness

Added `tests/e2e/reborn_webui_harness.py`.

This centralizes:

- `ironclaw-reborn serve` startup and teardown;
- local-dev and local-dev-yolo profiles;
- Reborn v2 browser/page fixtures;
- bearer headers and v2 thread/message/timeline helpers;
- mock LLM config generation and coverage env forwarding.

Updated existing Reborn tests to use the shared harness:

- `test_reborn_webui_v2_smoke.py`
- `test_reborn_v2_file_download.py`

Issue fixed:

- `test_reborn_v2_file_download.py` previously imported process helpers and a
  browser fixture from `test_reborn_webui_v2_smoke.py`, coupling test modules by
  import order. The shared harness removes that dependency.
- The first extraction left `test_reborn_v2_file_download.py` importing only the
  `reborn_v2_yolo_page` fixture. Pytest collected that fixture but not its
  imported dependencies (`reborn_v2_yolo_server`, `reborn_v2_browser`) when the
  file was run alone, so setup failed with `fixture 'reborn_v2_yolo_server' not
  found`. The scenario now imports the dependent fixtures explicitly.
- The extracted yolo-profile server fixture initially started
  `ironclaw-reborn serve` without `--confirm-host-access`. Current Reborn
  runtime policy correctly rejects `local-dev-yolo` without explicit operator
  acknowledgement, so the fixture now passes `--confirm-host-access` only for
  `local-dev-yolo`.
- Reborn first-party tools have internal capability ids such as
  `builtin.write_file`, but the provider-facing tool names sanitize dots as
  `__`. The mock LLM download-chip fixture was still emitting the legacy
  `write_file` name, then the internal capability id. It now emits
  `builtin__write_file`, which Reborn maps back to `builtin.write_file`.
- `local-dev-yolo` grants the host runtime profile, but the Tools settings
  global auto-approve switch remains authoritative for dispatch gates. The
  yolo harness now enables `/api/webchat/v2/settings/tools` before browser
  tests that expect write-file calls to proceed without an approval card.

### Step 2: First Legacy Core Port

Added `tests/e2e/scenarios/test_reborn_webui_v2_legacy_core.py`.

Ported initial behavior from legacy `test_connection.py` and basic
`test_chat.py`:

- authenticated shell loads;
- sidebar navigation reaches Reborn routes;
- missing token shows the Reborn login token view;
- a browser-sent message receives a mock LLM reply;
- sequential browser messages render user and assistant bubbles;
- empty/whitespace sends do not create messages.

CI update:

- `.github/workflows/reborn-e2e.yml` now runs the new legacy-core Reborn
  Playwright port with the existing WebUI v2 smoke scenario.

## Open Migration Buckets

Not yet ported:

- legacy attachment upload/PDF/PPTX extraction assertions;
- legacy SSE reconnect/history-reload edge cases;
- DOM pruning/resource-limit scenarios;
- HTML injection and markdown sanitization scenarios;
- tool approval scenarios;
- skills/settings/extension lifecycle scenarios;
- OAuth/product-auth flows;
- Slack/Telegram/channel pairing scenarios;
- routines/automations/admin/operator flows;
- Emulate provider full-path scenarios against standalone Reborn where the
  current test still routes through legacy `/api/chat/*`.

## Issues Found

No Reborn product defects have been confirmed yet in this branch. The first
confirmed issues were test-harness coupling between Reborn scenario files, an
imported-fixture dependency gap, and a missing `local-dev-yolo`
`--confirm-host-access` acknowledgement in the extracted fixture. All are test
harness issues fixed in this branch.
