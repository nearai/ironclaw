# IronClaw Codebase Analysis — Tunnels & Mobile Pairing

> Updated: 2026-02-26 | Version: v0.12.0

## 1. Overview

IronClaw binds its web gateway to a local address (`127.0.0.1` by default). This is intentional: local-only access means no exposure to the internet by default, which reduces the attack surface. However, several use cases require external reachability:

- **Mobile access**: reaching the agent from a phone when away from the local network
- **Team sharing**: exposing an IronClaw instance to colleagues over the internet
- **Webhook receivers**: accepting inbound HTTP callbacks from external services (GitHub, Stripe, Telegram bot API, etc.)
- **Remote routines**: triggering reactive routines from outside the LAN

The tunnel subsystem (`src/tunnel/`) solves this by wrapping external tunnel binaries behind a uniform Rust trait. The gateway calls `start()` after binding its local port and `stop()` on shutdown. The tunnel subsystem manages subprocess lifecycle so the rest of the codebase sees only a public URL string.

The mobile pairing subsystem (`src/pairing/`) solves a related but distinct problem: once a tunnel URL exists, how does the agent know which mobile users are allowed to send it messages? Pairing gates inbound DMs from channels like Telegram and Slack, requiring an out-of-band code exchange before the agent will respond to an unknown sender.

---

## 2. Tunnel Abstraction (`tunnel/mod.rs`)

### The `Tunnel` Trait

All tunnel backends implement a single async trait defined in `src/tunnel/mod.rs`:

```rust
#[async_trait::async_trait]
pub trait Tunnel: Send + Sync {
    fn name(&self) -> &str;
    async fn start(&self, local_host: &str, local_port: u16) -> Result<String>;
    async fn stop(&self) -> Result<()>;
    async fn health_check(&self) -> bool;
    fn public_url(&self) -> Option<String>;
}
```

- `name()` — returns a human-readable provider identifier (e.g., `"cloudflare"`, `"ngrok"`).
- `start(local_host, local_port)` — spawns the external tunnel binary pointed at `local_host:local_port`, waits for it to print a public URL on stdout/stderr, stores it, and returns it. On failure the method kills the child process and returns an error.
- `stop()` — kills the child process gracefully and clears the stored public URL.
- `health_check()` — returns `true` if the underlying subprocess is still alive (child PID is present). The `CustomTunnel` variant can also poll an HTTP health endpoint instead.
- `public_url()` — synchronous accessor; returns `None` before `start()` or after `stop()`. Uses `std::sync::RwLock` (not `tokio::sync::RwLock`) so this never blocks an async executor.

### Shared Infrastructure

Two internal helpers reduce boilerplate across backends:

```rust
pub(crate) type SharedUrl = Arc<std::sync::RwLock<Option<String>>>;
pub(crate) type SharedProcess = Arc<Mutex<Option<TunnelProcess>>>;
```

`SharedUrl` uses a standard (sync) `RwLock` so the `public_url()` method can be called from synchronous contexts without `.await`. `SharedProcess` uses a Tokio async `Mutex` because `start()` and `stop()` are async and may hold the guard across await points.

`kill_shared()` is a shared helper that locks the process guard, calls `child.kill().await`, waits for the child to exit, and sets the guard to `None`.

### Configuration Types

Each provider has a dedicated config struct:

| Struct | Key fields |
|--------|-----------|
| `CloudflareTunnelConfig` | `token: String` |
| `TailscaleTunnelConfig` | `funnel: bool`, `hostname: Option<String>` |
| `NgrokTunnelConfig` | `auth_token: String`, `domain: Option<String>` |
| `CustomTunnelConfig` | `start_command: String`, `health_url: Option<String>`, `url_pattern: Option<String>` |

These are composed into `TunnelProviderConfig`, which carries the `provider` string and one optional inner config per provider type.

### Factory Function

`create_tunnel(config: &TunnelProviderConfig) -> Result<Option<Box<dyn Tunnel>>>` is the single entry point for constructing a tunnel at runtime. It matches on `config.provider`:

- `"none"` or `""` — returns `Ok(None)`, meaning no tunnel is used.
- `"cloudflare"` — requires `config.cloudflare` to be `Some`; errors with a message referencing `TUNNEL_CF_TOKEN` if absent.
- `"tailscale"` — uses `config.tailscale` if present, otherwise applies defaults (`funnel: false`, `hostname: None`).
- `"ngrok"` — requires `config.ngrok` to be `Some`; errors with a message referencing `TUNNEL_NGROK_TOKEN` if absent.
- `"custom"` — requires `config.custom` to be `Some`; errors with a message referencing `TUNNEL_CUSTOM_COMMAND` if absent.
- Anything else — returns a hard error listing valid provider names.

---

## 3. Tunnel Backends

### 3.1 No Tunnel (`none.rs`)

`NoneTunnel` is a zero-allocation no-op. It satisfies the `Tunnel` trait but performs no subprocess management:

- `start()` returns `format!("http://{local_host}:{local_port}")` — the raw local address.
- `stop()` and `health_check()` always succeed or return `true`.
- `public_url()` always returns `None` because there is no externally reachable URL.

This is the default when `TUNNEL_PROVIDER` is unset or set to `"none"`. The agent can still be reached from the local machine, but not from external networks.

### 3.2 Cloudflare Tunnel (`cloudflare.rs`)

`CloudflareTunnel` wraps the `cloudflared` binary, which must be on `PATH`.

**How it works:**

1. `start()` spawns `cloudflared tunnel --no-autoupdate run --token <token> --url http://<host>:<port>`.
2. `cloudflared` prints log lines to **stderr**. The implementation reads stderr line by line using a `tokio::io::BufReader`.
3. URL extraction: each line is scanned for the substring `"https://"`. When found, the URL is extracted by slicing from that index to the next whitespace character (or end of line).
4. A 30-second deadline with 5-second per-line timeouts is applied. If no URL appears within 30 seconds, the child is killed and an error is returned.
5. On success, the URL is stored in `SharedUrl` and the child handle in `SharedProcess`.

**Token modes:**

The token passed via `TUNNEL_CF_TOKEN` is a Cloudflare Zero Trust tunnel token obtained from the Cloudflare dashboard. This always produces a stable, named tunnel URL (e.g., `https://your-tunnel.example.com`). Cloudflare's "quick tunnel" (temporary `*.trycloudflare.com` URLs, no account required) is not directly exposed through this implementation — that would require running `cloudflared tunnel --url <origin>` without a `--token` flag, which is not currently wired up.

**Shutdown:** `stop()` clears the URL and calls `kill_shared()`, which sends SIGKILL to the `cloudflared` process.

**Health check:** Checks whether the child process PID is still present (i.e., the process has not exited).

### 3.3 Ngrok (`ngrok.rs`)

`NgrokTunnel` wraps the `ngrok` binary, which must be on `PATH`.

**How it works:**

1. `start()` constructs arguments: `ngrok http <host>:<port> [--domain <domain>] --log stdout --log-format logfmt`.
2. The auth token is passed via the `NGROK_AUTHTOKEN` environment variable (not a CLI flag), which is the method ngrok recommends for non-interactive use.
3. ngrok writes structured `logfmt` output to **stdout**. The implementation reads stdout line by line.
4. URL extraction: each line is scanned for the substring `"url=https://"`. When found, `"url="` (4 characters) is skipped and the URL is extracted to the next whitespace.
5. A 15-second deadline with 3-second per-line timeouts is applied. If no URL appears in 15 seconds, the child is killed and an error is returned.

**Custom domains:** If `TUNNEL_NGROK_DOMAIN` is set, `--domain <domain>` is appended to the ngrok command. Custom domains require an ngrok paid plan and produce stable subdomains instead of ephemeral `*.ngrok-free.app` URLs.

**Shutdown and health:** Same pattern as Cloudflare — `kill_shared()` on stop, PID presence check for health.

### 3.4 Tailscale (`tailscale.rs`)

`TailscaleTunnel` wraps the `tailscale` CLI, which must be installed and authenticated (`tailscale up` must have been run).

**Two modes:**

- `tailscale serve` (default, `funnel: false`): exposes the port within the Tailscale network (tailnet) only. Machines on the same tailnet can reach it; the public internet cannot.
- `tailscale funnel` (`funnel: true`): exposes the port publicly via Tailscale's Funnel feature. The URL resolves globally, not just within the tailnet.

**Hostname resolution:**

If `hostname` is not provided in config, `start()` runs `tailscale status --json` with a 10-second timeout, parses the JSON output, and extracts `Self.DNSName`. The trailing dot is stripped (Tailscale appends `.` to DNS names). The resulting hostname looks like `machine-name.tail-xxxx.ts.net`.

**Subprocess lifecycle:**

`tailscale serve <target>` or `tailscale funnel <target>` is spawned as a long-running child. On `stop()`, `tailscale serve reset` or `tailscale funnel reset` is called first (to clean up the serve/funnel configuration on the Tailscale daemon), then the child is killed via `kill_shared()`.

**URL format:** `https://<hostname>` where hostname is either the configured override or the auto-detected DNS name from `tailscale status`.

### 3.5 Custom Tunnel (`custom.rs`)

`CustomTunnel` executes an arbitrary shell command, making it compatible with any tunnel tool not natively supported: `bore`, `serveo` (via SSH), a self-hosted frp instance, or a static reverse proxy.

**Command template substitution:**

The `start_command` string supports two placeholders:

- `{port}` — replaced with the local port number as a decimal string.
- `{host}` — replaced with the local host string.

Example commands from the source documentation:

- `bore local {port} --to bore.pub`
- `ssh -R 80:localhost:{port} serveo.net`

**Important limitation:** The command is split on whitespace before execution (`cmd.split_whitespace()`). Arguments containing spaces cannot be quoted — each token must be a single word.

**URL extraction (optional):**

If `url_pattern` is set, stdout is read for up to 15 seconds. Each line is scanned for `https://` or `http://` using `extract_url()`. If a URL is found and it contains the `url_pattern` substring, it is used as the public URL. This filter allows skipping internal URLs (e.g., ignoring `http://internal:1234` while matching `https://real.tunnel.io/abc`).

If `url_pattern` is not set, or no matching URL is found within the timeout, the fallback URL `http://{local_host}:{local_port}` is returned — the local address.

**Health checks (optional):**

If `health_url` is set, `health_check()` sends an HTTP GET to that URL with a 5-second timeout and returns `true` on any response. This is useful for tools that expose their own health endpoint. Without `health_url`, health is determined by PID presence.

---

## 4. Mobile Pairing (`pairing/`)

### Purpose

The pairing system gates inbound direct messages from channels such as Telegram, Slack, and similar messaging platforms. Before the agent responds to any sender, it verifies that sender is on the `allowFrom` list for that channel. Unknown senders receive a pairing code and must be approved by the owner via the CLI.

This prevents the agent from responding to arbitrary strangers who discover or guess the Telegram bot username.

As of v0.10.0, all WASM channel plugins support device pairing — not just selected channels. Telegram, Slack, Discord, and WhatsApp WASM channels all implement the pairing flow. Each WASM channel calls into the same `PairingStore` API (`upsert_request`, `is_sender_allowed`, `approve`), so the pairing behavior is consistent regardless of which channel the inbound message arrives on.

### How Pairing Works

1. An unknown sender messages the bot on Telegram (or another channel).
2. The channel handler calls `PairingStore::upsert_request(channel, sender_id, meta)`.
3. If no existing request exists for that sender, a new `PairingRequest` is created with a randomly generated 8-character code from the alphabet `ABCDEFGHJKLMNPQRSTUVWXYZ23456789` (visually unambiguous — no `I`, `O`, `1`, `0`).
4. The bot replies to the sender with their pairing code (e.g., `Your pairing code is: XK3NB7QR`).
5. The owner of the IronClaw instance runs `ironclaw pairing approve telegram XK3NB7QR` in their terminal.
6. The CLI calls `PairingStore::approve(channel, code)`, which moves the sender ID from pending requests to the `allowFrom` list.
7. Future messages from that sender pass the `is_sender_allowed()` check and are processed normally.

### PairingRequest Structure

```rust
pub struct PairingRequest {
    pub id: String,           // Sender identifier (user ID, username, etc.)
    pub code: String,         // 8-char pairing code
    pub created_at: String,   // RFC 3339 timestamp
    pub last_seen_at: String, // Updated on upsert; used for display
    pub meta: Option<serde_json::Value>, // Channel-specific metadata (chat_id, username, etc.)
}
```

### Storage Layout

All pairing data is stored as JSON files in `~/.ironclaw/`:

| File | Purpose |
|------|---------|
| `<channel>-pairing.json` | Pending pairing requests (version + requests array) |
| `<channel>-allowFrom.json` | Approved sender IDs (version + allow_from array) |
| `<channel>-approve-attempts.json` | Failed approve attempt timestamps (rate limiting) |

The `channel` name is normalized to lowercase and path-unsafe characters (`\`, `/`, `:`, `*`, `?`, `"`, `<`, `>`, `|`) are replaced with underscores before constructing file paths. This prevents path traversal.

### Security Properties

**Pairing code alphabet:** Uses 32 characters (`ABCDEFGHJKLMNPQRSTUVWXYZ23456789`), excluding visually similar characters. An 8-character code has 32^8 = ~1.1 trillion combinations.

**TTL:** Pending requests expire after 15 minutes (`PAIRING_PENDING_TTL_SECS = 900`). Expired requests are filtered out lazily on read and pruned during upsert.

**Capacity limit:** At most 3 pending requests per channel are held at a time (`PAIRING_PENDING_MAX = 3`). When the limit is reached, new requests return an empty code without creating an entry.

**Rate limiting on approve:** Failed approval attempts are tracked per channel. After 10 failed attempts within a 5-minute window (`PAIRING_APPROVE_RATE_LIMIT = 10`, `PAIRING_APPROVE_RATE_WINDOW_SECS = 300`), further approve calls return `PairingStoreError::ApproveRateLimited`. This limits brute-force guessing of pairing codes.

**File locking:** `upsert_request()` and `approve()` use `fs4::FileExt::lock_exclusive()` to prevent concurrent writers from corrupting the JSON files. The lock is released immediately after the write is complete and `sync_all()` has been called.

**Case-insensitive code matching:** `approve()` normalizes both the stored code and the input to uppercase before comparing, so users can type codes in any case.

**Sender lookup:** `is_sender_allowed()` checks both the numeric/string ID and the username (with and without `@` prefix, case-insensitive). This accommodates channels that may send either form.

---

## 5. CLI Integration (`cli/pairing.rs`)

The `ironclaw pairing` subcommand exposes two operations using `clap`:

### `ironclaw pairing list <channel> [--json]`

Lists all pending (non-expired) pairing requests for the given channel.

- Without `--json`: prints a human-readable table with columns: code, sender ID, metadata key=value pairs, and creation timestamp.
- With `--json`: prints the full `Vec<PairingRequest>` as pretty-printed JSON. Suitable for scripting or piping to `jq`.

Example output (human-readable):

```
Pairing requests (1):
  XK3NB7QR  user123  username=alice  2026-02-22T03:55:00+00:00
```

### `ironclaw pairing approve <channel> <code>`

Approves a pending pairing request by code.

- On success: prints `Approved telegram sender user123.` and exits 0.
- On wrong code: prints `No pending pairing request found for code: XK3NB7QR` and exits 1.
- On rate limit: prints a message asking the user to wait and exits 1.

The CLI delegates entirely to `PairingStore` and contains no business logic itself. Both functions accept an injected `&PairingStore`, which enables testing with a temporary directory without touching `~/.ironclaw/`.

---

## 6. Configuration Reference

### Tunnel Environment Variables

| Env Var | Default | Description |
|---------|---------|-------------|
| `TUNNEL_PROVIDER` | `none` | Tunnel backend to use: `none`, `cloudflare`, `ngrok`, `tailscale`, `custom` |
| `TUNNEL_CF_TOKEN` | — | Cloudflare Zero Trust tunnel token. Required when `TUNNEL_PROVIDER=cloudflare`. Obtain from Cloudflare dashboard under Zero Trust > Tunnels. |
| `TUNNEL_NGROK_TOKEN` | — | ngrok auth token. Required when `TUNNEL_PROVIDER=ngrok`. Obtain from dashboard.ngrok.com. |
| `TUNNEL_NGROK_DOMAIN` | — | Custom ngrok domain (requires paid plan). When set, passed as `--domain` to ngrok. |
| `TUNNEL_TS_FUNNEL` | `false` | Set to `true` to use `tailscale funnel` (public internet) instead of `tailscale serve` (tailnet only). |
| `TUNNEL_TS_HOSTNAME` | auto | Override the Tailscale hostname. Defaults to `Self.DNSName` from `tailscale status --json`. |
| `TUNNEL_CUSTOM_COMMAND` | — | Shell command for custom tunnel. Required when `TUNNEL_PROVIDER=custom`. Use `{port}` and `{host}` placeholders. |
| `TUNNEL_CUSTOM_HEALTH_URL` | — | HTTP URL to poll for custom tunnel health checks. If unset, child PID presence is used. |
| `TUNNEL_CUSTOM_URL_PATTERN` | — | Substring to match in custom tunnel stdout when extracting the public URL. |

### Pairing Environment Variables

The pairing store does not read environment variables directly. Its base directory is hardcoded to `~/.ironclaw/` (resolved via `dirs::home_dir()`). There is no configuration needed beyond invoking the correct channel name in CLI commands and channel handlers.

### Gateway Variables (relevant for tunnel use)

| Env Var | Default | Description |
|---------|---------|-------------|
| `GATEWAY_HOST` | `127.0.0.1` | Local address the gateway binds to. Change to `0.0.0.0` only if the network is trusted. |
| `GATEWAY_PORT` | `3000` | Local port the gateway listens on. This is the port tunnels forward to. |
| `GATEWAY_AUTH_TOKEN` | auto-generated | Bearer token required for all gateway API requests. If unset, a random 32-char token is generated and logged at startup. Generate with `openssl rand -hex 32` for production. **Never use default tokens in production.** |

---

## 7. Data Flow: End-to-End Mobile Access

The following summarizes how all pieces connect when a mobile user accesses IronClaw over a Cloudflare tunnel:

```
Mobile App
    |
    | HTTPS (*.example.com via Cloudflare)
    v
cloudflared (subprocess, managed by CloudflareTunnel)
    |
    | HTTP localhost:3000
    v
IronClaw Web Gateway (axum, bound to 127.0.0.1:3000)
    |
    | Bearer token validated by gateway auth middleware
    v
Agent Core (job queue, LLM, tools)
```

For channel-based access (Telegram):

```
Telegram user sends DM to bot
    |
    v
IronClaw Telegram channel handler
    |
    | PairingStore::is_sender_allowed()?
    |-- No --> upsert_request() --> send pairing code to user
    |                                  |
    |                          Owner runs: ironclaw pairing approve telegram <code>
    |                                  |
    |                          PairingStore::approve() --> adds to allowFrom
    |
    `-- Yes --> forward message to agent
```

---

## 8. Notes for Contributors

- All tunnel implementations share `SharedProcess` and `SharedUrl` via helpers in `mod.rs`. New backends should use `new_shared_process()` and `new_shared_url()` rather than constructing `Arc` wrappers manually.
- The `Tunnel` trait requires `Send + Sync`. Ensure any new backend does not hold `!Send` types (e.g., raw pointers, `Rc`).
- Pairing store operations are synchronous (blocking file I/O with `fs4` locking). This is intentional — pairing is a rare, low-frequency operation, and avoiding async file I/O keeps the implementation simple and auditable.
- Pairing file paths are channel-namespaced. A bug that allows a channel name to escape the `~/.ironclaw/` directory would be a path traversal vulnerability. The `safe_channel_key()` function in `store.rs` is the guard — any change to channel name handling must preserve its sanitization logic.
- The 3-slot pending request cap (`PAIRING_PENDING_MAX`) and 15-minute TTL (`PAIRING_PENDING_TTL_SECS`) are security-relevant constants. Raising them increases the brute-force window; do not change without understanding the threat model.
