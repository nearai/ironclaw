# Web Gateway Module

Browser-facing HTTP API and SSE/WebSocket real-time streaming. Axum-based, single-user with bearer token auth.

## File Map

| File | Role |
|------|------|
| `mod.rs` | Gateway builder, startup, `WebChannel` implementation, `with_*` builder methods |
| `server.rs` | `GatewayState`, `start_server()`, all Axum route registrations, inline handlers |
| `types.rs` | Request/response DTOs and `SseEvent` enum (source of truth for SSE contract) |
| `sse.rs` | `SseManager` — broadcast channel that fans out `SseEvent` to all connected SSE clients |
| `ws.rs` | WebSocket handler (`handle_ws_connection`) + `WsConnectionTracker` |
| `auth.rs` | Bearer token middleware (`Authorization: Bearer <GATEWAY_AUTH_TOKEN>`) |
| `log_layer.rs` | Tracing layer that tees log lines to the `/api/logs/events` SSE stream |
| `handlers/` | Handler functions split by domain: `chat`, `extensions`, `jobs`, `memory`, `routines`, `settings`, `skills`, `static_files` |
| `openai_compat.rs` | OpenAI-compatible proxy (`/v1/chat/completions`, `/v1/models`) |
| `util.rs` | Shared helpers (`build_turns_from_db_messages`, `truncate_preview`) |
| `static/` | Single-page app (HTML/CSS/JS) — embedded at compile time via `include_str!`/`include_bytes!` |

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
| POST | `/api/chat/auth-token` | Legacy auth shim; translates engine v2 auth gates or configures extensions directly |
| POST | `/api/chat/auth-cancel` | Legacy auth-cancel shim |

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
| `auth_required` | Legacy extension/auth event |
| `auth_completed` | Extension auth flow finished |
| `extension_status` | WASM channel activation status changed |
| `error` | Error from agent or gateway |
| `heartbeat` | SSE keepalive (empty payload) |

**SSE serialization:** Events use `#[serde(tag = "type")]` — the wire format is `{"type":"<variant>", ...fields}`. The SSE frame's `event:` field is set to the same string as `type` for easy `addEventListener` use in the browser.

**WebSocket envelope:** Over WebSocket, SSE events are wrapped as `{"type":"event","event_type":"<variant>","data":{...}}`. Ping/pong uses `{"type":"ping"}` / `{"type":"pong"}`. Client-to-server messages (`message`, `approval`, `auth_token`, `auth_cancel`) are defined in `WsClientMessage` in `types.rs`.

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

- **Broadcast buffer:** 256 events. A slow client that falls behind will miss events — the `BroadcastStream` silently drops lagged events. SSE clients are expected to reconnect and re-fetch history.
- **Max connections:** 100 total (SSE + WebSocket combined). Connections beyond the limit receive a 503 / are immediately dropped.
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

## Adding a New API Endpoint

1. Define request/response types in `types.rs`.
2. Implement the handler in the appropriate `handlers/*.rs` file (or inline in `server.rs` for simple handlers).
3. Register the route in `start_server()` in `server.rs` under the correct router (`public`, `protected`, or `statics`).
4. If it is an SSE or WebSocket endpoint, add its path to `allows_query_token_auth()` in `auth.rs`.
5. If it requires a new `GatewayState` field, add it to the struct and to both the `GatewayChannel::new()` initializer and `rebuild_state()` in `mod.rs`, then add a `with_*` builder method.
