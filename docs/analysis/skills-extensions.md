# IronClaw Codebase Analysis — Skills, Extensions & Hooks

> Updated: 2026-02-26 | Version: v0.12.0

## 1. Overview

IronClaw extends its capabilities through three interconnected plugin systems:

- **Skills** — SKILL.md files (YAML frontmatter + markdown prompt body) that inject domain-specific instructions into the LLM context at runtime. Skills operate purely at the prompt level and are subject to trust-based tool attenuation.
- **Extensions** — MCP servers and WASM tool/channel modules that add new callable tools to the agent. Extensions are installed, authenticated, and activated through conversational commands.
- **Hooks** — A lifecycle interception system that allows declarative rules, regex transforms, and outbound webhook notifications to be applied at six well-defined points in every agent operation.

All three systems are designed around a defense-in-depth model: skills cannot bypass tool restrictions, extensions are sandboxed at the WASM or MCP boundary, and hooks are validated before registration (no SSRF, forbidden headers blocked, IP ranges denied).

Source tree locations:

```
src/skills/          — SKILL.md parser, registry, selector, gating, attenuation, catalog
src/extensions/      — Extension manager, built-in registry, online discovery
src/hooks/           — Hook trait, registry, bundled implementations, bootstrap
src/channels/wasm/   — WASM channel runtime, capabilities, schema
```

---

## 2. Skills System

### 2.1 What is a Skill?

A skill is a `SKILL.md` file consisting of a YAML frontmatter block followed by a markdown prompt body. When a skill activates for a given user message, its prompt body is injected into the LLM system context, providing domain-specific instructions without any code-level changes to the agent.

Skills can serve as:

- **Prompt injectors** — domain expertise (writing assistant, deployment guide)
- **Activation-gated context** — only active when message content matches
- **Trust-scoped principals** — installed skills restrict tool access; trusted skills do not

Skills do not contain executable code. Executable extensions (WASM tools, MCP servers) are a separate system documented in Section 3.

### 2.2 SKILL.md Frontmatter Schema

The full YAML frontmatter schema, drawn from `src/skills/mod.rs` (`SkillManifest`, `ActivationCriteria`, `SkillMetadata`):

```yaml
---
name: my-skill              # required; [a-zA-Z0-9][a-zA-Z0-9._-]{0,63}
version: "1.0.0"            # optional; defaults to "0.0.0"
description: "..."          # optional; short human-readable description

activation:
  keywords:                 # optional list; max 20 entries, min 3 chars each
    - "write"
    - "email"
  patterns:                 # optional list of regex strings; max 5 entries
    - '(?i)\b(write|draft)\b.*\b(email|letter)\b'
  tags:                     # optional list; max 10 entries, min 3 chars each
    - "prose"
    - "communication"
  max_context_tokens: 2000  # optional; default 2000; prompt budget cap

metadata:
  openclaw:
    requires:
      bins:                 # optional; binaries that must be on PATH
        - "vale"
      env:                  # optional; environment variables that must be set
        - "VALE_CONFIG"
      config:               # optional; file paths that must exist
        - "/etc/vale.ini"
---

You are a writing assistant. When the user asks to write or edit...
```

Constraints enforced at load time (from `ActivationCriteria.enforce_limits()`):

| Field | Max entries | Min token length |
|-------|-------------|-----------------|
| `keywords` | 20 | 3 characters |
| `patterns` | 5 | — |
| `tags` | 10 | 3 characters |

Regex patterns are compiled with a 64 KiB state limit to prevent ReDoS via pathological inputs.

### 2.3 Skill Parser (`parser.rs`)

Source: `src/skills/parser.rs`

`parse_skill_md(content: &str) -> Result<ParsedSkill, SkillParseError>` is the entry point. It handles:

1. **BOM stripping** — UTF-8 BOM (`\u{feff}`) at the start is silently removed.
2. **Frontmatter detection** — The file must begin with `---` on the first non-blank line; otherwise `MissingFrontmatter` is returned.
3. **YAML extraction** — Content between the opening `---` and the next `---` on its own line is extracted and parsed with `serde_yml`.
4. **Name validation** — `validate_skill_name()` checks the name against `^[a-zA-Z0-9][a-zA-Z0-9._-]{0,63}$`.
5. **Limit enforcement** — `activation.enforce_limits()` is called to cap and filter keyword/tag/pattern lists.
6. **Body extraction** — Everything after the closing `---` line, with leading blank lines stripped, becomes the prompt body.
7. **Empty body rejection** — A prompt body consisting only of whitespace returns `EmptyPrompt`.

Error variants:

| Variant | Condition |
|---------|-----------|
| `MissingFrontmatter` | No opening or closing `---` delimiter found |
| `InvalidYaml(String)` | YAML parse failure |
| `EmptyPrompt` | Prompt body is whitespace-only |
| `InvalidName { name }` | Name fails the regex pattern |

### 2.4 Skill Registry (`registry.rs`)

Source: `src/skills/registry.rs`

`SkillRegistry` manages the in-memory set of loaded skills and the on-disk `~/.ironclaw/skills/` directory.

**Discovery** (`discover_all`): Scans three directories in priority order:

1. `<workspace>/skills/` — loaded as `SkillTrust::Trusted`, `SkillSource::Workspace`
2. `~/.ironclaw/skills/` — loaded as `SkillTrust::Trusted`, `SkillSource::User` (user-placed skills)
3. `~/.ironclaw/installed_skills/` — loaded as `SkillTrust::Installed`, `SkillSource::Installed` (registry-installed; restricted tool ceiling)

On name collision, the workspace skill wins; the user skill is silently skipped. Discovery is capped at 100 skills per directory (`MAX_DISCOVERED_SKILLS`).

**Supported layouts**:

- Flat: `skills/SKILL.md` directly in the skills directory
- Subdirectory: `skills/<name>/SKILL.md`

**Security checks performed per file** (`load_and_validate_skill`):

1. Symlink detection — both the directory entry and the file itself are checked via `symlink_metadata`. Symlinks are rejected.
2. Size limit — files larger than 64 KiB (`MAX_PROMPT_FILE_SIZE`) are rejected.
3. UTF-8 validation — non-UTF-8 bytes are rejected.
4. CRLF normalization — `\r\n` and lone `\r` are normalized to `\n` before parsing and hashing.
5. Gating checks — if `metadata.openclaw.requires` is present, all binary/env/config requirements are checked.
6. Token budget check — if the prompt body is estimated at more than twice `max_context_tokens` (approximated as 0.25 tokens/byte), the skill is rejected.
7. SHA-256 hash — computed over the normalized prompt content; stored as `"sha256:{hex}"`.

**Install flow** (`install_skill`):

```
parse_skill_md(content)
  -> normalize_line_endings(content)
  -> create ~/.ironclaw/skills/<name>/
  -> write SKILL.md to disk
  -> load_and_validate_skill (validates round-trip)
  -> commit_install (in-memory append)
```

The two-phase `prepare_install_to_disk` + `commit_install` split allows callers holding a registry lock to release the lock before the async disk write.

**Remove flow** (`remove_skill`):

Only `SkillSource::User` skills can be removed. `SkillSource::Workspace` and `SkillSource::Bundled` skills return `CannotRemove`. Files are deleted, then the empty directory is removed, then the in-memory entry is dropped.

**Storage location**: `~/.ironclaw/skills/<name>/SKILL.md` (subdirectory layout is always used for installed skills).

### 2.5 Skill Catalog (`catalog.rs`)

Source: `src/skills/catalog.rs`

`SkillCatalog` is a runtime HTTP client for the ClawHub public registry. The compiled-in default backend is `https://wry-manatee-359.convex.site` (a Convex backend). No compile-time catalog entries exist; the catalog is always fetched live.

**Configuration**: `CLAWHUB_REGISTRY` env var overrides the default base URL. The legacy `CLAWDHUB_REGISTRY` var is also checked as a fallback.

**Search API**: `GET {registry_url}/api/v1/search?q={query}`

Response format (ClawHub v1):

```json
[
  {
    "slug": "owner/skill-name",
    "displayName": "My Skill",
    "summary": "Does something useful",
    "version": "1.2.0",
    "score": 0.95
  }
]
```

Results are deserialized into `CatalogEntry` structs and capped at 25 results.

**Caching**: Search results are cached in memory with a 5-minute TTL, keyed by lowercased query string. The cache is capped at 50 entries (oldest evicted first on overflow).

**Download URL construction** (`skill_download_url`):

```
GET {registry_url}/api/v1/download?slug={url-encoded-slug}
```

The slug is URL-encoded to prevent query string injection via characters like `&`, `#`, `=`.

**Error handling**: All network errors return an empty result vector. Catalog search is best-effort and never blocks the agent.

### 2.6 Skill Gating (`gating.rs`)

Source: `src/skills/gating.rs`

Gating is the prerequisite check that runs before a skill is loaded. If any requirement fails, the skill is skipped with a warning.

`check_requirements(requirements: &GatingRequirements) -> GatingResult` is the async entry point. It offloads the synchronous subprocess calls to `tokio::task::spawn_blocking`.

The three gate types checked by `check_requirements_sync`:

| Gate type | Field | Check |
|-----------|-------|-------|
| Binary presence | `bins` | `which <name>` (Unix) or `where <name>` (Windows) exit code 0 |
| Environment variable | `env` | `std::env::var(name).is_ok()` |
| Config file | `config` | `std::path::Path::new(path).exists()` |

`GatingResult` carries `passed: bool` and `failures: Vec<String>` (human-readable descriptions of each failure).

All failures are accumulated; a skill fails if any single requirement is unmet. The registry logs all failures at `WARN` level and continues to the next skill.

### 2.7 Skill Selector (`selector.rs`)

Source: `src/skills/selector.rs`

`prefilter_skills(message, available_skills, max_candidates, max_context_tokens) -> Vec<&LoadedSkill>` performs the first phase of skill selection. It is entirely deterministic — no LLM involvement, no loaded skill content in context — preventing circular manipulation where a loaded skill could influence which skills get selected.

**Scoring algorithm** (`score_skill`):

| Signal | Points | Cap per skill |
|--------|--------|---------------|
| Keyword exact word match | +10 per keyword | 30 total |
| Keyword substring match | +5 per keyword | 30 total |
| Tag substring match | +3 per tag | 15 total |
| Regex pattern match | +20 per pattern | 40 total |

"Exact word match" is defined as the keyword appearing as a whitespace-separated token with punctuation stripped. Keywords and tags are pre-lowercased at load time (`lowercased_keywords`, `lowercased_tags` fields) to avoid per-message allocation. Regex patterns are pre-compiled at load time.

Skills with score 0 are excluded entirely. The remaining skills are sorted by score descending, then filtered by:

1. `max_candidates` limit (hard cap on number of selected skills)
2. `max_context_tokens` budget (skills are added in score order until the remaining token budget is exhausted)

The token cost of a skill is `max(declared_tokens, 1)`. If the prompt body is estimated at more than twice the declared budget (0.25 tokens/byte heuristic), the actual estimate is used and a warning is logged.

**Default limits** (from `SkillsConfig`):

- `max_active_skills`: 3
- `max_context_tokens`: 4000 (override via `SKILLS_MAX_CONTEXT_TOKENS`)

### 2.8 Skill Attenuation (`attenuation.rs`)

Source: `src/skills/attenuation.rs`

Attenuation is the security gate that restricts the LLM's tool access based on the trust level of active skills. The principle: a skill cannot expand its own authority, only constrain it.

`attenuate_tools(tools: &[ToolDefinition], active_skills: &[LoadedSkill]) -> AttenuationResult`

**Trust model**:

| Scenario | Effective ceiling | Result |
|----------|------------------|--------|
| No skills active | `Trusted` | All tools available |
| All active skills are `Trusted` | `Trusted` | All tools available |
| Any active skill is `Installed` | `Installed` | Read-only tools only |

The effective ceiling is the **minimum** trust level across all active skills (`SkillTrust` implements `Ord` with `Installed = 0 < Trusted = 1`). This prevents privilege escalation by mixing trusted and installed skills.

**Read-only tool allowlist** (tools kept when ceiling is `Installed`):

```rust
const READ_ONLY_TOOLS: &[&str] = &[
    "memory_search",
    "memory_read",
    "memory_tree",
    "time",
    "echo",
    "json",
    "skill_list",
    "skill_search",
];
```

All other tools — including `shell`, `http`, `memory_write`, file write tools, job creation tools — are removed from the tool list before it is sent to the LLM. The LLM cannot call tools it does not know exist.

`AttenuationResult` carries the filtered tool list, the effective `min_trust`, a human-readable `explanation`, and `removed_tools` (names of tools that were removed) for transparency and logging.

---

## 3. Extensions System (`extensions/`)

Source: `src/extensions/`

### 3.1 What Extensions Are

Extensions are the user-facing abstraction over two underlying plugin mechanisms:

- **MCP servers** — hosted HTTP services implementing the Model Context Protocol, authenticated via OAuth 2.1
- **WASM tools** — sandboxed WebAssembly modules providing tools to the agent, with capabilities declared in a sidecar JSON file
- **WASM channels** — WebAssembly modules that act as input channels (Telegram, Slack, etc.); currently require a restart to activate

Unlike skills (which inject prompt text), extensions add **callable tools** to the agent's tool list.

```
User: "add notion"
  -> extension_search("notion")   -> finds MCP server in built-in registry
  -> extension_install("notion")  -> saves config to mcp-servers.json / DB
  -> extension_auth("notion")     -> OAuth 2.1 flow, returns auth URL or token prompt
  -> extension_activate("notion") -> connects to MCP server, registers tools
```

### 3.2 Extension Kinds

Defined in `src/extensions/mod.rs` as `ExtensionKind`:

| Kind | Transport | Auth | Activation |
|------|-----------|------|------------|
| `McpServer` | HTTP (SSE or JSON-RPC) | OAuth 2.1 or manual token | Runtime, no restart needed |
| `WasmTool` | In-process WASM sandbox | Capabilities file (`auth` section) | Runtime, no restart needed |
| `WasmChannel` | In-process WASM sandbox | Capabilities file | Requires restart |

### 3.3 Extension Lifecycle

All operations flow through `ExtensionManager` (`src/extensions/manager.rs`):

**Search**: Queries `ExtensionRegistry` (built-in curated list) first. If no results and `discover=true`, runs `OnlineDiscovery` concurrently (URL pattern probing + GitHub search). Discovered entries are cached in-session.

**Install**:

- MCP server: validates URL, saves `McpServerConfig` to DB or `mcp-servers.json`
- WASM tool: downloads from HTTPS URL only, validates WASM magic bytes (`\0asm`), enforces 50 MB size cap, writes to `~/.ironclaw/tools/<name>.wasm`

**Auth**:

- MCP server: attempts full OAuth 2.1 PKCE flow; falls back to building an auth URL for non-interactive environments; falls back to manual token entry if OAuth is not supported
- WASM tool: checks `auth.env_var` first, then checks secrets store; if neither, returns instructions for manual token entry

**Activate**:

- MCP server: connects client, calls `list_tools`, registers `McpTool` wrappers in `ToolRegistry`
- WASM tool: loads `.wasm` + `.capabilities.json` via `WasmToolLoader`, registers in `ToolRegistry`; also reads `hooks` section from capabilities file and registers hook bundle

**Remove**:

- MCP server: unregisters tools with server's name prefix, removes MCP client, removes config entry
- WASM tool: unregisters from `ToolRegistry`, unregisters any hooks with `plugin.tool:<name>::` prefix, deletes `.wasm` and `.capabilities.json` files
- WASM channel: manual deletion required (`~/.ironclaw/channels/`) + restart

### 3.4 Built-in Extension Registry

`ExtensionRegistry` (`src/extensions/registry.rs`) ships with a hardcoded list of well-known MCP servers:

| Name | Display Name | URL | Auth |
|------|-------------|-----|------|
| `notion` | Notion | `https://mcp.notion.com/mcp` | DCR (Dynamic Client Registration) |
| `linear` | Linear | `https://mcp.linear.app/sse` | DCR |
| `github` | GitHub Copilot | `https://api.githubcopilot.com/mcp/` | DCR |
| `slack` | Slack | `https://mcp.slack.com` | DCR |
| `sentry` | Sentry | `https://mcp.sentry.dev/mcp` | DCR |
| `stripe` | Stripe | — | DCR |
| `cloudflare` | Cloudflare | `https://mcp.cloudflare.com/mcp` | DCR |
| `asana` | Asana | `https://mcp.asana.com/v2/mcp` | DCR |
| `intercom` | Intercom | `https://mcp.intercom.com/mcp` | DCR |

> **v0.12.0 note (#370):** Google Drive and Google Calendar were removed from the built-in registry — `mcp.google.com` does not exist and Google has no official remote MCP servers. The URLs for all remaining entries were corrected to match the providers' actual endpoints.

Search scoring: exact name match = 100 points, name contains token = 50, display name contains = 30, exact keyword match = 40, keyword contains = 20, description contains = 10.

### 3.5 Embedded Registry Catalog (v0.10.0)

The extension registry now ships with an embedded catalog of known extensions. This allows offline discovery of available extensions without requiring network access. The embedded catalog is updated with each release.

> **v0.12.0 note:** The extension registry catalog is embedded in the binary for offline-capable extension discovery.

### 3.6 WASM Bundle Install Pipeline (v0.10.0)

Extensions are now installed via a download-only pipeline (`ExtensionSource::Bundled` was removed). The pipeline:

1. Downloads the WASM binary from the registry
2. Validates the bundle hash
3. Extracts to `~/.ironclaw/extensions/{name}/`
4. Registers in the database

Onboarding integration: extensions can be installed during the `ironclaw onboard` wizard.

### 3.7 Online Discovery (`discovery.rs`)

`OnlineDiscovery` runs three concurrent strategies when the built-in registry has no results:

1. **URL pattern probing** — checks `https://mcp.{service}.com`, `https://mcp.{service}.app`, `https://mcp.{service}.dev`, `https://{service}.com/mcp`
2. **GitHub search** — queries `https://api.github.com/search/repositories?q={query}+topic:mcp-server`
3. **Validation** — each candidate is checked via `GET {origin}/.well-known/oauth-protected-resource`; a JSON 200 response confirms it is a real MCP server

All sources run concurrently with an 8-second timeout on GitHub search.

---

## 4. Hooks System (`hooks/`)

Source: `src/hooks/`

### 4.1 Hook Points

Six lifecycle interception points are defined as `HookPoint` variants:

| Hook point | `as_str()` | Fires when |
|-----------|-----------|-----------|
| `BeforeInbound` | `"beforeInbound"` | Before processing a user message |
| `BeforeToolCall` | `"beforeToolCall"` | Before executing any tool call |
| `BeforeOutbound` | `"beforeOutbound"` | Before sending a response to the user |
| `OnSessionStart` | `"onSessionStart"` | When a new session is created |
| `OnSessionEnd` | `"onSessionEnd"` | When a session is ended or pruned |
| `TransformResponse` | `"transformResponse"` | Final response transformation before turn completion |

### 4.2 Hook Events

Each `HookEvent` variant carries contextual data:

```rust
Inbound    { user_id, channel, content, thread_id }
ToolCall   { tool_name, parameters, user_id, context }
Outbound   { user_id, channel, content, thread_id }
SessionStart { user_id, session_id }
SessionEnd   { user_id, session_id }
ResponseTransform { user_id, thread_id, response }
```

The "primary content" field for modification purposes is `content` (Inbound/Outbound), `parameters` serialized as JSON (ToolCall), `response` (ResponseTransform), or `session_id` (session events).

### 4.3 Hook Outcomes

`HookOutcome` has three variants:

| Variant | Effect |
|---------|--------|
| `Continue { modified: None }` | Pass through unchanged |
| `Continue { modified: Some(value) }` | Replace primary content with `value` |
| `Reject { reason }` | Stop the chain; the operation is blocked |

Modifications chain: if hook A modifies content, hook B receives the already-modified version.

### 4.4 Hook Registry (`registry.rs`)

`HookRegistry` stores hooks as `Arc<dyn Hook>` with an associated priority (lower number = higher priority; default 100).

Key behaviors:

- Hooks are sorted by priority after every `register_with_priority` call.
- Registering a hook with a duplicate name replaces the existing entry (name must be unique).
- Hooks are cloned from the registry before execution so the read lock is released before any hook runs, allowing concurrent `register`/`unregister`/`run` calls.
- A `Reject` outcome stops the chain immediately and propagates as `HookError::Rejected`.
- Hook failure modes: `FailOpen` (continue on error/timeout) or `FailClosed` (reject on error/timeout). Default: `FailOpen`.
- Per-hook timeout: default 5 seconds, max 30 seconds.

### 4.5 Bundled Hook Types (`bundled.rs`)

Two declarative hook types can be defined in JSON without writing Rust code:

**Rule hooks** (`HookRuleConfig`):

```json
{
  "name": "redact-secret",
  "points": ["beforeInbound", "beforeOutbound"],
  "priority": 50,
  "failure_mode": "fail_open",
  "timeout_ms": 2000,
  "when_regex": "secret",
  "reject_reason": null,
  "replacements": [
    { "pattern": "api_key=[^ ]+", "replacement": "api_key=[redacted]" }
  ],
  "prepend": "[filtered] ",
  "append": null
}
```

Processing order: guard check (`when_regex`) → reject if `reject_reason` set → apply `replacements` → prepend → append.

**Outbound webhook hooks** (`OutboundWebhookConfig`):

```json
{
  "name": "notify-events",
  "points": ["onSessionStart", "onSessionEnd", "beforeToolCall"],
  "url": "https://hooks.example.com/ironclaw",
  "headers": { "X-Secret": "token" },
  "timeout_ms": 2000,
  "priority": 300,
  "max_in_flight": 32
}
```

Webhook delivery is fire-and-forget (spawned as a background task). The hook always returns `Continue` immediately.

Security restrictions on outbound webhooks:

- URL must use HTTPS (no HTTP)
- No credentials in URL (no `user:pass@host`)
- Forbidden hosts: `localhost`, `*.localhost`, `host.docker.internal`, GCP/AWS metadata endpoints
- Forbidden IPs: loopback, private ranges (RFC 1918), link-local, CGNAT (100.64/10), IPv6 unique-local/loopback/multicast, documentation ranges
- IPv4-mapped IPv6 addresses are unwrapped and checked as IPv4
- DNS resolution is performed at delivery time to catch SSRF via DNS rebinding
- Forbidden request headers: `Host`, `Authorization`, `Cookie`, `Proxy-Authorization`, `Forwarded`, `X-Real-IP`, `Transfer-Encoding`, `Connection`, `X-Forwarded-*`

### 4.6 Built-in Hooks

One built-in hook ships with IronClaw:

| Hook name | Priority | Points | Purpose |
|-----------|----------|--------|---------|
| `builtin.audit_log` | 25 | All 6 | Logs every lifecycle event at `DEBUG` level to the `hooks::audit` target |

### 4.7 Hook Bootstrap (`bootstrap.rs`)

At startup, `bootstrap_hooks()` registers hooks in three layers:

1. **Bundled** — registers `builtin.audit_log` (priority 25)
2. **Plugin** — scans active WASM tools and WASM channels for a `hooks` section in their `*.capabilities.json` files, registers any rule or webhook hooks found (source prefix: `plugin.tool:<name>::` or `plugin.channel:<name>::`)
3. **Workspace** — scans workspace documents matching `hooks/hooks.json` or `hooks/*.hook.json`, parses their `hooks` key as a `HookBundleConfig`, registers found hooks (source prefix: `workspace:<path>::`)

When a WASM extension is activated at runtime via `extension_activate`, its hooks are also registered immediately (source prefix: `plugin.tool:<name>::`). When removed, all hooks with that source prefix are unregistered.

### 4.8 Hook Configuration

Hooks are configured declaratively, not via environment variables. Three sources:

| Source | Location | Scope |
|--------|----------|-------|
| Capabilities file | `{tools_dir}/<name>.capabilities.json` | Per WASM tool/channel |
| Workspace file | `hooks/hooks.json` or `hooks/*.hook.json` | Per workspace |
| Bundled | Compiled into binary | Global |

Both `{ "rules": [...], "outbound_webhooks": [...] }` object form and `[{rule}, ...]` array shorthand (rules only) are accepted.

---

## 5. Remote Extension Registry (`extensions/registry.rs`, `extensions/discovery.rs`)

### 5.1 Built-in Registry Client

`ExtensionRegistry` holds a compile-time list of well-known MCP servers and a session-lived discovery cache. Fuzzy search is token-based with a multi-field scoring system:

```
exact name match     = 100 pts
name contains token  =  50 pts
display name match   =  30 pts
exact keyword match  =  40 pts
keyword contains     =  20 pts
description match    =  10 pts
```

### 5.2 Authentication with MCP Servers

The extension manager implements OAuth 2.1 with PKCE (`src/tools/mcp/auth.rs`):

1. Discover OAuth metadata via `{server_origin}/.well-known/oauth-protected-resource`
2. Attempt Dynamic Client Registration (DCR) if no `client_id` is configured
3. Build PKCE challenge (`code_verifier` + `code_challenge`)
4. Open browser or return auth URL for manual navigation
5. Start local callback listener to receive the authorization code
6. Exchange code + verifier for access token
7. Store token in `SecretsStore` under `{server_name}_token`

Fallback path: if OAuth is not supported, prompt the user for a manual API token.

**Remote OAuth Support (v0.10.0)**: OAuth callbacks now work on remote servers, not just localhost. This enables `ironclaw auth <extension>` to work when the agent is running headless on a remote machine.

### 5.3 WASM Tool Install Flow

1. Validate URL is HTTPS
2. HTTP GET with 60-second timeout
3. Check `Content-Length` header against 50 MB cap before downloading body
4. Validate WASM magic bytes (`\0asm`)
5. Write to `~/.ironclaw/tools/<name>.wasm`
6. Optionally download `<name>.capabilities.json` sidecar

### 5.4 Skill Installation from ClawHub

Skills are separate from extensions. The skill install flow uses the built-in `skill_install` tool:

1. `skill_search(query)` — calls `SkillCatalog.search()`, returns `CatalogEntry` list with slugs
2. `skill_install(slug)` — fetches `GET /api/v1/download?slug={slug}` from ClawHub
3. Response body is the raw `SKILL.md` content
4. `SkillRegistry.install_skill(content)` is called — parses, validates, writes to disk, adds to in-memory registry

---

## 6. WASM Channel Schema (`channels/wasm/schema.rs`)

Source: `src/channels/wasm/schema.rs`

### 6.1 Capabilities File Schema

Each WASM channel declares its permissions in a sidecar JSON file (e.g., `slack.capabilities.json`). Root schema (`ChannelCapabilitiesFile`):

```json
{
  "type": "channel",
  "name": "slack",
  "description": "Slack Events API channel",
  "setup": {
    "required_secrets": [
      {
        "name": "slack_bot_token",
        "prompt": "Enter your Slack bot token",
        "validation": "^xoxb-",
        "optional": false
      },
      {
        "name": "slack_signing_secret",
        "prompt": "Enter your Slack signing secret (or leave empty to auto-generate)",
        "optional": true,
        "auto_generate": { "length": 32 }
      }
    ],
    "validation_endpoint": "https://slack.com/api/auth.test"
  },
  "capabilities": {
    "http": {
      "allowlist": [
        { "host": "slack.com", "path_prefix": "/api/" }
      ],
      "credentials": {
        "slack_bot": {
          "secret_name": "slack_bot_token",
          "location": { "type": "bearer" },
          "host_patterns": ["slack.com"]
        }
      },
      "rate_limit": { "requests_per_minute": 50, "requests_per_hour": 1000 }
    },
    "secrets": { "allowed_names": ["slack_*"] },
    "channel": {
      "allowed_paths": ["/webhook/slack"],
      "allow_polling": false,
      "workspace_prefix": "channels/slack/",
      "emit_rate_limit": { "messages_per_minute": 100, "messages_per_hour": 5000 },
      "max_message_size": 65536,
      "callback_timeout_secs": 30,
      "webhook": {
        "secret_header": "X-Slack-Signature",
        "secret_name": "slack_signing_secret"
      }
    }
  },
  "config": {
    "signing_secret_name": "slack_signing_secret"
  }
}
```

### 6.2 Channel Capabilities (`capabilities.rs`)

Source: `src/channels/wasm/capabilities.rs`

`ChannelCapabilities` extends the tool-level `Capabilities` struct with channel-specific permissions:

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `tool_capabilities` | `ToolCapabilities` | — | HTTP, secrets, workspace_read from tool caps |
| `allowed_paths` | `Vec<String>` | `[]` | HTTP paths the channel can register for webhooks |
| `allow_polling` | `bool` | `false` | Whether the channel can use polling |
| `min_poll_interval_ms` | `u32` | 30000 | Minimum poll interval (30 seconds, enforced) |
| `workspace_prefix` | `String` | `"channels/{name}/"` | All workspace writes are scoped to this prefix |
| `emit_rate_limit` | `EmitRateLimitConfig` | 100/min, 5000/hr | Rate limit for outbound message emission |
| `max_message_size` | `usize` | 65536 (64 KB) | Maximum message content size in bytes |
| `callback_timeout` | `Duration` | 30 seconds | Timeout for host callbacks |

**Workspace path isolation**: All workspace writes from a WASM channel are automatically prefixed with `channels/{name}/`. Absolute paths, `..` components, and null bytes are rejected by `validate_workspace_path()`.

**Poll interval enforcement**: Even if a channel declares `min_poll_interval_ms: 1000`, it is clamped to `MIN_POLL_INTERVAL_MS = 30000` to prevent abuse.

### 6.3 Channel Communication Protocol

WASM channels implement a host function interface. The channel module exports:

- `on_start(config_json: &str) -> ChannelConfig` — returns HTTP endpoints and polling config to register
- `on_request(method, path, headers, body) -> Response` — handles inbound webhook HTTP requests
- `on_poll() -> Vec<IncomingMessage>` — called periodically if polling is enabled

The host provides functions the channel WASM can call:

- `emit_message(user_id, content)` — send a message to the agent
- `read_workspace(path)` / `write_workspace(path, content)` — scoped to `workspace_prefix`
- `read_secret(name)` — restricted to `secrets.allowed_names` patterns
- `http_fetch(url, headers, body)` — restricted to `http.allowlist`
- `log(level, message)` — structured logging

---

## 7. Configuration Reference

### 7.1 Skills Configuration

| Environment Variable | Default | Description |
|---------------------|---------|-------------|
| `SKILLS_ENABLED` | `true` | Enable the skills system (enabled by default as of v0.10.0) |
| `SKILLS_DIR` | `~/.ironclaw/skills/` | User skills directory |
| `SKILLS_MAX_ACTIVE` | `3` | Maximum simultaneously active skills |
| `SKILLS_MAX_CONTEXT_TOKENS` | `4000` | Total prompt token budget for skills |
| `CLAWHUB_REGISTRY` | `https://wry-manatee-359.convex.site` | Overrides the compiled-in Convex backend URL for the ClawHub registry |
| `CLAWDHUB_REGISTRY` | — | Legacy alias for `CLAWHUB_REGISTRY` |

Skills are **enabled by default** as of v0.10.0 (`SKILLS_ENABLED=true`). The compiled-in registry backend is a Convex URL (`https://wry-manatee-359.convex.site`), overridable via `CLAWHUB_REGISTRY`. The skill install flow was fixed to correctly handle the registry API and WASM binary download.

> **v0.12.0 note (#300):** As of v0.12.0, skills are **enabled by default**. The skills system no longer needs to be explicitly activated — it is active on every fresh installation. The registry loading and installation pipeline were fixed in this release.

### 7.2 Skill Directory Layout

| Path | Trust | Managed by |
|------|-------|-----------|
| `<workspace>/skills/` | `Trusted` | Workspace; highest priority |
| `~/.ironclaw/skills/` | `Trusted` | User; user-placed skills only |
| `~/.ironclaw/installed_skills/` | `Installed` | Registry; `install_skill` writes here (restricted tool ceiling) |

Workspace overrides user: if both contain a skill with the same name, the workspace version is used. Registry-installed skills use `SkillTrust::Installed` (read-only tool ceiling survives restarts), while user and workspace skills use `SkillTrust::Trusted` (full tool access).

### 7.3 Extension Configuration

| Environment Variable | Default | Description |
|---------------------|---------|-------------|
| `WASM_TOOLS_DIR` | `~/.ironclaw/tools/` | Directory for installed WASM tool binaries |
| `WASM_CHANNELS_DIR` | `~/.ironclaw/channels/` | Directory for installed WASM channel binaries |

MCP server configuration is persisted in the database (`settings` table, key `mcp_servers`) or falls back to `~/.ironclaw/mcp-servers.json`.

### 7.4 Hook Configuration

Hooks have no dedicated environment variables. They are entirely configured through:

- `hooks/hooks.json` or `hooks/*.hook.json` in the workspace
- The `hooks` key in a WASM tool or channel's `*.capabilities.json` file

The built-in `builtin.audit_log` hook is always registered at startup at priority 25.

### 7.5 Security Notes

- Skills from ClawHub are installed with `SkillTrust::Installed`, which restricts the tool ceiling to read-only tools regardless of what the skill prompt requests.
- User-placed skills (`~/.ironclaw/skills/` and workspace `skills/`) are loaded with `SkillTrust::Trusted`. Never place untrusted skills in these directories.
- Outbound webhook URLs must use HTTPS and cannot target private IPs, loopback, or cloud metadata endpoints.
- WASM tool downloads require HTTPS and are validated for WASM magic bytes before being written to disk.
- Symlinks in the skills directory are rejected to prevent directory traversal via symlink attacks.
