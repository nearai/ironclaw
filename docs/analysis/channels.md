# IronClaw Codebase Analysis — Channel System

> Updated: 2026-02-22 | Version: v0.9.0

---

## 1. Overview

IronClaw's channel system is the boundary layer between the outside world and the
agent loop. Every message the agent receives — whether typed interactively, posted
by a webhook, sent from a browser, or emitted by a WASM plugin — arrives through a
channel. The system is deliberately uniform: all channels produce a single merged
`MessageStream` that the agent loop consumes without knowing which transport
delivered any given message.

The module tree lives at `src/channels/`:

```
src/channels/
├── mod.rs           — public re-exports, module-level architecture diagram
├── channel.rs       — Channel trait, IncomingMessage, OutgoingResponse, StatusUpdate
├── manager.rs       — ChannelManager: start/merge/respond/shutdown
├── repl.rs          — Interactive REPL (rustyline + termimad)
├── http.rs          — HTTP webhook inbound endpoint (axum)
├── webhook_server.rs — Unified axum server that composes route fragments
├── web/             — Web gateway (browser UI, REST, SSE, WebSocket)
│   ├── mod.rs       — GatewayChannel Channel impl
│   ├── server.rs    — Router construction, GatewayState, RateLimiter
│   ├── auth.rs      — Bearer token middleware (constant-time)
│   ├── sse.rs       — SSE broadcast manager, CountedStream
│   ├── ws.rs        — WebSocket handler, WsConnectionTracker
│   ├── types.rs     — All request/response DTOs, SseEvent enum, WsClientMessage
│   ├── log_layer.rs — Tracing layer → SSE log streaming
│   ├── openai_compat.rs — /v1/chat/completions proxy
│   └── handlers/    — Per-resource handler modules (chat, jobs, memory, …)
└── wasm/            — WASM channel plugin runtime
    ├── mod.rs       — Public API re-exports
    ├── runtime.rs   — Wasmtime engine, compiled module cache
    └── host.rs      — ChannelHostState: emit_message, workspace writes, rate limiting
```

The overall data flow:

```
┌─────────────────────────────────────────────────────────────┐
│                        ChannelManager                        │
│                                                              │
│  ReplChannel  HttpChannel  GatewayChannel  WasmChannel ...  │
│       │            │             │              │            │
│       └────────────┴─────────────┴──────────────┘           │
│                          │                                   │
│                  select_all (futures)                        │
│                          │                                   │
│                    MessageStream ──► Agent Loop              │
└─────────────────────────────────────────────────────────────┘
```

---

## 2. The Channel Trait (`channel.rs`)

Every input source implements the `Channel` trait defined in
`src/channels/channel.rs`. The trait is async (via `async_trait`) and requires
`Send + Sync`, making channels safe to hand to the Tokio scheduler.

```rust
#[async_trait]
pub trait Channel: Send + Sync {
    fn name(&self) -> &str;
    async fn start(&self) -> Result<MessageStream, ChannelError>;
    async fn respond(&self, msg: &IncomingMessage, response: OutgoingResponse)
        -> Result<(), ChannelError>;
    async fn send_status(&self, _status: StatusUpdate, _metadata: &serde_json::Value)
        -> Result<(), ChannelError> { Ok(()) }
    async fn broadcast(&self, _user_id: &str, _response: OutgoingResponse)
        -> Result<(), ChannelError> { Ok(()) }
    async fn health_check(&self) -> Result<(), ChannelError>;
    async fn shutdown(&self) -> Result<(), ChannelError> { Ok(()) }
}
```

### Key Types

**`IncomingMessage`** — the normalized unit of work flowing into the agent:

| Field | Type | Description |
|-------|------|-------------|
| `id` | `Uuid` | Unique per-message ID (v4) |
| `channel` | `String` | Source channel name (e.g. `"repl"`, `"gateway"`) |
| `user_id` | `String` | Caller identity within the channel |
| `user_name` | `Option<String>` | Optional display name |
| `content` | `String` | The message text or command |
| `thread_id` | `Option<String>` | Conversation thread grouping |
| `received_at` | `DateTime<Utc>` | Wall-clock receipt time |
| `metadata` | `serde_json::Value` | Channel-specific routing metadata |

**`OutgoingResponse`** — what the agent sends back:

```rust
pub struct OutgoingResponse {
    pub content: String,
    pub thread_id: Option<String>,
    pub metadata: serde_json::Value,
}
```

**`StatusUpdate`** — real-time feedback while the agent is working:

```rust
pub enum StatusUpdate {
    Thinking(String),
    ToolStarted { name: String },
    ToolCompleted { name: String, success: bool },
    ToolResult { name: String, preview: String },
    StreamChunk(String),
    Status(String),
    JobStarted { job_id: String, title: String, browse_url: String },
    ApprovalNeeded { request_id: String, tool_name: String,
                     description: String, parameters: serde_json::Value },
    AuthRequired { extension_name: String, instructions: Option<String>,
                   auth_url: Option<String>, setup_url: Option<String> },
    AuthCompleted { extension_name: String, success: bool, message: String },
}
```

`StatusUpdate` variants are forwarded to the channel so it can render progress
inline. For the REPL this means ANSI terminal output. For the web gateway this
means broadcasting an SSE event. Channels that do not support live feedback
return `Ok(())` from the default implementations of `send_status` and
`broadcast`.

### Lifecycle

1. `start()` — the channel binds its transport and returns a `MessageStream`. The
   stream is `Pin<Box<dyn Stream<Item = IncomingMessage> + Send>>`. The channel
   owns the stream for its lifetime.
2. Messages arrive from the transport, are normalized to `IncomingMessage`, and
   pushed onto the stream.
3. When the agent has a response, it calls `respond()` on the originating channel.
4. Progress updates are pushed via `send_status()`.
5. Proactive notifications (heartbeat, alerts) use `broadcast()`.
6. On process shutdown, `shutdown()` is called to drain queues and close
   connections cleanly.

---

## 3. Channel Manager (`manager.rs`)

`ChannelManager` lives at `src/channels/manager.rs` and is the single coordinator
for all registered channels.

### Startup

```rust
pub async fn start_all(&self) -> Result<MessageStream, ChannelError>
```

`start_all()` iterates every registered channel, calls `channel.start().await`,
and collects the resulting streams. A channel failure is logged but does not abort
startup — the remaining channels continue. After starting channels, the injection
stream is merged in:

```rust
// Internal mpsc channel — lets background tasks push messages without
// being a full Channel implementation.
if let Some(inject_rx) = self.inject_rx.lock().await.take() {
    let inject_stream = tokio_stream::wrappers::ReceiverStream::new(inject_rx);
    streams.push(Box::pin(inject_stream));
}
let merged = stream::select_all(streams);
```

`futures::stream::select_all` multiplexes all streams into a single `MessageStream`
with fair round-robin polling. The injection receiver is `take()`n exactly once —
calling `start_all()` a second time would not re-attach the injector.

If zero channels start successfully, `start_all` returns
`Err(ChannelError::StartupFailed)`, which terminates the process.

### Injection Channel

```rust
pub fn inject_sender(&self) -> mpsc::Sender<IncomingMessage>
```

Background tasks (job monitors, the heartbeat system) that need to trigger the
agent loop without being a full channel implementation use
`manager.inject_sender()` to get a cloned `mpsc::Sender`. Messages sent on this
sender appear on the merged `MessageStream` exactly like channel messages. The
injection buffer holds up to 64 messages.

### Routing Responses

```rust
pub async fn respond(&self, msg: &IncomingMessage, response: OutgoingResponse)
    -> Result<(), ChannelError>
```

The `IncomingMessage.channel` field carries the source channel name. `respond()`
looks up the channel by name in the `HashMap<String, Box<dyn Channel>>` and
delegates. This works because the `channels` map is `Arc<RwLock<...>>`: reads are
concurrent, writes are exclusive and only happen during setup.

### Shutdown

```rust
pub async fn shutdown_all(&self) -> Result<(), ChannelError>
```

`shutdown_all()` iterates all channels, calls `channel.shutdown().await` on each,
and logs but does not propagate individual shutdown errors. This ensures all
channels get a chance to drain regardless of peer failures.

---

## 4. REPL Channel (`repl.rs`)

`ReplChannel` (`src/channels/repl.rs`) provides an interactive terminal interface
using **rustyline** for line editing and **termimad** for inline markdown
rendering.

### Features

- Persistent readline history at `~/.ironclaw/history`
- Tab completion for slash commands via `ReplHelper` (implements `Completer`)
- Inline command hints (grey suffix while typing)
- Markdown response rendering with a custom color skin (yellow headers, green
  code blocks, magenta italics)
- Debug mode toggle (`/debug`) that increases status verbosity
- Approval prompt rendering with a Unicode box-drawing card

### Slash Commands

| Command | Handled by | Description |
|---------|-----------|-------------|
| `/help` | REPL (local) | Print help, do not forward to agent |
| `/debug` | REPL (local) | Toggle verbose output |
| `/quit`, `/exit` | REPL (local) + agent | Forward `/quit` so agent loop shuts down |
| `/undo`, `/redo`, `/clear`, `/compact` | Agent | Forwarded as `IncomingMessage` |
| `/interrupt` | Agent | Sent on Ctrl+C |
| `/new`, `/thread`, `/resume` | Agent | Thread management |
| All others | Agent | Forwarded verbatim |

The REPL spawns a **blocking OS thread** (via `std::thread::spawn`) for readline,
because rustyline is not async. That thread sends messages via
`tokio::sync::mpsc::Sender::blocking_send`. The `ReceiverStream` wrapping the
receiver end is returned as the `MessageStream`.

### Streaming Output

The REPL supports streaming token output via `StatusUpdate::StreamChunk`. Chunks
are printed with `print!()` (no newline), and `io::stdout().flush()` after each
chunk. When the first chunk arrives, a separator line is printed. When `respond()`
is called and `is_streaming` is `true`, it prints a final newline and resets the
flag rather than re-rendering the full response.

### CRITICAL: The EOF / Service Mode Problem

When IronClaw runs as a background service (e.g. under **launchd** on macOS or
**systemd** on Linux), the init system redirects `stdin` from `/dev/null`. This
causes rustyline's `rl.readline()` to return `ReadlineError::Eof` immediately on
the very first call.

The Eof handler in `start()`:

```rust
Err(ReadlineError::Eof) => {
    // Ctrl+D: send /quit so the agent loop runs graceful shutdown
    let msg = IncomingMessage::new("repl", "default", "/quit");
    let _ = tx.blocking_send(msg);
    break;
}
```

This sends a `/quit` message, which the agent loop interprets as a shutdown
command. The service exits immediately after startup, before it has processed
a single real message.

**The fix is `CLI_ENABLED=false`.**

Setting this environment variable disables the REPL channel entirely. The channel
is never registered with `ChannelManager`, so no `MessageStream` from stdin is
created. The service continues running on its other configured channels (gateway,
HTTP webhook, WASM channels) without any stdin involvement.

```bash
# Correct service configuration:
CLI_ENABLED=false
GATEWAY_ENABLED=true
GATEWAY_AUTH_TOKEN=your-secret-token
```

This is not a bug in the REPL — it is by design. The REPL interprets EOF as the
user pressing Ctrl+D to exit, which is the correct terminal behavior. The service
operator is responsible for setting `CLI_ENABLED=false`.

---

## 5. HTTP Webhook Channel (`http.rs`, `webhook_server.rs`)

### HttpChannel

`HttpChannel` (`src/channels/http.rs`) accepts inbound HTTP POST requests and
converts them to `IncomingMessage` values. It is designed for receiving events from
external services (CI/CD systems, monitoring tools, Zapier, etc.).

The channel is not a standalone server. Instead, it exposes a `routes()` method
that returns an axum `Router` fragment with its state already applied:

```rust
pub fn routes(&self) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route("/webhook", post(webhook_handler))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))  // 64 KB limit
        .with_state(self.state.clone())
}
```

The actual TCP listener is managed by `WebhookServer`.

### Webhook Request Format

```json
{
  "content": "string (required, max 32 KB)",
  "thread_id": "string (optional)",
  "secret": "string (required when HTTP_WEBHOOK_SECRET is set)",
  "wait_for_response": false
}
```

### Authentication

The channel requires `HTTP_WEBHOOK_SECRET` to be set. If the env var is absent,
`start()` returns `Err(ChannelError::StartupFailed)`. Secret validation uses
**constant-time comparison** via the `subtle` crate:

```rust
bool::from(provided.as_bytes().ct_eq(expected_secret.as_bytes()))
```

This prevents timing-based oracle attacks where an attacker could infer the correct
secret one byte at a time by measuring response latency.

### Rate Limiting

The channel enforces 60 requests per minute using a sliding window counter
(`RateLimitState`). Excess requests receive `429 Too Many Requests`. Up to 100
concurrent `wait_for_response` requests are tracked.

### Synchronous Response Mode

When `"wait_for_response": true`, the webhook handler creates a `oneshot::channel`,
registers the sender under `msg.id`, and awaits the receiver with a 60-second
timeout. When the agent calls `respond()` on the channel, the response content is
sent on the oneshot, and the HTTP response body includes the agent's reply. This
allows external systems to drive the agent synchronously.

### WebhookServer

`WebhookServer` (`src/channels/webhook_server.rs`) owns the single TCP listener
for all HTTP-based channels. Channels contribute route fragments via
`add_routes()`, and `start()` merges them and spawns the server:

```rust
pub async fn start(&mut self) -> Result<(), ChannelError> {
    let mut app = Router::new();
    for fragment in self.routes.drain(..) {
        app = app.merge(fragment);
    }
    let listener = tokio::net::TcpListener::bind(self.config.addr).await?;
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    // Graceful shutdown via oneshot signal
    axum::serve(listener, app)
        .with_graceful_shutdown(async { let _ = shutdown_rx.await; })
        .await?;
}
```

Graceful shutdown drains in-flight requests before closing.

---

## 6. Web Gateway Channel

The `GatewayChannel` (`src/channels/web/mod.rs`) is the most capable channel. It
exposes a full browser-accessible interface with REST endpoints, SSE streaming,
WebSocket support, and an OpenAI-compatible API layer.

### 6.1 Server Setup (`server.rs`)

`GatewayState` is the shared state struct injected into every axum handler via
`State<Arc<GatewayState>>`:

```rust
pub struct GatewayState {
    pub msg_tx: tokio::sync::RwLock<Option<mpsc::Sender<IncomingMessage>>>,
    pub sse: SseManager,
    pub workspace: Option<Arc<Workspace>>,
    pub session_manager: Option<Arc<SessionManager>>,
    pub log_broadcaster: Option<Arc<LogBroadcaster>>,
    pub extension_manager: Option<Arc<ExtensionManager>>,
    pub tool_registry: Option<Arc<ToolRegistry>>,
    pub store: Option<Arc<dyn Database>>,
    pub job_manager: Option<Arc<ContainerJobManager>>,
    pub prompt_queue: Option<PromptQueue>,
    pub user_id: String,
    pub shutdown_tx: tokio::sync::RwLock<Option<oneshot::Sender<()>>>,
    pub ws_tracker: Option<Arc<WsConnectionTracker>>,
    pub llm_provider: Option<Arc<dyn LlmProvider>>,
    pub skill_registry: Option<Arc<std::sync::RwLock<SkillRegistry>>>,
    pub skill_catalog: Option<Arc<SkillCatalog>>,
    pub chat_rate_limiter: RateLimiter,  // 30 req / 60 sec
}
```

Optional fields are populated via builder methods on `GatewayChannel`
(`with_workspace()`, `with_session_manager()`, etc.) before `start()` is called.

The router is split into three groups:

- **Public routes** — no auth (health check only)
- **Protected routes** — Bearer token required (all API endpoints, WebSocket,
  OpenAI compat)
- **Static routes** — no auth (embedded HTML/CSS/JS served from memory)

CORS is restricted to `http://<bind_addr>:<port>` and `http://localhost:<port>`.
The gateway is local-first by design.

The `RateLimiter` struct uses `AtomicU64` for lock-free sliding-window enforcement
(30 requests per 60 seconds on chat endpoints). The compare-exchange loop in
`check()` is safe under concurrent access.

### 6.2 Authentication (`auth.rs`)

All protected endpoints go through `auth_middleware`:

```rust
pub async fn auth_middleware(
    State(auth): State<AuthState>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Response {
    // Check Authorization: Bearer <token>
    if let Some(auth_header) = headers.get("authorization")
        && let Ok(value) = auth_header.to_str()
        && let Some(token) = value.strip_prefix("Bearer ")
        && bool::from(token.as_bytes().ct_eq(auth.token.as_bytes()))
    {
        return next.run(request).await;
    }
    // Fall back to ?token=xxx for SSE EventSource (browsers cannot set headers)
    if let Some(query) = request.uri().query() {
        for pair in query.split('&') {
            if let Some(token) = pair.strip_prefix("token=")
                && bool::from(token.as_bytes().ct_eq(auth.token.as_bytes()))
            {
                return next.run(request).await;
            }
        }
    }
    (StatusCode::UNAUTHORIZED, "Invalid or missing auth token").into_response()
}
```

Key points:

- Both `Authorization: Bearer <token>` header and `?token=<token>` query parameter
  are accepted. The query-param fallback exists because the browser `EventSource`
  API cannot set custom headers.
- All comparisons use `subtle::ConstantTimeEq` to prevent timing attacks.
- If `GATEWAY_AUTH_TOKEN` is not set in the environment, `GatewayChannel::new()`
  generates a random 32-character alphanumeric token and prints it to the console
  at startup.

### 6.3 API Routes Reference

All protected routes require `Authorization: Bearer <GATEWAY_AUTH_TOKEN>`.
Static routes (`/`, `/style.css`, `/app.js`) and `/api/health` are public.

| Method | Path | Auth | Request Body | Response | Description |
|--------|------|------|--------------|----------|-------------|
| GET | `/api/health` | No | — | `{"status":"healthy","channel":"gateway"}` | Health check |
| POST | `/api/chat/send` | Yes | `{"content":"string","thread_id":"string?"}` | `{"message_id":"uuid","status":"accepted"}` | Send message to agent |
| POST | `/api/chat/approval` | Yes | `{"request_id":"string","action":"approve\|always\|deny","thread_id":"string?"}` | `ActionResponse` | Approve/deny tool execution |
| POST | `/api/chat/auth-token` | Yes | `{"extension_name":"string","token":"string"}` | `ActionResponse` | Submit extension auth token |
| POST | `/api/chat/auth-cancel` | Yes | `{"extension_name":"string"}` | `ActionResponse` | Cancel extension auth flow |
| GET | `/api/chat/events` | Yes | `?token=xxx` (query) | SSE stream | Live agent event stream |
| GET | `/api/chat/ws` | Yes | — | WebSocket upgrade | Bidirectional WebSocket |
| GET | `/api/chat/history` | Yes | `?limit=N&before=timestamp` | `{thread_id,turns:[...],has_more}` | Conversation history |
| GET | `/api/chat/threads` | Yes | — | `{assistant_thread,threads:[...],active_thread}` | List threads |
| POST | `/api/chat/thread/new` | Yes | — | Thread info | Start new conversation thread |
| GET | `/api/memory/tree` | Yes | — | `{entries:[{path,is_dir},...]}` | Memory filesystem tree |
| GET | `/api/memory/list` | Yes | `?path=string` | `{path,entries:[...]}` | List memory directory |
| GET | `/api/memory/read` | Yes | `?path=string` | `{path,content,updated_at}` | Read memory file |
| POST | `/api/memory/write` | Yes | `{"path":"string","content":"string"}` | `{path,status}` | Write memory file |
| POST | `/api/memory/search` | Yes | `{"query":"string","limit":N}` | `{results:[{path,content,score}]}` | Hybrid memory search |
| GET | `/api/jobs` | Yes | — | `{jobs:[...]}` | List all jobs |
| GET | `/api/jobs/summary` | Yes | — | `{total,pending,in_progress,completed,failed,stuck}` | Job counts |
| GET | `/api/jobs/{id}` | Yes | — | Full job detail | Get job details |
| POST | `/api/jobs/{id}/cancel` | Yes | — | `ActionResponse` | Cancel a job |
| POST | `/api/jobs/{id}/restart` | Yes | — | `ActionResponse` | Restart a failed job |
| POST | `/api/jobs/{id}/prompt` | Yes | `{"prompt":"string"}` | `ActionResponse` | Send follow-up prompt to Claude Code job |
| GET | `/api/jobs/{id}/events` | Yes | — | SSE stream | Job-scoped event stream |
| GET | `/api/jobs/{id}/files/list` | Yes | `?path=string` | `{entries:[...]}` | List job project files |
| GET | `/api/jobs/{id}/files/read` | Yes | `?path=string` | `{path,content}` | Read job project file |
| GET | `/api/logs/events` | Yes | — | SSE stream | Live tracing log stream |
| GET | `/api/extensions` | Yes | — | `{extensions:[...]}` | List extensions |
| GET | `/api/extensions/tools` | Yes | — | `{tools:[...]}` | List extension tools |
| POST | `/api/extensions/install` | Yes | `{"name":"string","url":"string?","kind":"string?"}` | `ActionResponse` | Install extension |
| POST | `/api/extensions/{name}/activate` | Yes | — | `ActionResponse` | Activate extension |
| POST | `/api/extensions/{name}/remove` | Yes | — | `ActionResponse` | Remove extension |
| GET | `/api/routines` | Yes | — | `{routines:[...]}` | List routines |
| GET | `/api/routines/summary` | Yes | — | `{total,enabled,disabled,failing,runs_today}` | Routine counts |
| GET | `/api/routines/{id}` | Yes | — | Full routine detail with recent runs | Get routine |
| POST | `/api/routines/{id}/trigger` | Yes | — | `ActionResponse` | Manually trigger routine |
| POST | `/api/routines/{id}/toggle` | Yes | — | `ActionResponse` | Enable/disable routine |
| DELETE | `/api/routines/{id}` | Yes | — | `ActionResponse` | Delete routine |
| GET | `/api/routines/{id}/runs` | Yes | — | `{runs:[...]}` | List routine run history |
| GET | `/api/skills` | Yes | — | `{skills:[...],count:N}` | List installed skills |
| POST | `/api/skills/search` | Yes | `{"query":"string"}` | `{catalog:[...],installed:[...],registry_url}` | Search ClawHub registry |
| POST | `/api/skills/install` | Yes | `{"name":"string","url":"string?","content":"string?"}` | `ActionResponse` | Install skill |
| DELETE | `/api/skills/{name}` | Yes | — | `ActionResponse` | Remove skill |
| GET | `/api/settings` | Yes | — | `{settings:[...]}` | List all settings |
| GET | `/api/settings/export` | Yes | — | `{settings:{key:value,...}}` | Export settings as map |
| POST | `/api/settings/import` | Yes | `{"settings":{key:value,...}}` | `ActionResponse` | Bulk import settings |
| GET | `/api/settings/{key}` | Yes | — | `{key,value,updated_at}` | Get single setting |
| PUT | `/api/settings/{key}` | Yes | `{"value":any}` | `ActionResponse` | Set single setting |
| DELETE | `/api/settings/{key}` | Yes | — | `ActionResponse` | Delete setting |
| GET | `/api/gateway/status` | Yes | — | Status info | Gateway health + connection counts |
| POST | `/v1/chat/completions` | Yes | OpenAI format | OpenAI format | OpenAI-compatible completions |
| GET | `/v1/models` | Yes | — | OpenAI models list | List available models |
| GET | `/` | No | — | HTML | Browser UI entry point |
| GET | `/style.css` | No | — | CSS | Stylesheet |
| GET | `/app.js` | No | — | JavaScript | SPA bundle |
| GET | `/projects/{id}/{*path}` | Yes | — | File content | Serve sandbox project files |

### 6.4 SSE Streaming (`sse.rs`)

The `SseManager` is a broadcast hub that delivers real-time events from the agent
to all connected browser tabs simultaneously.

```
Agent sends StatusUpdate
        │
        ▼
GatewayChannel::send_status()  →  converts to SseEvent
        │
        ▼
SseManager::broadcast(event)   →  broadcast::Sender<SseEvent>  (tokio)
        │
        ├──► SSE subscriber 1 (tab)  →  EventSource in browser
        ├──► SSE subscriber 2 (tab)  →  EventSource in browser
        └──► WebSocket subscriber    →  WsServerMessage::Event{...}
```

The broadcast channel holds up to 256 events. Slow clients that fall behind will
miss events (acceptable because the browser reconnects the EventSource and
re-fetches history). The connection limit is 100 concurrent SSE+WebSocket
connections combined, enforced atomically with `AtomicU64::fetch_update`.

Each SSE event is serialized as JSON and sent with a named event type:

```
event: stream_chunk
data: {"type":"stream_chunk","content":"Hello","thread_id":"t1"}

event: tool_started
data: {"type":"tool_started","name":"shell","thread_id":"t1"}
```

The `CountedStream` wrapper decrements the connection counter on `Drop`,
so the limit is automatically reclaimed when clients disconnect.

Keep-alive pings fire every 30 seconds to prevent proxy timeouts.

### 6.5 WebSocket (`ws.rs`)

The `/api/chat/ws` endpoint upgrades HTTP to WebSocket. Unlike SSE, WebSocket is
bidirectional — the client can send messages and receive events on the same
connection.

The handler spawns two concurrent tasks:

- **Sender task**: subscribes to the SSE broadcast and forwards events as
  `WsServerMessage::Event{event_type, data}` JSON frames.
- **Receiver loop**: reads `WsClientMessage` frames from the client.

Client-to-server message types (JSON, tagged with `"type"` field):

```json
{"type": "message", "content": "hello", "thread_id": "t1"}
{"type": "approval", "request_id": "uuid", "action": "approve|always|deny", "thread_id": "t1"}
{"type": "auth_token", "extension_name": "notion", "token": "sk-xxx"}
{"type": "auth_cancel", "extension_name": "notion"}
{"type": "ping"}
```

Server-to-client frames:

```json
{"type": "event", "event_type": "response", "data": {...}}
{"type": "pong"}
{"type": "error", "message": "..."}
```

The WebSocket path and the SSE path share the same `SseManager` broadcast
channel, so all connected clients (tabs and WebSocket connections alike) receive
the same event stream without duplication.

### 6.6 OpenAI-Compatible API (`openai_compat.rs`)

The `/v1/chat/completions` endpoint accepts standard OpenAI API requests,
translating them to IronClaw's internal `LlmProvider` interface. This allows any
tool or library that speaks the OpenAI API (LangChain, LlamaIndex, Cursor, etc.)
to use IronClaw as a drop-in backend by pointing `base_url` at the gateway.

**Request mapping:**

| OpenAI Field | IronClaw Internal |
|--------------|-------------------|
| `messages[].role` | `Role::{System,User,Assistant,Tool}` |
| `messages[].content` | `ChatMessage.content` |
| `messages[].tool_calls` | `ChatMessage.tool_calls: Vec<ToolCall>` |
| `tools[].function` | `ToolDefinition{name,description,parameters}` |
| `tool_choice` | Normalized to `"auto"`, `"required"`, or `"none"` |
| `stream: true` | Simulated chunking (LLM is called first, then chunked) |

**Streaming note:** The current implementation does not perform true server-side
streaming. When `stream: true`, the LLM call completes first (getting the full
response), then the response is split into ~20-character word-boundary chunks and
emitted as SSE data frames with a `[DONE]` sentinel. A custom response header
`x-ironclaw-streaming: simulated` signals this to callers. A `finish_reason` field
is included in the final chunk. The response includes `usage` token counts.

**Error mapping** follows OpenAI conventions:

| IronClaw error | HTTP status | OpenAI error type |
|----------------|------------|-------------------|
| `AuthFailed`, `SessionExpired` | 401 | `authentication_error` |
| `RateLimited` | 429 | `rate_limit_error` |
| `ContextLengthExceeded` | 400 | `invalid_request_error` |
| `ModelNotAvailable` | 404 | `invalid_request_error` |
| Other | 500 | `server_error` |

### 6.7 Log Streaming (`log_layer.rs`)

`WebLogLayer` is a `tracing_subscriber::Layer` that intercepts log events and
forwards them to `LogBroadcaster`. The broadcaster:

1. Runs every log message through `LeakDetector.scan_and_clean()` before it
   reaches any subscriber, preventing accidental secret exfiltration via logs.
2. Maintains a ring buffer of the 500 most recent entries so browsers that connect
   after startup still see the boot log.
3. Broadcasts live entries on a `broadcast::Sender<LogEntry>` (512-entry buffer).

The SSE endpoint at `/api/logs/events` replays recent entries to new subscribers
then streams live entries.

---

## 7. WASM Channel System (`wasm/`)

The WASM channel system enables dynamically loaded channel plugins — the same
mechanism used for Telegram, Slack, and WhatsApp integrations. It follows a
**Host-Managed Event Loop** pattern.

### Architecture

```
Host (Rust)                                WASM Module
────────────────────────────────────────   ───────────────────
HTTP Router      ─── on_http_request() ──► (parses, validates)
Polling Scheduler ─── on_poll() ─────────► (fetches from API)
Timer Scheduler   ─── on_timer() ────────► (scheduled work)
                                           │
                  ◄── emit_message() ───── (queues message)
                  ◄── http_request() ───── (outbound HTTP)
                  ◄── workspace_write() ── (scoped storage)
                  ◄── log() ─────────────  (tracing output)
```

The host manages all infrastructure. WASM modules define behavior through exported
callback functions. A **fresh WASM instance is created per callback** (the NEAR
pattern) — no mutable state persists between calls, eliminating a class of
state-pollution bugs.

### Runtime (`runtime.rs`)

`WasmChannelRuntime` wraps a Wasmtime `Engine` configured for:

- Cranelift compilation at `OptLevel::Speed`
- Fuel metering: 10 million fuel units per callback (prevents infinite loops)
- Memory limit: 50 MB per instance (channels may buffer messages)
- Callback timeout: 30 seconds

Modules are compiled once at load time and cached as `PreparedChannelModule`.
Instantiation is cheap (milliseconds) because compilation already happened.

### Host Capabilities (`host.rs`)

`ChannelHostState` extends the base tool `HostState` with channel-specific
capabilities:

| Host Function | Limit | Description |
|---------------|-------|-------------|
| `emit_message()` | 100 per callback | Queue a message to send to the agent |
| `workspace_write()` | Scoped to `channels/<name>/` | Persist state between callbacks |
| `http_request()` | Allowlist checked | Outbound HTTP (credentials injected) |
| `log()` | — | Forward to tracing |

Message content is capped at 64 KB. Rate limiting on `emit_message` uses a
sliding-window token bucket (configurable per channel in `capabilities.json`).
The minimum polling interval is enforced at 30 seconds to prevent API flooding.

### Security Model

| Threat | Mitigation |
|--------|------------|
| Path hijacking | `allowed_paths` in capabilities restricts registrable endpoints |
| Token exposure | Credentials injected at host boundary; WASM never sees raw values |
| State pollution | Fresh instance per callback — no shared mutable state |
| Workspace escape | All paths prefixed with `channels/<name>/` |
| Message spam | `emit_message` rate limited per capabilities config |
| Resource exhaustion | Fuel metering + memory limits + callback timeout |
| Polling abuse | Minimum 30-second interval enforced by host |

### Loading Channels

`WasmChannelLoader` discovers channel WASM files from a directory (default:
`~/.ironclaw/channels/`) and from bundled channels compiled into the binary.
Each loaded channel produces a `WasmChannel` that implements the `Channel` trait
and integrates with `ChannelManager` like any other channel.

### Bundled WASM Channels

The following channels are compiled into the IronClaw binary and loaded
automatically when their required secrets are configured.

#### WhatsApp (added in v0.9.0)

**Source:** `channels-src/whatsapp/` (`channels-src/whatsapp/whatsapp.capabilities.json`,
`channels-src/whatsapp/src/lib.rs`)

The WhatsApp channel integrates with the **WhatsApp Cloud API** (Meta) to
receive and respond to WhatsApp messages via webhook delivery. Unlike the
Telegram and Slack channels, WhatsApp is **webhook-only** — `allow_polling` is
`false` in its capabilities declaration, so the host never calls `on_poll()`.
All inbound traffic arrives through the registered HTTP path `/webhook/whatsapp`.

**Required configuration:**

| Secret / Env Var | Description |
|-----------------|-------------|
| `WHATSAPP_ACCESS_TOKEN` | WhatsApp Cloud API access token from the Meta Developer Portal. Validated at startup against `https://graph.facebook.com/v18.0/me`. |
| `WHATSAPP_VERIFY_TOKEN` | Webhook verify token sent by Meta during endpoint registration. Auto-generated (32 chars) if not provided. |

**Webhook registration:** The host registers the endpoint at
`/webhook/whatsapp` and handles the `hub.verify_token` challenge used by Meta
to verify ownership of the endpoint. Signature validation uses the
`X-Hub-Signature-256` header.

**Message handling:** The WASM module parses the incoming Cloud API webhook
payload (field `"messages"` within `"whatsapp_business_account"` entries) and
calls `emit_message()` for each inbound text message. The `user_id` is set to
the sender's WhatsApp phone number; the optional `user_name` is populated from
the `contacts` array when Meta includes it.

**Supported event types:**

| Webhook field | Handled | Notes |
|---------------|---------|-------|
| `messages` (type `text`) | Yes | Forwarded to agent as `IncomingMessage` |
| `statuses` (`delivered`, `read`, etc.) | Ignored | Parsed but not forwarded |
| `contacts` | Metadata only | Provides `user_name` for messages |

**Outbound replies:** `respond()` sends a `text` message back to the originating
phone number via `POST https://graph.facebook.com/v18.0/<phone_number_id>/messages`.
The access token is injected by the host at the HTTP boundary —
`{WHATSAPP_ACCESS_TOKEN}` placeholder in the request template — so the WASM
module never sees the raw credential.

**Rate limits (capabilities.json):**

| Limit | Value |
|-------|-------|
| Outbound HTTP to `graph.facebook.com` | 80 req/min, 1 000 req/hr |
| `emit_message()` calls per callback | 100 msg/min, 5 000 msg/hr |

**Workspace storage:** State is persisted under `channels/whatsapp/` (scoped
by the host; the module cannot escape this prefix).

**API version:** Cloud API `v18.0`. The `config.api_version` field in
`whatsapp.capabilities.json` controls the path segment; update it there when
upgrading to a newer Graph API version.

**Access control:** By default `allow_from` is empty (all senders accepted).
Set `config.owner_id` to restrict the channel to a single WhatsApp account ID,
or populate `allow_from` with an allowlist of phone numbers.

---

## 8. Configuration Reference

| Env Var | Default | Description |
|---------|---------|-------------|
| `CLI_ENABLED` | `true` | Enable the REPL channel. **Set `false` for service mode** to prevent immediate EOF shutdown when stdin is `/dev/null`. |
| `HTTP_ENABLED` | `false` | Enable HTTP webhook channel |
| `HTTP_HOST` | `127.0.0.1` | Webhook listen address |
| `HTTP_PORT` | `3000` | Webhook listen port |
| `HTTP_WEBHOOK_SECRET` | — | Required shared secret for webhook authentication (constant-time checked) |
| `HTTP_USER_ID` | `http` | User identity assigned to webhook messages |
| `GATEWAY_ENABLED` | `false` | Enable web gateway channel |
| `GATEWAY_HOST` | `127.0.0.1` | Gateway listen address |
| `GATEWAY_PORT` | `3000` | Gateway listen port |
| `GATEWAY_AUTH_TOKEN` | auto-generated | Bearer token for all protected API endpoints. If unset, a random 32-char token is generated and logged at startup. |
| `GATEWAY_USER_ID` | `default` | User identity for messages sent through the gateway |
| `WHATSAPP_ACCESS_TOKEN` | — | WhatsApp Cloud API access token (Meta Developer Portal). Required to enable the WhatsApp WASM channel. |
| `WHATSAPP_VERIFY_TOKEN` | auto-generated | Webhook verify token used during Meta endpoint registration. Auto-generated (32 chars) if unset. |

### Service Mode Example

```bash
# ~/.ironclaw/env (or systemd EnvironmentFile / launchd EnvironmentVariables)

# Disable REPL — stdin is /dev/null in service mode, which causes immediate
# EOF → /quit → graceful shutdown if this is left enabled.
CLI_ENABLED=false

# Enable web gateway for browser and API access
GATEWAY_ENABLED=true
GATEWAY_HOST=127.0.0.1
GATEWAY_PORT=3000
GATEWAY_AUTH_TOKEN=change-me-to-a-strong-random-token

# Enable HTTP webhook for external integrations (optional)
HTTP_ENABLED=true
HTTP_PORT=3000
HTTP_WEBHOOK_SECRET=change-me-to-a-webhook-secret
```

### Launchd Plist Example

```xml
<key>EnvironmentVariables</key>
<dict>
    <key>CLI_ENABLED</key>
    <string>false</string>
    <key>GATEWAY_ENABLED</key>
    <string>true</string>
    <key>GATEWAY_AUTH_TOKEN</key>
    <string>your-secret-token-here</string>
</dict>
```

---

## 9. Extending the Channel System

To add a new channel:

1. Create `src/channels/my_channel.rs`.
2. Implement `Channel` for your struct:
   - `name()` returns a unique lowercase identifier.
   - `start()` sets up the transport and returns a `MessageStream`.
   - `respond()` delivers the agent's reply back to the caller.
   - Optionally implement `send_status()` for live progress feedback.
3. Add a config section in `src/config/`.
4. Wire up in `main.rs` in the channel setup block:

   ```rust
   if config.my_channel.enabled {
       manager.add(Box::new(MyChannel::new(config.my_channel)));
   }
   ```

Channels that need a shared HTTP server should contribute route fragments to
`WebhookServer` via `add_routes()` rather than binding their own listener. This
keeps all webhook traffic on a single port.

For dynamically loaded channels (Telegram, Slack, WhatsApp, custom bots), use
the WASM channel system: implement the WASM callback exports and declare
capabilities in `channel.json`. The host runtime handles polling, HTTP, rate
limiting, and credential injection automatically.
