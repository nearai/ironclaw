# IronClaw Tool System — Developer Reference

Version: v0.12.0
Source: `src/tools/`

---

## 1. Overview

The tool system is the primary mechanism by which the IronClaw agent interacts with the
world. Every capability — reading a file, making an HTTP request, spawning a job, calling
an MCP server — is expressed as a `Tool` implementation registered in the `ToolRegistry`.

### Tool Categories

| Category | Source | Domain | Description |
|----------|--------|--------|-------------|
| Core utilities | `builtin/` | Orchestrator | echo, time, json, http |
| Filesystem | `builtin/file.rs` | Container | read_file, write_file, list_dir, apply_patch |
| Shell | `builtin/shell.rs` | Container | shell command execution |
| Memory | `builtin/memory.rs` | Orchestrator | memory_search, memory_write, memory_read, memory_tree |
| Jobs | `builtin/job.rs` | Orchestrator | create_job, list_jobs, job_status, cancel_job, job_events, job_prompt |
| Routines | `builtin/routine.rs` | Orchestrator | routine_create/list/update/delete/history |
| Extensions | `builtin/extension_tools.rs` | Orchestrator | tool_search/install/auth/activate/list/remove |
| Skills | `builtin/skill_tools.rs` | Orchestrator | skill_list/search/install/remove |
| HTML Converter | `builtin/html_converter.rs` | Orchestrator | HTML to Markdown conversion for HTTP responses |
| MCP client | `mcp/` | Orchestrator | Dynamic tools from MCP servers |
| WASM sandbox | `wasm/` | Orchestrator | Sandboxed WASM component tools |
| Builder | `builder/` | Orchestrator | build_software (LLM-driven code generation) |

### Execution Domains

Tools declare which execution domain they belong to via `ToolDomain`:

- `Orchestrator` — runs in the main agent process, full access to host state
- `Container` — intended for Docker sandbox execution; the agent dispatches these
  to an isolated container environment when sandbox is enabled

File and shell tools use `Container` domain to enforce isolation. When no sandbox is
configured, the tools still execute but with additional security layers applied on the host.

---

## 2. The Tool Trait (`tool.rs`)

Every tool in IronClaw implements this async trait defined in `src/tools/tool.rs`:

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    // Required
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> serde_json::Value;
    async fn execute(&self, params: serde_json::Value, ctx: &JobContext)
        -> Result<ToolOutput, ToolError>;

    // Optional — override defaults as needed
    fn estimated_cost(&self, _params: &serde_json::Value) -> Option<Decimal> { None }
    fn estimated_duration(&self, _params: &serde_json::Value) -> Option<Duration> { None }
    fn requires_sanitization(&self) -> bool { true }
    fn requires_approval(&self) -> bool { false }
    fn requires_approval_for(&self, _params: &serde_json::Value) -> bool { false }
    fn execution_timeout(&self) -> Duration { Duration::from_secs(60) }
    fn domain(&self) -> ToolDomain { ToolDomain::Orchestrator }
    fn schema(&self) -> ToolSchema { /* derived from name+description+parameters_schema */ }
}
```

### Method Details

| Method | Purpose |
|--------|---------|
| `name()` | Unique identifier used in registry lookup and LLM tool call |
| `description()` | Shown to the LLM in tool definitions; drives selection |
| `parameters_schema()` | JSON Schema object for the `parameters` field in OpenAI function calling format |
| `execute()` | Async execution with parsed params and job context |
| `requires_sanitization()` | Whether output passes through the safety sanitizer before reaching LLM |
| `requires_approval()` | Static approval flag; always prompts user before execution |
| `requires_approval_for()` | Dynamic approval check based on actual parameter values |
| `domain()` | Execution domain: `Orchestrator` or `Container` |
| `execution_timeout()` | Per-tool deadline; default 60 seconds |

### ToolOutput

```rust
pub struct ToolOutput {
    pub result: serde_json::Value,
    pub cost: Option<Decimal>,
    pub duration: Duration,
    pub raw: Option<String>,
}
```

- `result` is the JSON value returned to the LLM as the tool result
- `raw` carries the unmodified response before any truncation (for debugging)
- `cost` enables cost tracking; populated by tools that call external APIs

### ToolError

```rust
pub enum ToolError {
    InvalidParameters(String),
    ExecutionFailed(String),
    Timeout,
    NotAuthorized(String),
    RateLimited(String),
    ExternalService(String),
    Sandbox(String),
}
```

### ToolDomain

```rust
pub enum ToolDomain {
    Orchestrator,  // Main agent process
    Container,     // Docker isolation layer
}
```

### Implementing a Tool

Minimal implementation pattern from `CLAUDE.md`:

```rust
#[async_trait]
impl Tool for MyTool {
    fn name(&self) -> &str { "my_tool" }
    fn description(&self) -> &str { "Does something useful" }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "param": { "type": "string", "description": "A parameter" }
            },
            "required": ["param"]
        })
    }

    async fn execute(&self, params: serde_json::Value, ctx: &JobContext)
        -> Result<ToolOutput, ToolError>
    {
        let start = std::time::Instant::now();
        // ... do work ...
        Ok(ToolOutput::text("result", start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool { true }
}
```

---

## 3. Tool Registry (`registry.rs`)

`ToolRegistry` is the central directory of all available tools. It is held behind an
`Arc` and shared across concurrent jobs.

### Protected Tool Names

35 names are protected at startup. Dynamic tools (WASM, MCP) cannot shadow these names:

```
echo, time, json, http, shell,
read_file, write_file, list_dir, apply_patch,
memory_search, memory_write, memory_read, memory_tree,
create_job, list_jobs, job_status, cancel_job, job_events, job_prompt,
tool_search, tool_install, tool_auth, tool_activate, tool_list, tool_remove,
skill_list, skill_search, skill_install, skill_remove,
routine_create, routine_list, routine_update, routine_delete, routine_history,
build_software
```

### Registration Methods

```rust
// Startup registration — marks built-in names as protected
pub fn register_sync(&self, tool: Arc<dyn Tool>)

// Runtime registration — rejects tools that shadow protected names
pub async fn register(&self, tool: Arc<dyn Tool>) -> Result<(), RegistryError>
```

Dynamic tools registered via `register()` will receive `RegistryError::ProtectedName`
if they attempt to use any name from the protected list.

### Registration Groups

The registry assembles built-in tools in these groups during startup:

| Method | Tools Registered |
|--------|-----------------|
| `register_builtin_tools()` | echo, time, json, http |
| `register_dev_tools()` | shell, read_file, write_file, list_dir, apply_patch |
| `register_memory_tools()` | memory_search, memory_write, memory_read, memory_tree |
| `register_job_tools()` | create_job, list_jobs, job_status, cancel_job, job_events, job_prompt |
| `register_extension_tools()` | tool_search, tool_install, tool_auth, tool_activate, tool_list, tool_remove |
| `register_skill_tools()` | skill_list, skill_search, skill_install, skill_remove |
| `register_routine_tools()` | routine_create, routine_list, routine_update, routine_delete, routine_history |
| `register_builder_tool()` | build_software |
| `register_wasm()` | WASM tools from `~/.ironclaw/tools/` directory |
| `register_wasm_from_storage()` | WASM tools persisted in database |

> **v0.12.0 note (#346):** As of v0.12.0, the Telegram MTPRoto API tool is registered as `telegram-mtproto` and the Slack API tool as `slack-tool` (renamed to avoid name collisions with the WASM channel entries).

---

## 4. Built-in Tools Reference

### Summary Table

| Tool | Parameters (required*) | Domain | Approval |
|------|------------------------|--------|----------|
| `echo` | message* | Orchestrator | No |
| `time` | operation* | Orchestrator | No |
| `json` | operation*, data* | Orchestrator | No |
| `http` | method*, url* | Orchestrator | Yes |
| `read_file` | path* | Container | No |
| `write_file` | path*, content* | Container | No |
| `list_dir` | — | Container | No |
| `apply_patch` | path*, old_string*, new_string* | Container | No |
| `shell` | command* | Container | Yes |
| `memory_search` | query* | Orchestrator | No |
| `memory_write` | content* | Orchestrator | No |
| `memory_read` | path* | Orchestrator | No |
| `memory_tree` | — | Orchestrator | No |
| `create_job` | title*, description* | Orchestrator | No |
| `list_jobs` | — | Orchestrator | No |
| `job_status` | job_id* | Orchestrator | No |
| `cancel_job` | job_id* | Orchestrator | Yes |
| `job_events` | job_id* | Orchestrator | No |
| `job_prompt` | job_id*, content* | Orchestrator | Yes |
| `routine_create` | name*, trigger_type*, prompt* | Orchestrator | No |
| `routine_list` | — | Orchestrator | No |
| `routine_update` | name* | Orchestrator | No |
| `routine_delete` | name* | Orchestrator | No |
| `routine_history` | name* | Orchestrator | No |
| `tool_search` | query* | Orchestrator | No |
| `tool_install` | name* | Orchestrator | Yes |
| `tool_auth` | name* | Orchestrator | Yes |
| `tool_activate` | name* | Orchestrator | No |
| `tool_list` | — | Orchestrator | No |
| `tool_remove` | name* | Orchestrator | Yes |
| `skill_list` | — | Orchestrator | No |
| `skill_search` | query* | Orchestrator | No |
| `skill_install` | name* | Orchestrator | Yes |
| `skill_remove` | name* | Orchestrator | Yes |
| `build_software` | description* | Orchestrator | Yes |
| `html_to_markdown` | html*, url | Orchestrator | No |

---

### 4.1 `echo` (`builtin/echo.rs`)

Returns its input unchanged. Useful for testing tool plumbing.

```json
{
  "type": "object",
  "properties": {
    "message": { "type": "string", "description": "The message to echo back" }
  },
  "required": ["message"]
}
```

Security note: `requires_sanitization()` returns `false` — output bypasses the safety
sanitizer because the input is under agent control.

---

### 4.2 `time` (`builtin/time.rs`)

Date and time operations.

```json
{
  "type": "object",
  "properties": {
    "operation": {
      "type": "string",
      "enum": ["now", "parse", "format", "diff"],
      "description": "Operation to perform"
    },
    "timestamp": { "type": "string", "description": "ISO 8601 timestamp or Unix seconds" },
    "format":    { "type": "string", "description": "Format string for 'format' operation" },
    "timestamp2":{ "type": "string", "description": "Second timestamp for 'diff' operation" }
  },
  "required": ["operation"]
}
```

Operations:

- `now` — returns `{iso, unix, unix_millis}` for current time
- `parse` — parses a timestamp string and returns the same three fields
- `format` — formats a timestamp using a format string
- `diff` — computes the difference between `timestamp` and `timestamp2` in seconds

---

### 4.3 `json` (`builtin/json.rs`)

JSON manipulation. Fixed for OpenAI API compatibility (see Section 8).

```json
{
  "type": "object",
  "properties": {
    "operation": {
      "type": "string",
      "enum": ["parse", "query", "stringify", "validate"]
    },
    "data":      { "description": "JSON data to process (any value)" },
    "path":      { "type": "string", "description": "JSONPath query for 'query' operation" }
  },
  "required": ["operation", "data"]
}
```

Note: `data` has no `"type"` field — this means "accept any JSON value". An OpenAI API
bug rejects `"type": ["string", "null"]` union arrays; omitting `"type"` is the fix.

Operations:

- `parse` — parses a JSON string into a structured value
- `query` — runs a JSONPath expression against `data`
- `stringify` — serializes a value to a JSON string
- `validate` — checks whether a string is valid JSON

---

### 4.4 `http` (`builtin/http.rs`)

Outbound HTTP requests. Fixed for OpenAI API compatibility (see Section 8).

```json
{
  "type": "object",
  "properties": {
    "method": {
      "type": "string",
      "enum": ["GET", "POST", "PUT", "DELETE", "PATCH"]
    },
    "url":     { "type": "string", "description": "HTTPS URL to request" },
    "headers": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "name":  { "type": "string" },
          "value": { "type": "string" }
        }
      },
      "description": "Optional request headers"
    },
    "body":         { "description": "Optional request body (any value)" },
    "timeout_secs": { "type": "integer", "description": "Request timeout in seconds" }
  },
  "required": ["method", "url"]
}
```

Note: `body` has no `"type"` field for the same OpenAI compatibility reason as `data` in `json`.

Security controls enforced:

- HTTPS-only — HTTP URLs are rejected
- SSRF protection — blocks `localhost`, `127.0.0.1`, `::1`, all RFC 1918 private ranges,
  and the cloud metadata endpoint `169.254.254.254`
- DNS rebinding check — resolved IP addresses are validated against the SSRF blocklist
- 3xx redirect blocking — redirects are not followed
- Max response size: 5 MB (`MAX_RESPONSE_SIZE`)
- LeakDetector scans outbound URL, headers, and body for credential exposure
- `requires_approval: true` — every HTTP call prompts the user
- `requires_sanitization: true` — response is sanitized before reaching LLM

---

### 4.5 File Tools (`builtin/file.rs`)

All four file tools use `domain = Container` and enforce sandbox path restrictions.

#### `read_file`

```json
{
  "type": "object",
  "properties": {
    "path":   { "type": "string" },
    "offset": { "type": "integer", "description": "Line offset" },
    "limit":  { "type": "integer", "description": "Maximum lines to read" }
  },
  "required": ["path"]
}
```

- `MAX_READ_SIZE` = 1 MB hard cap on response size

#### `write_file`

```json
{
  "type": "object",
  "properties": {
    "path":    { "type": "string" },
    "content": { "type": "string" }
  },
  "required": ["path", "content"]
}
```

- `MAX_WRITE_SIZE` = 5 MB hard cap
- Rejects writes to workspace identity files: `HEARTBEAT.md`, `MEMORY.md`, `IDENTITY.md`,
  `SOUL.md`, `AGENTS.md`, `USER.md`, `daily/`, `context/`

#### `list_dir`

```json
{
  "type": "object",
  "properties": {
    "path":      { "type": "string" },
    "recursive": { "type": "boolean" },
    "max_depth": { "type": "integer" }
  }
}
```

- `MAX_DIR_ENTRIES` = 500 entries per call
- Automatically skips `node_modules`, `target`, `.git`, and similar large directories

#### `apply_patch`

```json
{
  "type": "object",
  "properties": {
    "path":        { "type": "string" },
    "old_string":  { "type": "string" },
    "new_string":  { "type": "string" },
    "replace_all": { "type": "boolean" }
  },
  "required": ["path", "old_string", "new_string"]
}
```

Path validation for all file tools: `normalize_lexical()` resolves `..` components
without filesystem access, then `validate_path()` checks the result against the sandbox
`base_dir`. Path traversal cannot escape the sandbox through symlinks or encoded
separators because normalization is lexical-only and reject is applied before any open.

---

### 4.6 `shell` (`builtin/shell.rs`)

Execute shell commands. Domain: Container.

```json
{
  "type": "object",
  "properties": {
    "command": { "type": "string" },
    "workdir": { "type": "string" },
    "timeout": { "type": "integer", "description": "Timeout in seconds" }
  },
  "required": ["command"]
}
```

Constants:

- `MAX_OUTPUT_SIZE` = 64 KB
- `DEFAULT_TIMEOUT` = 120 seconds

Security layers applied in order:

1. **Blocked commands** (`BLOCKED_COMMANDS`) — exact-match rejection of:
   `rm -rf /`, `:(){:|:&};:` (fork bomb), `dd if=/dev/zero`, `curl ... | sh`,
   `wget ... | sh`, and similar immediately destructive patterns.

2. **Dangerous patterns** (`DANGEROUS_PATTERNS`) — substring/regex rejection of:
   `sudo`, `doas`, `eval`, `$(curl`, `/etc/passwd`, `~/.ssh`, shell history files,
   and other privilege escalation or exfiltration vectors.

3. **Injection detection** (`detect_command_injection()`) — rejects:
   null bytes, `base64 ... | sh`, `printf %s ... | sh`, `xxd ... | sh`,
   DNS exfiltration patterns, netcat piping, `curl -d @file` credential posting,
   `wget --post-file`, and `rev | sh`.

4. **Never-auto-approve** (`NEVER_AUTO_APPROVE_PATTERNS`) — 35 patterns including
   `rm -rf`, `git push --force`, `DROP TABLE`, `DELETE FROM`, `truncate /`,
   and other irreversible operations. These always require explicit user approval
   regardless of the `requires_approval` flag.

5. **Environment scrubbing** — `env_clear()` removes all environment variables before
   execution, then the tool re-injects only the allowlist of ~30 safe variables
   (`SAFE_ENV_VARS`): `PATH`, `HOME`, `USER`, `LANG`, `TERM`, `TZ`, and similar.
   Secret environment variables like `API_KEY`, `TOKEN`, `PASSWORD` are never accessible
   to shell commands.

`requires_approval: true` — every shell invocation prompts the user.
`requires_approval_for()` also returns `true` for commands matching any NEVER_AUTO_APPROVE_PATTERNS.

---

### 4.7 Memory Tools (`builtin/memory.rs`)

Persistent workspace memory backed by the database (not the filesystem).

#### `memory_search`

```json
{
  "type": "object",
  "properties": {
    "query": { "type": "string" },
    "limit": { "type": "integer" }
  },
  "required": ["query"]
}
```

Performs hybrid FTS + vector search using Reciprocal Rank Fusion (RRF). Returns ranked
memory documents. Call before answering questions about prior work.

#### `memory_write`

```json
{
  "type": "object",
  "properties": {
    "content": { "type": "string" },
    "target":  { "type": "string", "description": "Path: memory, daily_log, heartbeat, or custom" },
    "append":  { "type": "boolean" }
  },
  "required": ["content"]
}
```

Identity files are protected from writes: `IDENTITY.md`, `SOUL.md`, `AGENTS.md`, `USER.md`.
These are injected into the LLM system prompt and must not be overwritten by tool calls.

#### `memory_read`

```json
{
  "type": "object",
  "properties": {
    "path": { "type": "string" }
  },
  "required": ["path"]
}
```

#### `memory_tree`

```json
{
  "type": "object",
  "properties": {
    "path":  { "type": "string" },
    "depth": { "type": "integer" }
  }
}
```

Returns a hierarchical JSON structure of the workspace file tree.

---

### 4.8 Job Tools (`builtin/job.rs`)

Manage parallel background jobs.

#### `create_job`

```json
{
  "type": "object",
  "properties": {
    "title":       { "type": "string" },
    "description": { "type": "string" },
    "wait":        { "type": "boolean", "description": "Wait for completion" },
    "mode":        { "type": "string", "enum": ["sandbox", "claude_code"] },
    "project_dir": { "type": "string" },
    "credentials": { "type": "array" }
  },
  "required": ["title", "description"]
}
```

`execution_timeout` is 660 seconds (11 minutes) when sandbox is configured.

#### `list_jobs`

```json
{
  "type": "object",
  "properties": {
    "filter": { "type": "string", "enum": ["active", "completed", "failed", "all"] }
  }
}
```

#### `job_status`

```json
{
  "type": "object",
  "properties": {
    "job_id": { "type": "string", "description": "Full UUID or 4+ char prefix" }
  },
  "required": ["job_id"]
}
```

#### `cancel_job`

`requires_approval: true`. Takes `job_id` as the only required parameter.

#### `job_events`

```json
{
  "type": "object",
  "properties": {
    "job_id": { "type": "string" },
    "limit":  { "type": "integer" }
  },
  "required": ["job_id"]
}
```

Enforces ownership check — only the job's owning session can read events.

#### `job_prompt`

```json
{
  "type": "object",
  "properties": {
    "job_id":  { "type": "string" },
    "content": { "type": "string" },
    "done":    { "type": "boolean" }
  },
  "required": ["job_id", "content"]
}
```

`requires_approval: true`. Enforces ownership check. Used to send interactive prompts
into a running job's input stream.

---

### 4.9 Routine Tools (`builtin/routine.rs`)

Manage scheduled and reactive automations.

#### `routine_create`

```json
{
  "type": "object",
  "properties": {
    "name":            { "type": "string" },
    "trigger_type":    { "type": "string", "enum": ["cron", "event", "webhook", "manual"] },
    "prompt":          { "type": "string" },
    "schedule":        { "type": "string", "description": "Cron expression for cron triggers" },
    "event_pattern":   { "type": "string" },
    "event_channel":   { "type": "string" },
    "context_paths":   { "type": "array" },
    "action_type":     { "type": "string" },
    "cooldown_secs":   { "type": "integer" }
  },
  "required": ["name", "trigger_type", "prompt"]
}
```

#### Other Routine Tools

| Tool | Required params | Notes |
|------|----------------|-------|
| `routine_list` | none | Returns all routines for the user |
| `routine_update` | `name` | Optional: `enabled`, `prompt`, `schedule`, `description` |
| `routine_delete` | `name` | Permanently removes the routine |
| `routine_history` | `name` | Optional `limit`; returns past execution records |

---

### 4.10 Extension Tools (`builtin/extension_tools.rs`)

Manage WASM and MCP tool extensions.

| Tool | Required | Approval | Notes |
|------|---------|----------|-------|
| `tool_search` | `query` | No | Optional `discover: bool` flag for ClawHub discovery |
| `tool_install` | `name` | Yes | Optional `url`, `kind` (wasm/mcp) |
| `tool_auth` | `name` | Yes | Triggers OAuth flow; auto-activates on success |
| `tool_activate` | `name` | No | Auto-triggers auth if 401 is returned |
| `tool_list` | — | No | Optional `kind` filter |
| `tool_remove` | `name` | Yes | Permanently unregisters the tool |

---

### 4.11 Skill Tools (`builtin/skill_tools.rs`)

Manage SKILL.md prompt extensions.

| Tool | Required | Approval | Notes |
|------|---------|----------|-------|
| `skill_list` | — | No | Optional `verbose: bool` |
| `skill_search` | `query` | No | Searches ClawHub registry |
| `skill_install` | `name` | Yes | Optional `url` or `content`; SSRF-protected fetch |
| `skill_remove` | `name` | Yes | Removes installed skill |

---

### 4.12 HTML to Markdown Converter (`builtin/html_converter.rs`)

Added in v0.10.0. Built-in tool and two-stage pipeline for converting HTML content to clean Markdown. Used internally by the `http` tool when fetching HTML pages, and exposed directly as the `html_to_markdown` built-in tool for web content ingestion and formatting.

**Feature flag:** `html-to-markdown` (enabled by default)

**Pipeline:**
1. **Readability extraction** (`readabilityrs`) — Extracts article content from HTML
2. **HTML-to-Markdown conversion** (`html_to_markdown_rs`) — Converts clean HTML to Markdown

**When feature is disabled:** Content passes through unchanged (returns raw HTML).

**Usage:**
```rust
use crate::tools::builtin::html_converter::convert_html_to_markdown;

let markdown = convert_html_to_markdown(html_content, "https://example.com/article")?;
```

**Error conditions:**
- Readability parser failure
- No content extracted from article
- HTML-to-Markdown conversion failure

---

### 4.13 Built-in Tool Rate Limiter (`src/tools/rate_limiter.rs`)

Added in v0.10.0. Shared rate limiter for built-in tool invocations (separate from WASM tool rate limiting). Provides per-tool, per-user sliding window rate limiting checked before every built-in tool execution.

**Integration:** The `RateLimiter` is shared as `Arc<RateLimiter>` in `ToolRegistry` and is checked before every built-in tool execution.

**State key:** `(user_id, tool_name)` pairs — different users and different tools maintain independent counters.

**Window types:** Per-minute and per-hour sliding windows.

**Scope:** Applied to built-in tools including:
- `shell` — Shell command execution
- `http` — HTTP requests
- `write_file` — File write operations
- Other mutating built-in tools

**Algorithm:** Simplified sliding window counter
- Tracks request counts for current minute and hour windows
- Resets counters when window expires
- In-memory only (resets on process restart)

**Per-tool configuration:**
```rust
pub struct ToolRateLimitConfig {
    pub requests_per_minute: u32,
    pub requests_per_hour: u32,
}
```

**Default Limits:**
| Window | Limit |
|--------|-------|
| Per-minute | 60 requests |
| Per-hour | 1000 requests |

**Rate Limit Result:**
```rust
pub enum RateLimitResult {
    Allowed { remaining_minute: u32, remaining_hour: u32 },
    Limited { retry_after: Duration, limit_type: LimitType },
}
```

**Note:** Rate limit state is in-memory only and resets on process restart. State is not persisted to the database.

---

## 5. MCP Client (`mcp/`)

The MCP subsystem connects IronClaw to external MCP servers, exposing their tools as
native IronClaw tools through automatic name prefixing.

### Architecture

```
~/.ironclaw/mcp-servers.json  (or DB: settings["mcp_servers"])
          │
          ▼
McpServersFile ──► McpServerConfig (per server)
          │
          ▼
McpClient (one per server)
  ├── Streamable HTTP transport (JSON-RPC 2.0)
  ├── McpSessionManager (session ID persistence)
  ├── OAuth 2.1 + PKCE token management
  └── McpToolWrapper (implements Tool trait per discovered tool)
```

### Configuration (`mcp/config.rs`)

Config file: `~/.ironclaw/mcp-servers.json`

```json
{
  "servers": [
    {
      "name": "notion",
      "url": "https://mcp.notion.com/mcp",
      "enabled": true,
      "description": "Notion workspace tools",
      "oauth": {
        "client_id": "...",
        "scopes": ["read_content", "update_content"],
        "use_pkce": true
      }
    }
  ],
  "schema_version": 1
}
```

`McpServerConfig` fields:

| Field | Type | Notes |
|-------|------|-------|
| `name` | string | Prefix added to all tools from this server |
| `url` | string | HTTPS required for non-localhost servers |
| `oauth` | optional | See `OAuthConfig` below |
| `enabled` | bool | Whether to connect on startup |
| `description` | string | Human-readable description |

`OAuthConfig` fields:

| Field | Type | Default | Notes |
|-------|------|---------|-------|
| `client_id` | string | — | OAuth application client ID |
| `authorization_url` | optional | discovered | Override from well-known endpoint |
| `token_url` | optional | discovered | Override from well-known endpoint |
| `scopes` | array | `[]` | Requested OAuth scopes |
| `use_pkce` | bool | `true` | Always true; S256 PKCE challenge |
| `extra_params` | object | `{}` | Additional authorization request params |

`requires_auth()` returns `true` if oauth is configured OR the server URL is a remote
HTTPS host. Localhost servers are exempt from auth requirements.

Token storage in SecretsStore (AES-256-GCM):

- `mcp_{name}_access_token`
- `mcp_{name}_refresh_token`
- `mcp_{name}_client_id`

### Client (`mcp/client.rs`)

`McpClient` speaks JSON-RPC 2.0 over Streamable HTTP (MCP protocol version `2024-11-05`).

Key behaviors:

- **Session management** — `Mcp-Session-Id` header is tracked via `McpSessionManager`
  (in-memory, 30-minute idle timeout, auto-cleanup of stale sessions)
- **Tool naming** — All tools discovered from a server are registered as
  `{server_name}_{tool_name}` to prevent collisions between servers
- **Auto token refresh** — 401 responses trigger `refresh_access_token()` and one
  automatic retry before propagating the error
- **SSE response parsing** — Both SSE streaming responses and plain JSON responses
  are parsed to extract the JSON-RPC result
- **McpToolWrapper** — Each discovered tool is wrapped in an implementation of the
  `Tool` trait; `requires_sanitization: true` is always set; `requires_approval`
  mirrors the tool's `destructive_hint` annotation

### Protocol (`mcp/protocol.rs`)

Protocol version: `"2024-11-05"`

```rust
pub struct McpRequest {
    pub jsonrpc: String,  // "2.0"
    pub id: u64,
    pub method: String,
    pub params: serde_json::Value,
}
```

Helper constructors:

- `McpRequest::initialize(client_info)` — sent on first connection
- `McpRequest::list_tools()` — discovers available tools
- `McpRequest::call_tool(name, args)` — invokes a tool
- `McpRequest::initialized_notification()` — confirms initialization

`McpTool` annotation fields (from `McpToolAnnotations`):

| Field | Meaning |
|-------|---------|
| `destructive_hint` | Maps to `requires_approval` in the wrapper |
| `side_effects_hint` | Tool modifies external state |
| `read_only_hint` | Tool only reads, no side effects |
| `execution_time_hint` | Expected duration category |

### Authentication (`mcp/auth.rs`)

Full OAuth 2.1 with PKCE implementation:

1. **Discovery** — fetches `/.well-known/oauth-protected-resource` to find the
   authorization server, then `/.well-known/oauth-authorization-server` for endpoints.
   Falls back to pre-configured `OAuthConfig` URLs.
2. **Dynamic Client Registration (DCR)** — registers the client with the authorization
   server if no `client_id` is stored yet
3. **PKCE** — S256 code challenge: 32 random bytes, SHA-256 hashed, base64url-encoded
4. **Browser flow** — `authorize_mcp_server()` opens the browser and listens on
   `OAUTH_CALLBACK_PORT` for the authorization code redirect
5. **Token storage** — access and refresh tokens are stored encrypted in `SecretsStore`
6. **Token refresh** — `refresh_access_token()` works with both pre-configured and
   DCR-discovered token endpoints

### Session Management (`mcp/session.rs`)

`McpSessionManager` is a `tokio::sync::RwLock<HashMap<String, McpSession>>` keyed by
server name.

Session lifecycle:

- Created on first `get_or_create()` call for a server
- `update_session_id()` stores the `Mcp-Session-Id` returned in server responses
- `mark_initialized()` is called after the initialize/initialized handshake completes
- Sessions expire after 30 minutes of idle time (`is_stale(1800)`)
- `cleanup_stale()` removes expired sessions; can be called periodically
- `terminate()` removes a session immediately (e.g., on error)

---

## 6. WASM Tool Runtime (`wasm/`)

WASM tools run inside a Wasmtime sandbox with capability-based access control.
The design follows NEAR blockchain patterns: compile once at registration, instantiate
fresh per execution.

### Architecture

```
WASM Tool bytes
    │
    ▼
WasmToolRuntime.prepare()
    ├── Validate via wasmparser
    ├── Compile via Wasmtime (Component Model, wasm32-wasip2)
    ├── Cache PreparedModule
    └── Spawn epoch-ticker thread (500ms intervals)

Per execution:
WasmToolWrapper.execute()
    ├── RateLimiter.check_and_record()
    ├── Fresh Wasmtime Store + HostState
    ├── Fuel metering (CPU limit)
    ├── Epoch deadline (infinite-loop guard)
    ├── AllowlistValidator → CredentialInjector (for HTTP)
    └── LeakDetector scans response
```

### Security Constraints

| Threat | Mitigation |
|--------|------------|
| CPU exhaustion | Fuel metering (default configurable) |
| Memory exhaustion | `ResourceLimiter`, 10 MB default |
| Infinite loops | Epoch interruption (500 ms ticks) + tokio timeout |
| Filesystem access | No WASI FS; only `host::workspace_read` via capability |
| Network access | `AllowlistValidator` — allowlisted HTTPS endpoints only |
| Credential exposure | `CredentialInjector` — secrets never enter WASM memory |
| Secret exfiltration | `LeakDetector` scans all outputs |
| Log spam | Max 1000 log entries; 4 KB per message (truncated) |
| Path traversal | Validates paths: no `..`, no `/` prefix, no null bytes, no Windows paths |
| Trap recovery | Instance discarded on trap; never reused |
| Side channels | Fresh instance created per execution |
| Rate abuse | `RateLimiter`: per-(user, tool) sliding window counter |
| WASM tampering | BLAKE3 hash verification on load from storage |
| Direct tool access | Tool aliasing — WASM uses aliases, never real tool names |

### Runtime Configuration (`wasm/runtime.rs`)

```rust
pub struct WasmRuntimeConfig {
    pub default_limits: ResourceLimits,  // memory, fuel, timeout
    pub fuel_config: FuelConfig,          // enabled flag + limit
    pub cache_compiled: bool,             // cache PreparedModule in HashMap
    pub cache_dir: Option<PathBuf>,       // disk cache location
    pub optimization_level: OptLevel,     // Cranelift opt level
}
```

Default: `cache_compiled = true`, `optimization_level = Speed`.
Testing config: 1 MB memory, 100,000 fuel, 5 second timeout, no caching, `OptLevel::None`.

`WasmToolRuntime` spawns a named background thread `"wasm-epoch-ticker"` on creation.
The thread calls `engine.increment_epoch()` every 500 ms. Any WASM store with an
exceeded epoch deadline traps immediately, preventing infinite loops from stalling
the executor.

### Capabilities (`wasm/capabilities.rs`)

All capabilities are opt-in. Default is no access.

```rust
pub struct Capabilities {
    pub workspace_read: Option<WorkspaceCapability>,
    pub http:           Option<HttpCapability>,
    pub tool_invoke:    Option<ToolInvokeCapability>,
    pub secrets:        Option<SecretsCapability>,
}
```

Builder API:

```rust
let caps = Capabilities::none()
    .with_workspace_read(vec!["context/".to_string()])
    .with_http(HttpCapability::new(vec![
        EndpointPattern::host("api.openai.com").with_path_prefix("/v1/"),
    ]))
    .with_tool_invoke(aliases)
    .with_secrets(vec!["openai_*".to_string()]);
```

`HttpCapability` fields:

- `allowlist: Vec<EndpointPattern>` — patterns the tool may call
- `credentials: HashMap<String, CredentialMapping>` — secret injection rules
- `rate_limit: RateLimitConfig` — default: 60/min, 1000/hr
- `max_request_bytes` — 1 MB default
- `max_response_bytes` — 10 MB default
- `timeout` — 30 seconds default

`EndpointPattern` fields:

- `host` — exact hostname or `*.example.com` wildcard
- `path_prefix` — optional path constraint (e.g., `/v1/`)
- `methods` — optional method list; empty = all methods

`SecretsCapability` — glob matching for allowed secret names (e.g., `openai_*`).
WASM can only check _existence_ of secrets, never read their values.

`ToolInvokeCapability` — alias-to-real-name mapping. WASM calls tools via alias;
the alias resolves to a real tool name at the host boundary.

### Host State (`wasm/host.rs`)

`HostState` is the per-execution state equivalent to NEAR's VMLogic:

```rust
pub struct HostState {
    logs: Vec<LogEntry>,          // up to 1000, 4 KB each
    logging_enabled: bool,
    capabilities: Capabilities,
    logs_dropped: usize,
    user_id: Option<String>,
    http_request_count: u32,      // max 50 per execution
    tool_invoke_count: u32,       // max 20 per execution
}
```

Per-execution hard limits applied by `HostState`:

- `MAX_REQUESTS_PER_EXECUTION` = 50 HTTP requests
- `MAX_INVOKES_PER_EXECUTION` = 20 tool invocations

`workspace_read()` validates paths before reading:

- Blocks absolute paths (leading `/`)
- Blocks `..` path traversal
- Blocks null bytes
- Blocks Windows-style paths (`C:\`, `D:`)

### Credential Injector (`wasm/credential_injector.rs`)

```
WASM requests HTTP ──► Host receives request ──► Match credentials by host
                                                      │
                                                      ▼
                                          Decrypt secret from SecretsStore
                                                      │
                                          Inject into request:
                                          ├── Authorization: Bearer {token}
                                          ├── Authorization: Basic {base64}
                                          ├── X-Custom-Header: {value}
                                          └── ?query_param={value}
```

`CredentialLocation` variants: `AuthorizationBearer`, `AuthorizationBasic { username }`,
`Header { name, prefix }`, `QueryParam { name }`, `UrlPath` (handled by caller).

The injector's allowed-list check (`is_secret_allowed()`) uses the same glob matching
as `SecretsCapability`. An empty allowed list causes `AccessDenied` for any secret,
even if the mapping matches the host.

### Allowlist Validator (`wasm/allowlist.rs`)

`AllowlistValidator` validates every HTTP request from WASM before it is executed.

URL parsing is strict:

- Rejects non-HTTP/HTTPS schemes
- Rejects URLs with userinfo (`user:pass@host`) to prevent allowlist bypass
- Normalizes path: resolves `..`, `.`, `%2e%2e`, rejects `%2F` encoded separators
- Validates percent-encoding character by character

Denial reasons:

- `EmptyAllowlist` — no patterns configured
- `InvalidUrl` — parse failure, userinfo present, or unsafe encoding
- `InsecureScheme` — non-HTTPS when `require_https = true` (default)
- `HostNotAllowed` — host matches no pattern
- `PathNotAllowed` — host matches but path prefix doesn't
- `MethodNotAllowed` — host and path match but method is restricted

### Rate Limiter (`wasm/rate_limiter.rs`)

`RateLimiter` is a global in-memory sliding window counter keyed by `(user_id, tool_name)`.

```rust
pub enum RateLimitResult {
    Allowed { remaining_minute: u32, remaining_hour: u32 },
    Limited { retry_after: Duration, limit_type: LimitType },
}

pub enum LimitType {
    PerMinute,
    PerHour,
}
```

Default `RateLimitConfig`: 60 requests/minute, 1000 requests/hour.
Limits are per-user per-tool — different users and different tools have independent counters.
`check_and_record()` atomically checks and increments under a write lock.

### Capabilities File

Each WASM tool binary may have a sidecar `{tool_name}.capabilities.json` file:

```json
{
  "http": {
    "allowed_endpoints": [
      { "host": "api.example.com", "path_prefix": "/v1/", "methods": ["POST"] }
    ]
  },
  "workspace": true,
  "secrets": {
    "allowed": ["API_KEY", "openai_*"]
  }
}
```

The `capabilities_schema` module (`wasm/capabilities_schema.rs`) handles parsing these
files; the `loader` module discovers tool binaries and their sidecar files from
`~/.ironclaw/tools/` and from database storage.

---

## 7. Dynamic Tool Builder (`builder/`)

The builder allows the agent to create new WASM tools at runtime using LLM-driven
code generation. It is exposed as the built-in `build_software` tool.

### Build Loop (`builder/core.rs`)

The `LlmSoftwareBuilder` runs an iterative agent loop (similar to Codex):

```
1. Analyze requirement  ─► Parse description, determine type/language
2. Generate scaffold    ─► Create initial project files via write_file
3. Implement code       ─► Write the actual implementation
4. Build/compile        ─► Run cargo/npm/go build via shell
5. Fix errors           ─► Parse compiler errors, modify code, retry
6. Test                 ─► Run tests, fix failures
7. Validate             ─► For WASM tools, verify interface compliance
8. Register             ─► Add to ToolRegistry if auto_register = true
```

Max iterations: `BuilderConfig.max_iterations` (default 10).
Timeout: `BuilderConfig.timeout` (default 600 seconds).

The builder detects when the LLM is "stuck in planning mode" (returning text rather than
calling tools) and fails fast after 2 consecutive text-only responses to avoid wasting
iterations.

### `build_software` Tool Schema

```json
{
  "type": "object",
  "properties": {
    "description": { "type": "string", "description": "Natural language description of what to build" },
    "type": {
      "type": "string",
      "enum": ["wasm_tool", "cli_binary", "library", "script"]
    },
    "language": {
      "type": "string",
      "enum": ["rust", "python", "typescript", "bash"]
    }
  },
  "required": ["description"]
}
```

`requires_approval: true` — building software always requires user confirmation.

If `type` or `language` are not specified, the LLM analyzes the description and selects
them. For agent-usable tools, it is strongly biased toward `wasm_tool` + `rust`.

### BuildRequirement and SoftwareType

```rust
pub enum SoftwareType {
    WasmTool,    // WASM component for agent use — preferred for all tool requests
    CliBinary,   // Standalone CLI binary for human users
    Library,     // Library/crate
    Script,      // Python, Bash, etc.
    WebService,  // HTTP service
}

pub enum Language {
    Rust,
    Python,
    TypeScript,
    JavaScript,
    Go,
    Bash,
}
```

### BuildPhase Tracking

`BuildResult` records phase transitions in `logs: Vec<BuildLog>`:

```rust
pub enum BuildPhase {
    Analyzing,    // Parsing the requirement
    Scaffolding,  // Creating project structure
    Implementing, // Writing code (write_file calls)
    Building,     // Running build commands
    Testing,      // Running tests
    Fixing,       // Handling errors
    Validating,   // WASM interface validation
    Registering,  // Adding to ToolRegistry
    Packaging,    // Final artifact preparation
    Complete,
    Failed,
}
```

### WASM Tool Context Injection

When `software_type == WasmTool`, the build system prompt includes:

- The WIT interface definition for host functions
- `Cargo.toml` template with `wit-bindgen` dependency and `cdylib` crate type
- `src/lib.rs` template showing `wit_bindgen::generate!`, `Guest` trait implementation,
  `execute()`, `schema()`, and `description()` exports
- Build commands: `cargo component build --release`
- Capabilities file format for granting HTTP/workspace/secrets access
- Critical rules: never panic (return `Response { error }` instead), never use secrets
  directly (use placeholder URLs; host injects credentials)

### WASM Validation (`builder/validation.rs`)

`WasmValidator` parses WASM binary sections using `wasmparser` to validate built modules:

- `max_size`: 10 MB default
- `required_exports`: must include `"run"` (WASM interface entry point)
- `allowed_import_modules`: `env`, `wasi_snapshot_preview1`, `wasi`
- Warns on WASI filesystem functions (`fd_write`, `path_open`, etc.)
- Warns on WASI socket functions (`sock_send`, `sock_recv`)

`ValidationResult` includes `is_valid`, `errors`, `warnings`, `exports`, `imports`,
and `size_bytes`.

### Template Engine (`builder/templates.rs`)

`TemplateEngine` provides project scaffolding templates:

```rust
pub enum TemplateType {
    WasmTool,
    CliBinary,
    Library,
    Script,
    WebService,
}
```

Templates generate the initial directory structure and boilerplate files that the
LLM then iterates on during the build loop.

### Test Harness (`builder/testing.rs`)

`TestHarness` runs test suites and parses results:

```rust
pub struct TestCase {
    pub name: String,
    pub input: serde_json::Value,
    pub expected_output: serde_json::Value,
}

pub struct TestResult {
    pub passed: bool,
    pub actual: Option<serde_json::Value>,
    pub error: Option<String>,
}
```

---

## 8. OpenAI JSON Schema Compatibility

### The Problem

The OpenAI API rejects tool schemas that use JSON Schema union type arrays in the
`"type"` field. Specifically:

```json
{ "type": ["string", "null"] }
```

This is valid per JSON Schema spec (Draft 4+) for expressing "string or null", but
OpenAI's function calling implementation rejects it with a validation error.

### The Fix

For parameters that accept any value (equivalent to "any type or null"), the `"type"`
field is omitted entirely. A JSON Schema property with no `"type"` field accepts all
JSON values, which achieves the intended "any value" semantics.

### Affected Parameters

**`json` tool — `data` parameter** (`builtin/json.rs`):

Before (broken):

```json
"data": { "type": ["string", "null"], "description": "JSON data to process" }
```

After (fixed):

```json
"data": { "description": "JSON data to process (any value)" }
```

A test `test_json_tool_schema_data_has_type` verifies that the schema for `data` does
NOT contain a `"type"` field — the test name is misleading but the assertion is that
no type constraint is present.

**`http` tool — `body` parameter** (`builtin/http.rs`):

Before (broken):

```json
"body": { "type": ["string", "null"], "description": "Optional request body" }
```

After (fixed):

```json
"body": { "description": "Optional request body (any value)" }
```

### General Rule

When writing tool schemas in IronClaw:

- Use `{ "type": "string" }` for required string parameters
- Use `{ "type": "string" }` with the parameter absent from `"required"` for optional
  string parameters
- For parameters that accept any JSON value (including null, object, array, or
  primitive): omit `"type"` entirely — do NOT use `"type": ["T", "null"]`

---

## 9. Configuration Reference

### Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `SANDBOX_ENABLED` | `true` | Enable Docker sandbox for Container-domain tools |
| `SANDBOX_IMAGE` | `ironclaw-worker:latest` | Docker image for worker containers |
| `SANDBOX_MEMORY_LIMIT_MB` | `2048` | Container memory limit |
| `SANDBOX_TIMEOUT_SECS` | `120` | Container execution timeout |
| `SANDBOX_POLICY` | `readonly` | `readonly`, `workspace_write`, or `full_access` |
| `SANDBOX_CPU_SHARES` | `1024` | Relative CPU weight for containers |

### MCP Server Configuration

MCP servers are configured in `~/.ironclaw/mcp-servers.json` or in the database
under the `mcp_servers` settings key. The file schema version is `1`.

Servers with a remote HTTPS URL always require authentication. Authentication state
(tokens) is stored encrypted in the SecretsStore using AES-256-GCM.

### WASM Tool Directories

| Path | Purpose |
|------|---------|
| `~/.ironclaw/tools/` | User-installed WASM tool binaries |
| `{binary}.capabilities.json` | Sidecar capabilities file for each tool |
| Database (`wasm_tools` table) | Persisted tool binaries and metadata |

### Tool Approval Model

The approval system has three levels of granularity:

1. `requires_approval()` — static; always prompts before execution regardless of params
2. `requires_approval_for(params)` — dynamic; prompts only when params match dangerous patterns
3. No approval methods overridden — the tool executes without user confirmation

Tools that are always approval-required: `http`, `shell`, `cancel_job`, `job_prompt`,
`tool_install`, `tool_auth`, `tool_remove`, `skill_install`, `skill_remove`,
`build_software`.

Shell adds `requires_approval_for()` on top of static approval, meaning any command
matching `NEVER_AUTO_APPROVE_PATTERNS` (35 patterns) triggers an additional approval
prompt even in contexts where static approval is bypassed.

**Consolidated Tool Approval (v0.10.0):** Tool approval was refactored into a single
param-aware method (`consolidate tool approval`). The multi-tool approval flow can
resume after the user approves or denies a batch — approval decisions are applied
per-tool within the batch and execution continues for approved tools without restarting
the entire flow.

### Security Architecture Summary

All external tool output passes through `SafetyLayer` before reaching the LLM:

```xml
<tool_output name="search" sanitized="true">
[escaped content]
</tool_output>
```

The four-layer safety pipeline:

1. **Sanitizer** — Pattern detection, content escaping for injection vectors
2. **Validator** — Length limits, encoding validation, forbidden pattern rejection
3. **Policy** — Rules with severity (`Critical`, `High`, `Medium`, `Low`) and actions
   (`Block`, `Warn`, `Review`, `Sanitize`)
4. **LeakDetector** — Scans for 15+ secret patterns (API keys, tokens, private keys,
   connection strings); actions per pattern are `Block`, `Redact`, or `Warn`

LeakDetector runs at two points: after tool execution (before output reaches LLM)
and after LLM response generation (before output reaches the user). For the `http` tool
it additionally scans the outbound request URL, headers, and body before sending.
