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

### Step 3: Legacy Rendering Safety Port

Added `tests/e2e/scenarios/test_reborn_webui_v2_legacy_rendering.py`.

Ported behavior from legacy `test_html_injection.py` through the real Reborn
WebUI v2 chat caller path:

- assistant markdown containing script, iframe, and event-handler payloads is
  sanitized before rendering;
- user-supplied HTML-shaped text remains plain text and is escaped in the DOM;
- assistant messages do not create script DOM nodes.

CI update:

- `.github/workflows/reborn-e2e.yml` now includes the rendering-safety port in
  the Reborn WebUI v2 Playwright job.

### Step 4: Legacy Attachment Browser Port

Added `tests/e2e/scenarios/test_reborn_webui_v2_legacy_attachments.py`.

Ported the first attachment behavior from legacy `test_chat.py` through the
real Reborn WebUI v2 browser path:

- staged multiple files in the composer;
- rendered attachment chips before send;
- rendered user-message attachment cards after send;
- proved text extracted from a browser-uploaded document reaches the mock LLM;
- verified attachment cards survive a full page reload from the v2 timeline.

Issue fixed:

- The mock LLM's missing-slash-skill heuristic inspected the fully
  model-visible user message. Reborn correctly appends generated
  `<attachments>` context containing `/workspace/...` storage paths, and the
  mock mistook those paths for user-typed slash skills before the canned
  attachment response could match. The heuristic now strips generated
  attachment context only for slash-skill detection; the canned-response matcher
  still sees extracted attachment text.

CI update:

- `.github/workflows/reborn-e2e.yml` now includes the attachment browser port
  in the Reborn WebUI v2 Playwright job.

### Step 4b: Legacy Attachment Validation Port

Extended `test_reborn_webui_v2_legacy_attachments.py`.

Ported the legacy attachment batch-validation intent to Reborn's staging alert
UI and server-advertised attachment limits:

- count limit rejects the 11th staged file and keeps the first 10;
- per-file limit rejects files larger than 5 MB;
- total-message limit keeps files that fit and rejects the file that would push
  the batch over 10 MB;
- unsupported MIME types are rejected before send.

Legacy-only / non-1:1 case:

- The legacy gateway allowed files-only sends through `/api/chat/send`.
  Reborn's v2 browser and DTO contract require non-empty message text, so that
  behavior is intentionally not copied as a browser affordance. Attachment-only
  storage/model projection remains covered at lower contract layers and the
  Reborn browser port verifies text-plus-attachment send/reload.

### Step 4c: Legacy Attachment Extraction Port

Extended `test_reborn_webui_v2_legacy_attachments.py`.

Ported the model-payload assertions from legacy attachment tests to Reborn's
native attachment projection:

- PDF, text, and PPTX extracted text reach the mock LLM request;
- image attachments are sent through the multimodal data URL path when the
  selected model is vision-classified;
- corrupt PDF attachments still reach the model with Reborn's
  `text extraction unavailable` fallback marker.

Behavior adjustment:

- The legacy v1 gateway persisted an explicit `[Failed to extract ...]`
  placeholder. Reborn stores `extracted_text = None` on extraction failure and
  renders `[Document attached - text extraction unavailable]` in the
  model-visible attachment block. The Reborn port asserts that fallback instead
  of the legacy string.
- The shared Reborn harness now has a dedicated vision-model fixture selecting
  `gpt-4o` while still routing to the deterministic mock LLM. The default
  `mock-model` remains intentionally text-only, matching Reborn's production
  vision gate that drops image parts for non-vision models while preserving the
  stored attachment pointer.

### Step 5: Legacy Chat Action Port

Added `tests/e2e/scenarios/test_reborn_webui_v2_legacy_chat_actions.py`.

Ported the per-message copy behavior from legacy `test_chat.py` to Reborn's
message action buttons:

- user-message copy writes the user-authored raw text;
- assistant-message copy writes the raw markdown content, not rendered link
  text;
- the copy action flips to the copied state and then returns to the normal
  action label.

Legacy-only / non-1:1 cases in the same area:

- v1 slash autocomplete was tied to the legacy gateway chat input and
  `#slash-autocomplete` DOM. Reborn does not currently expose that widget; its
  slash command handling is routed through the Reborn send path and targeted
  auth/product tests.
- v1 turn-cost events no longer append legacy message badges in Reborn's
  message renderer; cost is surfaced in Reborn's activity/admin/project
  surfaces instead of the old `turn_cost` DOM event path.

CI update:

- `.github/workflows/reborn-e2e.yml` now includes the chat-action port in the
  Reborn WebUI v2 Playwright job.

### Step 6: Legacy Approval UI Port

Added `tests/e2e/scenarios/test_reborn_webui_v2_legacy_approval.py`.

Ported the browser-visible approval-card behavior from legacy
`test_tool_approval.py` to Reborn's WebChat v2 gate surface:

- approval gate SSE frames render the Reborn approval card;
- tool name, reason, structured details, and long command payloads render in
  the card;
- long command details start collapsed and can be expanded with the visible
  `View full command` action;
- approve, deny, and approve-and-always all post to the v2
  `/api/webchat/v2/threads/{thread}/runs/{run}/gates/{gate}/resolve` endpoint;
- the always-allow checkbox changes the primary button action from normal
  approve to approve-and-always.

Behavior adjustment:

- Legacy v1 posted approval decisions through `/api/chat/approval` and rendered
  resolved text inside the old `.approval-card` DOM. Reborn resolves approvals
  through the run-scoped gate endpoint and removes the pending card after a
  terminal resolution response. The port asserts the v2 request contract and
  hidden-card outcome instead of legacy resolved-copy text.
- Reborn's v1-compatible `approve(requestId, action, kind)` wrapper always
  includes `always: false` for normal approve/deny and `always: true` only for
  approve-and-always. The browser port records that shape explicitly.

CI update:

- `.github/workflows/reborn-e2e.yml` now includes the approval UI port in the
  Reborn WebUI v2 Playwright job.

### Step 7: Legacy SSE and History Persistence Port

Added `tests/e2e/scenarios/test_reborn_webui_v2_legacy_sse_history.py`.

Ported the durable-history and reconnect intent from legacy
`test_message_persistence.py` and `test_sse_reconnect.py`:

- a completed Reborn v2 text turn is reloaded from `/api/webchat/v2` timeline
  after a full browser page reload;
- user and assistant bubbles remain visible after reload;
- a hidden-tab SSE pause closes the active stream and a visible-tab resume
  opens a fresh v2 EventSource;
- the resumed EventSource carries the last observed event cursor via
  `after_cursor`;
- a visibility pause/resume does not refetch the timeline or re-render the
  message DOM when no terminal run event occurred.

Behavior adjustment:

- Legacy v1 relied on in-page globals such as `eventSource`,
  `sseHasConnectedBefore`, `currentThreadId`, and `/api/chat/history`.
  Reborn's equivalent behavior lives in the `useSSE` hook and v2 timeline API.
  The port asserts the caller-visible effect: fresh EventSource URLs include
  `after_cursor` after resume, and already-rendered history is not torn down.

CI update:

- `.github/workflows/reborn-e2e.yml` now includes the SSE/history port in the
  Reborn WebUI v2 Playwright job.

## Open Migration Buckets

Not yet ported:

- remaining legacy chat UI affordances that have Reborn equivalents;
- remaining legacy SSE/history edge cases such as active-thread fallback and
  read-only external-channel refresh behavior where Reborn has a matching
  product concept;
- DOM pruning/resource-limit scenarios;
- deeper tool approval scenarios that need real Reborn runtime/tool execution,
  persistence, or recovery beyond the browser approval-card contract;
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

The attachment browser port initially failed with the mock response
`Skill '/workspace' is not installed or was not found.` That was traced to the
test mock's slash-skill detector treating generated attachment storage paths as
explicit slash skills. Reborn's attachment projection was behaving as intended;
the mock heuristic was fixed.
