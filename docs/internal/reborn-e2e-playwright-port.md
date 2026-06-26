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

### Step 8: Legacy Skills Settings Port

Added `tests/e2e/scenarios/test_reborn_webui_v2_legacy_skills.py`.

Ported the lifecycle intent from legacy `test_skills.py` to the standalone
Reborn WebUI v2 Settings route:

- Skills settings renders the add-skill form and learned auto-activation
  default control;
- adding a user-managed skill posts `name` and `content` to the v2 install
  endpoint with `X-Confirm-Action: true`;
- editing loads SKILL.md content, saves updated content with
  `X-Confirm-Action: true`, and exits edit mode;
- deleting a user-managed skill shows the native confirm dialog, posts DELETE
  with `X-Confirm-Action: true`, and removes the card after the query reload;
- system and workspace skills remain visible but hide edit, delete, and
  auto-activation controls.

Behavior adjustment:

- The existing legacy tests already mocked `/api/webchat/v2/skills`, but still
  drove the legacy web shell. The port moves those assertions to
  `/v2/settings/skills` under `ironclaw-reborn serve`.
- Reborn's form labels render visibly through the shared design-system field
  component, but the current DOM does not expose those labels to Playwright's
  `get_by_label`. The port uses the stable Reborn placeholders instead.
- The Playwright version in this repo does not expose `page.expect_dialog`, so
  the delete-confirm assertion records and accepts the dialog from the
  `dialog` event handler.

CI update:

- `.github/workflows/reborn-e2e.yml` now includes the Skills settings port in
  the Reborn WebUI v2 Playwright job.

### Step 9: Legacy Extension Lifecycle Port

Added `tests/e2e/scenarios/test_reborn_webui_v2_legacy_extensions.py`.

Ported the extension lifecycle intent from legacy `test_extensions.py` to the
standalone Reborn WebUI v2 Extensions surface:

- registry entries render through `/v2/extensions/registry`;
- registry search filters available extensions;
- installing a registry extension posts the v2 `package_ref` payload and
  refreshes the installed projection;
- installed extensions render status, description, and capability disclosure;
- activating an inactive installed extension posts to the v2 activate endpoint;
- removing an installed extension posts to the v2 remove endpoint through the
  card overflow menu and removes the projection;
- channel and MCP tabs render installed and available entries from the v2
  extension registry/list endpoints.

Behavior adjustment:

- Legacy v1 grouped extension lifecycle under Settings subtabs with legacy
  `/api/extensions*`, `/api/pairing*`, and injected auth-card helpers. Reborn
  exposes the install/manage lifecycle under the top-level `/v2/extensions/*`
  page and talks only to `/api/webchat/v2/extensions*` plus the v2
  connectable-channel projection.
- Reborn cards intentionally use a compact overflow menu for secondary actions
  such as remove. The port asserts the current card/menu behavior rather than
  legacy always-visible action buttons.

CI update:

- `.github/workflows/reborn-e2e.yml` now includes the extension lifecycle port
  in the Reborn WebUI v2 Playwright job.

### Step 10: Legacy Routine/Automation Management Port

Added `tests/e2e/scenarios/test_reborn_webui_v2_legacy_automations.py`.

Ported the user-visible management intent from legacy routine/automation
coverage to the standalone Reborn Automations page:

- scheduled automations render from `/api/webchat/v2/automations`;
- the browser passes the expected `limit=50` and `run_limit=25` list query;
- completed automations are hidden from the default list and fetched through
  `include_completed=true` when the Completed filter is selected;
- filters for Failures, Running, Paused, and Completed show the matching rows;
- the scheduler-disabled response flag surfaces the visible warning;
- outbound delivery defaults render current/available targets and save the
  selected final-reply target through `/api/webchat/v2/outbound/preferences`;
- pause, resume, and delete actions post to the v2 mutation endpoints, with
  delete protected by the native confirm dialog.

Behavior adjustment:

- Legacy v1 routines used `/api/routines/*`, in-page upgrade globals, and the
  legacy chat/routines shell. Reborn has a hidden `/v2/routines` page, but its
  client API is currently a TODO stub around v1 endpoints. The functional
  Reborn successor for scheduled work management is `/v2/automations`, backed
  by `/api/webchat/v2/automations` and the outbound delivery defaults API.
  This step ports the behavior there and records the routines page stub as a
  remaining product parity gap.

CI update:

- `.github/workflows/reborn-e2e.yml` now includes the automations port in the
  Reborn WebUI v2 Playwright job.

### Step 11: Legacy Pending Message Port

Added `tests/e2e/scenarios/test_reborn_webui_v2_legacy_pending_messages.py`.

Ported the caller-visible pending-message intent from legacy
`test_pending_user_messages.py` to Reborn's v2 chat surface:

- a composer send appends the user message optimistically before the v2 send
  request resolves;
- a terminal Reborn projection event reloads the timeline and reconciles the
  optimistic message with the confirmed `accepted_message_ref` row without
  duplicating the user message;
- a failed v2 send renders one error-state optimistic user message and exposes
  the Retry affordance.

Behavior adjustment:

- Legacy v1 exposed `_pendingUserMessages` and `loadHistory()` globals, so
  tests asserted private map cleanup directly. Reborn keeps pending messages in
  React hook state and intentionally preserves a failed optimistic row in the
  visible thread with error styling. The port asserts behavior through the
  composer, `/api/webchat/v2/threads/:id/messages`, terminal SSE projection,
  and `/timeline` reload instead of private hook internals.

CI update:

- `.github/workflows/reborn-e2e.yml` now includes the pending-message port in
  the Reborn WebUI v2 Playwright job.

### Step 12: Legacy Settings Search Port

Added `tests/e2e/scenarios/test_reborn_webui_v2_legacy_settings_search.py`.

Ported the caller-visible settings search intent from legacy
`test_settings_search.py` to Reborn's Settings surface:

- tools search filters the v2 tool-permission rows and updates the visible
  result count;
- clearing search restores the full tool list;
- tools no-match state renders the Reborn empty-filter copy;
- skills search filters installed/workspace skill cards through the mounted
  Settings toolbar;
- channels search filters the v2 extension-backed messaging and MCP groups;
- empty Skills/Channels searches render the shared Reborn settings empty state.

Issue fixed:

- Reborn Settings tabs already accepted `searchQuery`, and the
  `SettingsToolbar` component already implemented the search input, but
  `SettingsPage` did not render that toolbar. The search state therefore could
  never change from the browser. `SettingsPage` now mounts `SettingsToolbar`
  and wires it to the existing import/export/search state.
- The WebUI v2 asset build script watched only the top-level `static/`
  directory. Editing `static/js/**` did not reliably trigger a Cargo rebuild of
  embedded assets, so local E2E runs could keep serving stale `dist/app.js`.
  The build script now emits `rerun-if-changed` for every served static file.
- Settings Channels ignored `channel.display_name` for installed channel
  extensions and fell back to internal package names. The card presenter now
  prefers the installed channel display name, matching the top-level extension
  surface and the registry fallback behavior.

Behavior adjustment:

- Legacy Users search covered the old admin users table. Reborn's
  `fetchUsers`, `createUser`, and `updateUser` settings client methods are
  currently TODO stubs, so Users search remains an operator/admin parity gap
  rather than a meaningful browser port in this step.

CI update:

- `.github/workflows/reborn-e2e.yml` now includes the settings-search port in
  the Reborn WebUI v2 Playwright job.

### Step 13: Legacy Extension Channel Label Regression Port

Extended `tests/e2e/scenarios/test_reborn_webui_v2_legacy_extensions.py`.

Ported the action-label intent from legacy
`test_settings_extensions_labels.py` to Reborn's top-level
`/v2/extensions/channels` surface:

- unauthenticated channel extensions show `Configure`, not `Reconfigure`;
- authenticated channel extensions show `Reconfigure`;
- `setup_required` channel extensions expose one primary configure action and
  do not duplicate it with a second setup menu item;
- clicking `Reconfigure` opens the local configure modal and does not post to
  the activation endpoint.

Issue fixed:

- Reborn extension cards could add duplicate configuration menu actions for
  channel extensions: `setup_required` channels could show both primary
  `Configure` and overflow `Setup`, while ready/authenticated channels could
  receive overlapping `Reconfigure` entries. The card presenter now suppresses
  those duplicate overflow actions.

Behavior adjustment:

- The legacy Settings Channels UI used `Setup` for unauthenticated fallback
  setup. Reborn's top-level Extensions UI uses `Configure` for the same
  unauthenticated configuration action and `Reconfigure` after credentials are
  present, so the port asserts Reborn terminology while preserving the original
  regression intent: do not label an unauthenticated channel as already
  reconfigurable.

### Step 14: Legacy Tool Permissions Port

Added `tests/e2e/scenarios/test_reborn_webui_v2_legacy_tool_permissions.py`.

Ported the caller-visible intent from legacy `test_tool_permissions.py` to
Reborn's Tools settings surface:

- `/v2/settings/tools` renders the global auto-approve control and tool rows;
- changing a mutable tool permission through the Reborn `<select>` posts the
  v2 `/api/webchat/v2/settings/tools/{capability_id}` request and survives a
  browser reload;
- selecting `Follow global` posts the v2 `default` state;
- locked tools show the lock affordance and badge but no editable permission
  select;
- the global auto-approve switch posts to
  `/api/webchat/v2/settings/tools`;
- a real Reborn server API check reads the authoritative tool catalog, updates
  a mutable tool to `disabled`, resets it to `default`, and verifies locked
  tools reject writes when the catalog exposes one.

Behavior adjustment:

- Legacy v1 used per-row toggle buttons and `/api/settings/tools/*`. Reborn
  uses one select per mutable tool, `Follow global` as the no-override state,
  and the `/api/webchat/v2/settings/tools/*` endpoint family.

Testability adjustment:

- Reborn Tools rows now expose stable `data-testid="settings-tool-row"` and
  `data-tool-name` attributes, and locked rows expose
  `data-testid="settings-tool-lock"`. The labels and selects remain accessible
  controls; the row hooks avoid brittle DOM ancestry assertions for locked
  tool coverage.

### Step 15: Legacy CSP Browser-Safety Port

Added `tests/e2e/scenarios/test_reborn_webui_v2_legacy_csp.py`.

Ported the security and browser-safety intent from legacy `test_csp.py` to the
real Reborn WebUI v2 shell:

- authenticated `/v2/` reloads do not emit CSP refusal or content-security
  console errors;
- the rendered DOM does not contain inline event handler attributes such as
  `onclick`, `onchange`, or `onerror`;
- a fresh authenticated page load does not raise browser `pageerror` events;
- core Reborn shell controls remain wired through React handlers: sidebar
  collapse/expand, Settings navigation, Settings search clear, and sidebar
  New chat.

Behavior adjustment:

- The legacy button-wiring check referenced v1-only element IDs such as
  `send-btn`, `restart-btn`, and `logs-clear-btn`. Reborn uses React routes and
  accessible controls instead, so the port asserts visible user behavior rather
  than legacy IDs.

CI update:

- `.github/workflows/reborn-e2e.yml` now includes the CSP/browser-safety port
  in the Reborn WebUI v2 Playwright job.

### Step 16: Legacy Tool Activity History Port

Added `tests/e2e/scenarios/test_reborn_webui_v2_legacy_message_persistence.py`.

Ported the tool-call history-card intent from legacy
`test_message_persistence.py` to Reborn's timeline projection:

- a real `ironclaw-reborn serve` browser turn dispatches the builtin echo
  capability through the `local-dev-yolo` profile;
- the assistant response and tool activity are visible before reload;
- after a full page reload, the timeline rehydrates the assistant reply and
  collapsed activity run from durable `capability_display_preview` records;
- expanding the activity run reveals the persisted echo tool card;
- expanding the tool card shows the persisted result preview.

Testability adjustment:

- Reborn activity runs now expose stable `data-testid` hooks for the run,
  run toggle, run item list, tool card, tool-card toggle, and tool detail
  panel. Tool cards also expose `data-tool-name` and `data-tool-status` so
  tests can assert the persisted card without depending on styling classes.

Harness adjustment:

- The mock LLM now has a Reborn-specific `reborn builtin echo ...` trigger
  that emits `builtin__echo`, matching the provider-facing name for
  Reborn's `builtin.echo` capability. The existing legacy `echo ...` trigger
  is unchanged for the legacy gateway/v2-engine tests that still use the
  unqualified legacy tool name.

CI update:

- `.github/workflows/reborn-e2e.yml` now includes the message-persistence
  activity-card port in the Reborn WebUI v2 Playwright job.

### Step 17: Legacy Tool Execution API Port

Added `tests/e2e/scenarios/test_reborn_webui_v2_legacy_tool_execution.py`.

Ported the direct API intent from legacy `test_tool_execution.py` to
standalone Reborn's `/api/webchat/v2` surface:

- a v2 message can dispatch the builtin echo capability and persist a
  completed `capability_display_preview` record containing the echoed output;
- a v2 message can dispatch the builtin time capability and persist a
  completed preview containing the time result;
- a normal non-tool message still finalizes an assistant response and does not
  create capability-preview records.

Behavior adjustment:

- Legacy gateway history exposed tool calls as unqualified names (`echo`,
  `time`) under `/api/chat/history`. Standalone Reborn exposes first-party
  capabilities as `builtin.echo` and `builtin.time`, with provider-facing tool
  names `builtin__echo` and `builtin__time`, and stores browser-visible tool
  results as `capability_display_preview` timeline records.

Harness adjustment:

- The mock LLM now has a Reborn-specific `reborn builtin time` trigger that
  emits `builtin__time`, matching the provider-facing name for Reborn's
  `builtin.time` capability. The existing legacy `what time` trigger is
  unchanged for legacy gateway/v2-engine tests.

CI update:

- `.github/workflows/reborn-e2e.yml` now includes the tool-execution API port
  in the Reborn WebUI v2 Playwright job.

### Step 18: Legacy DOM Resource-Limit Port

Added `tests/e2e/scenarios/test_reborn_webui_v2_legacy_dom_resource_limits.py`.

Ported the user-visible resource-limit intent from legacy
`test_dom_resource_limits.py` to Reborn's history model:

- the chat page requests initial timeline history with `limit=50`;
- the DOM renders only the first 50 timeline messages on initial load, even
  when older history is available;
- older history is fetched only after the user activates the visible "Load older
  messages" control;
- one explicit load appends one older 50-message page without auto-loading the
  rest of a long thread.

Behavior adjustment:

- Legacy gateway used global browser functions (`addMessage`,
  `pruneOldMessages`, `connectSSE`, `jobEvents`) and capped a growing in-page
  transcript with DOM pruning. Reborn does not expose those v1 globals. Its
  equivalent browser resource contract is timeline pagination through the
  `/api/webchat/v2/threads/{thread_id}/timeline` boundary, with 50-message
  pages and explicit user-driven older-page loading.

Frontend harness adjustment:

- Added stable Reborn message-list selectors for the scroll container, content
  container, and "Load older messages" control so paging/resource assertions do
  not depend on incidental Tailwind class structure.

CI update:

- `.github/workflows/reborn-e2e.yml` now includes the DOM resource-limit port
  in the Reborn WebUI v2 Playwright job.

## Open Migration Buckets

Not yet ported:

- remaining legacy chat UI affordances that have Reborn equivalents;
- remaining legacy SSE/history edge cases such as active-thread fallback and
  read-only external-channel refresh behavior where Reborn has a matching
  product concept;
- remaining DOM/resource-limit scenarios for Reborn-specific SSE reconnect
  timer cleanup and any future capped long-running activity stores;
- deeper tool approval scenarios that need real Reborn runtime/tool execution,
  persistence, or recovery beyond the browser approval-card and persisted
  activity-card contracts;
- remaining settings/extension lifecycle scenarios beyond Settings search,
  Skills, tool permissions, channel label regressions, and the top-level extension
  install/manage surface;
- OAuth/product-auth flows;
- Slack/Telegram/channel pairing scenarios;
- admin/operator flows, including Reborn Settings Users search once the v2
  users endpoints replace the current TODO client stubs;
- legacy `/v2/routines` parity, because the current Reborn routines page still
  uses TODO client stubs instead of real v2 endpoints;
- Emulate provider full-path scenarios against standalone Reborn where the
  current test still routes through legacy `/api/chat/*`.

## Issues Found

The first confirmed issues were test-harness coupling between Reborn scenario
files, an imported-fixture dependency gap, and a missing `local-dev-yolo`
`--confirm-host-access` acknowledgement in the extracted fixture. All are test
harness issues fixed in this branch.

The attachment browser port initially failed with the mock response
`Skill '/workspace' is not installed or was not found.` That was traced to the
test mock's slash-skill detector treating generated attachment storage paths as
explicit slash skills. Reborn's attachment projection was behaving as intended;
the mock heuristic was fixed.

The routine/automation inspection found a product parity gap: the Reborn
`/v2/routines` page imports a client API whose operations still return TODO
stub responses and reference legacy v1 routine endpoints. The migrated tests
therefore target Reborn's real `/v2/automations` surface for scheduled-work
management, while leaving dedicated routines-page parity as open work.

The settings-search inspection found a product defect: Reborn Settings search
logic existed inside the tabs and toolbar component, but the toolbar was not
mounted by `SettingsPage`, so no browser-visible control could update
`searchQuery`. This branch mounts the toolbar and covers tools, skills, and
channels search through the real `/v2/settings/*` routes.

The same step found a frontend build-system defect: `cargo build` embedded the
committed WebUI v2 bundle, and the static crate's build script did not track
individual asset-file mtimes. Source edits under `static/js/**` could therefore
be invisible to a local rebuild until `static/dist/app.js` was regenerated and
embedded. The build script now tracks served asset files explicitly.

The settings Channels card also displayed installed channel package names
instead of `display_name` when no registry entry was present. That is fixed in
the Settings Channels presenter.

The extension label port found duplicate channel configuration menu actions in
the top-level Extensions surface. `setup_required` channels no longer get a
secondary overflow `Setup` action when the primary `Configure` button already
covers setup, and ready/authenticated channels no longer receive overlapping
`Reconfigure` menu items.

The tool-permission port did not require a behavior fix in the Reborn settings
API: the v2 endpoint already persisted mutable tool overrides and rejected
locked-tool writes. It did add stable row hooks to the Tools tab so locked-tool
coverage can assert the real row without depending on incidental DOM ancestry.

The CSP/browser-safety port did not require a Reborn product fix. Initial
focused failures were selector mismatches in the port (`New` is a scoped
sidebar button in Reborn, not a legacy `New thread` link).

The tool-activity history port did not require a Reborn product behavior fix:
durable `capability_display_preview` records already rehydrated the activity
card after reload. It did add stable test hooks to the activity UI and a
Reborn-specific mock trigger so the browser test dispatches `builtin__echo`
instead of the legacy unqualified `echo` tool name.

The tool-execution API port confirmed the same namespacing distinction for
`builtin.time`. No Reborn behavior fix was required; the port asserts Reborn's
native capability ids and timeline-preview records instead of legacy
`/api/chat/history` turn fields.

The same settings inspection confirmed an admin parity gap: Reborn Settings
Users client methods still return TODO stub responses instead of calling real
v2 users endpoints. Legacy Users search is left open until those endpoints are
implemented.
