# Web Gateway Module

Browser-facing HTTP API and SSE/WebSocket real-time streaming. Axum-based, single-user with bearer token auth.

## File Map

| File | Role |
|------|------|
| `mod.rs` | Gateway builder, startup, `WebChannel` implementation, `with_*` builder methods |
| `server.rs` | **Backward-compat shim only** — re-exports `GatewayState` / `start_server` / related types from `platform::*` so external callers (`src/main.rs`, `src/app.rs`, integration tests) keep resolving `crate::channels::web::server::*`. All feature handlers have migrated to `features/<slice>/`. Stage 6 deletes this file entirely once those callers flip to `platform::*`. |
| `platform/router.rs` | `start_server()` + Axum route composition (public / protected / statics / projects) and the cross-cutting layer stack (CORS, body limit, panic catch, static security headers, CSP). Single coupling point between platform and features. |
| `platform/state.rs` | `GatewayState`, `RateLimiter`, `PerUserRateLimiter`, `WorkspacePool`, `FrontendHtmlCache`, `FrontendCacheKey`, `ActiveConfigSnapshot`, `PromptQueue`, `RoutineEngineSlot`. Canonical home for shared gateway state. |
| `platform/static_files.rs` | CSP directive set + `BASE_CSP_HEADER` (single source of truth), the workspace-backed layout/widget readers (`read_layout_config`, `load_resolved_widgets`, `read_widget_manifest`, `LAYOUT_PATH`, `WIDGETS_DIR`, `MAX_WIDGET_*`), frontend HTML bundle assembly (`build_frontend_html`), and the unauthenticated static handlers: `/`, `/style.css`, `/app.js`, `/theme.css`, `/favicon.ico`, `/i18n/*`, `/admin*`, `/api/health`, plus the authenticated `/projects/{id}/...` file-serving routes. |
| `types.rs` | Request/response DTOs and `SseEvent` enum (source of truth for SSE contract) |
| `platform/sse.rs` | `SseManager` — broadcast channel that fans out `SseEvent` to all connected SSE clients. Re-exported as `channels::web::sse` for backward compat. |
| `platform/ws.rs` | WebSocket handler (`handle_ws_connection`) + `WsConnectionTracker`. Re-exported as `channels::web::ws`. |
| `platform/auth.rs` | Bearer token middleware (`Authorization: Bearer <GATEWAY_AUTH_TOKEN>`) + DB-token + OIDC extractors. Re-exported as `channels::web::auth`. |
| `platform/legacy_auth.rs` | Temporary v1 thread-level auth-mode shim: `handle_legacy_auth_token_submission`, `handle_legacy_auth_cancel`, `clear_auth_mode`, `clear_auth_mode_for_thread`. Both the `server.rs` HTTP handlers and `platform/ws.rs` consume these; co-locating them under `platform/` is what lets both reach the implementation without a feature → server.rs back-edge. Delete alongside `/api/chat/auth-token` and `/api/chat/auth-cancel` once the gateway retires the no-`request_id` path. |
| `platform/engine_dispatch.rs` | Shared engine-channel dispatch wrappers: `dispatch_engine_submission`, `dispatch_engine_external_callback`, `dispatch_onboarding_ready_followup`. Lives in platform because `features/chat/`, `features/extensions/`, and `features/pairing/` all compose them. |
| `log_layer.rs` | Tracing layer that tees log lines to the `/api/logs/events` SSE stream |
| `features/extensions/` | Nine extension lifecycle routes — `/api/extensions`, `/api/extensions/readiness`, `/api/extensions/tools`, `/api/extensions/install`, `/api/extensions/{name}/activate`, `/api/extensions/{name}/remove`, `/api/extensions/registry`, `/api/extensions/{name}/setup` (GET+POST). Every handler that takes `{name}` from the URL path validates via `ExtensionName::new` at the boundary (400 on path-traversal / invalid chars / oversized). Owns the `derive_activation_status`, `derive_onboarding`, `extension_phase_for_web`, and `apply_extension_readiness_to_response` helpers. Routes setup-submit through the `AuthManager` canonical resolver + `platform::engine_dispatch`. Migrated from `server.rs` in ironclaw#2599 stage 4d. |
| `features/jobs/` | Nine sandbox-job routes — `/api/jobs`, `/api/jobs/summary`, `/api/jobs/{id}`, `/api/jobs/{id}/cancel`, `/api/jobs/{id}/restart`, `/api/jobs/{id}/prompt`, `/api/jobs/{id}/events`, `/api/jobs/{id}/files/list`, `/api/jobs/{id}/files/read`. Migrated from `handlers/jobs.rs` in ironclaw#2599 stage 5. |
| `features/routines/` | Seven routine management routes — `/api/routines`, `/api/routines/summary`, `/api/routines/{id}` (GET+DELETE), `/api/routines/{id}/trigger`, `/api/routines/{id}/toggle`, `/api/routines/{id}/runs`. Merges the previously split `handlers/routines.rs` + `server.rs::routines_runs_handler` into one slice; the two copies of `routines_runs_handler` are now one canonical definition. Migrated in ironclaw#2599 stage 5. |
| `features/settings/` | Eight settings routes — `/api/settings`, `/api/settings/export`, `/api/settings/import`, `/api/settings/{key}` (GET/PUT/DELETE), plus the `/api/admin/tool-policy` dependencies via `resolve_settings_store` (now `pub(crate)` for `handlers/tool_policy.rs`). Migrated from `handlers/settings.rs` in ironclaw#2599 stage 5. |
| `features/chat/` | Ten chat routes end-to-end — `/api/chat/send`, `/api/chat/approval`, `/api/chat/gate/resolve`, `/api/chat/auth-token` (legacy v1 shim), `/api/chat/auth-cancel` (legacy v1 shim), `/api/chat/ws`, `/api/chat/events`, `/api/chat/history`, `/api/chat/threads`, `/api/chat/thread/new`. Owns chat-private helpers: `is_local_origin` (CSRF-gate for WS), `pending_gate_extension_name` (routes through the canonical `AuthManager::resolve_extension_name_for_auth_flow`), in-progress reconciliation (`reconcile_in_progress_with_turns` + satellites), `turn_info_from_in_memory_turn`, `thread_state_label` / `turn_state_label`, `summary_live_state`, and the `ChatEventsQuery` / `HistoryQuery` request DTOs. Absorbed the four live handler duplicates formerly in `handlers/chat.rs`, which has been deleted. Migrated from `server.rs` in ironclaw#2599 stage 4c. |
| `features/logs/` | `GET /api/logs/events` + `GET/PUT /api/logs/level` — runtime log stream and log-level knob. Migrated from `server.rs` in ironclaw#2599 stage 4b. |
| `features/oauth/` | First feature slice landed per ironclaw#2599 stage 4a: OAuth callback (`/oauth/callback`), channel-relay event webhook (`/relay/events`), and the Slack-specific relay OAuth completion flow (`/oauth/slack/callback`). Owns its private helpers (`oauth_error_page`, `redact_oauth_state_for_logs`). |
| `features/pairing/` | `GET /api/pairing/{channel}` + `POST /api/pairing/{channel}/approve` — WASM channel pairing approvals. Validates the URL path through `ExtensionName::new` at the handler boundary so invalid channel names reject with 400 instead of silently routing to a lookup-miss. Migrated from `server.rs` in ironclaw#2599 stage 4b. |
| `features/status/` | `GET /api/gateway/status` — runtime snapshot for the admin dashboard (uptime, SSE/WS counts, cost / usage aggregates, active config). Owns the `GatewayStatusResponse` DTO. Migrated from `server.rs` in ironclaw#2599 stage 4b. |
| `handlers/` | Transitional feature handlers that haven't migrated to `features/<slice>/` yet: `auth`, `engine`, `frontend`, `llm`, `memory`, `secrets`, `skills`, `system_prompt`, `tokens`, `tool_policy`, `users`, `webhooks`. Targeted for migration per ironclaw#2599 if churn / slice-boundary pressure justifies it. |
| `openai_compat.rs` | OpenAI-compatible proxy (`/v1/chat/completions`, `/v1/models`) |
| `util.rs` | Shared helpers (`web_incoming_message`, `build_turns_from_db_messages`, `images_to_attachments`, `truncate_preview`) |
| `static/` | Single-page app (HTML/CSS/JS) — embedded at compile time via `include_str!`/`include_bytes!` |

## Platform vs. feature layering (ironclaw#2599)

The target layout is a `platform/` subtree (router, state, auth, SSE,
WS, static serving) that feature handlers depend on.

**The "no back-edges" rule has one intentional exception: the router.**
Route composition is inherently the coupling point where transport
meets features — `platform/router.rs` imports every feature handler it
registers. Every *other* platform submodule (state, static_files,
auth, sse, ws) must stay handler-agnostic, and
`scripts/check_gateway_boundaries.py` (wired into the `code_style`
CI workflow as of ironclaw#2599 stage 5) enforces this: it fails the
build on any added import from `platform/*` (except `router.rs`) into
`handlers/*`, `features/*`, or the `server.rs` compatibility shim. The
allowlist is empty as of stage 4b — every pre-existing back-edge has
been migrated into `platform/` proper. The mechanism stays in place
for narrowly-scoped, reviewer-approved exceptions if a future
migration step needs one.

The flat `handlers/` folder is a transitional fallback — individual
handlers will migrate into `features/<slice>/` directories once their
platform dependencies are narrowed to a per-slice `Deps` view. When
adding a new platform-level concern, put it under `platform/`; when
adding a new feature handler, keep it under `handlers/` for now but
design it so the surface it consumes from `GatewayState` is a narrow
subset that can later be replaced by a typed `Deps` alias.

## API Routes

### Public (no auth)
| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/health` | Health check |
| GET | `/oauth/callback` | OAuth callback for extension auth |

### Chat
| Method | Path | Description |
|--------|------|-------------|
| POST | `/api/chat/send` | Send message → queues to agent loop |
| GET | `/api/chat/events` | SSE stream of agent events |
| GET | `/api/chat/ws` | WebSocket alternative to SSE |
| GET | `/api/chat/history` | Paginated turn history for a thread |
| GET | `/api/chat/threads` | List threads (returns `assistant_thread` + regular threads) |
| POST | `/api/chat/thread/new` | Create new thread |
| POST | `/api/chat/gate/resolve` | Resolve a pending engine v2 gate (approve, deny, credential, cancel) |
| POST | `/api/chat/approval` | Legacy approval shim; translates to unified gate resolution |
| POST | `/api/chat/auth-token` | Temporary legacy auth-mode shim for prompts without gate `request_id` |
| POST | `/api/chat/auth-cancel` | Temporary legacy auth-mode cancel shim for prompts without gate `request_id` |

### Memory
| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/memory/tree` | Workspace directory tree |
| GET | `/api/memory/list` | List files at a path |
| GET | `/api/memory/read` | Read a workspace file |
| POST | `/api/memory/write` | Write a workspace file |
| POST | `/api/memory/search` | Hybrid FTS + vector search |

### Jobs (sandbox)
| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/jobs` | List sandbox jobs |
| GET | `/api/jobs/summary` | Aggregated stats |
| GET | `/api/jobs/{id}` | Job detail |
| POST | `/api/jobs/{id}/cancel` | Cancel a running job |
| POST | `/api/jobs/{id}/restart` | Restart a failed job |
| POST | `/api/jobs/{id}/prompt` | Send follow-up prompt to Claude Code bridge |
| GET | `/api/jobs/{id}/events` | SSE stream for a specific job |
| GET | `/api/jobs/{id}/files/list` | List files in job workspace |
| GET | `/api/jobs/{id}/files/read` | Read a file from job workspace |

### Skills
| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/skills` | List installed skills |
| POST | `/api/skills/search` | Search ClawHub registry + local skills |
| POST | `/api/skills/install` | Install a skill from ClawHub or by URL/content |
| DELETE | `/api/skills/{name}` | Remove an installed skill |

### Extensions
| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/extensions` | Installed extensions |
| GET | `/api/extensions/tools` | All registered tools (from tool registry) |
| POST | `/api/extensions/install` | Install extension |
| GET | `/api/extensions/registry` | Available extensions from registry manifests |
| POST | `/api/extensions/{name}/activate` | Activate installed extension |
| POST | `/api/extensions/{name}/remove` | Remove extension |
| GET/POST | `/api/extensions/{name}/setup` | Extension setup wizard |

Extension lifecycle note:
- Web install, activate, and OAuth callback flows should route through `ExtensionManager::ensure_extension_ready(...)` rather than sequencing `auth()` and `activate()` independently in handlers.
- Preserve the existing `ActionResponse` wire shape, but derive it from `EnsureReadyOutcome` so browser UX stays stable while lifecycle control remains kernel-owned.

## Unified Extension Onboarding

The browser must have one canonical onboarding path for installable extensions and channels.

Canonical states:

- `setup_required`
- `auth_required`
- `pairing_required`
- `ready`
- `failed`

Identity invariant:

- `credential_name` is backend-only and may be a raw secret key like `telegram_bot_token`.
- `extension_name` is the browser/setup identity and must be the installed extension/channel name like `telegram`.

Do not mix them.

Rules:

- Chat and Settings must both route installable extension/channel auth into `/api/extensions/{name}/setup`.
- `gate_required`, `HistoryResponse.pending_gate`, and `onboarding_state` must all carry enough normalized data for the frontend to render the same onboarding flow.
- Frontend code must not infer setup routing from `resume_kind.Authentication.credential_name` when an `extension_name` is available or recoverable via the shared backend resolver.
- Generic auth cards are only for non-extension credential prompts or OAuth-only flows that do not have extension setup UI.
- If an auth-related change adds a new identity derivation path, stop and consolidate it into the shared backend resolver instead.

Identity types at the web boundary:

These rules are enforced by check #8 in `scripts/pre-commit-safety.sh`
(`CREDNAME`). Suppress individual intentional uses with
`// web-identity-exempt: <reason>`.

- **Setup / configure / activate routes take `ExtensionName`, not `String`.**
  Any handler on `/api/extensions/{name}/...` whose path segment is the
  extension identity MUST parse it at entry via
  `ExtensionName::new(&name).map_err(|e| (StatusCode::BAD_REQUEST, ...))?`
  before the value reaches extension lookup, SSE broadcast, or any
  `from_trusted` wrap. A path-traversal or malformed slug must return 400.

- **Web request/response DTOs and web handlers must not reference
  `CredentialName`.** Credential identity is a backend concern. The web
  layer accepts and emits `ExtensionName`; the dispatcher / auth manager
  resolves credential identity from it server-side. If you find yourself
  importing `CredentialName` in `src/channels/web/**`, you're on the
  wrong side of the boundary — push the resolution into
  `bridge::auth_manager` and have the handler consume its output.

- **Auth-flow extension resolution happens in one place.** The only
  supported way to map an auth gate → extension name is
  `AuthManager::resolve_extension_name_for_auth_flow`. Web handlers,
  TUI channels, relay adapters, and SSE broadcasters must call through
  it rather than re-deriving an extension name from `pending.action_name`,
  a credential-name prefix, or a format-string. Four recent identity
  bugs (#2561, #2473, #2512, #2574) were duplicate-resolution drift —
  this rule exists to make a fifth impossible.

Current consolidation points:

- `src/bridge/auth_manager.rs`: `resolve_extension_name_for_auth_flow(...)` — **canonical resolver, single source of truth**
- `src/bridge/router.rs`: `resolve_auth_gate_extension_name(...)` — thin wrapper for gate display/submit
- `src/channels/web/server.rs`: `pending_gate_extension_name(...)` — thin wrapper for history/pending-gate hydration
- `crates/ironclaw_gateway/static/app.js`: `handleOnboardingState(...)` as the canonical client entrypoint

All three of the backend wrappers above delegate to the canonical resolver
or return `Option<ExtensionName>`; they must not duplicate its logic.

Legacy cleanup note:

- The only remaining browser compatibility path for engine v1 auth mode is `pending_auth` token submit/cancel through `/api/chat/auth-token` and `/api/chat/auth-cancel`.
- That path exists solely for prompts that do not carry a gate `request_id`.
- Do not expand it. When v1 auth mode is removed, delete these endpoints and the corresponding no-`request_id` branch in `static/app.js`.

### Routines
| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/routines` | List routines |
| GET | `/api/routines/summary` | Aggregated stats (total/enabled/disabled/unverified/failing/runs_today) |
| GET | `/api/routines/{id}` | Routine detail with recent run history |
| POST | `/api/routines/{id}/trigger` | Manually trigger a routine |
| POST | `/api/routines/{id}/toggle` | Enable/disable a routine |
| DELETE | `/api/routines/{id}` | Delete a routine |
| GET | `/api/routines/{id}/runs` | List runs for a specific routine |

### User Management (admin — requires `admin` role, see `docs/USER_MANAGEMENT_API.md`)
| Method | Path | Description |
|--------|------|-------------|
| POST | `/api/admin/users` | Create a new user (returns one-time token) |
| GET | `/api/admin/users` | List all users |
| GET | `/api/admin/users/{id}` | Get a single user |
| PATCH | `/api/admin/users/{id}` | Update user profile/metadata |
| DELETE | `/api/admin/users/{id}` | Delete user and all data |
| POST | `/api/admin/users/{id}/suspend` | Suspend a user |
| POST | `/api/admin/users/{id}/activate` | Re-activate a user |
| GET | `/api/admin/usage` | Per-user LLM usage stats |
| GET | `/api/admin/usage/summary` | System-wide usage summary for the admin dashboard |
| GET | `/api/admin/users/{user_id}/secrets` | List a user's secrets (names only) |
| PUT | `/api/admin/users/{user_id}/secrets/{name}` | Create or update a user's secret |
| DELETE | `/api/admin/users/{user_id}/secrets/{name}` | Delete a user's secret |

### Profile (self-service)
| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/profile` | Get own profile |
| PATCH | `/api/profile` | Update own display name/metadata |

### Tokens (self-service)
| Method | Path | Description |
|--------|------|-------------|
| POST | `/api/tokens` | Create API token (returns plaintext once) |
| GET | `/api/tokens` | List own tokens |
| DELETE | `/api/tokens/{id}` | Revoke a token |

### Settings
| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/settings` | List all settings |
| GET | `/api/settings/export` | Export all settings as a map |
| POST | `/api/settings/import` | Bulk-import settings from a map |
| GET | `/api/settings/{key}` | Get a single setting |
| PUT | `/api/settings/{key}` | Set a single setting |
| DELETE | `/api/settings/{key}` | Delete a setting |

### Other
| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/logs/events` | Live log stream (SSE) |
| GET/PUT | `/api/logs/level` | Get/set log level at runtime |
| GET | `/api/pairing/{channel}` | Admin-only list of pending pairing requests |
| POST | `/api/pairing/{channel}/approve` | Authenticated user self-claims a pairing code |
| GET | `/api/gateway/status` | Server uptime, connected clients, config |
| POST | `/v1/chat/completions` | OpenAI-compatible LLM proxy |
| GET | `/v1/models` | OpenAI-compatible model list |

### Static / Project files
| Method | Path | Description |
|--------|------|-------------|
| GET | `/` | Single-page app HTML |
| GET | `/theme.css` | Shared theme tokens for the web and admin SPAs |
| GET | `/style.css` | App stylesheet |
| GET | `/app.js` | App JavaScript |
| GET | `/favicon.ico` | Favicon (cached 1 day) |
| GET | `/projects/{project_id}/` | Job workspace browser (redirects) |
| GET | `/projects/{project_id}/{*path}` | Serve file from job workspace (auth required) |

## SSE Event Types (`SseEvent` in `types.rs`)

The SSE contract — every field is `#[serde(tag = "type")]`:

| Type | When emitted |
|------|-------------|
| `response` | Final text response from agent |
| `stream_chunk` | Streaming token (partial response) |
| `thinking` | Agent status update during reasoning |
| `tool_started` | Tool call began |
| `tool_completed` | Tool call finished (includes success/error) |
| `tool_result` | Tool output preview |
| `status` | Generic status message |
| `job_started` | Sandbox job created |
| `job_message` | Message from sandbox worker |
| `job_tool_use` | Tool invoked inside sandbox |
| `job_tool_result` | Tool result from sandbox |
| `job_status` | Sandbox job status update |
| `job_result` | Sandbox job final result |
| `gate_required` | Engine v2 gate requires user input (approval/auth/external) |
| `gate_resolved` | Engine v2 gate was resolved |
| `approval_needed` | Legacy approval event |
| `onboarding_state` | Unified extension/channel onboarding state update (`setup_required`, `auth_required`, `pairing_required`, `ready`, `failed`) |
| `extension_status` | WASM channel activation status changed |
| `error` | Error from agent or gateway |
| `heartbeat` | SSE keepalive (empty payload) |

**SSE serialization:** Events use `#[serde(tag = "type")]` — the wire format is `{"type":"<variant>", ...fields}`. The SSE frame's `event:` field is set to the same string as `type` for easy `addEventListener` use in the browser.

**Compatibility note:** `onboarding_state` intentionally replaces the older `auth_required`, `auth_completed`, `pairing_required`, and `pairing_completed` SSE event types. Non-bundled SSE consumers must migrate to `onboarding_state`; the gateway still accepts legacy WebSocket client messages `auth_token` and `auth_cancel` as temporary aliases during the browser v1-auth compatibility window.

**SSE event IDs / reconnect:** Chat SSE frames now also include an `id:` field in the form `<boot_uuid>:<counter>`. Browser reconnects can supply the last seen ID either via the standard `Last-Event-ID` header or the `last_event_id` query parameter (used by the web UI because `EventSource` reconnect state is recreated in JavaScript). IDs are process-scoped: after a server restart, old IDs are ignored and the client rebuilds thread history from `/api/chat/history`. **Note:** Event IDs are only available on the SSE `subscribe()` path. `subscribe_raw()` (used by WebSocket and the Responses API) returns `AppEvent` without IDs — WebSocket clients rely on their own reconnect semantics rather than event-ID dedup.

**WebSocket envelope:** Over WebSocket, SSE events are wrapped as `{"type":"event","event_type":"<variant>","data":{...}}`. Ping/pong uses `{"type":"ping"}` / `{"type":"pong"}`. Client-to-server messages (`message`, `approval`) are defined in `WsClientMessage` in `types.rs`.

**To add a new SSE event:** Use the `add-sse-event` skill (`/add-sse-event`). It scaffolds the Rust variant, serialization, broadcast call, and frontend handler. Also add a matching arm to `WsServerMessage::from_sse_event()` in `types.rs`.

## Auth

All protected routes require `Authorization: Bearer <GATEWAY_AUTH_TOKEN>`. The token is set via `GATEWAY_AUTH_TOKEN` env var. Missing/wrong token → 401. The `Bearer` prefix is compared case-insensitively (RFC 6750).

**Query-string token auth (`?token=xxx`):** Because `EventSource` and WebSocket upgrades cannot set custom headers from the browser, three endpoints also accept the token as a URL query parameter: `/api/chat/events`, `/api/logs/events`, and `/api/chat/ws`. All other endpoints reject query-string tokens. If you add a new SSE or WebSocket endpoint, register its path in `allows_query_token_auth()` in `auth.rs`.

**If no `GATEWAY_AUTH_TOKEN` is configured**, a random 32-character alphanumeric token is generated at startup and printed to the console.

Rate limiting: chat send endpoints are capped at **30 messages per 60 seconds** (sliding window, not per-IP).

## GatewayState

The shared state struct (`server.rs`) holds refs to all subsystems. Fields are `Option<Arc<T>>` so the gateway can start even when optional subsystems (workspace, sandbox, skills) are disabled. Always null-check before use in handlers.

Key fields:
- `msg_tx` — `RwLock<Option<mpsc::Sender<IncomingMessage>>>` — sends messages to the agent loop; set when `start()` is called on the `Channel`.
- `sse` — `SseManager` — broadcast hub; call `state.sse.broadcast(event)` from any handler.
- `ws_tracker` — `Option<Arc<WsConnectionTracker>>` — tracks WS connection count separately from SSE.
- `chat_rate_limiter` — `RateLimiter` — 30 req/60 s sliding window shared across all chat send callers.
- `scheduler` — `Option<SchedulerSlot>` — used to inject follow-up messages into running agent jobs.
- `cost_guard` — `Option<Arc<CostGuard>>` — exposes token usage / cost totals in the status endpoint.
- `startup_time` — `Instant` — used to compute uptime in the gateway status response.
- `registry_entries` — `Vec<RegistryEntry>` — loaded once at startup from registry manifests; used by the available extensions API without hitting the network.

Subsystems are wired via `with_*` builder methods on `GatewayChannel` (`mod.rs`). Each call rebuilds `Arc<GatewayState>` — safe to call before `start()`, not after.

## SSE / WebSocket Connection Limits

Both SSE and WebSocket share the same `SseManager` broadcast channel. Key characteristics:

- **Broadcast buffer:** `SSE_BROADCAST_BUFFER` env var (default `1024`, clamped to 65,536 max). A slow client that falls behind will miss events — the `BroadcastStream` silently drops lagged events. SSE clients are expected to reconnect and re-fetch history.
- **Max connections:** `GATEWAY_MAX_CONNECTIONS` (default `100`) total across SSE + WebSocket. Connections beyond the limit receive a 503 / are immediately dropped.
- **SSE keepalive:** Axum's `KeepAlive` sends an empty event every **30 seconds** to prevent proxy timeouts.
- **WebSocket:** Two tasks per connection — a sender task (broadcast → WS frames) and a receiver loop (WS frames → agent). When the client disconnects, the sender is aborted and both the SSE connection counter and WS tracker counter are decremented.

## CORS and Security Headers

CORS is restricted to the gateway's own origin (same IP+port and `localhost`+port). Allowed methods: GET, POST, PUT, DELETE. Allowed headers: `Content-Type`, `Authorization`. Credentials are allowed.

All responses include:
- `X-Content-Type-Options: nosniff`
- `X-Frame-Options: DENY`

**Request body limit:** 10 MB (`DefaultBodyLimit::max(10 * 1024 * 1024)`), sized for image uploads (#725). Larger payloads return 413.

## Pending Gates

Classic agent approvals are in-memory, but engine v2 pauses live in the unified pending-gate store with file-backed recovery under `~/.ironclaw/pending-gates.json`. `HistoryResponse.pending_gate` rehydrates from that store so cards survive thread switches, SSE reconnects, and process restarts. Gate UI must remain thread-scoped: stale cards from another thread should not be rendered or resolved in the current thread.

The chat history contract also carries a lightweight `HistoryResponse.in_progress` payload for durable in-flight turn state. Use it to rebuild the visible user message plus "Processing..." affordance after refresh or thread switches. Do not persist transient SSE-only thinking text as normal conversation messages.

## Adding a New API Endpoint

1. Define request/response types in `types.rs`.
2. Implement the handler in the appropriate `handlers/*.rs` file (or inline in `server.rs` for simple handlers).
3. Register the route in `start_server()` in `server.rs` under the correct router (`public`, `protected`, or `statics`).
4. If it is an SSE or WebSocket endpoint, add its path to `allows_query_token_auth()` in `auth.rs`.
5. If it requires a new `GatewayState` field, add it to the struct and to both the `GatewayChannel::new()` initializer and `rebuild_state()` in `mod.rs`, then add a `with_*` builder method.
