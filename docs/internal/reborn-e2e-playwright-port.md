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
- the first browser-created chat appears in the Reborn app sidebar as a normal
  conversation row using the derived first-message title;
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
- selected assistant message content populates only `text/plain` clipboard data
  and leaves `text/html` empty;
- the copy action flips to the copied state and then returns to the normal
  action label.

Issue fixed:

- Reborn WebUI v2 did not have the legacy chat selection-copy guard, so copying
  selected rendered markdown could allow the browser to populate rich HTML
  clipboard data. `MessageList` now handles copy events inside the chat surface,
  writes only the selected plain text, clears other clipboard formats, and
  prevents the default rich-copy behavior.

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
- bare approval keywords (`yes`, `no`, `always`) sent while no approval gate is
  pending create ordinary chat turns instead of being treated as hidden approval
  responses.

Behavior adjustment:

- Legacy v1 posted approval decisions through `/api/chat/approval` and rendered
  resolved text inside the old `.approval-card` DOM. Reborn resolves approvals
  through the run-scoped gate endpoint and removes the pending card after a
  terminal resolution response. The port asserts the v2 request contract and
  hidden-card outcome instead of legacy resolved-copy text.
- Reborn's v1-compatible `approve(requestId, action, kind)` wrapper always
  includes `always: false` for normal approve/deny and `always: true` only for
  approve-and-always. The browser port records that shape explicitly.
- Legacy had a text-alias interception path for pending approval cards. Reborn
  blocks arbitrary sends while a gate is pending, so the migrated text-keyword
  regression focuses on the no-pending-gate contract that those words remain
  normal chat content.

Issue fixed:

- The first migrated keyword regression waited for assistant-message rendering
  before sending the next keyword, but assistant rendering can win the race
  against the composer send gate reopening. A full migrated-suite rerun exposed
  this as a dropped third keyword. The test now waits on the composer
  `data-send-disabled="false"` contract before each send.

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
- an in-app sidebar switch from one thread to another opens the second thread's
  SSE stream without carrying the first thread's `after_cursor`.
- two browser tabs open to the same Reborn thread both receive the same
  terminal run update and reload the v2 timeline to render the assistant reply.
- excess Reborn v2 SSE event streams for one `(tenant, user)` are rejected at
  the per-caller concurrency cap with a retryable 429 response.
- idle Reborn v2 SSE event streams emit keepalive comment frames so browser and
  proxy connections are not silently dropped while no projection events exist.

Behavior adjustment:

- Legacy v1 relied on in-page globals such as `eventSource`,
  `sseHasConnectedBefore`, `currentThreadId`, and `/api/chat/history`.
  Reborn's equivalent behavior lives in the `useSSE` hook and v2 timeline API.
  The port asserts the caller-visible effect: fresh EventSource URLs include
  `after_cursor` after resume, and already-rendered history is not torn down.
- Legacy stale-`Last-Event-ID` coverage was adapted to Reborn's route-scoped
  `threadId` model. The Reborn port verifies that a prior thread cursor is
  dropped on thread switch instead of replayed against a different
  `/api/webchat/v2/threads/:id/events` stream.

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
- registry keyword disclosures show available extension keywords;
- installing a registry extension posts the v2 `package_ref` payload and
  refreshes the installed projection;
- installed extensions render status, description, and capability disclosure;
- installed extensions whose backend payload has `tools: null` render a no
  capabilities state instead of breaking the card;
- activating an inactive installed extension posts to the v2 activate endpoint;
- removing an installed extension posts to the v2 remove endpoint through the
  card overflow menu only after user confirmation and removes the projection;
- cancelling the remove confirmation keeps the extension card and sends no
  remove request;
- setup-required extensions open the Reborn configure modal, fetch setup
  metadata, submit manual secrets and fields to the v2 setup endpoint, and close
  on success;
- configure modal Cancel, backdrop click, and Escape dismissal close the modal
  without submitting setup;
- pressing Enter in a manual configuration input submits the same v2 setup
  payload as the Save button;
- Telegram channel configuration accepts and preserves bot tokens with the
  colon, underscore, and hyphen characters used by real Telegram bot tokens;
- failed setup responses keep the configure modal open and show the server
  message so the user can correct the credential;
- activation responses that include legacy-style `auth_url` values reject
  non-HTTPS URLs and still accept mixed-case HTTPS schemes;
- OAuth setup start posts provider/scopes metadata to the v2 OAuth-start
  endpoint;
- OAuth authorization URLs are opened only when they parse as HTTPS, including
  mixed-case `HTTPS://` schemes;
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
- Reborn configure-success currently closes the modal without rendering the
  setup response message as a toast. The port asserts the durable behavior
  contract: the modal closes and the v2 setup payload is posted.

Issue fixed:

- Reborn's extension overflow-menu Remove action previously posted the remove
  request immediately. The hook now shows a native confirmation prompt before
  calling the v2 remove mutation, matching the destructive-action guard already
  present in the legacy extension lifecycle tests.

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
- recent run history renders status, thread id, run id, current-run metadata,
  log navigation, and run-thread navigation through the Reborn detail panel;
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
- an unconfirmed optimistic message survives switching away from the thread and
  back through the real sidebar while the refreshed timeline is still empty;
- Reborn projection run-status events mark the active thread as `Running` in
  the app sidebar and clear that marker after a terminal run event reloads the
  timeline;
- a failed v2 send renders one error-state optimistic user message and exposes
  the Retry affordance.

Behavior adjustment:

- Legacy v1 exposed `_pendingUserMessages` and `loadHistory()` globals, so
  tests asserted private map cleanup directly. Reborn keeps pending messages in
  React hook state and intentionally preserves a failed optimistic row in the
  visible thread with error styling. The port asserts behavior through the
  composer, `/api/webchat/v2/threads/:id/messages`, terminal SSE projection,
  and `/timeline` reload instead of private hook internals.
- The legacy reconnect-race test forced a v1 SSE reconnect to call
  `loadHistory()`. Reborn's v2 visibility reconnect keeps the rendered history
  intact and does not refetch by itself, so the functional port uses the real
  sidebar thread switch and timeline reload path to protect the same invariant:
  an unconfirmed optimistic message is not erased by a history refresh.
- Legacy background-thread processing indicators relied on v1 thread metadata
  and unread badges. Reborn's current standalone browser only receives
  per-active-thread run status through the v2 EventSource; the port asserts the
  supported sidebar state-store behavior for the active thread and leaves
  background-thread fan-out to the existing user-scoped stream/list enrichment
  follow-up.

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

- Legacy Users search covered the old admin users table. Reborn's Admin page
  still depends on `pages/admin/lib/admin-api.js`, whose users, dashboard, and
  usage methods intentionally return TODO stub payloads until v2 admin
  endpoints replace the legacy `/api/admin/*` contracts. Admin/operator
  browser parity therefore remains open rather than a meaningful port in this
  step.

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
  create capability-preview records;
- a single Reborn turn can dispatch both builtin echo and builtin time calls,
  persist completed previews for both capabilities, and finalize the
  multi-tool summary response;
- a sequential Reborn planned loop can run echo, observe the result, run time,
  observe the result, and then finalize the mock's multi-step completion
  response without looping.

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
- The mock LLM also has a Reborn-specific `reborn parallel echo and time`
  trigger that emits `builtin__echo` plus `builtin__time`. The legacy
  `parallel echo and time` trigger remains unchanged for the unqualified
  gateway/v2-engine tool surface. The existing `multi step echo then time`
  trigger already selects Reborn provider names from the advertised tool list.

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
- an SSE error schedules Reborn's first reconnect timeout, and hiding the tab
  clears that pending reconnect timeout instead of leaving it active.

Behavior adjustment:

- Legacy gateway used global browser functions (`addMessage`,
  `pruneOldMessages`, `connectSSE`, `jobEvents`) and capped a growing in-page
  transcript with DOM pruning. Reborn does not expose those v1 globals. Its
  equivalent browser resource contract is timeline pagination through the
  `/api/webchat/v2/threads/{thread_id}/timeline` boundary, with 50-message
  pages and explicit user-driven older-page loading.
- Legacy v1 tracked reconnect/status-polling interval leaks. Reborn's matching
  risk is the `useSSE` reconnect timeout. The port instruments browser timers
  and isolates the 2000ms first-reconnect delay because the SPA also has
  unrelated React Query timers in the same page.

Frontend harness adjustment:

- Added stable Reborn message-list selectors for the scroll container, content
  container, and "Load older messages" control so paging/resource assertions do
  not depend on incidental Tailwind class structure.

CI update:

- `.github/workflows/reborn-e2e.yml` now includes the DOM resource-limit port
  in the Reborn WebUI v2 Playwright job.

### Step 19: Legacy Product Auth Prompt Port

Added `tests/e2e/scenarios/test_reborn_webui_v2_legacy_auth_flows.py`.

Ported the browser-visible auth prompt intent from legacy OAuth/MCP/skill auth
flows to Reborn's WebUI v2 chat gate model:

- an `auth_required` SSE prompt with `challenge_kind=manual_token` renders the
  manual token card, rejects empty submit, trims the entered token, calls
  `/api/reborn/product-auth/manual-token/submit`, and resolves the paused run
  with `credential_provided`;
- an `auth_required` SSE prompt with `challenge_kind=oauth_url` renders the
  OAuth authorization card, opens only HTTPS authorization URLs with
  `noopener,noreferrer`, shows the waiting state after opening, and lets Cancel
  resolve the gate as `cancelled`;
- non-HTTPS authorization URLs are not written as clickable `href`s and do not
  call `window.open`.

Behavior adjustment:

- Legacy auth tests exercised v1 extension setup endpoints, MCP install/activate
  flows, and chat-mode token capture. Reborn WebUI v2 exposes auth prompts as
  typed run gates over `/api/webchat/v2/*`, with token storage handled by the
  product-auth endpoint and run continuation handled by gate resolution. The
  Reborn port therefore tests the browser caller contract at the SSE prompt,
  product-auth submit, and gate-resolution boundaries rather than the legacy
  `/api/extensions/*` chat-auth mode.

Frontend harness adjustment:

- Added stable Reborn auth-gate selectors for the shared auth shell, manual
  token input, and OAuth authorization action. These hooks are presentation
  neutral and preserve the component-owned security checks.

CI update:

- `.github/workflows/reborn-e2e.yml` now includes the product-auth prompt port
  in the Reborn WebUI v2 Playwright job.

### Step 20: Legacy Channel Connect / Pairing Port

Added `tests/e2e/scenarios/test_reborn_webui_v2_legacy_channel_connect.py`.

Ported the browser-visible pairing-card intent from legacy
`test_channel_pairing_flow.py` to Reborn's chat-owned channel connect flow:

- a `connect slack` chat command queries
  `/api/webchat/v2/channels/connectable`;
- a Slack `inbound_proof_code` connect action renders the Slack pairing card
  instead of sending a normal chat turn;
- the proof code input trims whitespace and redeems through
  `/api/webchat/v2/extensions/pairing/redeem`;
- the card displays the success message and can be dismissed;
- the test blocks `/api/webchat/v2/threads/*/messages` to prove the connect
  command does not fall through to normal message submission.

Behavior adjustment:

- Legacy pairing tests used v1 `handleOnboardingState(...)` globals and
  `/api/pairing/{channel}/approve`. Reborn's browser path resolves connect
  commands through typed connectable-channel metadata and redeems Slack pairing
  codes through the v2 extensions pairing endpoint. Lower-level pairing
  authorization, invalid-code, and admin/member access checks remain legacy API
  coverage until Reborn exposes a matching operator/member pairing API.

Frontend harness adjustment:

- Added stable selectors for the Reborn channel-connect card and Slack pairing
  section so pairing assertions do not depend on layout classes.

CI update:

- `.github/workflows/reborn-e2e.yml` now includes the channel connect/pairing
  port in the Reborn WebUI v2 Playwright job.

### Step 21: Legacy Project Overview Port

Added `tests/e2e/scenarios/test_reborn_webui_v2_legacy_projects.py`.

Ported the project overview and drill-in intent from legacy
`test_project_detail.py` to the current Reborn Projects surface:

- the `/v2/projects` page loads project cards from
  `/api/webchat/v2/projects`;
- project metadata goals are surfaced in the scoped project cards;
- the Projects search input filters by project name, description, and goals;
- opening a project workspace routes to `/v2/projects/{project_id}` and reads
  the selected project through `/api/webchat/v2/projects/{project_id}`;
- the test runs against the real Reborn shell and route-mocks only the v2
  project endpoints.

Behavior adjustment:

- Legacy project detail tests used `/api/engine/projects/overview`,
  `/api/engine/missions`, `/api/engine/threads`, and project widget endpoints.
  Reborn currently has real v2 project list/create/read/update/delete and
  membership ACL endpoints, but project missions, project threads, and widgets
  still return TODO stubs in the client adapter. This port covers the
  implemented Reborn project contract and leaves the deeper mission/thread/widget
  drill-in behavior as a product parity gap.

Frontend harness adjustment:

- Added stable Projects selectors for project cards, project search, workspace
  shells, and open-workspace actions. The hooks carry `data-project-id` so tests
  can assert the Reborn project boundary without depending on visual layout.

CI update:

- `.github/workflows/reborn-e2e.yml` now includes the project overview port in
  the Reborn WebUI v2 Playwright job.

### Step 22: Legacy Responses API Port

Added `tests/e2e/scenarios/test_reborn_webui_v2_legacy_responses_api.py`.

Ported the API-level intent from legacy `test_responses_api.py` to the
standalone Reborn OpenAI-compatible route mount:

- `/v1/responses` accepts non-streaming text input and returns a completed
  `response` object with a `resp_` id;
- `/api/v1/responses` works as the route alias for legacy untyped Responses
  message input;
- `previous_response_id` continues a conversation and `GET
  /api/v1/responses/{id}` retrieves the resulting response;
- streaming requests return an SSE lifecycle on the Reborn projection stream;
- unauthenticated requests are rejected before side effects;
- invalid empty text input and empty input-item arrays return field-scoped
  `400` errors.

Behavior adjustment:

- Reborn now accepts the legacy `input=[{"role": "user", "content": "..."}]`
  message-item shape and normalizes it to the internal typed Responses payload
  before submitting the product-workflow turn.
- The legacy streaming assertion only required raw SSE events. Reborn can
  complete a fast response with `response.created` and `response.completed`
  events without an intermediate `response.output_text.delta`, so the migrated
  test asserts the lifecycle events rather than requiring a delta.
- The original port left legacy `x_context.notification_response` injection
  open; Step 73 closes that gap with a Reborn route-level context contract.

Harness adjustment:

- Added a scenario-local binary fixture that builds `ironclaw-reborn` with
  `openai-compat-beta` before starting `serve`. The existing WebUI v2 harness
  remains on `webui-v2-beta`; this test opts into the extra feature only where
  the Responses route mount is required.

CI update:

- `.github/workflows/reborn-e2e.yml` now includes the Responses API port in the
  Reborn WebUI v2 Playwright job and raises the per-test timeout to 180 seconds
  so the feature-gated binary build in this scenario does not flake on slower
  runners.

### Step 23: Legacy SSE Thread-Switch Cursor Port

Extended `tests/e2e/scenarios/test_reborn_webui_v2_legacy_sse_history.py`.

Ported the remaining stale-cursor intent from legacy `test_sse_reconnect.py`
to Reborn's in-app thread switch path:

- seeded two v2 threads through route-mocked `/api/webchat/v2/threads` and
  timeline responses;
- opened thread A, emitted a `keep_alive` frame with a cursor, then selected
  thread B through the real sidebar button;
- asserted the new thread B EventSource URL targets
  `/api/webchat/v2/threads/thread-legacy-sse-b/events`;
- asserted the thread B stream carries the bearer token but no inherited
  `after_cursor`.

Behavior adjustment:

- Legacy browser tests also covered root-refresh fallback to server
  `active_thread` and skipping read-only external-channel active threads. The
  standalone Reborn v2 UI is URL/thread-list driven and does not expose those
  legacy globals or read-only external active-thread semantics, so the
  functional port protects the matching Reborn invariant: event cursors are
  scoped to the active `threadId` route and reset when that route changes.

### Step 24: Legacy Auth Duplicate Response Port

Extended `tests/e2e/scenarios/test_reborn_webui_v2_legacy_auth_flows.py`.

Ported the browser-visible intent from legacy
`test_auth_no_duplicate_response.py` to Reborn's WebChat v2 auth-gate path:

- emitted an `auth_required` SSE frame carrying the same auth-instruction text
  that legacy previously duplicated as a `response` event;
- asserted the instructions render inside the manual-token auth gate;
- asserted no assistant message bubble contains the auth instructions;
- asserted exactly one auth gate is present for the prompt.

Behavior adjustment:

- The legacy regression test observed the old gateway's raw `/api/chat/*` SSE
  events and specifically forbade a duplicate `response` event. Reborn WebChat
  v2 consumes typed thread events and renders auth prompts through gate state,
  so the migrated coverage protects the equivalent user-facing contract:
  authentication instructions appear as a gate only, not as duplicate assistant
  transcript text.

### Step 25: Legacy Approval Text-Intercept Review

Reviewed legacy `test_tool_approval.py` against Reborn's approval contract.

Already-covered functional ports:

- approval card rendering, tool/action detail rendering, payload expansion, and
  approve/deny/always resolution are covered by
  `test_reborn_webui_v2_legacy_approval.py`;
- bare `yes`/`no`/`always` with no pending gate send as normal chat in
  `test_reborn_legacy_bare_approval_keywords_send_as_chat_without_gate`;
- attempting to send any new message while a Reborn approval gate is pending is
  blocked locally and shows a non-error waiting message in
  `test_reborn_v2_approval_gate_blocks_composer_send`.

Behavior adjustment:

- Legacy v1 allowed the composer to remain writable with an approval card in
  the DOM, then intercepted text aliases such as `yes`, `no`, `approve`,
  `deny`, and `/approve` by finding the matching unresolved card and posting
  legacy `/api/chat/gate/resolve` or `/api/chat/approval` requests. Reborn
  WebChat v2 instead disables all composer sends while `pendingGate` is set and
  resolves the explicit gate through
  `/api/webchat/v2/threads/{thread_id}/runs/{run_id}/gates/{gate_ref}/resolve`.
  The old same-thread/other-thread text-intercept matrix is therefore legacy
  v1 DOM/API behavior, not an additional Reborn port target unless product
  requirements change to reintroduce text approval aliases.

### Step 26: Legacy OAuth Callback Completion Port

Extended `tests/e2e/scenarios/test_reborn_webui_v2_legacy_auth_flows.py`.

Ported the browser-visible callback-success portion of legacy
`test_extension_oauth.py` to Reborn's product-auth prompt flow:

- opened an OAuth auth gate from an `auth_required` SSE frame;
- emitted the same-origin callback completion signal used by Reborn's
  server-side product-auth callback page;
- asserted the pending OAuth gate clears only when the completion payload
  matches the active `turn_run_ref` and `gate_ref`.

Behavior adjustment:

- Legacy extension OAuth tests hit `/oauth/callback` directly, exchanged mock
  provider tokens, checked replay rejection, and asserted extension/tool
  authenticated state. Reborn's WebChat v2 browser owns only the prompt and
  completion-signal behavior; provider token exchange and replay/removal
  invalidation are host-runtime endpoint contracts and remain open until a
  standalone Reborn product-auth endpoint fixture exists for those paths.

### Step 27: Legacy Usage Event Rendering Guard Port

Extended `tests/e2e/scenarios/test_reborn_webui_v2_legacy_sse_history.py`.

Ported legacy `test_turn_cost_event_does_not_render_message_badge` to Reborn's
WebChat v2 event stream:

- seeded a Reborn thread with an assistant message;
- dispatched a non-chat `turn_cost` frame through the EventSource `message`
  fallback path;
- asserted the transcript still contains exactly one assistant message;
- asserted no `.turn-cost-badge`, token count, or cost text appears in the
  message body.

Behavior adjustment:

- Legacy v1 had a named `turn_cost` SSE event and a historical badge renderer.
  Reborn WebChat v2 does not render per-message cost badges from chat-stream
  events; usage belongs to the admin usage surfaces. The migrated test protects
  the matching user-visible invariant that accounting frames do not mutate the
  chat transcript.

### Step 28: Legacy Slash Autocomplete Review

Reviewed legacy `test_slash_autocomplete_shows_commands_and_skills` against
the current Reborn WebChat v2 chat surface.

Already-covered functional Reborn behavior:

- channel-connect commands are handled through chat input and do not create
  user/assistant transcript messages when they resolve to a pairing action;
  see `test_reborn_legacy_slack_connect_command_renders_pairing_card_and_redeems_code`;
- skill usage hints are rendered on the Reborn Settings Skills surface, covered
  by `test_reborn_webui_v2_legacy_skills.py` and settings-search coverage.

Behavior adjustment:

- Legacy v1 exposed an inline `#slash-autocomplete` menu that mixed built-in
  slash commands with installed skills and inserted `/{skill} ` into the chat
  input. Reborn WebChat v2 currently has no equivalent inline slash menu. It
  uses a separate command palette for navigation/actions and handles only
  channel-connect intent phrases in the chat composer. The legacy slash menu
  test is therefore not a direct Reborn port target unless the Reborn product
  intentionally adds an inline skill/command autocomplete surface.

### Step 29: Legacy Admin/Operator Review

Reviewed legacy `test_admin_api.py` and related operator/admin browser
coverage against the current Reborn Admin page.

Current blocker:

- `crates/ironclaw_webui_v2_static/static/js/pages/admin/lib/admin-api.js`
  explicitly avoids legacy `/api/admin/*` calls and returns TODO stub payloads
  for users, user detail, create/update/delete/suspend/activate, token
  creation, usage summary, and usage rows;
- the Reborn Admin route is registered but hidden while those v2 endpoint
  contracts are missing.

Behavior adjustment:

- Legacy admin API/browser tests are not portable to Reborn WebUI v2 yet. A
  real port requires v2 admin endpoints and a non-stub Admin API client; until
  then, browser tests would only validate placeholder data rather than the
  legacy user/secret/usage lifecycle behavior.

### Step 30: Legacy Generic Pairing Review

Reviewed legacy `test_pairing.py` and `test_channel_pairing_flow.py` against
the current Reborn Extensions and channel-connect surfaces.

Already-covered functional Reborn behavior:

- Slack personal proof-code pairing through chat input and
  `/api/webchat/v2/extensions/pairing/redeem` is covered by
  `test_reborn_legacy_slack_connect_command_renders_pairing_card_and_redeems_code`.

Current blocker:

- the generic Extensions channel `PairingSection` is rendered for channel
  packages in `pairing_required` or `pairing` state, but
  `fetchPairingRequests` and `approvePairingCode` in
  `pages/extensions/lib/extensions-api.js` still return local TODO stub
  responses instead of calling v2 pairing endpoints.

Behavior adjustment:

- Legacy generic pairing-list, member/admin access, optional `thread_id`, and
  channel-name sanitization tests target the legacy `/api/pairing/*` endpoint
  family. Those are not direct Reborn WebUI v2 ports until standalone Reborn
  exposes matching v2 generic pairing list/approve endpoints. Current Reborn
  coverage remains scoped to the Slack proof-code redeem contract that exists.

### Step 31: Legacy Plan Mode and Portfolio Review

Reviewed legacy `test_plan_mode.py` and `test_portfolio.py` against the current
Reborn WebChat v2, Projects, Settings, and Skills surfaces.

Already-covered functional Reborn behavior:

- Reborn chat send/reply, message rendering, reload persistence, and transcript
  safety are covered by the migrated chat-core, rendering, attachment, and SSE
  history scenarios;
- generic skill visibility and usage-hint searchability are covered by
  `test_reborn_webui_v2_legacy_skills.py` and Settings search coverage;
- real project entity list/detail/create/update/delete and membership behavior
  is covered by the Reborn project overview port.

Current blocker:

- legacy plan-mode tests assert visible checklist cards, approval/status/list
  behavior, and `/plan` command parsing through the legacy chat UI. Reborn
  still has lower-level `/plan` parsing code in the shared agent path, but the
  WebChat v2 browser surface does not expose a plan checklist/card renderer or
  matching plan interaction controls;
- legacy portfolio tests assert a portfolio-specific tab/widget, widget
  positions, and share-modal behavior. Reborn Projects is the closest current
  product surface, but `pages/projects/lib/projects-api.js` still marks
  per-project missions, threads, and widgets as TODO stubs until v2 endpoints
  land. There is no Reborn browser widget/share-modal contract to assert today.

Behavior adjustment:

- Legacy plan-mode browser tests are not direct Reborn ports until standalone
  Reborn exposes a WebChat v2 plan projection and user controls for checklist
  approval/status/list behavior.
- Legacy portfolio widget/share tests are not direct Reborn ports until the
  Reborn project/widget contract exists. Current Reborn coverage remains scoped
  to project overview behavior and generic skill visibility rather than the
  legacy portfolio-specific widget UI.

### Step 32: Legacy OAuth, Extension OAuth, and MCP Auth Review

Reviewed legacy `test_extension_oauth.py`, `test_mcp_auth_flow.py`,
`test_oauth_refresh.py`, `test_oauth_credential_fallback.py`,
`test_oauth_url_parameters.py`, and `test_skill_oauth_flow.py` against current
Reborn product-auth, extension setup, and WebChat v2 auth-gate surfaces.

Already-covered functional Reborn behavior:

- WebChat v2 manual-token auth gates submit credentials through
  `/api/reborn/product-auth/manual-token/submit` and resume the matching run
  gate through the v2 gate-resolution endpoint;
- WebChat v2 OAuth auth gates render only HTTPS authorization links, open them
  with `noopener,noreferrer`, reject non-HTTPS values, and clear only matching
  OAuth callback completion events;
- extension activate/setup browser flows reject non-HTTPS `auth_url` and
  `authorization_url` values and accept uppercase HTTPS URLs;
- Reborn v2 extension setup endpoints expose Google OAuth requirements through
  `/api/webchat/v2/extensions/{package_id}/setup`;
- Reborn product-auth callback behavior, including one-shot claim semantics,
  completed-callback replay without duplicate continuation dispatch, invalid
  state rejection, and durable completed-flow replay, is covered by Rust
  product-auth/composition contract tests.

Current blocker:

- legacy extension OAuth and MCP auth scenarios exercise the legacy
  `/api/extensions/*` lifecycle plus public `/oauth/callback` route. Standalone
  Reborn's browser route is the v2 `/api/webchat/v2/extensions/*` setup/activate
  surface and its OAuth callback handling is product-auth service/composition
  logic, not the old gateway `/oauth/callback` extension controller;
- legacy hosted refresh and credential-fallback tests mutate/check legacy
  `secrets` rows and legacy tool registry behavior around v1 WASM extension
  execution. Reborn's equivalent path is host-runtime/product-auth credential
  resolution and extension v2 lifecycle contract coverage, not the legacy DB
  shape or `/api/chat/*` retry loop;
- legacy skill OAuth tests drive v1 skill frontmatter registration,
  `/api/chat/send`, and text-token auth mode. Reborn WebChat v2 exposes
  structured auth gates and product-auth continuations instead of the legacy
  free-text auth mode.

Behavior adjustment:

- The direct Reborn Playwright surface is already covered for auth prompts,
  extension OAuth-start URL safety, and callback-completion UI handling. The
  remaining legacy OAuth/MCP tests should not be copied line-for-line unless
  Reborn intentionally exposes equivalent v2 callback HTTP routes, hosted
  refresh test fixtures, or full browser-driven extension/MCP OAuth lifecycle
  hooks. Until then, the durable callback and credential semantics belong in
  Reborn Rust contract tests, with browser coverage limited to the v2 surfaces
  users can actually operate.

### Step 33: Legacy Ownership, Multi-Tenant, and Engine-Visibility Review

Reviewed legacy `test_owner_scope.py`, `test_ownership_model.py`,
`test_multi_tenant_greeting.py`, and `test_v2_thread_visibility.py` against
current Reborn WebUI v2 and product-workflow caller-scope contracts.

Already-covered functional Reborn behavior:

- Reborn WebUI v2 browser login and no-token rejection are covered by the
  migrated core shell/auth tests;
- Reborn v2 chat send/reply, sidebar title creation, and timeline persistence
  are covered by migrated browser chat and message-persistence tests;
- `/api/webchat/v2/session` returns the host-minted authenticated tenant/user
  identity and capabilities in WebUI v2 handler contract tests;
- Reborn composition tests assert that untrusted request bodies cannot inject
  `tenant_id`, `user_id`, or `scope` to divert thread creation away from the
  authenticated caller;
- WebUI v2 stream concurrency limits are caller-scoped and shared across SSE and
  WebSocket transports in handler contract tests.

Current blocker:

- legacy owner-scope tests exercise the old web gateway, owner-scoped HTTP
  webhook channel, and legacy routines tab/API. Standalone Reborn does not
  expose the same `/api/chat/*`, HTTP webhook, or `/api/routines` browser
  contract; the visible Reborn routines page remains TODO-stubbed as documented
  earlier;
- legacy multi-tenant greeting tests create users through `/api/admin/users` and
  assert per-user assistant greeting persistence through legacy chat history.
  Reborn's Admin API adapter is currently TODO-stubbed, and WebChat v2 has no
  matching initial-greeting assistant-thread contract;
- legacy engine-visibility tests cover the old `ENGINE_V2=true` gateway split
  between `/api/chat/threads`, `/api/engine/threads`, and synthesized legacy
  history for deep-linked engine execution threads. Standalone Reborn WebUI v2
  uses product-workflow thread/timeline projections instead of exposing that
  legacy engine sidebar split.

Behavior adjustment:

- The Reborn port should keep caller-scope and authentication guarantees in
  caller-level WebUI/ProductWorkflow contract tests and browser-test only the
  visible v2 chat/session surfaces. Legacy owner HTTP webhook, routines,
  admin-created user greeting, and engine-v2 visibility scenarios are not
  direct Playwright ports until Reborn intentionally exposes equivalent
  standalone v2 product surfaces.

### Step 34: Legacy Agent-Loop Recovery Port

Extended `test_reborn_webui_v2_legacy_tool_execution.py`.

Ported issue-1780 recovery behavior that has a current Reborn WebUI v2
equivalent:

- a model-requested built-in time tool call with invalid arguments reaches a
  finalized assistant summary instead of leaving the run hanging;
- a streamed, length-truncated tool-call-shaped assistant response finalizes as
  visible assistant text and does not create a capability activity card.

Test harness issue fixed:

- the mock LLM's issue-1780 recovery triggers emitted legacy tool names
  (`time`, `echo`) even when standalone Reborn advertised provider-facing
  names (`builtin__time`, `builtin__echo`). The mock now preserves legacy names
  for legacy requests and selects the Reborn names only when the request
  advertises them.

Current blocker:

- this first pass left empty-model-reply and low-iteration loop-cap browser
  behavior open. Later Reborn ports cover empty replies through failed-run
  projection and loop caps through a low-iteration v2 turn harness.

### Step 35: Legacy Webhook, Widget, Routines, and Cleanup Review

Reviewed legacy `test_webhook.py`, `test_widget_customization.py`,
`test_extension_uninstall_cleanup.py`, `test_routine_event_batch.py`,
`test_routine_full_job.py`, and `test_routine_oauth_credential_injection.py`.

Already-covered functional Reborn behavior:

- Reborn top-level extension install/manage/configure browser behavior is
  covered by `test_reborn_webui_v2_legacy_extensions.py`;
- Reborn extension setup and OAuth-start safety are covered by the migrated
  extension and auth-flow ports plus Rust product-auth callback contracts;
- Reborn Automations has browser coverage for the current standalone scheduled
  work surface;
- Reborn Projects and Workspace have separate v2 overview coverage for the
  current project/workspace surfaces.

Current blocker:

- legacy webhook tests target the standalone HTTP channel server's `/webhook`
  endpoint, including HMAC header authentication, deprecated body-secret
  compatibility, content-type/JSON validation, and queued message ids.
  Standalone Reborn WebUI v2 does not expose a matching HTTP-channel webhook
  product surface;
- legacy widget customization tests rely on chat-driven `memory_write` calls
  into `.system/gateway/*`, dynamic `custom.css`, legacy tab/widget discovery,
  and per-user/multi-tenant gateway HTML bundle behavior. Reborn WebUI v2 uses a
  static React app and has no equivalent gateway widget loader or CSS injection
  contract;
- legacy uninstall cleanup tests inspect the legacy `secrets` DB rows after
  `/api/extensions/*` remove operations. Reborn v2 extension lifecycle and
  product-auth cleanup use different services and do not expose that legacy
  table-level contract through WebUI v2 Playwright;
- legacy routine event/full-job/OAuth tests exercise `/api/routines/*`,
  `/api/jobs/*`, the legacy routines tab, HTTP-channel event triggers, and
  routine-scoped OAuth credential fallback. The current Reborn routines page is
  still TODO-stubbed, and the migrated Reborn coverage is intentionally scoped
  to the real Automations surface.

Behavior adjustment:

- These legacy files are not line-for-line Reborn Playwright ports. A true
  functional port requires Reborn-native v2 webhook/channel ingress contracts,
  a v2 widget/customization system if that product capability is retained,
  lifecycle cleanup contract tests against Reborn product-auth/extension
  services, and non-stub routines endpoints/pages. Until those surfaces exist,
  the branch should keep Reborn tests focused on the v2 surfaces that are
  implemented today.

### Step 36: Legacy Slack, Telegram, and Channel Approval Review

Reviewed legacy `test_channel_approval_gates.py`, `test_slack_e2e.py`,
`test_telegram_e2e.py`, `test_telegram_hot_activation.py`,
`test_telegram_pairing_chat_claim.py`, and `test_telegram_token_validation.py`.

Already-covered functional Reborn behavior:

- Reborn WebChat v2 approval cards and gate-resolution requests are covered by
  `test_reborn_webui_v2_legacy_approval.py`;
- Reborn product-workflow contract tests cover v2 gate resolution, scoped
  approval fallback, automation-trigger approval fallback, and operator approval
  config persistence;
- Reborn Slack personal proof-code connect/redeem behavior is covered by the
  migrated channel-connect Playwright test and composition route tests;
- Reborn extension setup/configure modal behavior, including Telegram token
  field preservation, is covered by the migrated extensions tests;
- Slack and Telegram v2 adapter crates carry parser/rendering/authentication
  unit and contract coverage for their current Reborn adapter boundaries.

Current blocker:

- legacy channel approval tests drive old Telegram/Slack WASM-channel webhook
  fixtures, legacy `/api/chat/history` pending gates, text aliases (`yes`,
  `no`, `always`) inside channel DMs, and cross-channel resolution through
  legacy `/api/chat/approval`. Reborn WebChat v2 intentionally uses structured
  gate cards and v2 gate-resolution endpoints, while product adapter approval
  routing is covered below the browser layer;
- legacy Slack tests assert the old channel server's URL verification, HMAC
  headers, DM/app-mention parsing, bot/subtype ignores, thread replies, file
  attachments, and malformed payload handling. Standalone Reborn WebUI v2 does
  not expose that legacy Slack webhook server surface as Playwright API/UI;
- legacy Telegram tests assert the old Telegram channel server's webhook/polling
  modes, pairing, unauthorized-user rejection, group mention filtering, long
  message chunking, Markdown fallback, document-download failure, and malformed
  payload resilience. Current Reborn Telegram v2 behavior is implemented in
  adapter crates and setup projections rather than an equivalent WebUI v2
  browser surface;
- legacy Telegram hot-activation and pairing-chat tests mix legacy extension
  activation state, generic pairing APIs, and Telegram DM command interception.
  Generic pairing APIs remain TODO-stubbed in the Reborn Extensions client as
  documented earlier.

Behavior adjustment:

- The remaining Slack/Telegram/channel tests should be ported as Reborn adapter
  or product-workflow contract tests where the v2 adapter boundary exists, not
  as WebUI v2 Playwright tests against legacy webhook controllers. New browser
  ports become appropriate only when standalone Reborn exposes channel-specific
  setup, pairing, and webhook/DM approval flows as first-class v2 surfaces.

### Step 37: Legacy Engine-v2, WASM Lifecycle, and Provider-Fixture Review

Reviewed the remaining legacy/general scenario files that had not been named
explicitly in this work log:

- `test_agent_loop_recovery.py`
- `test_emulate_reborn_provider_contracts.py`
- `test_mission_gmail_3133.py`
- `test_routines_tab_after_v2_upgrade.py`
- `test_v2_activity_shell.py`
- `test_v2_auth_oauth_matrix.py`
- `test_v2_engine_approval_flow.py`
- `test_v2_engine_auth_cancel.py`
- `test_v2_engine_auth_flow.py`
- `test_v2_engine_error_handling.py`
- `test_v2_engine_oauth_google.py`
- `test_v2_engine_tool_lifecycle.py`
- `test_v2_github_pat_flow.py`
- `test_v2_gsuite_oauth_flow.py`
- `test_v2_kernel_auth_gateway_flow.py`
- `test_v2_notion_mcp_oauth_flow.py`
- `test_wasm_lifecycle.py`

Current Reborn coverage:

- `test_agent_loop_recovery.py` is represented by Step 34's Reborn tool-loop
  recovery ports, except for the empty-model-reply UX decision documented there;
- `test_emulate_reborn_provider_contracts.py` is already a provider-fixture
  contract suite for Reborn-capable Google/GitHub/Slack integrations. It is not
  a browser/WebUI migration target;
- WebChat v2 approval, auth, tool execution, activity-card persistence,
  product-auth card rendering, OAuth URL safety, and manual-token submission
  behavior are covered by the migrated Reborn WebUI v2 Playwright tests plus
  Reborn/product-workflow Rust contract tests;
- the Reborn OpenAI-compatible Responses API has direct migrated coverage in
  `test_reborn_webui_v2_legacy_responses_api.py`;
- the current full migrated standalone Reborn WebUI v2 suite is passing at 102
  tests.

Current blocker:

- most remaining `test_v2_*` files are not standalone Reborn WebUI v2 tests.
  They start the old gateway with `ENGINE_V2=true` and exercise legacy
  `/api/chat/*`, `/api/engine/*`, `/api/chat/approval`,
  `/api/chat/gate/resolve`, `/oauth/callback`, and old hash/tab shell behavior;
- `test_v2_activity_shell.py` and `test_routines_tab_after_v2_upgrade.py`
  target the old browser shell's tab-routing decisions and legacy routines
  fallback. Reborn has a different sidebar/application shell and the current
  routines page still uses TODO client stubs;
- `test_mission_gmail_3133.py` targets the legacy mission/routine bridge,
  `/api/engine/missions`, legacy extension Gmail OAuth setup, and the legacy
  `/oauth/callback` resume path. Current Reborn WebUI v2 has no equivalent
  first-class mission detail/fire/resume browser surface;
- `test_v2_engine_approval_flow.py`, `test_v2_engine_tool_lifecycle.py`, and
  `test_v2_engine_error_handling.py` are valuable old engine-v2 gateway
  contract tests, but their history DTO, tool names, and approval endpoints are
  not the standalone Reborn WebChat v2 DTOs. Native Reborn parity should be
  covered through the Reborn runner/driver/executor and WebChat v2 timeline
  contracts rather than copying the legacy gateway harness;
- `test_v2_engine_auth_flow.py`, `test_v2_engine_auth_cancel.py`,
  `test_v2_engine_oauth_google.py`, `test_v2_auth_oauth_matrix.py`,
  `test_v2_kernel_auth_gateway_flow.py`, `test_v2_github_pat_flow.py`,
  `test_v2_gsuite_oauth_flow.py`, and `test_v2_notion_mcp_oauth_flow.py`
  mix old gateway chat/history endpoints with newer product-auth route probes.
  The route-level product-auth checks are useful, but the skipped browser tests
  explicitly require a `webui-v2-beta`/Reborn-native harness and should be
  converted only after those flows are exposed through standalone Reborn's
  `/api/webchat/v2/*` contracts;
- `test_wasm_lifecycle.py` validates legacy `/api/extensions/*` registry,
  install, setup, activate, remove, reinstall, and response-field semantics for
  WASM extensions. Reborn WebUI v2 currently covers the browser install/manage
  surface and extension setup/auth URL safety, but not the exact legacy
  registry/install/remove API shape or `secrets` table side effects.

Behavior adjustment:

- Keep the old `ENGINE_V2=true` gateway tests as legacy compatibility or
  historical contract coverage until each behavior has a Reborn-native boundary.
  A functional Reborn port means re-expressing those scenarios against the
  Reborn runner/driver/executor, product-auth services, adapter crates, and
  `/api/webchat/v2/*` DTOs. It is not a mechanical endpoint rename.
- Porting the WASM lifecycle suite requires either Reborn v2 extension lifecycle
  endpoints with real install/remove/setup contracts, or lower-level Reborn
  extension-service contract tests that assert the new API shape instead of the
  old `/api/extensions/*` responses.

### Step 38: Legacy Extension Reinstall-State Port

Extended `test_reborn_webui_v2_legacy_extensions.py`.

Ported the user-visible remove/reinstall invariant from legacy
`test_wasm_lifecycle.py` to Reborn's `/v2/extensions` UI:

- a configured extension can be removed through the installed extension card;
- after removal, the same registry entry returns to the available install list;
- reinstalling the extension produces a fresh unconfigured card with the
  `setup_required` state instead of reusing the previous authenticated state;
- the reinstalled card exposes `Configure`, not `Reconfigure`;
- saving a fresh token after reinstall posts the new setup payload through the
  Reborn v2 extension setup endpoint.

Behavior adjustment:

- The port asserts Reborn's current lifecycle DTOs and browser states
  (`/api/webchat/v2/extensions/*`, `package_ref`, `needs_setup`,
  `onboarding_state`) instead of the legacy `/api/extensions/*` response field
  names and secret-table assertions.

### Step 39: Legacy Blank Existing Secret Port

Extended `test_reborn_webui_v2_legacy_extensions.py`.

Ported the caller-visible invariant from legacy
`test_configure_empty_secret_skipped`:

- a configured extension's setup modal marks an existing manual secret as
  configured;
- the password input stays blank with a "leave blank to keep" placeholder;
- saving without entering a replacement token submits an empty `secrets` object
  through the Reborn v2 setup endpoint instead of sending an empty string that
  could clear the stored secret.

Behavior adjustment:

- Reborn does not expose the legacy secret table through the browser test, so
  the port asserts the boundary payload emitted by the modal. The lower-level
  service remains responsible for preserving omitted secrets.

### Step 40: Legacy Activation Failure Port

Extended `test_reborn_webui_v2_legacy_extensions.py`.

Ported the user-facing invariant from legacy
`test_activate_before_configure_rejected`:

- activating an installed extension can return `success: false` with an
  actionable setup/configuration message;
- Reborn surfaces that message in the Extensions page action toast;
- the failed activation does not mutate the card to active;
- the extension remains in the installed state with the `Activate` action still
  available.

Behavior adjustment:

- The port uses Reborn's v2 activate envelope from
  `/api/webchat/v2/extensions/{package_id}/activate` and browser card state,
  rather than the legacy `/api/extensions/{name}/activate` response shape.
  The test harness now applies active-state mutation only after a successful
  activation response.

### Step 41: Legacy Install Failure Port

Extended `test_reborn_webui_v2_legacy_extensions.py`.

Ported the user-facing error-path intent from legacy
`test_install_nonexistent` / `test_install_empty_name` to Reborn's Registry tab:

- an install request can return `success: false` with a specific error message;
- Reborn surfaces that message in the Extensions page action toast;
- a failed install does not add the extension to the installed list;
- the registry card remains in the available state with `Install` still
  available for a later retry.

Behavior adjustment:

- Reborn's browser does not submit free-form extension names, so the port
  asserts failed install handling through a real registry card and v2
  `package_ref` payload rather than legacy invalid-name API requests. The test
  harness now mutates registry/installed state only after a successful install
  response.

### Step 42: Legacy Remove Failure Port

Extended `test_reborn_webui_v2_legacy_extensions.py`.

Ported the user-facing error-path intent from legacy `test_remove_noninstalled`
to Reborn's installed extension card:

- a remove request can return `success: false` with a specific failure message;
- Reborn surfaces that message in the Extensions page action toast;
- a failed remove does not drop the installed extension card;
- the card remains active/manageable after the failed remove.

Behavior adjustment:

- Reborn's browser can only remove an installed card, so this port asserts the
  visible failed-remove contract instead of issuing a legacy remove request for
  a non-installed free-form name. The test harness now mutates installed and
  registry state only after a successful remove response.

### Step 43: Legacy Setup Load Failure Port

Extended `test_reborn_webui_v2_legacy_extensions.py`.

Ported the user-facing error-path intent from legacy `test_setup_noninstalled`
and `test_configure_noninstalled` to Reborn's configure modal:

- opening configuration for an installed extension can receive a non-2xx setup
  schema response;
- Reborn renders the setup load failure inside the modal with the humanized v2
  error code;
- the modal does not render a Save action for a failed setup schema load;
- no setup submit request is emitted.

Behavior adjustment:

- Reborn's browser opens configuration from an installed card rather than a
  free-form extension name. The port asserts the current v2
  `/api/webchat/v2/extensions/{package_id}/setup` error boundary and modal
  behavior instead of legacy `/api/extensions/{name}/setup` not-installed
  requests.

### Step 44: Legacy Activation Success State Port

Extended `test_reborn_webui_v2_legacy_extensions.py`.

Ported the browser-visible part of legacy
`test_extension_active_after_configure`, `test_tools_registered_after_activate`,
and `test_activate_already_active_idempotent`:

- a successful v2 activation response flips the installed extension card to the
  `active` state;
- the primary `Activate` action disappears after activation;
- the extension's capability disclosure remains available after the state
  transition.

Behavior adjustment:

- Reborn WebUI v2 exposes activation as a card-level action and capability list
  projection, not the legacy `/api/extensions/tools` endpoint. The port asserts
  the current browser state and v2 activation envelope rather than legacy tools
  registry rows.

### Step 45: Legacy Registry No-Match Search Port

Extended `test_reborn_webui_v2_legacy_extensions.py`.

Ported the user-visible invariant from legacy
`test_registry_search_no_match`:

- a registry search for a nonsense token clears previously visible registry
  cards;
- Reborn renders the Registry tab's no-match empty state;
- no install request is emitted while the filtered registry is empty.

Behavior adjustment:

- The legacy test asserted the raw `/api/extensions/registry?query=...`
  response body. Reborn's browser currently fetches the registry once and
  filters client-side, so the port asserts the visible no-match state in
  `/v2/extensions/registry` instead of a query-parameter API response.

### Step 46: Legacy Auto-Resolved Setup Port

Extended `test_reborn_webui_v2_legacy_extensions.py`.

Ported the user-visible setup-schema invariant from legacy
`test_gmail_setup_schema_auto_resolves`:

- an OAuth-backed extension can return a setup schema with no user-facing
  secrets or fields;
- Reborn renders the configure modal's "No configuration required" state;
- the modal does not expose Save, Authorize, or manual password inputs;
- no setup-submit or OAuth-start request is emitted.

Behavior adjustment:

- The legacy test asserted Gmail's backend setup JSON directly. Reborn's
  browser port asserts the corresponding modal behavior for the v2
  `/api/webchat/v2/extensions/{package_id}/setup` projection instead of
  pinning legacy `client_id` / `client_secret` auto-resolution internals.

### Step 47: Legacy Multi-Extension Isolation Port

Extended `test_reborn_webui_v2_legacy_extensions.py`.

Ported the browser-visible list/isolation invariant from legacy
`test_install_gmail`, `test_gmail_fields`, and `test_both_extensions_listed`:

- installing one registry extension does not hide or clobber another available
  extension;
- installing a second registry extension records a distinct v2 `package_ref`
  request;
- both extensions remain visible in the installed registry group;
- installed registry entries no longer expose the `Install` action.

Behavior adjustment:

- The legacy test asserted backend `/api/extensions` names and Gmail-specific
  `has_auth` fields. Reborn's port asserts the v2 Registry tab projection and
  distinct `package_ref` install requests for two extension kinds instead of
  legacy names.

### Step 48: Legacy Project Search Empty-State Recovery Port

Extended `test_reborn_webui_v2_legacy_projects.py` and fixed the Reborn
Projects grid.

Ported the project explorer filtering recovery intent from legacy
`test_project_detail.py` to the current Reborn `/v2/projects` surface:

- project search can narrow the visible workspace cards to zero matches;
- Reborn shows the no-match empty state for the filtered result;
- the search input remains visible while the no-match state is rendered;
- clearing the same input restores the project cards without a page reload.

Real issue fixed:

- `ProjectsGrid` previously returned the no-match empty panel before rendering
  the explorer shell. That removed the search box as soon as a query had zero
  matches, trapping the user in the filtered-empty state. The component now only
  uses the top-level empty panel for a true empty project list; filtered-empty
  results keep the search controls mounted and render the no-match panel inside
  the explorer body.

Behavior adjustment:

- Legacy `test_project_detail.py` drills through old engine project cards into
  mission/thread/widget detail panes. Those detail panes remain legacy
  engine-specific, so this port targets Reborn's real v2 project overview/search
  contract and documents the remaining drill-in parity separately.

### Step 49: Legacy Logs Control Wiring Port

Extended `test_reborn_webui_v2_legacy_csp.py`.

Ported the logs-button portion of legacy
`test_buttons_still_functional_after_csp_migration` to Reborn's `/v2/logs`
surface:

- route-mocked the Reborn operator logs API with a scoped log entry;
- verified the Logs page renders the entry through the real toolbar and list;
- clicked the Reborn `Pause` control and verified polling stops while paused;
- accepted the clear confirmation, clicked `Clear`, and verified the rendered
  log entries are removed.

Behavior adjustment:

- Legacy checked fixed DOM IDs (`logs-pause-btn`, `logs-clear-btn`) on the old
  gateway shell. Reborn has React toolbar buttons with accessible names instead
  of stable element IDs, so the port drives the visible controls by role/name
  and asserts their observable behavior.

### Step 50: Legacy Extension Removal State Port

Extended `test_reborn_webui_v2_legacy_extensions.py`.

Ported the browser-visible state invariant behind legacy
`test_removed_not_in_extensions`, `test_removed_extension_not_listed`, and
`test_removed_not_in_registry_installed`:

- removing an installed extension submits the v2 remove request for its
  `package_ref`;
- the Registry tab no longer renders an installed section for that extension;
- the same catalog entry returns as `available` with an `Install` action;
- the removed entry no longer exposes installed-only actions or the active
  status badge.

Behavior adjustment:

- Legacy asserted three separate backend lists (`/api/extensions`,
  `/api/extensions/tools`, and `/api/extensions/registry`). Reborn's port
  asserts the unified v2 Registry projection, which is what operators see after
  the extensions and registry queries are invalidated.

### Step 51: Legacy Extension OAuth Start Failure Port

Extended `test_reborn_webui_v2_legacy_extensions.py` and fixed the Reborn
extension OAuth setup hook.

Ported the failure invariant behind legacy `test_oauth_configure_returns_auth_url`
and `test_oauth_activate_returns_auth_url`:

- clicking `Authorize` submits the v2 OAuth-start request with provider, scopes,
  account label, and invocation id;
- if OAuth start returns `success: false`, the configure modal keeps the error
  visible instead of treating the response as a successful no-op;
- the placeholder popup opened for browser-popup compatibility is closed on the
  failure path;
- the configure modal remains open so the operator can retry or cancel.

Real issue fixed:

- `useOauthSetup` only rejected invalid authorization URLs. A server response
  such as `{success:false, message:"..."}` without an `authorization_url` flowed
  through the success handler, closed the placeholder popup, refreshed state,
  and showed no error. The hook now matches manual setup submission by throwing
  on explicit `success:false`.

Behavior adjustment:

- Legacy asserted the old `/api/extensions/{name}/setup` response body directly.
  Reborn's port exercises the v2 Configure modal and OAuth-start endpoint
  projection because that is the current operator-visible setup flow.

### Step 52: Legacy Empty-Reply Visible Failure Port

Extended `test_reborn_webui_v2_legacy_tool_execution.py`.

Ported the user-visible terminal behavior behind legacy
`test_empty_reply_uses_chat_fallback`:

- a failed Reborn run-status projection with `invalid_output` and an empty-reply
  summary renders a visible error bubble;
- the composer unlocks after the terminal failure;
- the client-only `err-*` failure bubble survives the terminal-status timeline
  refresh;
- Reborn does not fabricate a durable assistant transcript row for invalid
  model output.

Behavior adjustment:

- Legacy asserted an assistant fallback message containing "error" and "empty".
  Reborn's runner rejects empty model output as `InvalidOutput` and does not
  write an assistant transcript for that failed run. The Reborn port protects
  the equivalent WebUI contract: operators see a terminal failure bubble with a
  sanitized summary and can continue the conversation.

### Step 53: Legacy Setup-Required Install Prompt Port

Extended `test_reborn_webui_v2_legacy_extensions.py` and fixed the Reborn
Extensions install flow.

Ported the operator-facing behavior behind legacy
`test_install_wasm_channel_triggers_configure` to Reborn's
`/v2/extensions/channels` surface:

- installing a registry channel that requires setup submits the v2 install
  request with its `package_ref`;
- the success toast still renders;
- the Configure modal opens immediately after the successful install;
- the modal fetches and renders the v2 setup schema without submitting anything
  until the operator saves.

Real issue fixed:

- `useExtensions.installMutation` only showed the install result and invalidated
  extension queries. Setup-required installs left the operator on the registry
  card and required a second manual Configure click. Registry cards now pass a
  `configureAfterInstall` hint for setup/auth/channel entries, and
  `ExtensionsPage` opens the existing Configure modal after successful installs
  that do not return an immediate `auth_url`.

Behavior adjustment:

- Legacy tested old `/api/extensions/install` plus a v1 configure modal on the
  Settings Channels tab. The Reborn port exercises the current top-level
  `/v2/extensions` Channels tab and `/api/webchat/v2/extensions/*` setup
  contract.

### Step 54: Legacy Channel Pairing Claim Port

Extended `test_reborn_webui_v2_legacy_extensions.py` and fixed the generic
Reborn channel pairing form.

Ported the user-facing member pairing behavior from legacy
`test_member_pairing_claim_submission_shows_success` and
`test_member_pairing_claim_failure_shows_error` to Reborn's
`/v2/extensions/channels` surface:

- a channel in `pairing_required` state renders the generic pairing section;
- no primary `Activate` button appears alongside the pairing UI;
- submitting a code trims whitespace and posts the v2 pairing redeem request
  with `{channel, code}`;
- a successful redemption shows the pairing success copy and clears the input;
- a failed redemption shows the endpoint error and keeps the entered code
  available for retry.

Real issue fixed:

- `approvePairingCode` in the Reborn extensions API still returned a local
  placeholder response (`Pairing requires a v2 pairing endpoint.`), so generic
  channel pairing could not complete from the Channels tab even though the
  composition layer already exposes `/api/webchat/v2/extensions/pairing/redeem`.
  The generic pairing form now uses that v2 redeem endpoint, matching the
  Slack proof-code flow.

Behavior adjustment:

- Legacy split pairing into member/admin variants and old
  `/api/pairing/{channel}/approve` endpoints. Reborn's browser-visible port
  covers member self-claim through the v2 redeem endpoint. Admin pending-request
  listing remains a lower-level/non-stub Reborn surface gap until a v2 pending
  pairing list endpoint exists.

### Step 55: Legacy Pending-Approval Send Block Port

Extended `test_reborn_webui_v2_legacy_approval.py`.

Ported the user-facing invariant behind legacy
`test_waiting_for_approval_message_no_error_prefix`:

- an open Reborn approval gate disables message sending;
- the composer shows the non-error status text `Resolve the approval request
  before sending another message.`;
- pressing Enter while the gate is open keeps the draft intact;
- no optimistic user message, assistant message, system error message, or
  `/api/webchat/v2/threads/{thread_id}/messages` request is produced.

Behavior adjustment:

- Legacy v1 accepted the second send and returned a non-error assistant/status
  message from `/api/chat/send`. Reborn blocks the send locally while a gate is
  pending, so the port asserts the equivalent operator contract at the current
  WebChat v2 boundary: the user sees a non-error waiting state and the blocked
  input does not become a failed chat message.

### Step 56: Legacy Near-Cap Response Projection Port

Extended `test_reborn_webui_v2_legacy_dom_resource_limits.py`.

Ported the response-integrity intent behind legacy DOM-cap scenarios including
`test_response_intact_near_dom_cap`, `test_real_user_message_prunes_before_response`,
and streaming-preservation coverage:

- a Reborn timeline page can start at the current 50-message browser boundary;
- a `projection_update` text item appends a visible assistant response beyond
  that initial page;
- a later projection for the same text id updates that assistant bubble in place
  instead of duplicating it;
- older unloaded history remains unloaded until the user explicitly loads older
  messages.

Behavior adjustment:

- Legacy v1 enforced a fixed large DOM cap and pruned older nodes while keeping
  in-flight responses intact. Reborn WebUI v2 keeps long history bounded through
  timeline paging, while projection text items are deduped by projection id, so
  the port asserts the equivalent response-integrity behavior at the current SSE
  projection boundary.

### Step 57: Legacy Stale Replay Timeline Dedupe Port

Extended `test_reborn_webui_v2_legacy_sse_history.py`.

Ported the remaining duplicate-protection intent from legacy
`test_reconnect_with_stale_last_event_id_does_not_duplicate_messages` to
Reborn's terminal projection and timeline refresh path:

- seeded a Reborn thread with one user message;
- emitted a terminal `projection_update` run-status frame to drive the real
  `onRunSettled` timeline refresh;
- returned a refreshed timeline containing duplicate copies of the same user
  and assistant message ids, matching the stale/replayed-data failure shape;
- asserted the browser renders each message id once.

Behavior adjustment:

- Legacy v1 forced reconnect with an old `Last-Event-ID` and then rebuilt
  `/api/chat/history` without duplicating messages. Reborn v2 resumes streams
  with `after_cursor` and refreshes durable state through
  `/api/webchat/v2/threads/{thread_id}/timeline`, so the equivalent regression
  target is id-based dedupe during the terminal timeline refresh that follows a
  replayed or stale projection sequence.

### Step 58: Legacy Extensions Revisit Reload Port

Extended `test_reborn_webui_v2_legacy_extensions.py`.

Ported legacy `test_extensions_tab_reloads_on_revisit` to the Reborn
Extensions page:

- opened the route-mocked Reborn Extensions Registry surface;
- recorded the initial `/api/webchat/v2/extensions` and registry requests;
- navigated away through the real SPA sidebar route;
- returned to Extensions through the real sidebar route;
- asserted the installed-extension and registry queries refetch on remount.

Issue found and fixed:

- Reborn's global React Query default keeps data fresh for 10 seconds. The
  Extensions page inherited that default, so a quick revisit showed cached
  installed/registry/connectable-channel state and did not hit the v2 endpoints.
  `useExtensions` now sets `refetchOnMount: "always"` for mutable extension
  catalog queries so install/remove/setup changes made outside the current tab
  are visible when operators return to the page.

Behavior adjustment:

- Legacy v1 called `loadExtensions()` whenever the Settings Extensions subtab
  was revisited. Reborn's equivalent is remounting `/v2/extensions/*`; the port
  asserts query refetches at that SPA boundary instead of old `/api/extensions`
  globals.

### Step 59: Legacy Active Thread Summary-Refresh Port

Extended `test_reborn_webui_v2_legacy_pending_messages.py`.

Ported the user-facing invariant behind legacy
`test_sidebar_refresh_keeps_active_thread_outside_summary_window` to Reborn's
URL/thread-id driven chat surface:

- opened a deep-linked Reborn thread whose sidebar cache entry was title-less,
  so a successful send triggers the real thread-list invalidation path;
- changed the mocked refreshed thread summary to omit the active thread and
  include a newer thread, matching the legacy "outside summary window" shape;
- sent through the real composer and asserted the message still targeted the
  active `/v2/chat/{thread_id}` route;
- asserted the refreshed sidebar can show the newer summary thread without
  retargeting the active chat or losing the optimistic user message.

Behavior adjustment:

- Legacy v1 guarded a mutable `currentThreadId` plus `/api/chat/threads`
  summary refresh. Reborn's equivalent source of truth is the route-scoped
  `activeThreadId`; the port protects that a sidebar refetch does not navigate
  or retarget the composer when the active thread is absent from the summary
  response.

### Step 60: Legacy Connection and Ownership API Port

Extended `test_reborn_webui_v2_legacy_core.py`.

Ported the API-level health/auth/ownership intent from legacy
`test_connection.py` and `test_ownership_model.py` to Reborn's public WebUI v2
endpoints:

- `/api/health` returns the healthy startup response from the real
  `ironclaw-reborn serve` process;
- unauthenticated and invalid bearer requests to `/api/webchat/v2/session` are
  rejected before returning caller state;
- authenticated `/api/webchat/v2/session` returns the host-minted tenant and
  user identity for the Reborn WebUI v2 test operator;
- the same session payload exposes the operator capability, deployment feature
  gates, and inline attachment budgets used by the browser composer.

Behavior adjustment:

- Legacy ownership tests read/write old `/api/settings/*` rows and create
  additional users through `/api/admin/users`. Reborn's standalone v2 surface
  uses host-authenticated session identity plus `/api/webchat/v2/*` endpoints;
  the port therefore asserts the visible session/auth contract and leaves
  legacy admin-created multi-user greeting flows in the product-gap bucket until
  Reborn exposes equivalent v2 admin/user provisioning behavior.

### Step 61: Legacy Pending Welcome Suppression Port

Extended `test_reborn_webui_v2_legacy_pending_messages.py`.

Ported the user-facing invariant from legacy
`test_welcome_card_hidden_when_pending` to Reborn's empty landing state:

- opened an empty Reborn thread through the real chat route with mocked v2
  session/thread/timeline endpoints;
- asserted the empty landing headline is visible before the user sends;
- held the send request in flight so the optimistic pending message remains
  active;
- asserted the pending user message renders and the empty landing headline is
  removed while the send is still unresolved.

Behavior adjustment:

- Legacy v1 manipulated `_pendingUserMessages` and `.welcome-card` directly.
  Reborn's equivalent is the React `showLanding` branch: pending optimistic
  messages increase `messages.length`, so the empty landing is replaced by the
  message list without relying on v1 globals.

### Step 62: Legacy Approval Resolve In-Flight Disable Port

Extended `test_reborn_webui_v2_legacy_approval.py`.

Ported the actionable part of legacy
`test_approval_approve_disables_buttons` to Reborn's gate-resolution card:

- emitted a Reborn approval gate through the WebChat v2 event stream;
- held the `/api/webchat/v2/threads/{thread}/runs/{run}/gates/{gate}/resolve`
  request open so the card remained visible during resolution;
- asserted the approve, deny, and always-allow controls are disabled while the
  resolve request is in flight;
- asserted the resolve payload is still the v2 gate-resolution payload and the
  card clears after the delayed response completes.

Issue fixed:

- Reborn's `ApprovalCard` did not track an in-flight resolve request, so a fast
  double click could submit duplicate gate-resolution requests before the
  pending gate cleared. The card now keeps a local resolving state and disables
  the primary, deny, and always-allow controls until the async resolve action
  finishes.

Behavior adjustment:

- Legacy v1 left a resolved card in place with an `Approved` / `Denied` status
  label. Reborn's v2 card clears after the run-scoped resolve endpoint accepts
  the decision, so the port protects duplicate-submit prevention rather than
  copying the legacy resolved-label DOM.

### Step 63: Legacy Approval Payload Collapse Port

Extended `test_reborn_webui_v2_legacy_approval.py`.

Completed the remaining toggle behavior from legacy `test_approval_params_toggle`
for Reborn's long approval payload preview:

- asserted a long command starts truncated;
- expanded it with `View full command`;
- collapsed it again with `Show preview`;
- asserted the long tail is hidden again after collapse.

Behavior adjustment:

- Legacy v1 toggled a raw `.approval-params` block. Reborn exposes the same
  operator control as `View full command` / `Show preview` on the structured
  approval-details section, so the assertion follows those accessible controls.

### Step 64: Legacy GitHub PAT Browser Non-Retention Port

Extended `test_reborn_webui_v2_legacy_auth_flows.py`.

Ported the skipped browser assertion from `test_v2_github_pat_flow.py` that a
submitted GitHub personal access token is not retained in the page after manual
auth completes:

- submitted a fake GitHub PAT through the Reborn manual-token auth gate;
- asserted the product-auth submit boundary received the trimmed token;
- asserted the auth gate and password input disappeared after run-gate
  resolution;
- asserted the token pattern was absent from visible page text, page HTML,
  `localStorage`, and `sessionStorage`.

Behavior adjustment:

- The legacy skipped browser test drove the old `ENGINE_V2=true` gateway,
  `/api/chat/*` routes, and provider fixture. Reborn's browser-equivalent
  contract is the typed `auth_required` WebChat v2 gate plus
  `/api/reborn/product-auth/manual-token/submit`; the port therefore protects
  the no-retention browser boundary while leaving full provider execution to
  lower-level Reborn/product-auth contracts.

### Step 65: Legacy Notion OAuth Card Render Port

Extended `test_reborn_webui_v2_legacy_auth_flows.py`.

Ported the skipped browser render assertion from
`test_v2_notion_mcp_oauth_flow.py` that an OAuth auth card visibly identifies
the provider that requires authorization:

- emitted a Reborn WebChat v2 `auth_required` prompt with
  `challenge_kind=oauth_url`;
- supplied Notion-specific provider, account label, headline, body, and HTTPS
  authorization URL metadata;
- asserted the Reborn OAuth gate renders the Notion headline/account label and
  provider-specific authorization action.

Behavior adjustment:

- The legacy skipped test triggered a Notion MCP OAuth challenge through the old
  gateway chat path. The Reborn port covers the browser projection contract for
  provider-labelled OAuth gates. End-to-end MCP bearer injection remains a
  lower-level runtime/product-auth concern until a standalone Reborn provider
  harness exposes that full path through `/api/webchat/v2/*`.

### Step 66: Legacy GSuite OAuth Card Render Port

Extended `test_reborn_webui_v2_legacy_auth_flows.py`.

Ported the skipped browser render assertion from
`test_v2_gsuite_oauth_flow.py` that the OAuth card exposes a visible
authorization action when a Google Workspace credential is required:

- emitted a Reborn WebChat v2 `auth_required` prompt with
  `challenge_kind=oauth_url`;
- supplied Google Workspace provider/account metadata and an HTTPS Google OAuth
  authorization URL;
- asserted the Reborn OAuth gate renders the Google Workspace label, keeps the
  authorization action visible, and writes the expected HTTPS `href`.

Behavior adjustment:

- The legacy skipped test triggered the prompt by sending chat through the old
  GSuite provider fixture. The Reborn port protects the browser projection and
  URL-safety contract for provider-labelled OAuth gates. Full Google provider
  execution, token exchange, and per-user provider isolation remain service or
  adapter-level Reborn follow-ups until standalone WebChat v2 exposes that
  provider harness directly.

### Step 67: Reborn Agent File Download CI Inclusion

Updated `.github/workflows/reborn-e2e.yml`.

Moved the existing `test_reborn_v2_file_download.py` browser scenario into the
Reborn WebUI v2 CI test list:

- keeps the shared Reborn WebUI v2 smoke job exercising the agent-produced
  `/workspace` file path flow;
- covers browser rendering of assistant-referenced project-file chips;
- covers both direct chip-download and preview-modal download paths for
  agent-written CSV/PDF artifacts;
- ensures the `local-dev-yolo` Reborn harness, mock LLM `builtin.write_file`
  mapping, and authenticated blob-download path stay in the PR gate rather than
  remaining supplemental local coverage.

Behavior adjustment:

- This scenario is Reborn-native rather than a direct legacy gateway endpoint
  port, but it protects the functional browser/download outcome expected from
  legacy file/artifact workflows on the current WebUI v2 surface.

### Step 68: Legacy Files-Only Attachment Port

Extended `test_reborn_webui_v2_legacy_attachments.py` and fixed the Reborn
composer/history rendering path.

Ported the files-only attachment behavior from legacy `test_chat.py`:

- staged a PDF and text file without entering message text;
- sent the attachment-only draft through the real Reborn composer;
- asserted the live user message renders attachment cards without exposing the
  backend placeholder or raw `<attachments>` context;
- reloaded the page and asserted the persisted timeline rehydrates the same
  attachment cards without placeholder text.

Issue fixed:

- Reborn's composer displayed the send button state as payload-aware, but
  `handleSend` returned early when the text area was empty. Staged files could
  not be sent without extra typed text. The composer now treats staged
  attachments as a valid payload, sends a backend-safe `(files attached)`
  placeholder when the wire contract needs non-empty content, and keeps the UI
  display content empty for attachment-only messages.
- Persisted timeline rows containing that placeholder are projected back to
  empty text when they also carry user attachments, so reload/history rendering
  matches the live attachments-only view.

### Step 69: Legacy Failed-Send Retry Port

Extended `test_reborn_webui_v2_legacy_pending_messages.py` and fixed the Reborn
failed-message retry action.

Ported the actionable retry affordance behind legacy pending-message failure
cleanup:

- forced the first Reborn message send to fail with a service-unavailable
  response;
- asserted the failed optimistic row renders a single visible error and retry
  action;
- clicked `Retry message`;
- asserted the same message content is submitted again and the old error/retry
  affordance is cleared from the replacement optimistic row.

Issue fixed:

- Reborn rendered a `Retry message` button for failed optimistic user messages,
  but `useChat` exposed `retryMessage` as a no-op compatibility stub. The
  handler now removes the failed row and resubmits the stored original payload
  through the normal send path. Failed rows keep their retry content and staged
  attachment payload so text and attachment sends can be retried consistently.

### Step 70: Legacy OAuth Completion Isolation Port

Extended `test_reborn_webui_v2_legacy_auth_flows.py`.

Ported the isolation intent behind legacy onboarding/OAuth completion tests:

- opened a Reborn OAuth auth gate for one run gate;
- dispatched the browser storage completion event used by Reborn's product-auth
  callback bridge, but with a different `gate_ref`;
- asserted the active OAuth prompt remains visible and no unrelated completion
  clears the pending gate.

Behavior result:

- no Reborn product change was required. `useChat` already matches callback
  completions by `turn_run_ref` and `gate_ref`; this browser test now protects
  that isolation contract.

### Step 71: Legacy Manual Auth Cancel Port

Extended `test_reborn_webui_v2_legacy_auth_flows.py`.

Ported the browser-visible part of legacy v2 auth-cancel behavior to Reborn's
manual-token gate:

- emitted a Reborn manual-token `auth_required` prompt;
- clicked the gate's `Cancel` action before entering a token;
- asserted no product-auth manual-token submit request was sent;
- asserted the run-scoped gate resolve endpoint received a `cancelled`
  resolution with a client action id;
- asserted the prompt clears from the thread.

Behavior adjustment:

- legacy v2 accepted typed chat text such as `cancel` while an auth gate was
  pending. Reborn blocks new composer sends during pending gates and exposes
  cancellation through the structured gate action, so the port protects the
  same cancellation outcome through the current UI contract.

### Step 72: Responses API Rate-Limit Harness Hardening

Hardened `test_reborn_webui_v2_legacy_responses_api.py` after a full migrated
suite run exposed a transient `429 rate_limited` response on the second
Responses create request in the continue/retrieve scenario.

Issue found:

- the Responses API scenario passes in isolation, but the full migrated suite
  shares the same mock-backed model environment and fixed Reborn user identity
  across many browser/API tests. Under that suite pressure, a create request can
  hit a temporary rate-limit response even though the behavior under test is
  previous-response continuation and retrieval, not rate-limit enforcement.

Adjustment:

- `_create_response` now retries short-lived 429 responses before asserting the
  normal 200 contract. This keeps production rate limits intact while preventing
  unrelated shared-fixture pressure from flaking the migrated Responses
  compatibility coverage.

Harness cleanup:

- `test_reborn_webui_v2_legacy_pending_messages.py` now mocks Reborn's current
  attachment capability field names (`max_count`, `max_file_bytes`,
  `max_total_bytes`) instead of legacy names, so pending-message tests read the
  same session shape as production.

### Step 73: Legacy Responses Context Injection Port

Extended `test_reborn_webui_v2_legacy_responses_api.py` and fixed the Reborn
OpenAI-compatible Responses workflow.

Ported the legacy `x_context.notification_response` approval/rejection coverage:

- accepted `x_context` on Reborn `POST /v1/responses` and `/api/v1/responses`;
- kept the legacy `context` alias for compatibility;
- injected a sanitized, human-readable context summary into the submitted
  product-workflow payload before the user input reaches the Reborn turn;
- enforced the same 10 KiB context limit before any product-workflow side
  effect;
- covered the handoff with Rust handler-contract tests and the browser-suite
  API E2E approval/rejection scenarios.

Issue fixed:

- Reborn's Responses DTO did not carry `x_context`, so legacy clients could
  send notification approval/rejection context and receive a normal response
  while the structured context was silently dropped before the agent turn.

### Step 74: Legacy Responses Untyped Message Input Port

Extended `test_reborn_webui_v2_legacy_responses_api.py` and fixed the Reborn
OpenAI-compatible Responses request DTO.

Ported the legacy message-array shape from `test_responses_api.py`:

- accepted `input=[{"role": "user", "content": "..."}]` without requiring a
  `type: "message"` discriminator;
- normalized untyped message items into Reborn's internal
  `openai_compat.responses_input.v1` payload before the product-workflow turn;
- kept existing typed `message`, `function_call`, and `function_call_output`
  request forms intact;
- covered the behavior at the Rust route-handler boundary and in the migrated
  Python E2E Responses scenario.

Issue fixed:

- The initial Reborn Responses port had to alter the legacy test input to the
  typed Responses item shape. That left clients using the older
  role/content-only message form with a `400` even though legacy IronClaw
  accepted it.

### Step 75: Legacy Responses Empty Text Rejection Port

Extended `test_reborn_webui_v2_legacy_responses_api.py` and fixed the Reborn
OpenAI-compatible Responses workflow validation.

Ported the legacy empty-input error from `test_responses_api.py`:

- rejected `input: ""` and whitespace-only text with a field-scoped
  `400 invalid_request` before any product-workflow side effect;
- kept the existing empty input-item array rejection;
- covered the validation at the Rust route-handler boundary and in the migrated
  Python E2E Responses scenario.

Issue fixed:

- Reborn previously normalized an empty text input into a product-workflow
  message payload instead of rejecting it at the Responses route boundary. That
  diverged from legacy `/v1/responses`, which rejected empty input directly.

### Step 76: Legacy EventSource Error Reconnect Port

Extended `test_reborn_webui_v2_legacy_sse_history.py`.

Ported the legacy EventSource reconnect-after-disconnect contract from
`test_sse_reconnect.py` to Reborn WebUI v2:

- opened a fake browser `EventSource` through the real chat page;
- emitted a cursor-bearing `keep_alive` frame so the hook records the latest
  stream position;
- triggered the real `onerror` reconnect path;
- asserted the next stream URL carries the bearer token and
  `after_cursor=cursor-before-error`.

Behavior adjustment:

- Legacy v1 explicitly called `connectSSE()` and relied on private globals plus
  `/api/chat/history` reload timing. Reborn's public behavior is a per-thread
  event stream that resumes via `after_cursor`, so the migrated test asserts the
  caller-visible reconnect URL rather than the removed v1 history-reload hook.

### Step 77: Legacy Extension Install OAuth URL Port

Extended `test_reborn_webui_v2_legacy_extensions.py`.

Ported the install-response OAuth popup behavior from legacy `test_extensions.py`
to Reborn's extension registry flow:

- installed a registry extension through the real Reborn Extensions page;
- mocked the Reborn `/api/webchat/v2/extensions/install` response with
  `auth_url`;
- asserted the browser opens the HTTPS authorization URL through `window.open`;
- asserted the install request uses Reborn's structured
  `{ package_ref: { kind, id } }` payload.

Behavior adjustment:

- Legacy assertions targeted `/api/extensions/install`, legacy auth-card globals,
  and unqualified extension names. Reborn's equivalent install boundary is the
  WebUI v2 extension registry action and the package-ref DTO; auth UI after
  provider callback remains covered by the product-auth/browser prompt tests.

### Step 78: Legacy Extension Install Auth URL Safety Port

Extended `test_reborn_webui_v2_legacy_extensions.py`.

Completed the install-side security coverage for the legacy
`test_oauth_url_injection_blocked` regression:

- mocked a successful Reborn extension install response with a non-HTTPS
  `auth_url`;
- asserted the browser shows the existing HTTPS-only error;
- asserted `window.open` is not called;
- asserted the install request still reaches the Reborn package-ref install
  boundary exactly once.

Behavior adjustment:

- Legacy grouped activate/configure URL validation under the old extension
  APIs. Reborn has separate install, activate, and configure OAuth branches, so
  the migrated suite now pins install URL safety independently from the existing
  activation and configure OAuth URL-safety tests.

### Step 79: Legacy Manual Auth Submit Failure Port

Extended `test_reborn_webui_v2_legacy_auth_flows.py`.

Ported the retry behavior from legacy `test_auth_card_submit_error` to Reborn's
manual-token product-auth gate:

- emitted a Reborn `manual_token` `auth_required` prompt;
- mocked `/api/reborn/product-auth/manual-token/submit` returning a failed
  credential-save response;
- asserted the manual-token gate remains visible and retryable;
- asserted the token input and submit action are re-enabled after the failure;
- asserted the paused run is not resolved when credential storage fails.

Behavior adjustment:

- Legacy v1 submitted the raw token directly to `/api/chat/gate/resolve` and
  rendered the backend's prose error. Reborn first stores credentials through
  product-auth and resolves the gate only after receiving a credential ref, so
  the migrated test pins the safer two-step contract and the localized
  credential-save failure message.

### Step 80: Legacy Auth Prompt Replacement Port

Extended `test_reborn_webui_v2_legacy_auth_flows.py`.

Ported the modal/global auth prompt replacement behavior from legacy
`test_auth_card_replaces_existing_same_extension` and
`test_auth_card_for_different_extension_replaces_existing_prompt`:

- emitted one Reborn `manual_token` `auth_required` prompt;
- emitted a second `manual_token` prompt for a different provider before
  resolving the first;
- asserted only one auth gate remains visible;
- asserted the first prompt body is gone and the second prompt's headline,
  account label, and body are rendered;
- submitted a token and asserted Reborn sends the credential to the second
  gate ref only.

Behavior adjustment:

- Legacy v1 replaced DOM cards keyed by extension name. Reborn has a single
  pending run gate, so the migrated test asserts the user-visible single-prompt
  invariant and verifies the submit path targets the replacement
  `auth_request_ref`.

### Step 81: Legacy Pairing Enter-Key Submit Port

Extended `test_reborn_webui_v2_legacy_extensions.py`.

Completed the keyboard path from legacy `test_pairing_card_submit_success` for
Reborn's channel pairing section:

- opened a Telegram channel in `pairing_required` state;
- filled the pairing-code input with padded lowercase text;
- pressed Enter inside the input instead of clicking the submit button;
- asserted the Reborn pairing redeem endpoint receives the trimmed uppercase
  code;
- asserted the pairing success state renders and clears the input.

Behavior adjustment:

- Legacy submitted to `/api/pairing/telegram/approve` with a thread id from the
  chat card. Reborn's equivalent channel surface submits
  `{ channel, code }` to `/api/webchat/v2/extensions/pairing/redeem`, so the
  migrated test protects the keyboard affordance and Reborn DTO normalization
  instead of the removed legacy chat-card payload.

Issue found and fixed:

- The shared Reborn `PairingSection` trimmed manually entered codes but did not
  uppercase them before redeeming. The legacy gateway normalized pairing codes
  with `trim().toUpperCase()`, and provider-issued pairing codes are rendered
  uppercase. Reborn now applies the same normalization for both click-submit
  and Enter-submit paths.

### Step 82: Legacy Slack Pairing Enter-Key Submit Port

Extended `test_reborn_webui_v2_legacy_channel_connect.py`.

Ported the same keyboard pairing affordance to Reborn's Slack connect command
card:

- loaded the Reborn connectable-channels response with a Slack
  `inbound_proof_code` action;
- entered `connect slack` in the chat composer and asserted no normal chat send
  is required;
- filled the Slack proof-code input with padded lowercase text;
- pressed Enter inside the input;
- asserted the pairing redeem endpoint receives the trimmed uppercase code and
  the success state renders.

Issue found and fixed:

- Slack connect cards use the shared `SlackPairingSection`, not the generic
  Extensions `PairingSection` fixed in Step 81. That component also only
  trimmed manual codes before redeeming. It now uppercases codes before both
  click and Enter submission so Slack proof-code redemption matches the legacy
  normalization contract.

### Step 83: Legacy Slack Pairing Failure Retry Port

Extended `test_reborn_webui_v2_legacy_channel_connect.py`.

Ported the failed-pairing retry behavior from legacy
`test_pairing_card_submit_error` to Reborn's Slack connect command card:

- opened a Slack `inbound_proof_code` connect card from the chat composer;
- submitted a bad proof code through the Slack pairing section;
- mocked `/api/webchat/v2/extensions/pairing/redeem` returning a failed
  response;
- asserted the inline error is visible and the connect card remains open;
- asserted the proof-code input retains the user's code for retry;
- asserted the redeem request still sends the normalized uppercase code.

Issue found and fixed:

- `SlackPairingSection` cleared the proof-code input immediately after sending
  the redeem request. A failed request therefore forced users to retype the
  code. The component now clears the input only on successful redemption,
  matching the generic Reborn pairing section and the legacy retry behavior.

### Step 84: Legacy Approval Denied Status Port

Extended `test_reborn_webui_v2_legacy_approval.py`.

Ported the visible denied-state behavior from legacy
`test_approval_deny_shows_denied` to Reborn's approval gate flow:

- opened a browser-stubbed approval gate for `builtin.shell`;
- denied the gate through the Reborn approval card;
- mocked the resolve response as a resumed run, matching the Reborn path that
  continues after a denied gate;
- asserted the approval card hides and the corresponding tool activity card is
  visible with `data-tool-status="declined"` and the `gate_declined` detail.

Behavior mapping:

- Legacy v1 kept the old approval-card DOM around with resolved text such as
  `Denied`. Reborn removes the pending approval card after resolution and
  projects the denied outcome into the live activity timeline. The port checks
  that Reborn-native visible state instead of the removed v1 resolved-copy
  element.

### Step 85: Legacy Configure Field Variants Port

Extended `test_reborn_webui_v2_legacy_extensions.py`.

Ported the setup-field rendering intent from legacy
`test_configure_modal_field_variants` to Reborn's v2 Extensions configure
modal:

- opened a configured extension's setup modal through the real Extensions page;
- mocked v2 setup metadata containing required, optional, provided, and
  auto-generated manual secrets plus an optional text field;
- asserted the visible labels, `configured` badge, optional badges, existing
  secret keep-placeholder, auto-generate hint, and text-field placeholder;
- asserted dismissing the modal does not submit setup payloads.

Issue found and fixed:

- `ConfigureModal` used modal visuals but did not expose dialog semantics to the
  browser accessibility tree. The modal shell now renders `role="dialog"`,
  `aria-modal="true"`, and an `aria-labelledby` link to its title so tests and
  assistive tooling can address it as a named dialog.

### Step 86: Legacy Configure Setup URL Port

Extended `test_reborn_webui_v2_legacy_extensions.py`.

Ported the credentials-link intent from legacy `test_auth_card_with_setup_url`
to Reborn's v2 Extensions configure modal:

- opened the real Reborn Extensions configure modal with setup metadata that
  includes an onboarding `setup_url`;
- asserted an HTTPS setup URL renders as a `Get credentials` link with
  `_blank` and `noopener noreferrer`;
- asserted a non-HTTPS setup URL does not render a clickable credentials link
  and does not leave a `javascript:` href in the modal.

Issue found and fixed:

- `ConfigureModal` wrote onboarding `setup_url` values directly into `href`.
  The modal now parses setup URLs and renders only HTTPS links, matching the
  stricter auth/OAuth URL handling elsewhere in the Reborn Extensions UI.

### Step 87: Legacy Selector-Sensitive Extension Name Port

Extended `test_reborn_webui_v2_legacy_extensions.py`.

Ported the intent from legacy
`test_auth_and_configure_helpers_escape_selector_sensitive_extension_names` to
Reborn's package-ref based Extensions UI:

- opened the real Reborn Extensions Installed tab with an extension whose
  display name and package id contain quotes;
- opened the configure modal through the extension card instead of using legacy
  global DOM helpers;
- submitted a manual secret through the v2 setup endpoint;
- asserted the setup request preserved the quoted package id and payload.

Behavior mapping:

- Legacy v1 had global auth/configure helper functions that queried DOM nodes
  by extension-name attributes and therefore needed selector escaping. Reborn
  no longer exposes those globals; the corresponding Reborn risk is URL/path
  encoding of `package_ref.id` and role/text-based card selection. The port
  verifies that path.

### Step 88: Legacy Other-Thread Approval Isolation Port

Extended `test_reborn_webui_v2_legacy_approval.py`.

Ported the isolation intent from legacy approval tests that ensured approval
state from one thread did not intercept another thread's normal sends:

- opened two mocked Reborn WebChat v2 threads in the browser;
- emitted a pending approval gate on thread A and verified the active composer
  is locally blocked;
- switched through the real Reborn thread sidebar to thread B;
- asserted thread B has no stale approval card, its composer is enabled, and a
  normal chat message posts to thread B's v2 `/messages` endpoint.

Behavior mapping:

- Legacy v1 also tested slash/text approval aliases crossing thread boundaries.
  Reborn has no hidden text-alias approval path; approval resolution is through
  the run-scoped gate card. The Reborn-equivalent risk is stale per-thread gate
  state leaking across route/sidebar thread changes, so the port asserts that
  boundary directly.

### Step 89: Legacy Foreign-Thread Auth Prompt Isolation Port

Extended `test_reborn_webui_v2_legacy_auth_flows.py`.

Ported the isolation intent from legacy onboarding/auth-required tests that
ensured auth prompts for another thread did not render into or block the active
thread:

- opened two mocked Reborn WebChat v2 threads in the browser;
- emitted a pending manual-token auth gate on thread A and verified the active
  composer is locally blocked;
- switched through the real Reborn thread sidebar to thread B;
- asserted thread B has no stale auth gate, its composer is enabled, and a
  normal chat message posts to thread B's v2 `/messages` endpoint.

Behavior mapping:

- Legacy v1 tracked unread counters for foreign-thread onboarding events in a
  global map. Reborn receives events through the active thread's route-scoped
  EventSource and has no equivalent global auth-card renderer. The
  Reborn-equivalent risk is stale auth gate state leaking across thread changes,
  so the port asserts that boundary directly.

### Step 90: Legacy Background Thread Processing Indicator Port

Extended `test_reborn_webui_v2_legacy_pending_messages.py` and fixed the
Reborn sidebar thread presenter.

Ported the user-visible affordance behind legacy
`test_background_thread_shows_processing_indicator`:

- opened a Reborn chat route on one active quiet thread;
- returned another thread in the v2 thread list with `state: "Processing"`;
- asserted the background row renders the existing `Running` sidebar indicator;
- asserted the active quiet thread does not inherit that state and does not
  show the in-thread typing indicator.

Issue found and fixed:

- The live Reborn sidebar (`components/sidebar-threads.js`) read only the
  browser-local thread-state store. It ignored server-provided thread summary
  states, so background processing threads could lose their sidebar indicator
  after a thread-list refresh or page load. The presenter now maps summary
  `Processing`/`Running` to the existing running presentation,
  `AwaitingApproval` to needs-attention, and failed/interrupted states to the
  failed presentation, while local live state still takes precedence.

### Step 91: Legacy Automation Run History Navigation Port

Extended `test_reborn_webui_v2_legacy_automations.py`.

Ported the visible run-history intent from legacy routine execution/failure UI
coverage to the Reborn Automations detail panel:

- selected a failed scheduled automation and asserted the detail panel exposes
  the needs-review status, failed success rate, recent-runs section, error
  status, thread id, and run id;
- clicked the run's Logs action and asserted navigation to the scoped Reborn
  logs route with both `thread_id` and `run_id`;
- selected a running automation, asserted the current-run id and backing thread
  are visible, then opened the run's chat thread through the detail action.

Behavior mapping:

- Legacy routine tests inspected `/api/routines/*`, `/api/jobs/*`, and legacy
  routine/job pages. Reborn's portable browser surface is the Automations detail
  panel: it receives `recent_runs` from `/api/webchat/v2/automations`, maps
  terminal/running status into the panel, and links accepted runs to Reborn
  chat and logs routes. Routine event triggers, full-job endpoints, and
  routine-scoped OAuth credential injection remain open because they still lack
  a non-stub Reborn v2 routines surface.

### Step 92: Legacy Project Scoped Conversation Port

Extended `test_reborn_webui_v2_legacy_projects.py` and fixed Reborn project
workspace navigation.

Ported the project-to-thread workflow intent from legacy project drill-in
coverage to the current Reborn Projects workspace:

- opened a real Reborn project workspace from `/v2/projects`;
- clicked the workspace `New conversation` action;
- asserted the v2 thread-create request includes the selected `project_id`;
- asserted the browser lands on the new `/v2/chat/{thread_id}` route with the
  chat composer visible.

Issue found and fixed:

- The Projects page created a scoped thread with `threadsState.createThread(projectId)`
  but then navigated to `/chat` with the thread id only in router state.
  `ChatPage` clears active thread state on routes without `:threadId`, so the
  newly-created project thread was immediately lost. The workspace action now
  navigates directly to `/chat/{newThreadId}`.

Behavior mapping:

- Legacy project tests could drill into engine mission/thread detail panels.
  Those Reborn project child APIs are still TODO stubs, so this step protects
  the implemented project-thread boundary: a project workspace can create and
  open a chat thread scoped to that project through the v2 thread API.

### Step 93: Legacy Project Creation Draft Port

Extended `test_reborn_webui_v2_legacy_projects.py` and fixed the remaining
Projects-to-chat navigation path.

Ported the top-level project creation intent from the legacy Projects UI onto
the current Reborn Projects overview:

- opened `/v2/projects` with real v2 project-list fixtures;
- clicked the overview `New project` action;
- asserted the v2 thread-create request is an unscoped chat thread creation
  with a `client_action_id`;
- asserted the browser lands on `/v2/chat/{thread_id}` with the project
  creation prompt seeded into the composer.

Issue found and fixed:

- The Projects overview `New project` action created a thread but navigated to
  `/chat` with the thread id only in router state. `ChatPage` clears active
  thread state whenever the URL has no `:threadId`, so the seeded project
  creation draft could be attached to no active thread. The action now
  navigates directly to `/chat/{newThreadId}` and keeps only the composer draft
  in router state.

Validation:

- `tests/e2e/.venv/bin/pytest tests/e2e/scenarios/test_reborn_webui_v2_legacy_projects.py -q`
  -> `4 passed`

Behavior mapping:

- Legacy project creation was intertwined with the old engine project and
  mission surfaces. Reborn's implemented equivalent is the guided chat entry
  point for creating a project; child mission/thread/widget drill-in remains
  open until the v2 project child endpoints replace the current TODO stubs.

### Step 94: Legacy Generic Channel Connect Pairing Port

Extended `test_reborn_webui_v2_legacy_channel_connect.py` and fixed the
chat-owned connect-card renderer.

Ported the non-Slack pairing-card intent from legacy pairing UI coverage to
Reborn's chat connect-command path:

- loaded `/api/webchat/v2/channels/connectable` with a Telegram
  `inbound_proof_code` action;
- entered `connect telegram` in the chat composer and blocked normal
  `/api/webchat/v2/threads/*/messages` sends to prove the command is handled
  locally;
- asserted the connect card renders Telegram-specific title/instructions;
- submitted a padded lowercase proof code through the generic pairing section;
- asserted `/api/webchat/v2/extensions/pairing/redeem` receives
  `{ channel: "telegram", code: "PAIR-2468" }`;
- asserted the success copy renders and the input clears.

Issue found and fixed:

- `ChannelConnectCard` only rendered a redeemable proof-code form for Slack.
  Non-Slack channels advertising the same `inbound_proof_code` strategy showed
  instructions but no input, so a typed Reborn connect action could not be
  completed from chat. The chat card now reuses the generic Reborn
  `PairingSection` and v2 pairing redeem endpoint for non-Slack
  `inbound_proof_code` channels while preserving Slack's specialized copy and
  selectors.

Validation:

- `tests/e2e/.venv/bin/pytest tests/e2e/scenarios/test_reborn_webui_v2_legacy_channel_connect.py -q`
  -> `4 passed`

Behavior mapping:

- Legacy pairing UI tests used v1 onboarding globals and
  `/api/pairing/{channel}/approve`. Reborn's implemented chat-owned equivalent
  is a typed connectable-channel action plus the v2 pairing redeem endpoint.
  Admin pending-pairing enumeration and channel webhook behavior remain open
  until standalone Reborn exposes matching v2 surfaces.

### Step 95: Legacy Looping Tool-Call Termination Port

Extended `test_reborn_webui_v2_legacy_tool_execution.py`.

Ported the functional intent of legacy
`test_looping_tool_calls_terminate_under_low_iteration_limit` to Reborn's
standalone WebChat v2 turn path:

- added `IRONCLAW_REBORN_PLANNED_DEFAULT_ITERATION_LIMIT` as a local harness
  override for the default planned loop family, because legacy
  `AGENT_MAX_TOOL_ITERATIONS` does not configure Reborn's planner budget;
- submitted the mock LLM's `issue 1780 loop forever` trigger through the v2
  message API while a live WebChat v2 SSE stream was open;
- asserted the stream observes one completed `builtin.echo` invocation under
  the low cap and then receives a terminal failed-run projection.

Issue found:

- Reborn's low-cap loop path is bounded, but the terminal category currently
  projects as `driver_protocol_violation` rather than legacy
  `iteration_limit`. With the cap set to one, the run completes one echo call
  and then fails during result-only exit application. The port records that
  current behavior instead of pretending the old category is already preserved.

Validation:

- `tests/e2e/.venv/bin/pytest tests/e2e/scenarios/test_reborn_webui_v2_legacy_tool_execution.py -q`
  -> 7 passed
- `cargo test -p ironclaw_agent_loop default_family_iteration_limit_can_be_overridden_for_harnesses`
  -> passed
- `cargo build -p ironclaw_reborn_cli --features webui-v2-beta`
  -> passed

Behavior mapping:

- Legacy coverage used the gateway server with `AGENT_MAX_TOOL_ITERATIONS=2`.
  Reborn's current equivalent is the WebChat v2 failed-run projection for the
  same mock repeated-tool trigger. The exact failure taxonomy remains a parity
  gap.

### Step 96: Legacy V2 Shell Removed-Tab Port

Extended `test_reborn_webui_v2_legacy_core.py`.

Ported the remaining browser-visible intent from legacy
`test_v2_activity_shell.py` that still has a Reborn shell equivalent:

- asserted Reborn's sidebar exposes the supported `Automations` work surface;
- asserted removed legacy `Routines` and `Missions` navigation entries are not
  present in the standalone Reborn WebUI v2 sidebar.

Behavior mapping:

- Legacy `ENGINE_V2=true` hid `Routines` and used `Missions` as the gateway v2
  work surface. Standalone Reborn keeps the legacy routines/missions routes
  hidden from primary navigation and exposes Automations for the implemented
  browser workflow.

## Open Migration Buckets

Not yet ported:

- remaining legacy chat UI affordances that have Reborn equivalents; the legacy
  inline slash autocomplete menu has no current Reborn v2 surface and is
  documented above;
- remaining legacy SSE/history edge cases only where Reborn exposes a matching
  product concept; active-thread fallback and read-only external-channel refresh
  are legacy v1 routing semantics rather than current standalone Reborn v2 UI
  behavior, while active-thread retention after summary refresh, route-scoped
  cursor reset, EventSource error reconnect, stale replay dedupe, multi-tab
  fan-out, keepalive comments, connection limits, and reload persistence are
  covered;
- remaining DOM/resource-limit scenarios for any future capped long-running
  activity stores beyond the current timeline paging, near-cap response
  projection, background-thread processing summary, and SSE reconnect-timeout
  cleanup coverage;
- deeper tool approval scenarios that need real Reborn runtime/tool execution,
  persistence, or recovery beyond the browser approval-card, denied activity,
  local send-blocking, cross-thread isolation, and persisted activity-card
  contracts; legacy text-alias interception is v1 behavior superseded by
  Reborn's disabled-composer gate flow;
- remaining settings/extension lifecycle scenarios beyond Settings search,
  Skills, tool permissions, channel label regressions, extension revisit
  refetch, and the top-level extension install/manage/configure surface,
  including any future persistence-backed extension-service contracts not
  visible in the browser;
- deeper OAuth/product-auth install/callback flows beyond browser prompt
  handling, prompt replacement, cross-thread isolation, extension OAuth-start
  URL safety, and existing Rust callback contracts, including hosted provider
  refresh and provider-backed extension/MCP setup only where standalone Reborn
  exposes matching v2 browser endpoints or fixtures;
- remaining Slack/Telegram/channel pairing scenarios beyond Reborn proof-code
  connect cards and the generic member self-claim form, especially lower-level
  admin pending pairing APIs once standalone Reborn exposes a matching v2
  pending-list endpoint;
- remaining Slack/Telegram/channel approval and webhook scenarios beyond
  WebChat v2 approval cards and product-workflow/adapter contracts, because the
  legacy tests target old Slack/Telegram WASM-channel controllers and text alias
  behavior rather than current standalone Reborn WebUI v2 surfaces;
- admin/operator flows, including Users, dashboard, and usage views, because the
  current Reborn Admin API adapter intentionally returns TODO stub payloads until
  v2 admin endpoints replace the legacy `/api/admin/*` contracts;
- legacy `/v2/routines` parity, because the current Reborn routines page still
  uses TODO client stubs instead of real v2 endpoints;
- legacy HTTP webhook/channel ingress parity, because standalone Reborn WebUI
  v2 has no matching `/webhook` HTTP-channel product surface;
- legacy gateway widget customization parity, because Reborn WebUI v2 has no
  dynamic `.system/gateway/*` widget/CSS loader contract;
- legacy extension uninstall secret-table cleanup parity, because those tests
  inspect legacy `secrets` rows behind `/api/extensions/*` rather than Reborn
  product-auth/extension service contracts;
- legacy routine event/full-job/OAuth-credential parity beyond the Reborn
  Automations run-history panel, because those tests target legacy
  `/api/routines/*`, `/api/jobs/*`, HTTP-channel triggers, routine-scoped OAuth
  fallback, and the legacy routines tab;
- legacy project mission/thread/widget drill-in parity, because the current
  Reborn project page maps real project entities but still uses TODO client
  stubs for per-project missions, threads, widgets, and detail actions;
- legacy plan-mode browser parity, because the current Reborn WebChat v2 UI has
  no visible plan checklist/card surface or matching plan interaction controls;
- legacy portfolio widget/share parity, because current Reborn project widgets
  are TODO client stubs and no portfolio-specific widget/share-modal contract
  exists in WebUI v2;
- remaining legacy owner-scope, multi-tenant greeting, and engine-v2 visibility
  parity beyond the Reborn session/auth API coverage, because those tests target
  old gateway/admin/routine/engine endpoints rather than current standalone
  Reborn WebUI v2 product surfaces;
- legacy empty-model-reply durable assistant-transcript parity remains
  intentionally unported because Reborn treats model `InvalidOutput` as a
  failed run without writing an assistant transcript row; browser-visible
  failure parity is covered by Step 52;
- legacy `ENGINE_V2=true` gateway contract parity for old `/api/chat/*`,
  `/api/engine/*`, `/api/chat/approval`, `/api/chat/gate/resolve`, and
  `/oauth/callback` scenarios. These should be replaced with Reborn-native
  runner/driver/executor, product-auth, and `/api/webchat/v2/*` coverage rather
  than copied to the standalone WebUI harness;
- legacy WASM lifecycle API parity, because `test_wasm_lifecycle.py` asserts
  exact `/api/extensions/*` registry/install/setup/remove/reinstall response
  fields and old extension auth state. Reborn browser coverage now includes the
  remove/reinstall fresh-setup invariant, but not the legacy API shape or
  secret-table side effects;
- provider-fixture full-path parity for Google/GitHub/Slack/Notion flows where
  the current tests still use old gateway chat/history endpoints or remain
  browser-skipped pending a Reborn-native `webui-v2-beta` harness;
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

The configure-modal extension port found two Reborn product defects. First,
`success: false` setup responses kept the modal open but did not show the
server's failure message; setup submission now treats that envelope as a
mutation error so the existing modal error region renders it. Second, extension
install/activate auth URLs and configure OAuth authorization URLs were opened or
navigated without validating the scheme. Extension auth popup handling now
parses URLs and allows only HTTPS before calling `window.open` or assigning
`popup.location.href`.

The tool-permission port did not require a behavior fix in the Reborn settings
API: the v2 endpoint already persisted mutable tool overrides and rejected
locked-tool writes. It did add stable row hooks to the Tools tab so locked-tool
coverage can assert the real row without depending on incidental DOM ancestry.

The CSP/browser-safety port did not require a Reborn product fix. Initial
focused failures were selector mismatches in the port (`New` is a scoped
sidebar button in Reborn, not a legacy `New thread` link).

The product-auth prompt port added stable auth-gate hooks but did not require a
behavior fix. The existing Reborn OAuth card already rejected non-HTTPS
authorization URLs before opening a popup, and the manual-token path already
trimmed the submitted token before calling the product-auth endpoint.

The approval in-flight port found a Reborn browser defect: approval actions did
not disable while the run-scoped gate-resolution request was pending, allowing a
fast double click to submit duplicate decisions. `ApprovalCard` now tracks an
in-flight resolving state and disables the approve, deny, and always-allow
controls until the async action finishes.

The other-thread approval isolation port did not require a Reborn product fix:
the pending gate state is already scoped to the active thread route, and normal
message sends on a different thread continue through the v2 `/messages`
endpoint.

The foreign-thread auth prompt isolation port did not require a Reborn product
fix: pending auth gate state is already scoped to the active thread route, and
normal message sends on a different thread continue through the v2 `/messages`
endpoint.

The background-thread processing port found a Reborn browser defect: the live
sidebar ignored server-provided thread summary states and only rendered
locally-observed active-thread state. `SidebarThreads` now maps summary states
onto the existing per-thread presentation so background processing,
needs-attention, and failed indicators survive thread-list refreshes.

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

The project overview port confirmed a project-detail parity gap: Reborn's
Projects page now uses the real `/api/webchat/v2/projects` surface for project
entities and can create project-scoped chat threads, but its missions, threads,
widgets, mission detail, and thread detail client functions still return TODO
stubs. The migrated tests therefore cover list/search/open-workspace and
project-scoped chat creation behavior while leaving the legacy engine-v2
mission/thread/widget drill-in tests open until matching Reborn endpoints land.

The project creation draft port found the same route-state issue in the
top-level Projects `New project` action: the handler created a thread but
navigated to `/chat` without the thread id in the URL. The browser now opens
`/chat/{thread_id}` and preserves the seeded project-creation composer draft.

The generic channel-connect pairing port found that chat-owned connect cards
only rendered a proof-code form for Slack. Non-Slack `inbound_proof_code`
actions now reuse the generic Reborn pairing form and v2 redeem endpoint, so
typed Telegram-style connect actions can be completed from chat instead of
displaying instructions with no input.

The Responses API port confirmed a route-contract difference: Reborn's
OpenAI-compatible Responses API accepts typed Responses input items and rejects
empty item arrays, while legacy coverage used an untyped message list and empty
text input. The migrated test follows Reborn's DTO and leaves legacy
`x_context.notification_response` context-injection behavior to the dedicated
Step 73 coverage.
