# IronClaw Network Security Reference

This document catalogs every network-facing surface in IronClaw, its authentication mechanism, bind address, security controls, and known findings. Use this as the authoritative reference during code reviews that touch network-facing code.

**Last updated:** 2026-02-18

---

## Network Surface Inventory

| Listener | Default Port | Default Bind | Auth Mechanism | Config Env Var | Source File |
|----------|-------------|-------------|----------------|----------------|-------------|
| Web Gateway | 3000 | `127.0.0.1` | Bearer token (constant-time) | `GATEWAY_HOST`, `GATEWAY_PORT`, `GATEWAY_AUTH_TOKEN` | `src/channels/web/server.rs:158` |
| HTTP Webhook Server | 8080 | `0.0.0.0` | Shared secret (body field) | `HTTP_HOST`, `HTTP_PORT`, `HTTP_WEBHOOK_SECRET` | `src/channels/webhook_server.rs:55` |
| Orchestrator Internal API | 50051 | `127.0.0.1` (macOS/Win) / `0.0.0.0` (Linux) | Per-job bearer token (constant-time) | `ORCHESTRATOR_PORT` | `src/orchestrator/api.rs:106` |
| OAuth Callback Listener | 9876 | `127.0.0.1` | None (ephemeral, 5-min timeout) | N/A (hardcoded) | `src/cli/oauth_defaults.rs:89` |
| Sandbox HTTP Proxy | 0 (auto) | `127.0.0.1` | None (loopback only) | N/A (auto-assigned) | `src/sandbox/proxy/http.rs:100` |

---

## 1. Web Gateway

**Source:** `src/channels/web/server.rs`, `src/channels/web/auth.rs`

### Bind Address

Configurable via `GATEWAY_HOST` (default `127.0.0.1`) and `GATEWAY_PORT` (default `3000`). The gateway is designed as a local-first, single-user service.

**Reference:** `src/config.rs:872` (host default), `src/config.rs:880` (port default)

### Authentication

Bearer token middleware applied to all `/api/*` routes via `route_layer`. Token checked in two locations:

1. `Authorization: Bearer <token>` header (primary)
2. `?token=<token>` query parameter (fallback for SSE `EventSource` which cannot set headers)

Both paths use **constant-time comparison** via `subtle::ConstantTimeEq` (`ct_eq`).

**Reference:** `src/channels/web/auth.rs:31` (header), `src/channels/web/auth.rs:40` (query)

If `GATEWAY_AUTH_TOKEN` is not set, a random hex token is generated at startup.

### Unauthenticated Routes

| Route | Purpose |
|-------|---------|
| `/api/health` | Health check endpoint |
| `/` | Static HTML (embedded) |
| `/style.css` | Static CSS (embedded) |
| `/app.js` | Static JS (embedded) |

### CORS Policy

Restricted to same-origin. Only two origins are allowed:

- `http://<bind_ip>:<bind_port>`
- `http://localhost:<bind_port>`

Allowed methods: `GET`, `POST`, `PUT`, `DELETE`. Allowed headers: `Content-Type`, `Authorization`. Credentials allowed.

**Reference:** `src/channels/web/server.rs:281-300`

### WebSocket Origin Validation

The `/api/chat/ws` endpoint validates the `Origin` header before upgrading:

1. Origin header is **required** — missing Origin returns 403 (browsers always send it for WS upgrades; absence implies a non-browser client)
2. Origin host is extracted by stripping scheme and port, then compared **exactly** against `localhost`, `127.0.0.1`, and `[::1]`
3. Partial matches like `localhost.evil.com` are rejected because the check extracts the host portion before the first `:` or `/`

**Reference:** `src/channels/web/server.rs:557-591`

### Rate Limiting

Chat endpoint (`/api/chat/send`) enforces a sliding-window rate limit: **30 requests per 60 seconds** (global, not per-IP — single-user gateway).

**Reference:** `src/channels/web/server.rs:49-110` (RateLimiter), `src/channels/web/server.rs:147` (chat_rate_limiter), `src/channels/web/server.rs:364` (check)

### Body Limits

- Global: **1 MB** max request body (`DefaultBodyLimit::max(1024 * 1024)`)
- **Reference:** `src/channels/web/server.rs:308`

### Project File Serving

The `/projects/{project_id}/*` routes serve files from project directories. These are **behind auth middleware** to prevent unauthorized file access.

**Reference:** `src/channels/web/server.rs:270-277`

### Security Headers

The gateway sets the following security headers on all responses (via `SetResponseHeaderLayer::if_not_present`, so handlers can override):

- `X-Content-Type-Options: nosniff` — prevents MIME-sniffing
- `X-Frame-Options: DENY` — prevents clickjacking via iframes

**Reference:** `src/channels/web/server.rs:305-312`

---

## 2. HTTP Webhook Server

**Source:** `src/channels/webhook_server.rs`, `src/channels/http.rs`

### Bind Address

Configurable via `HTTP_HOST` (default `0.0.0.0`) and `HTTP_PORT` (default `8080`).

**WARNING:** The default bind address is `0.0.0.0`, meaning the webhook server listens on **all interfaces** by default. This is intentional (webhooks must be reachable from external services like Telegram/Slack), but operators should be aware of the exposure.

**Reference:** `src/config.rs:851` (host default), `src/config.rs:859` (port default)

### Authentication

Webhook secret is passed **in the JSON request body** (`secret` field), not as a header. The secret is compared using **constant-time** `subtle::ConstantTimeEq` (`ct_eq`).

The secret is required to start the channel — if `HTTP_WEBHOOK_SECRET` is not set, `start()` returns an error.

**Reference:** `src/channels/http.rs:174-200` (validation), `src/channels/http.rs:310-314` (required check)

### Rate Limiting

**60 requests per minute**, enforced via a mutex-protected sliding window.

**Reference:** `src/channels/http.rs:55` (MAX_REQUESTS_PER_MINUTE), `src/channels/http.rs:146-164` (enforcement)

### Body Limits

- JSON body: **64 KB** max (`MAX_BODY_BYTES`)
- Message content: **32 KB** max (`MAX_CONTENT_BYTES`)
- Pending synchronous responses: **100 max** (`MAX_PENDING_RESPONSES`)
- Synchronous response timeout: **60 seconds**

**Reference:** `src/channels/http.rs:49-58`

### Routes

| Route | Auth | Purpose |
|-------|------|---------|
| `/health` | None | Health check |
| `/webhook` | Webhook secret | Receive messages |

---

## 3. Orchestrator Internal API

**Source:** `src/orchestrator/api.rs`, `src/orchestrator/auth.rs`

### Bind Address

Platform-dependent:

- **macOS / Windows**: `127.0.0.1:<port>` — Docker Desktop routes `host.docker.internal` through its VM to `127.0.0.1`
- **Linux**: `0.0.0.0:<port>` — containers reach the host via the Docker bridge gateway (`172.17.0.1`), which is not loopback

Default port: `50051`.

**Reference:** `src/orchestrator/api.rs:93-102`

### Authentication

Per-job bearer tokens validated by `worker_auth_middleware`:

1. Tokens are **cryptographically random** (32 bytes, hex-encoded = 64 chars)
2. Tokens are **scoped to a specific job_id** — a token for job A cannot access endpoints for job B
3. Comparison uses **constant-time** `subtle::ConstantTimeEq`
4. Tokens are **ephemeral** (in-memory only, never persisted to disk or DB)
5. Tokens and associated credential grants are **revoked** when the container is cleaned up

**Reference:** `src/orchestrator/auth.rs:53-57` (create), `src/orchestrator/auth.rs:60-67` (validate), `src/orchestrator/auth.rs:99-109` (generate_token)

### Token Extraction

The middleware extracts the job UUID from the URL path (`/worker/{job_id}/...`) and validates the `Authorization: Bearer` header against the stored token for that specific job.

**Reference:** `src/orchestrator/auth.rs:117-137` (middleware), `src/orchestrator/auth.rs:140-147` (path extraction)

### Credential Grants

The orchestrator can grant per-job access to specific secrets from the encrypted secrets store. Grants are:

- Stored alongside the token in the `TokenStore`
- Scoped to specific `(secret_name, env_var)` pairs
- Revoked when the job token is revoked
- Decrypted on-demand when the worker requests `/worker/{job_id}/credentials`

**Reference:** `src/orchestrator/auth.rs:23-33` (CredentialGrant), `src/orchestrator/api.rs:377-431` (get_credentials_handler)

### Routes

| Route | Auth | Purpose |
|-------|------|---------|
| `/health` | None | Health check |
| `/worker/{job_id}/job` | Per-job token | Get job description |
| `/worker/{job_id}/llm/complete` | Per-job token | Proxy LLM completion |
| `/worker/{job_id}/llm/complete_with_tools` | Per-job token | Proxy LLM tool completion |
| `/worker/{job_id}/status` | Per-job token | Report worker status |
| `/worker/{job_id}/complete` | Per-job token | Report job completion |
| `/worker/{job_id}/event` | Per-job token | Send job events (SSE broadcast) |
| `/worker/{job_id}/prompt` | Per-job token | Poll for follow-up prompts |
| `/worker/{job_id}/credentials` | Per-job token | Retrieve decrypted credentials |

---

## 4. OAuth Callback Listener

**Source:** `src/cli/oauth_defaults.rs`

### Bind Address

Always binds to **loopback only**: `127.0.0.1:9876`. Falls back to `[::1]:9876` (IPv6 loopback) if IPv4 binding fails for reasons other than `AddrInUse`. If the port is already in use, the error is returned immediately (fail-fast).

**Reference:** `src/cli/oauth_defaults.rs:63` (port constant), `src/cli/oauth_defaults.rs:87-105` (bind logic)

### Lifecycle

The listener is **ephemeral** — it is started only when an OAuth flow is initiated (e.g., `ironclaw tool auth <name>`) and shut down after the callback is received or the timeout expires.

### Timeout

**5-minute timeout** (`Duration::from_secs(300)`). If the user does not complete the OAuth flow in the browser within 5 minutes, the listener shuts down.

**Reference:** `src/cli/oauth_defaults.rs:129`

### Security Controls

- **HTML escaping**: Provider names displayed in the landing page are HTML-escaped to prevent XSS (escapes `&`, `<`, `>`, `"`, `'`)
- **Error parameter checking**: The handler checks for `error=` in the callback query string before extracting the auth code
- **URL decoding**: Callback parameters are URL-decoded safely

**Reference:** `src/cli/oauth_defaults.rs:196-210` (html_escape)

### Built-in OAuth Credentials

Google OAuth client ID and secret are compiled into the binary (with compile-time override via `IRONCLAW_GOOGLE_CLIENT_ID` / `IRONCLAW_GOOGLE_CLIENT_SECRET`). As noted in the source, Google Desktop App client secrets are [not actually secret](https://developers.google.com/identity/protocols/oauth2/native-app) per Google's documentation.

**Reference:** `src/cli/oauth_defaults.rs:34-41`

---

## 5. Sandbox HTTP Proxy

**Source:** `src/sandbox/proxy/http.rs`, `src/sandbox/proxy/allowlist.rs`, `src/sandbox/proxy/policy.rs`

### Bind Address

Always binds to **`127.0.0.1`** (localhost only). Port is auto-assigned (port `0`). Falls back to `[::1]` (IPv6 loopback) if IPv4 is unavailable.

**Reference:** `src/sandbox/proxy/http.rs:100`

### Purpose

Acts as an HTTP/HTTPS proxy for Docker sandbox containers. Containers are configured with `http_proxy` / `https_proxy` environment variables pointing to this proxy, so all outbound HTTP traffic is routed through it.

### Domain Allowlisting

All requests are validated against a domain allowlist before being forwarded:

- **Empty allowlist = deny all** (fail-secure default)
- Supports exact matches and wildcard patterns (`*.example.com`)
- Validates URL scheme (HTTP/HTTPS only, rejects `ftp://`, `file://`, etc.)

**Reference:** `src/sandbox/proxy/allowlist.rs:96-153`

### HTTPS Tunneling (CONNECT)

- CONNECT requests for HTTPS tunneling are subject to the same allowlist
- **30-minute timeout** on established tunnels to prevent indefinite holds
- **No MITM**: the proxy cannot inspect or inject credentials into HTTPS traffic (by design — containers that need credentials must use the orchestrator's `/worker/{job_id}/credentials` endpoint)

**Reference:** `src/sandbox/proxy/http.rs:205-324` (CONNECT handling)

### Credential Injection (HTTP only)

For plain HTTP requests to allowed hosts, the proxy can inject credentials:

- Bearer tokens in `Authorization` header
- Custom headers (e.g., `X-API-Key`)
- Query parameters
- Credentials are resolved at request time from the encrypted secrets store
- Credentials never enter the container's environment or filesystem

**Reference:** `src/sandbox/proxy/http.rs:350-388`

### Hop-by-Hop Header Filtering

The proxy strips hop-by-hop headers to prevent header-based attacks: `connection`, `keep-alive`, `proxy-authenticate`, `proxy-authorization`, `te`, `trailers`, `transfer-encoding`, `upgrade`.

**Reference:** `src/sandbox/proxy/http.rs:443-456`

### Docker Container Security

Containers that use the proxy are configured with defense-in-depth:

| Control | Setting | Reference |
|---------|---------|-----------|
| Capabilities | Drop ALL, add only CHOWN | `src/sandbox/container.rs:280-284` |
| Privilege escalation | `no-new-privileges:true` | `src/sandbox/container.rs:283` |
| Root filesystem | Read-only (except FullAccess policy) | `src/sandbox/container.rs:286` |
| User | Non-root (UID 1000:1000) | `src/sandbox/container.rs:312` |
| Network | Bridge mode (isolated) | `src/sandbox/container.rs:279` |
| Tmpfs | `/tmp` (512 MB), `/home/sandbox/.cargo/registry` (1 GB) | `src/sandbox/container.rs:288-298` |
| Auto-remove | Enabled | `src/sandbox/container.rs:278` |
| Output limits | Configurable max stdout/stderr | `src/sandbox/container.rs:336-422` |
| Timeout | Enforced with forced container removal | `src/sandbox/container.rs:143-159` |

---

## Egress Controls

### WASM Tool HTTP Requests

WASM tools execute HTTP requests through the host runtime, subject to:

1. **Endpoint allowlist** — declared in `<tool>.capabilities.json`, validated by `AllowlistValidator`
   - Host matching (exact or wildcard)
   - Path prefix matching
   - HTTP method restriction
   - HTTPS required by default
   - Userinfo in URLs (`user:pass@host`) rejected to prevent allowlist bypass
   - Path traversal (`../`, `%2e%2e/`) normalized and blocked
   - Invalid percent-encoding rejected
   - **Reference:** `src/tools/wasm/allowlist.rs`

2. **Credential injection** — secrets injected at the host boundary by `CredentialInjector`
   - WASM code never sees actual credential values
   - Secrets must be in the tool's `allowed_secrets` list
   - Injection supports: Bearer header, Basic auth, custom header, query parameter
   - **Reference:** `src/tools/wasm/credential_injector.rs`

3. **Leak detection** — `LeakDetector` scans both outbound requests and inbound responses for secret patterns
   - Runs at two points: before sending and after receiving
   - Uses Aho-Corasick for fast multi-pattern matching
   - **Reference:** `src/safety/leak_detector.rs`

### Built-in HTTP Tool

The `http` tool (`src/tools/builtin/http.rs`) has its own SSRF protections:

| Protection | Details | Reference |
|-----------|---------|-----------|
| HTTPS only | Rejects `http://` URLs | `src/tools/builtin/http.rs:44-48` |
| Localhost blocked | Rejects `localhost` and `*.localhost` | `src/tools/builtin/http.rs:55-59` |
| Private IP blocked | Rejects RFC 1918, loopback, link-local, multicast, unspecified | `src/tools/builtin/http.rs:89-107` |
| DNS rebinding | Resolves hostname and checks all resolved IPs against blocklist | `src/tools/builtin/http.rs:72-84` |
| Cloud metadata | Blocks `169.254.169.254` (AWS/GCP metadata endpoint) | `src/tools/builtin/http.rs:97` |
| Redirect blocking | Returns error on 3xx responses (prevents SSRF via redirect) | `src/tools/builtin/http.rs:282-287` |
| Response size limit | **5 MB** max, enforced both via Content-Length header and streaming | `src/tools/builtin/http.rs:20`, `src/tools/builtin/http.rs:297-329` |
| Outbound leak scan | Scans URL, headers, and body for secrets before sending | `src/tools/builtin/http.rs:265-268` |
| Approval required | Requires user approval before execution | `src/tools/builtin/http.rs:356` |
| Timeout | 30 seconds default | `src/tools/builtin/http.rs:31` |
| No redirects | `redirect::Policy::none()` — redirects are not followed | `src/tools/builtin/http.rs:31` |

### MCP Client

MCP servers are external processes accessed via HTTP. The MCP client (`src/tools/mcp/client.rs`) uses `reqwest` with a 30-second timeout but has **no SSRF protections** — it connects to whatever URL is configured for the MCP server. This is by design: MCP servers are user-configured integrations, not untrusted destinations.

**Reference:** `src/tools/mcp/client.rs:68-71`

### Sandbox Domain Allowlists

Sandbox containers route all HTTP traffic through the proxy, which enforces a domain allowlist. The allowlist is built from:

1. A default set of domains (`src/sandbox/config.rs:134` — `default_allowlist()`)
2. Additional domains from `SANDBOX_EXTRA_DOMAINS` env var (comma-separated)

**Reference:** `src/config.rs:1473-1474`

---

## Authentication Mechanisms Summary

| Mechanism | Constant-Time | Used By | Reference |
|-----------|:------------:|---------|-----------|
| Gateway bearer token | Yes | Web gateway (header + query) | `src/channels/web/auth.rs:31,40` |
| Webhook shared secret | Yes | HTTP webhook (`ct_eq` comparison) | `src/channels/http.rs:176` |
| Per-job bearer token | Yes | Orchestrator worker API | `src/orchestrator/auth.rs:65` |
| OAuth callback | N/A | CLI OAuth flow (no auth, loopback-only) | `src/cli/oauth_defaults.rs:89` |
| Sandbox proxy | N/A | No auth (loopback-only, ephemeral) | `src/sandbox/proxy/http.rs:100` |

---

## Known Security Findings

### 1. ~~Webhook secret comparison is not constant-time~~ (Resolved)

**Severity:** Low
**Location:** `src/channels/http.rs:176`
**Status:** Resolved — webhook secret now uses `subtle::ConstantTimeEq` (`ct_eq`), consistent with web gateway and orchestrator auth.

### 2. No TLS at the application layer

**Severity:** Low (for local deployment)
**Details:** None of the listeners terminate TLS. All communication is plain HTTP.
**Mitigation:** The web gateway and OAuth callback bind to loopback by default. For production, users are expected to front the gateway with a reverse proxy (nginx, Caddy) or tunnel (Cloudflare, ngrok) that provides TLS.
**Recommendation:** Document the requirement for a TLS-terminating reverse proxy in deployment guides.

### 3. Orchestrator binds to `0.0.0.0` on Linux

**Severity:** Medium
**Location:** `src/orchestrator/api.rs:98-99`
**Details:** On Linux, the orchestrator API binds to all interfaces because Docker containers reach the host via the bridge gateway (`172.17.0.1`), not loopback. This means the API is reachable from any network interface on the host.
**Mitigation:** All `/worker/*` endpoints require per-job bearer tokens (constant-time, cryptographically random). The `/health` endpoint is the only unauthenticated route. Firewall rules should block external access to port 50051.
**Recommendation:** Document firewall requirements for Linux deployments. Consider binding to the Docker bridge IP (`172.17.0.1`) instead of `0.0.0.0`.

### 4. ~~HTTP webhook server binds to `0.0.0.0` by default~~ (Resolved)

**Severity:** Low
**Location:** `src/config.rs:851`, `src/main.rs`
**Status:** Resolved — a `tracing::warn!` is now emitted at startup when `HTTP_HOST` is `0.0.0.0`, advising operators to set `HTTP_HOST=127.0.0.1` to restrict to localhost.

### 5. ~~Missing security headers on web gateway~~ (Resolved)

**Severity:** Low
**Status:** Resolved — `X-Content-Type-Options: nosniff` and `X-Frame-Options: DENY` are now set on all responses via `SetResponseHeaderLayer::if_not_present`.

### 6. WebSocket connection limit

**Severity:** Info
**Details:** The SSE broadcaster enforces a subscriber limit. When exceeded, WebSocket upgrades are rejected with a warning log. The exact limit is configured in the `SseManager`.
**Reference:** `src/channels/web/ws.rs:76-77`

---

## Review Checklist for Network Changes

Use this checklist for any PR that adds or modifies network-facing code.

### New Listener

- [ ] **Bind address**: Does it bind to loopback (`127.0.0.1`) or all interfaces (`0.0.0.0`)? Justify if `0.0.0.0`.
- [ ] **Port configuration**: Is the port configurable via env var? Is a sensible default set?
- [ ] **Authentication**: Is auth required? If yes, is it constant-time? If no, why not?
- [ ] **Rate limiting**: Is there a rate limiter? What are the limits?
- [ ] **Body size limit**: Is `DefaultBodyLimit` (or equivalent) set?
- [ ] **Graceful shutdown**: Does the listener support graceful shutdown?
- [ ] **Inventory update**: Is this document updated with the new listener?

### New Route on Existing Listener

- [ ] **Auth layer**: Is the route behind the auth middleware? If public, why?
- [ ] **Input validation**: Are path parameters, query parameters, and body fields validated?
- [ ] **Error responses**: Do error responses avoid leaking internal details?

### Egress (Outbound HTTP)

- [ ] **SSRF protection**: Does the code block private IPs, localhost, and cloud metadata endpoints?
- [ ] **DNS rebinding**: Are resolved IPs checked (not just the hostname)?
- [ ] **Redirect handling**: Are redirects blocked or validated?
- [ ] **Response size**: Is there a max response size?
- [ ] **Timeout**: Is a request timeout set?
- [ ] **Leak detection**: Is the outbound request scanned for secrets?

### Credential Handling

- [ ] **Constant-time comparison**: Are secrets compared with `subtle::ConstantTimeEq`?
- [ ] **No logging**: Are credentials excluded from log messages?
- [ ] **Ephemeral storage**: Are tokens stored in memory only (not persisted)?
- [ ] **Scope**: Are credentials scoped to the minimum necessary (per-job, per-tool)?
- [ ] **Revocation**: Are credentials revoked when no longer needed?

### Container / Sandbox

- [ ] **Capabilities**: Are all capabilities dropped except what's needed?
- [ ] **Filesystem**: Is the root filesystem read-only?
- [ ] **User**: Does the container run as non-root?
- [ ] **Network**: Is network access routed through the proxy?
- [ ] **Timeout**: Is there an execution timeout with forced cleanup?
- [ ] **Output limits**: Are stdout/stderr capped?
