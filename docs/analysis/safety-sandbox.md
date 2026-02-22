# IronClaw Codebase Analysis — Safety & Sandbox Security Model

> Updated: 2026-02-22 | Version: v0.9.0

## 1. Overview

IronClaw implements a defense-in-depth security model across multiple independent layers. Each layer is designed to contain a distinct class of threat, so that a bypass of one layer does not compromise the whole system.

The primary threats addressed are:

- **Prompt injection**: External data (web pages, files, tool outputs) attempting to override LLM instructions or hijack the agent's behavior.
- **Credential exfiltration**: Malicious WASM tool code or injected instructions attempting to read and exfiltrate API keys, tokens, or private keys.
- **Server-Side Request Forgery (SSRF)**: Tool code attempting to reach internal services, cloud metadata endpoints, or unauthorized external hosts via outbound HTTP.
- **Malicious tool code**: Dynamically-built or third-party WASM tools with hostile intent running inside the agent process.
- **Cross-job lateral movement**: A container for Job A attempting to interfere with Job B's resources or token.

The model is layered: safety checks run on LLM inputs and outputs; WASM tools are isolated inside a capability sandbox with a network proxy; shell execution and code-running jobs run inside isolated Docker containers; and every container-to-host channel is authenticated with a per-job bearer token.

---

## 2. Security Architecture Diagram

```
[User Input]
    |
    v
[Hook Pre-intercept]  (planned: channel-level input hooks)
    |
    v
[Safety Layer]                              src/safety/
    |-- Sanitizer      strip malicious content, escape special tokens
    |-- Validator      policy enforcement, length/encoding checks
    |-- Leak Detector  catch credential exfiltration attempts in tool output
    +-- Post-processor wrap output in <tool_output> XML for structural boundary
    |
    v
[Tool Execution]
    |
    |-- WASM Sandbox (capability-based)     src/tools/wasm/
    |       |-- Fuel metering (CPU limit)
    |       |-- Memory limits
    |       |-- Capability allowlist        capabilities.rs
    |       |-- Credential injection        credential_injector.rs
    |       +-- Leak scan on outbound HTTP  leak_detector.rs
    |
    |-- SSRF Proxy (HTTP allowlist)         src/sandbox/proxy/
    |       |-- Domain allowlist            allowlist.rs
    |       |-- Path allowlist              policy.rs
    |       |-- Credential injection        http.rs
    |       +-- CONNECT tunnel validation
    |
    +-- Docker Sandbox (shell / code exec)  src/sandbox/ + src/orchestrator/
            |-- Isolated container (ephemeral)
            |-- Per-job auth token (in-memory, never persisted)
            |-- Network isolation (bridge network, proxy required)
            |-- Non-root user (UID 1000)
            |-- Capability drop (cap_drop: ALL, add back only CHOWN)
            +-- Bind mount restricted to ~/.ironclaw/projects/
    |
    v
[Hook Post-intercept]  (planned: output hooks)
```

---

## 3. Safety Layer (`src/safety/`)

The safety layer is the first line of defense against prompt injection. It applies to all external data before that data reaches the LLM context. The `SafetyLayer` struct in `src/safety/mod.rs` wraps four sub-components and exposes them through a single `sanitize_tool_output` / `validate_input` / `wrap_for_llm` API.

### 3.1 Sanitizer (`src/safety/sanitizer.rs`)

The sanitizer detects and neutralizes prompt injection patterns in external content. It uses two detection mechanisms:

**Aho-Corasick fast string matching** (case-insensitive) for a static list of known bad phrases:

| Pattern | Severity | Threat |
|---|---|---|
| `ignore previous` | High | Instruction override |
| `ignore all previous` | Critical | Full context override |
| `disregard` | Medium | Instruction override |
| `forget everything` | High | Context reset |
| `you are now` | High | Role manipulation |
| `act as` | Medium | Role manipulation |
| `pretend to be` | Medium | Role manipulation |
| `system:` | Critical | System message injection |
| `assistant:` | High | Response injection |
| `user:` | High | User message injection |
| `<\|` / `\|>` | Critical | Special token injection (GPT-style) |
| `[INST]` / `[/INST]` | Critical | Instruction token injection (Llama-style) |
| `new instructions` | High | Context replacement |
| `updated instructions` | High | Context replacement |
| `` ```system `` | High | Code block instruction injection |
| `` ```bash\nsudo `` | Medium | Dangerous command injection |

**Regex patterns** for structured attacks:

| Pattern | Severity | Threat |
|---|---|---|
| `base64[:\s]+[A-Za-z0-9+/=]{50,}` | Medium | Encoded payload |
| `eval\s*\(` | High | Code evaluation attempt |
| `exec\s*\(` | High | Code execution attempt |
| `\x00` | Critical | Null byte injection |

**Escaping behavior**: When a Critical-severity match is found, the sanitizer escapes the entire content — stripping null bytes, backslash-escaping `<|` / `|>` / `[INST]` tokens, and prefixing role lines (`system:`, `user:`, `assistant:`) with `[ESCAPED]`. For non-critical matches, warnings are emitted but content is passed through.

### 3.2 Validator (`src/safety/validator.rs`)

The validator performs structural checks on inputs and tool parameters:

- **Empty input**: rejected with `ValidationErrorCode::Empty`
- **Length bounds**: max 100,000 bytes by default; min 1 byte (configurable via `with_max_length` / `with_min_length`)
- **Null bytes**: rejected with `ValidationErrorCode::InvalidEncoding`
- **Forbidden patterns**: caller-configurable set of lowercase substrings; match returns `ValidationErrorCode::ForbiddenContent`
- **Whitespace ratio**: inputs with >90% whitespace and length >100 bytes generate a warning (potential padding attack)
- **Excessive repetition**: runs of more than 20 identical characters generate a warning (potential obfuscation)

The `validate_tool_params` method recursively walks a `serde_json::Value` and applies the same checks to all string leaf values. This means a WASM tool cannot smuggle injection payloads through nested JSON parameters.

### 3.3 Leak Detector (`src/safety/leak_detector.rs`)

The leak detector scans content for recognizable secret patterns at two points: before outbound HTTP requests from WASM tools (to prevent exfiltration), and in tool outputs before they reach the LLM (to prevent accidental exposure).

Each pattern specifies a `LeakAction`:

| Pattern | Severity | Action |
|---|---|---|
| OpenAI API key (`sk-proj-...`) | Critical | Block |
| Anthropic API key (`sk-ant-api...`) | Critical | Block |
| AWS Access Key ID (`AKIA...`) | Critical | Block |
| GitHub classic token (`ghp_...`, `gho_...`, etc.) | Critical | Block |
| GitHub fine-grained PAT (`github_pat_...`) | Critical | Block |
| Stripe key (`sk_live_...`, `sk_test_...`) | Critical | Block |
| NEAR AI session token (`sess_...`) | Critical | Block |
| PEM RSA private key header | Critical | Block |
| SSH private key header (OpenSSH/EC/DSA) | Critical | Block |
| Google API key (`AIza...`) | High | Block |
| Slack token (`xox[baprs]-...`) | High | Block |
| Twilio key (`SK[a-f0-9]{32}`) | High | Block |
| SendGrid key (`SG.[...].[...]`) | High | Block |
| Bearer token (`Bearer <token>`) | High | Redact |
| Authorization header with token | High | Redact |
| High-entropy 64-char hex strings | Medium | Warn |

The three actions mean:

- **Block**: `scan_and_clean` returns `Err`, tool output is replaced with `[Output blocked due to potential secret leakage]`.
- **Redact**: matched text is replaced in-place with `[REDACTED]`, content otherwise passes through.
- **Warn**: the match is logged via `tracing::warn!`, content passes through unchanged.

For HTTP request scanning (`scan_http_request`), the URL, all header values, and the request body are each scanned. The body uses `String::from_utf8_lossy` to prevent an attacker from prepending a non-UTF-8 byte to bypass scanning.

Performance: a prefix-based Aho-Corasick matcher rapidly eliminates most of the content before the more expensive regex patterns are applied.

### 3.4 Policy (`src/safety/policy.rs`)

The policy layer applies a set of named rules with four possible actions — `Warn`, `Block`, `Review`, `Sanitize` — based on regex matching. Default rules in `Policy::default()`:

| Rule ID | Pattern | Severity | Action |
|---|---|---|---|
| `system_file_access` | `/etc/passwd`, `/etc/shadow`, `.ssh/`, `.aws/credentials` | Critical | Block |
| `crypto_private_key` | private key / seed phrase / mnemonic + 64-char hex | Critical | Block |
| `sql_pattern` | `DROP TABLE`, `DELETE FROM`, `INSERT INTO`, `UPDATE ... SET` | Medium | Warn |
| `shell_injection` | `; rm -rf`, `; curl ... \| sh`, backtick subshells | Critical | Block |
| `excessive_urls` | 10+ URLs in sequence | Low | Warn |
| `encoded_exploit` | `base64_decode`, base64-eval patterns, `atob(` | High | Sanitize |
| `obfuscated_string` | 500+ character run without spaces | Medium | Warn |

### 3.5 Output Wrapping (`sanitize_tool_output` + `wrap_for_llm`)

After sanitization, the `SafetyLayer.sanitize_tool_output` method applies checks in this order:

1. Truncate if output exceeds `max_output_length` (default 100,000 bytes, configurable via `SAFETY_MAX_OUTPUT_LENGTH`).
2. Run `LeakDetector.scan_and_clean` — block or redact secrets.
3. Run `Policy.check` — if any rule returns `Block`, replace output with a fixed string.
4. If injection check is enabled (or policy requires sanitize), run `Sanitizer.sanitize`.

The final content is then wrapped by `wrap_for_llm`:

```xml
<tool_output name="tool_name" sanitized="true">
[escaped content]
</tool_output>
```

XML attribute and content escaping (`&`, `<`, `>`, `"`) is applied. This structural boundary makes it syntactically clear to the LLM that the enclosed content is untrusted external data, not trusted instructions.

---

## 4. WASM Sandbox (`src/tools/wasm/`)

WASM tools run inside a `wasmtime` sandbox. Each tool call gets a fresh invocation with hard resource limits. The sandbox enforces the principle of least privilege: WASM modules have no access to the host environment by default. Every capability must be explicitly declared and granted.

**Fuel metering (CPU limit)**: The wasmtime engine uses fuel to cap computational work per tool call. When fuel is exhausted, execution is terminated with a trap. This prevents runaway loops or intentionally slow tools from blocking the agent.

**Memory limits**: Maximum WASM linear memory is configured per-module via `src/tools/wasm/limits.rs`. A module that attempts to grow beyond the limit receives a WASM memory trap.

### 4.1 Capabilities (`src/tools/wasm/capabilities.rs`)

The `Capabilities` struct defines four opt-in capability types. By default, all fields are `None`:

```
Capabilities {
    workspace_read:  Option<WorkspaceCapability>   // read files from agent workspace
    http:            Option<HttpCapability>        // make outbound HTTP requests
    tool_invoke:     Option<ToolInvokeCapability>  // call other tools by alias
    secrets:         Option<SecretsCapability>     // check if named secrets exist
}
```

**WorkspaceCapability**: grants read access to workspace paths filtered by `allowed_prefixes`. An empty prefix list allows all paths (within safety constraints). The actual read implementation is injected at runtime via the `WorkspaceReader` trait, decoupling the WASM runtime from the workspace module.

**HttpCapability**: grants outbound HTTP access with the following fields:

- `allowlist: Vec<EndpointPattern>` — each pattern has a `host` (supports `*.example.com` wildcards), optional `path_prefix`, and optional method restriction
- `credentials: HashMap<String, CredentialMapping>` — credential mappings keyed by name; credentials are injected at the host boundary, never passed to WASM
- `rate_limit: RateLimitConfig` — defaults to 60 requests/minute, 1000/hour
- `max_request_bytes: usize` — default 1 MB
- `max_response_bytes: usize` — default 10 MB
- `timeout: Duration` — default 30 seconds

**ToolInvokeCapability**: grants the ability to call other tools, but only by alias. WASM tools never see the real tool name. The `resolve_alias` method translates an alias to a real tool name; any alias not in the map is denied. Rate limiting applies to alias calls too.

**SecretsCapability**: grants the ability to check whether a named secret exists. WASM tools can never read secret values through this capability. The `is_allowed` method supports glob patterns: `"openai_*"` allows checking `openai_key`, `openai_org`, etc.

All capability checks happen on the host side before any WASM host function executes.

### 4.2 Credential Injection (`src/tools/wasm/credential_injector.rs`)

The `CredentialInjector` resolves secrets from the `SecretsStore` (AES-256-GCM encrypted on disk) and injects them into HTTP requests at the host boundary. The WASM module code never receives the actual credential value.

**Injection flow**:

1. WASM calls the `http_request` host function with a URL, method, headers, and body.
2. The host function extracts the target host from the URL.
3. `CredentialInjector.inject(user_id, host, store)` is called:
   a. `find_credentials_for_host` matches the host against all `CredentialMapping.host_patterns`.
   b. For each match, `is_secret_allowed` checks the secret name against the WASM tool's allowed list (from `SecretsCapability.allowed_names`). This is a second gate — only secrets explicitly listed in the tool's capability declaration are accessible.
   c. `store.get_decrypted(user_id, secret_name)` decrypts the secret from the AES-256-GCM store.
   d. The decrypted value is injected into the request as a header or query parameter per `CredentialLocation`.
4. The injected request is forwarded to the network.
5. The WASM module receives only the response body and status code.

**Supported injection locations**:

- `AuthorizationBearer`: adds `Authorization: Bearer <value>`
- `AuthorizationBasic { username }`: encodes `username:value` as base64 and adds `Authorization: Basic <encoded>`
- `Header { name, prefix }`: adds a custom header with optional prefix (e.g., `X-Api-Key: <value>`)
- `QueryParam { name }`: appends a query parameter

The WASM tool schema exposed to the LLM never references credential names or values. The LLM cannot reason about or instruct the agent to extract credentials through this path.

---

## 5. SSRF Proxy (`src/sandbox/proxy/`)

All outbound HTTP from Docker worker containers is routed through a host-side HTTP proxy. The proxy validates every request against a domain allowlist before forwarding and injects credentials into approved requests.

The proxy runs as a standalone Tokio task inside the main agent process. It listens on a configurable port (default auto-assigned, set via `SANDBOX_PROXY_PORT`). Containers are started with the `http_proxy` / `https_proxy` environment variables pointing to `host.docker.internal:<port>`.

### 5.1 Allowlist Format (`src/sandbox/proxy/allowlist.rs`)

`DomainAllowlist` holds a `Vec<DomainPattern>`. Each pattern is either:

- **Exact**: `"api.example.com"` — only that hostname matches.
- **Wildcard**: `"*.example.com"` — the base domain and all subdomains match (e.g., `api.example.com`, `v2.api.example.com`, `example.com`).

Matching is case-insensitive. An empty allowlist denies all requests. `DomainValidationResult` is either `Allowed` or `Denied(reason_string)`.

**Default allowlist** (from `src/sandbox/config.rs`):

```
Package registries:   crates.io, static.crates.io, index.crates.io,
                      registry.npmjs.org, proxy.golang.org,
                      pypi.org, files.pythonhosted.org

Documentation:        docs.rs, doc.rust-lang.org, nodejs.org,
                      go.dev, docs.python.org

Version control:      github.com, raw.githubusercontent.com,
                      api.github.com, codeload.github.com

Common APIs:          api.openai.com, api.anthropic.com, api.near.ai
```

Per-tool or per-job allowlist entries can be added via `NetworkProxyBuilder.allow_domain()` or `with_allowlist()`.

### 5.2 HTTP and HTTPS Handling (`src/sandbox/proxy/http.rs`)

For plain HTTP requests, the proxy calls `handle_request`:

1. Parses the target URL from the `Request-URI`.
2. Constructs a `NetworkRequest` and passes it to the `NetworkPolicyDecider`.
3. If `Deny`: returns HTTP 403 Forbidden with the denial reason.
4. If `Allow` or `AllowWithCredentials`: calls `forward_request`, which copies headers (excluding hop-by-hop headers), optionally injects credentials, copies the body, and sends via `reqwest::Client`.

For HTTPS, the proxy handles the `CONNECT` method in `handle_connect`:

1. Extracts `host:port` from the CONNECT target.
2. Validates the host against the allowlist.
3. If allowed: returns `200 OK` to signal the client to begin TLS negotiation, then spawns a bidirectional TCP tunnel with a 30-minute timeout using `tokio::io::copy_bidirectional`.
4. If denied: returns `403 Forbidden`.

**Credential injection limitation with CONNECT**: The proxy cannot inspect or modify TLS-encrypted traffic flowing through a CONNECT tunnel. Credentials that must be injected into HTTPS headers use the orchestrator's `GET /worker/{id}/credentials` endpoint instead — the container fetches them and sets them as environment variables before starting the execution loop.

Hop-by-hop headers (`connection`, `keep-alive`, `proxy-authenticate`, `proxy-authorization`, `te`, `trailers`, `transfer-encoding`, `upgrade`) are stripped before forwarding.

### 5.3 Policy Enforcement (`src/sandbox/proxy/policy.rs`)

`NetworkPolicyDecider` is a trait:

```rust
async fn decide(&self, request: &NetworkRequest) -> NetworkDecision;
```

`NetworkDecision` is one of:

- `Allow` — forward as-is.
- `AllowWithCredentials { secret_name, location }` — forward with credential injection.
- `Deny { reason }` — reject with HTTP 403.

`DefaultPolicyDecider` implements the trait using a `DomainAllowlist` and a `Vec<CredentialMapping>`. It first checks the allowlist; if the domain is not listed, it returns `Deny`. If the domain is listed and matches a credential mapping, it returns `AllowWithCredentials`. Otherwise it returns `Allow`.

Credential mappings support glob host patterns (`*.example.com`). The `find_credential` method iterates all mappings and calls `host_matches_pattern`.

Three built-in deciders are provided for different policy levels:

- `DefaultPolicyDecider` — allowlist + credentials (used for `ReadOnly` and `WorkspaceWrite` policies)
- `AllowAllDecider` — unrestricted (used for `FullAccess` policy; no proxy enforcement)
- `DenyAllDecider` — deny everything (useful for testing or fully air-gapped configurations)

---

## 6. Sandbox Policies (`src/sandbox/config.rs`)

Three policy levels control both filesystem access and network access for containers:

| Policy | Filesystem | Network | Typical Use |
|---|---|---|---|
| `ReadOnly` | `/workspace` bind-mounted read-only | Proxied, allowlist only | Code review, analysis, read-only fetches |
| `WorkspaceWrite` | `/workspace` bind-mounted read-write | Proxied, allowlist only | Code generation, building, test runs |
| `FullAccess` | Full host filesystem | Unrestricted (no proxy) | Trusted admin tasks; should be used sparingly |

The `SandboxPolicy::has_full_network()` method returns `true` only for `FullAccess`. The `NetworkProxyBuilder` checks this flag and uses `AllowAllDecider` instead of `DefaultPolicyDecider` when it is set.

Volume bind mounts are validated by `validate_bind_mount_path` in `src/orchestrator/job_manager.rs`. The function canonicalizes the supplied path and verifies it is under `~/.ironclaw/projects/` before passing it to the Docker API. Paths outside that prefix are rejected with an error.

---

## 7. Docker Orchestrator (`src/orchestrator/`)

The orchestrator manages container lifecycle and provides an internal HTTP API for worker-to-host communication.

### 7.1 Container Lifecycle

`ContainerJobManager.create_job` is the entry point for spawning a new sandboxed job:

1. Generate a per-job auth token via `TokenStore.create_token(job_id)` (32 cryptographically random bytes, hex-encoded, 64 characters).
2. Store any credential grants via `TokenStore.store_grants(job_id, grants)`.
3. Call `create_job_inner`:
   a. Connect to Docker daemon (connection is cached after first use).
   b. Determine `orchestrator_url` (`172.17.0.1:<port>` on Linux, `host.docker.internal:<port>` on macOS/Windows).
   c. Build container env vars: `IRONCLAW_WORKER_TOKEN`, `IRONCLAW_JOB_ID`, `IRONCLAW_ORCHESTRATOR_URL`, and optionally `IRONCLAW_WORKSPACE`.
   d. Validate and create volume bind mount (only `~/.ironclaw/projects/` is permitted).
   e. For `ClaudeCode` mode: inject `ANTHROPIC_API_KEY` or `CLAUDE_CODE_OAUTH_TOKEN` as an env var.
   f. Set Linux capabilities: `cap_drop: ALL`, `cap_add: CHOWN`.
   g. Set `security_opt: no-new-privileges:true`.
   h. Set tmpfs for `/tmp` (512 MB, no persistence).
   i. Set `user: 1000:1000` (non-root).
   j. Set memory limit and CPU shares.
   k. Create and start the container.
4. On any failure, revoke the token and remove the handle.

**Container cleanup**: `stop_job` sends a `stop_container` with 10 seconds grace, then `remove_container --force`, and revokes the token. Containers cannot persist between jobs. `complete_job` does the same cleanup when the worker self-reports completion.

**Two job modes** (`JobMode`):

- `Worker`: runs `ironclaw worker --job-id <uuid> --orchestrator-url <url>` — a standard agent loop with proxied LLM calls and a limited tool set.
- `ClaudeCode`: runs `ironclaw claude-bridge --job-id <uuid> ...` — spawns the `claude` CLI inside the container and streams NDJSON events back to the orchestrator.

### 7.2 Internal API (`src/orchestrator/api.rs`)

The orchestrator exposes an internal HTTP API on a separate port (default 50051, not accessible from outside the host). All `/worker/` routes are protected by `worker_auth_middleware`:

| Endpoint | Method | Purpose |
|---|---|---|
| `/worker/{id}/job` | GET | Worker fetches job description |
| `/worker/{id}/llm/complete` | POST | Proxy LLM completion (no tools) |
| `/worker/{id}/llm/complete_with_tools` | POST | Proxy LLM completion with tool calls |
| `/worker/{id}/status` | POST | Worker reports iteration status |
| `/worker/{id}/complete` | POST | Worker reports job completion |
| `/worker/{id}/event` | POST | Worker sends a streaming event |
| `/worker/{id}/prompt` | GET | Worker polls for follow-up prompts |
| `/worker/{id}/credentials` | GET | Worker fetches granted credentials |
| `/health` | GET | Unauthenticated liveness check |

On Linux, the API binds to `0.0.0.0:<port>` because containers reach the host via the Docker bridge gateway (`172.17.0.1`), not loopback. The `worker_auth_middleware` is the authentication gate for all `/worker/` endpoints on Linux.

On macOS and Windows, Docker Desktop routes `host.docker.internal` through its VM to `127.0.0.1`, so the API binds to `127.0.0.1:<port>` and the loopback interface provides an additional isolation layer.

---

## 8. Worker Runtime (`src/worker/`)

The worker binary runs inside Docker containers. It connects to the orchestrator over HTTP and uses a `ProxyLlmProvider` so that LLM calls never leave the container directly — all LLM traffic is forwarded through the orchestrator's `/worker/{id}/llm/` endpoints.

**Startup sequence** (`WorkerRuntime::new`):

1. Read `IRONCLAW_WORKER_TOKEN` from environment.
2. Create `WorkerHttpClient` with the token and orchestrator URL.
3. Instantiate `ProxyLlmProvider` backed by the HTTP client.
4. Instantiate `SafetyLayer` with `injection_check_enabled: true`.
5. Register only container-safe tools via `ToolRegistry::register_container_tools()` — shell, read_file, write_file, list_dir, apply_patch. Network tools requiring direct API access are not available inside the container.

**Execution loop** (`WorkerRuntime::run`):

1. Fetch job description from orchestrator.
2. Fetch credential grants from `/worker/{id}/credentials` and store in `extra_env` (a `HashMap` passed to child processes via `Command::envs()`, not via `std::env::set_var` which would be unsafe in a multi-threaded tokio runtime).
3. Run the reasoning loop up to `max_iterations` (default 50) times, with a total `timeout` (default 10 minutes).
4. Each tool output is passed through `SafetyLayer.sanitize_tool_output` and `wrap_for_llm` before being added to the LLM context.
5. Report completion or timeout to the orchestrator.

### 8.1 Claude Bridge (`src/worker/claude_bridge.rs`)

When a job runs in `ClaudeCode` mode, the `ClaudeBridgeRuntime` is started instead of `WorkerRuntime`. It:

1. Copies auth files from a read-only host mount at `/home/sandbox/.claude-host` into the writable `/home/sandbox/.claude` (if the mount is present). Symlinks are skipped to prevent following links outside the mount.
2. Writes a project-level `/workspace/.claude/settings.json` with an explicit tool allowlist (`permissions.allow`). This replaces `--dangerously-skip-permissions` with a defense-in-depth approach: only listed tools auto-approve; unknown tools would time out harmlessly in a non-interactive container.
3. Fetches the job description from the orchestrator.
4. Fetches credential grants and injects them into the child process via `Command::envs()`.
5. Spawns `claude -p "<task>" --output-format stream-json --max-turns <n>`.
6. Reads NDJSON output line by line and forwards each event (`message`, `tool_use`, `tool_result`, `result`) to the orchestrator via `POST /worker/{id}/event`.
7. Polls for follow-up prompts via `GET /worker/{id}/prompt` (2-second intervals). If a prompt arrives, it resumes the Claude session with `--resume <session_id>`.

### 8.2 Proxy LLM (`src/worker/proxy_llm.rs`)

`ProxyLlmProvider` implements the `LlmProvider` trait and routes all calls through `WorkerHttpClient`:

- `complete(request)` sends `POST /worker/{id}/llm/complete`
- `complete_with_tools(request)` sends `POST /worker/{id}/llm/complete_with_tools`

The worker holds no LLM API keys. The orchestrator holds the real credentials and forwards requests to the actual LLM provider. Cost tracking also happens on the orchestrator side; `cost_per_token()` returns zeros.

---

## 9. Auth Token Security (`src/orchestrator/auth.rs`)

Per-job auth tokens enforce job isolation at the orchestrator API layer.

**Token generation**: 32 bytes from `rand::thread_rng()` (a CSPRNG on all supported platforms), hex-encoded to 64 ASCII characters. Tokens are generated fresh for each job and stored only in the in-memory `TokenStore` — never logged, serialized, or written to the database.

**Token validation** uses the `subtle` crate's `ConstantTimeEq` trait:

```rust
stored.as_bytes().ct_eq(token.as_bytes()).into()
```

Constant-time comparison prevents timing side-channel attacks. An attacker cannot determine the correct token by measuring how long a failed comparison takes.

**Job scoping**: The middleware `worker_auth_middleware` extracts the `{job_id}` UUID from the URL path and validates the bearer token against that specific job's entry in the token map. A token issued for Job A will fail validation on a Job B endpoint, even if both tokens are valid. This prevents a compromised container from accessing another job's data or credentials.

**Credential grants**: `CredentialGrant` pairs map a `secret_name` (stored in `SecretsStore`) to an `env_var` name the container expects. Grants are stored alongside the token and revoked atomically when the token is revoked. Only secrets explicitly listed in the grant are accessible via the `/credentials` endpoint.

**Token revocation**: `TokenStore.revoke(job_id)` removes both the token and all credential grants for that job in a single write-lock operation.

**Gateway auth**: The web-facing `GATEWAY_AUTH_TOKEN` is a separate token used by external HTTP clients. It is compared using constant-time equality in the web gateway's auth middleware.

---

## 10. Container Security Hardening (`Dockerfile.worker`)

The worker image (`Dockerfile.worker`) applies several hardening measures at image build time:

- **Non-root user**: `useradd -m -u 1000 -s /bin/bash sandbox` and `USER sandbox`. The container runs as UID 1000, matching the orchestrator's `user: 1000:1000` container config.
- **Multi-stage build**: The Rust builder stage compiles the binary; the final stage is `debian:bookworm-slim` with no Rust toolchain exposed in the production image layers.
- **Workspace ownership**: `/workspace` is owned by `sandbox`, preventing writes to other filesystem locations in the default configuration.
- **`~/.claude` directory**: pre-created and owned by `sandbox` so Claude Code can write state files without requiring elevated privileges.

At runtime, the orchestrator applies additional hardening via `HostConfig`:

```
cap_drop: ["ALL"]                          Drop all Linux capabilities
cap_add:  ["CHOWN"]                        Add back only what is needed
security_opt: ["no-new-privileges:true"]   Prevent privilege escalation via setuid
tmpfs: {"/tmp": "size=512M"}               Ephemeral /tmp, no persistence across runs
memory: <limit_bytes>                      Hard memory cap enforced by cgroup
cpu_shares: 1024                           Relative CPU weight
network_mode: "bridge"                     Containers cannot reach each other's loopback
```

---

## 11. Configuration Reference

| Env Var | Default | Description |
|---|---|---|
| `SAFETY_MAX_OUTPUT_LENGTH` | `100000` | Maximum tool output length in bytes before truncation |
| `SAFETY_INJECTION_CHECK_ENABLED` | `true` | Enable prompt injection scanning on tool outputs |
| `SANDBOX_ENABLED` | `true` | Enable Docker sandbox for tool execution (requires Docker) |
| `SANDBOX_IMAGE` | `ironclaw-worker:latest` | Docker image for worker containers |
| `SANDBOX_MEMORY_LIMIT_MB` | `2048` | Memory limit per container in megabytes |
| `SANDBOX_TIMEOUT_SECS` | `120` | Default execution timeout per container in seconds |
| `SANDBOX_CPU_SHARES` | `1024` | Relative CPU weight for containers |
| `SANDBOX_EXTRA_DOMAINS` | unset | Comma-separated domains added to default allowlist |
| `SANDBOX_POLICY` | `readonly` | Sandbox policy: `readonly`, `workspace_write`, or `full_access` |
| `GATEWAY_AUTH_TOKEN` | auto-generated | Bearer token for web gateway API access. If unset, a random 32-char token is generated and logged at startup. Strongly recommended to set explicitly for repeatable deployments. |
| `GATEWAY_ENABLED` | `true` | Enable the web-facing gateway |
| `GATEWAY_HOST` | `127.0.0.1` | Bind address for the web gateway |
| `GATEWAY_PORT` | `3000` | Port for the web gateway |
| `CLAUDE_CODE_ENABLED` | `false` | Enable ClaudeCode bridge mode for sandbox jobs |
| `CLAUDE_CODE_MODEL` | `sonnet` | Model passed to the `claude` CLI |
| `CLAUDE_CODE_MAX_TURNS` | `50` | Max agentic turns per Claude Code session |
| `CLAUDE_CONFIG_DIR` | `~/.claude` | Host config dir for Claude credential extraction |

**Recommended token generation**:

```zsh
openssl rand -hex 32
```

Use the output as `GATEWAY_AUTH_TOKEN`. The 32-byte (64-hex-character) length matches the per-job token format used internally and provides 256 bits of entropy.
