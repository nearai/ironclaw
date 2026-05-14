# V2 Frontend Parity Tasks

This is the build plan for bringing `crates/ironclaw_gateway_reborn/static` to parity with the legacy frontend in `crates/ironclaw_gateway/static`.

Use this as the working checklist. Each task should leave the frontend usable, update this file, and run only lightweight checks unless the implementation changes Rust/backend behavior.

## Working Rules

- Keep work scoped to `crates/ironclaw_gateway_reborn/static` unless the backend API is missing.
- Prefer existing V2 patterns: React hooks under `js/pages/*/hooks`, API wrappers under `lib/*-api.js`, presentational components under `components`.
- If a task touches auth, secrets, restart, listeners, outbound HTTP, sandboxing, or approvals, do a security pass before finishing.
- Do not run broad Rust tests for frontend-only changes. Prefer `node --check` on changed JS modules, `git diff --check`, and browser smoke tests when a gateway is already running.
- If backend behavior changes, update the relevant subsystem docs and check `FEATURE_PARITY.md`.

## Completed

- [x] 2026-04-28: Auth parity pass for OAuth provider buttons, URL `?token=` auto-login, cookie/OIDC session probing, `/auth/logout` cleanup, and `/api/profile` avatar/account/admin-role UI filtering.
- [x] 2026-04-28: Settings import/export JSON toolbar, backed by `/api/settings/export` and `/api/settings/import`.
- [x] 2026-04-28: Settings toolbar search and back-to-inference navigation.
- [x] 2026-04-28: LLM provider management for provider listing, custom provider add/edit/delete, built-in provider configuration, atomic activation, connection tests, model fetching, and provider override persistence.
- [x] 2026-04-28: Restart-needed banner action with Docker-gated availability, confirmation, `/restart` chat dispatch, reconnect progress, and rejection/timeout feedback.
- [x] 2026-04-28: TEE shield with attestation discovery, summary popover, and copy-report feedback.
- [x] 2026-04-28: Skills import/remove management for HTTPS URL and pasted `SKILL.md` content, installed metadata, activation triggers, and no-reload refresh.
- [x] 2026-04-28: Chat send reliability and history rendering parity for pending-message reinjection, retry/cooldown UI, done-without-response recovery, visibility-aware SSE reconnect, persisted attachment parsing, grouped tool activity, and generated-image cache fallback.

## P0: Verify Completed High-Risk Parity

### Auth Parity Browser Pass

Status: Ready for manual verification.

Build/verify:
- [ ] Open `/v2/login` with token-only auth and confirm login persists through V2 navigation.
- [ ] Open `/v2?token=<token>` and confirm auto-login strips or stops relying on the query token after storing it.
- [ ] Confirm OAuth provider buttons are hidden when `/auth/providers` returns no providers.
- [ ] Confirm cookie/OIDC session probing reaches `/api/profile` and renders avatar/account/admin visibility correctly.
- [ ] Confirm `/auth/logout` clears the V2 session and returns to login.

Likely files if fixes are needed:
- `js/app/auth.js`
- `js/pages/login/login-page.js`
- `js/pages/login/hooks/useOAuthProviders.js`
- `js/pages/login/components/oauth-provider-buttons.js`
- `js/components/sidebar-footer.js`

Light checks:
- `node --check` on changed JS files.
- Browser smoke through login/logout only.

### LLM Provider Management Browser Pass

Status: Ready for manual verification.

Build/verify:
- [ ] Settings -> Inference shows built-in providers from `/api/llm/providers`.
- [ ] Custom provider add/edit/delete persists through `llm_custom_providers`.
- [ ] Built-in provider configure persists through `llm_builtin_overrides`.
- [ ] Active provider switch writes `llm_backend` and `selected_model` atomically via `/api/settings/import`.
- [ ] Connection test posts to `/api/llm/test_connection` with provider id/type for vaulted keys.
- [ ] Fetch models posts to `/api/llm/list_models` and fills the model selector.
- [ ] Unconfigured providers cannot be activated and open the configure dialog.

Likely files if fixes are needed:
- `js/pages/settings/components/inference-tab.js`
- `js/pages/settings/components/provider-management.js`
- `js/pages/settings/components/provider-card.js`
- `js/pages/settings/components/provider-dialog.js`
- `js/pages/settings/hooks/useLlmProviders.js`
- `js/pages/settings/hooks/useProviderDialogForm.js`
- `js/pages/settings/hooks/useProviderManagementActions.js`
- `js/pages/settings/lib/llm-providers.js`
- `js/pages/settings/lib/settings-api.js`

Light checks:
- `node --check` on changed JS files.
- `git diff --check`.

## P1: Restart And TEE UI

### Restart Banner Action

Status: Completed.

Build:
- [x] Add a restart action button to `RestartBanner`.
- [x] Read `gatewayStatus.restart_enabled` from outlet context or status hook.
- [x] Disable or explain restart when `restart_enabled` is false.
- [x] On confirm, send `/restart` through the gateway chat/system-command path used by V1.
- [x] Show progress state while the gateway drops and reconnects.
- [x] Keep restart UI unavailable for non-gateway or non-Docker contexts.

Likely files:
- `js/pages/settings/components/restart-banner.js`
- `js/pages/settings/settings-page.js`
- `js/hooks/useGatewayStatus.js`
- `js/lib/api.js` or a settings-local API helper

Backend/API to inspect before changing:
- V1 restart flow in `static/js/core/init-auth.js`.
- Gateway status route: `/api/gateway/status`.
- Restart command behavior in `src/agent/commands.rs`.

Acceptance:
- [x] Restart-needed banner has an actionable button.
- [x] Button is hidden or disabled when `restart_enabled` is false.
- [x] User gets confirmation before restart.
- [x] Failure message is visible if restart is rejected.

Light checks:
- [x] `node --check` on changed JS files.
- [ ] Manual browser click with restart disabled.

### TEE Shield And Attestation

Status: Completed.

Build:
- [x] Port V1 TEE status display into V2 top-level layout.
- [x] Show shield state from gateway status or TEE endpoint data.
- [x] Add attestation popover with report summary.
- [x] Add copy-attestation-report action.
- [x] Keep missing/unavailable attestation quiet and non-blocking.

Likely files:
- `js/layout/gateway-layout.js`
- `js/components/sidebar-footer.js`
- `js/hooks/useGatewayStatus.js`
- New `js/components/tee-shield.js`

V1 reference:
- `static/js/core/gateway-tee.js`

Acceptance:
- [x] Shield renders only when data is available.
- [x] Popover content does not expose secrets.
- [x] Copy action writes the report and gives user feedback.

## P1: Skills Management

Status: Completed without ClawHub search per current scope.

Build:
- [ ] Add ClawHub search UI. Skipped by request.
- [ ] Add install skill by name. Skipped with ClawHub/catalog flow by request.
- [x] Add install skill by HTTPS URL.
- [x] Add direct `SKILL.md` content import.
- [x] Add remove/uninstall if backend support remains active.
- [x] Refresh installed skills after install/remove.
- [x] Render activation triggers and metadata comparable to V1.

Likely files:
- `js/pages/settings/components/skills-tab.js`
- `js/pages/settings/hooks/useSkills.js`
- `js/pages/settings/lib/settings-api.js`
- New skills components if `skills-tab.js` grows too large.

Backend/API:
- `GET /api/skills`
- `POST /api/skills/search`
- `POST /api/skills/install`
- `DELETE /api/skills/{name}`

V1 reference:
- `static/js/surfaces/skills.js`

Acceptance:
- [ ] Searching shows loading, empty, error, and result states. Skipped by request.
- [ ] Installing by registry result works. Skipped by request.
- [x] Installing by URL validates HTTPS before submit.
- [x] Removing asks for confirmation.
- [x] Installed list updates without full page reload.

Light checks:
- [x] `node --check` on changed JS files.
- Browser smoke against a local gateway if available.

## P1: Chat Reliability And History

### Send Reliability

Status: Completed.

Build:
- [x] Reinject pending user message if DB persistence races history reload.
- [x] Show retry action when send fails.
- [x] Add 429 cooldown handling with visible countdown or disabled send state.
- [x] Detect "done without response" and offer reload/recover.
- [x] Close/reconnect SSE based on document visibility without losing active turn state.

Likely files:
- `js/pages/chat/hooks/useChat.js`
- `js/pages/chat/hooks/useSSE.js`
- `js/pages/chat/lib/useChatEvents.js`
- `js/pages/chat/components/message-list.js`
- `js/pages/chat/components/chat-input.js`

V1 references:
- `static/js/surfaces/chat.js`
- `static/js/core/sse.js`
- `static/js/core/history.js`

Acceptance:
- [x] Failed sends do not silently disappear.
- [x] Retried sends preserve thread id and attachments.
- [x] 429 responses do not allow rapid repeat submits.
- [x] SSE reconnect does not duplicate assistant messages.

### History And Rendering

Status: Completed.

Build:
- [x] Parse persisted `<attachments>...</attachments>` in historical user messages.
- [x] Render all historical tool calls, not only the first.
- [x] Add V1-style expandable grouped tool activity.
- [x] Improve generated-image fallback/cache when history lacks `data_url`.

Likely files:
- `js/pages/chat/hooks/useHistory.js`
- `js/pages/chat/components/message-bubble.js`
- `js/pages/chat/components/tool-activity.js`
- `js/pages/chat/components/message-list.js`
- `js/pages/chat/lib/useChatEvents.js`

Acceptance:
- [x] Historical user attachments match live attachment rendering.
- [x] Multiple tool calls in one turn are visible.
- [x] Grouped tool activity can be expanded/collapsed.
- [x] Generated images remain visible after reload when recoverable.

Light checks:
- [x] `node --check` on changed chat JS files.
- [x] `git diff --check`.
- [ ] Browser smoke against a local gateway if available.

## P1: Thread Sidebar Parity

Status: Not started.

Build:
- [ ] Merge `assistant_thread` into displayed thread list.
- [ ] Add channel badges.
- [ ] Add unread badges.
- [ ] Add active/processing indicators from SSE events.

Likely files:
- `js/pages/chat/hooks/useThreads.js`
- `js/pages/chat/components/thread-sidebar.js`
- `js/components/sidebar-threads.js`

Backend/API:
- `GET /api/chat/threads`
- SSE events from `/api/chat/events`

Acceptance:
- [ ] Assistant thread appears in the same navigable list.
- [ ] Current thread remains stable across refresh.
- [ ] Active processing state clears when a turn completes or fails.

## P2: Extension And Onboarding Depth

Status: Partially done. Basic install/activate/remove/configure exists.

Build:
- [ ] Render richer V1 auth/onboarding overlays from SSE.
- [ ] Add auth cancel flow.
- [ ] Add token-submit flow for legacy no-`request_id` auth prompts.
- [ ] Render restart/setup instructions in channel states.
- [ ] Add manual WASM install form outside registry entries.
- [ ] Add manual MCP install/config form outside registry entries.

Likely files:
- `js/pages/extensions/extensions-page.js`
- `js/pages/extensions/components/configure-modal.js`
- `js/pages/extensions/components/installed-tab.js`
- `js/pages/extensions/components/registry-tab.js`
- `js/pages/extensions/components/mcp-tab.js`
- `js/pages/extensions/hooks/useExtensions.js`
- `js/pages/extensions/lib/extensions-api.js`
- Chat auth UI if onboarding appears in chat.

Backend/API:
- `/api/extensions`
- `/api/extensions/registry`
- `/api/extensions/install`
- `/api/extensions/{name}/activate`
- `/api/extensions/{name}/remove`
- `/api/extensions/{name}/setup`
- `/api/chat/auth-token`
- `/api/chat/auth-cancel`
- `/api/chat/gate/resolve`

Security notes:
- Do not derive extension identity from credential names.
- Keep setup routing on extension names.
- Do not expand legacy auth-token behavior beyond compatibility.

Acceptance:
- [ ] Installable extension/channel auth routes to the same setup UI from Chat and Settings.
- [ ] Cancel/token submit works for legacy auth prompts.
- [ ] Manual install forms validate inputs and show backend errors.
- [ ] Restart/setup instructions are visible where backend provides them.

## P2: Widget And Plugin Frontend API

Status: Not started.

Build:
- [ ] Add `window.IronClaw.registerWidget`.
- [ ] Add `IronClaw.registerChatRenderer`.
- [ ] Add widget tab slots.
- [ ] Add safe widget API for authenticated same-origin fetch and event subscription.
- [ ] Add share modal helper if still needed by widgets.

Likely files:
- `js/app/app.js`
- `js/layout/gateway-layout.js`
- `js/pages/chat/components/message-bubble.js`
- `js/pages/projects/components/project-widgets.js`
- New `js/lib/widgets.js`

V1 references:
- `static/js/core/widgets.js`
- `static/styles/components/share-modal.css`

Security notes:
- Lock down global object replacement.
- Keep same-origin checks for API fetch.
- Do not expose raw bearer tokens to widgets.

Acceptance:
- [ ] A test widget can register a tab and render.
- [ ] A chat renderer can match and render a message.
- [ ] Widget API rejects cross-origin fetch targets.

## P2: Chat Composer Parity

Status: Not started.

Build:
- [ ] Add slash command autocomplete.
- [ ] Include skill-based dynamic slash commands.
- [ ] Add tab ghost suggestion behavior.
- [ ] Add approval text shortcuts: `yes`, `always`, `deny`.
- [ ] Detect read-only/non-gateway threads and disable composer.

Likely files:
- `js/pages/chat/components/chat-input.js`
- `js/pages/chat/components/suggestion-chips.js`
- `js/pages/chat/hooks/useChat.js`
- New `js/pages/chat/hooks/useSlashCommands.js`

V1 references:
- `static/js/surfaces/chat.js`
- Command list in `src/agent/commands.rs`

Acceptance:
- [ ] `/` opens command suggestions.
- [ ] Keyboard navigation works without mouse.
- [ ] Approval shortcuts resolve the pending approval rather than sending chat.
- [ ] Read-only channel state is explicit and prevents submit.

## P3: Active Work Surface

Status: Not started.

Build:
- [ ] Port V1 active work store/bar for jobs, missions, and engine threads.
- [ ] Feed live progress snapshots from SSE/job events.
- [ ] Link active work items to Jobs, Missions, Projects, or Chat detail views.

Likely files:
- `js/layout/gateway-layout.js`
- `js/pages/jobs/hooks/useJobs.js`
- `js/pages/missions/hooks/useMissions.js`
- `js/pages/projects/hooks/useProjectsOverview.js`
- New `js/hooks/useActiveWork.js`
- New `js/components/active-work-bar.js`

V1 references:
- `static/js/core/activity-store.js`
- `static/styles/surfaces/activity.css`

Acceptance:
- [ ] Active work appears globally, not only inside a page.
- [ ] Completed/failed work clears or changes state predictably.
- [ ] Links preserve current route context.

## P3: Global Keyboard And Accessibility Helpers

Status: Not started.

Build:
- [ ] Add shortcuts overlay.
- [ ] Add global Ctrl/Cmd tab switching.
- [ ] Add global search focus helper.
- [ ] Add scroll-to-bottom button behavior in chat.
- [ ] Audit focus traps for modals and popovers touched by V2 parity work.

Likely files:
- `js/app/app.js`
- `js/layout/gateway-layout.js`
- `js/pages/chat/components/message-list.js`
- `js/pages/chat/chat-page.js`
- New `js/hooks/useGlobalShortcuts.js`

Acceptance:
- [ ] Shortcuts never steal focus while typing in inputs/textareas.
- [ ] Overlay is keyboard accessible and dismissible.
- [ ] Scroll-to-bottom appears only when useful.

## Compared Legacy Files

Use these as parity references before implementing each task:

- V1 auth/restart: `static/js/core/init-auth.js`
- V1 SSE/reconnect: `static/js/core/sse.js`
- V1 history/rendering: `static/js/core/history.js`, `static/js/core/render.js`, `static/js/core/tool-activity.js`
- V1 chat: `static/js/surfaces/chat.js`
- V1 config/providers: `static/js/surfaces/config.js`
- V1 settings: `static/js/surfaces/settings.js`
- V1 skills: `static/js/surfaces/skills.js`
- V1 widgets: `static/js/core/widgets.js`
- V2 app shell: `v2/js/app/app.js`, `v2/js/layout/gateway-layout.js`
- V2 API wrapper: `v2/js/lib/api.js`
- V2 chat: `v2/js/pages/chat/`
- V2 settings: `v2/js/pages/settings/`

## Lightweight Validation Commands

Run only the checks that match the files changed:

```bash
node --check crates/ironclaw_gateway_reborn/static/js/path/to/changed-file.js
git diff --check
```

Optional if a gateway is already running:

```bash
open http://localhost:3000/v2
```
