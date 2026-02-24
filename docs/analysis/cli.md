# IronClaw Codebase Analysis — CLI Interface

> Updated: 2026-02-24 | Version: v0.11.1

## 1. Overview

IronClaw's CLI is built with **clap v4** using the derive macro pattern. The top-level binary accepts an optional subcommand; if none is provided, the agent starts in interactive REPL/TUI mode.

```
ironclaw [OPTIONS] [SUBCOMMAND]
```

**Global options** (available on all subcommands via `global = true`):

| Flag | Description |
|------|-------------|
| `--cli-only` | Run in interactive CLI mode only, disabling other channels (HTTP, WASM) |
| `--no-db` | Skip database connection (useful for testing or offline use) |
| `-m, --message <MSG>` | Single-message mode: send one message and exit |
| `-c, --config <PATH>` | Configuration file path (default: env vars and `~/.ironclaw/config.toml`) |
| `--no-onboard` | Skip the first-run onboarding wizard check |

The `Cli` struct is defined in `src/cli/mod.rs`. The `should_run_agent()` method returns `true` when the command is `None` (no subcommand) or `Some(Command::Run)`, making agent startup the default behavior.

---

## 2. Complete Subcommand Reference

The full command tree derived from `src/cli/mod.rs` and `src/app.rs`:

| Command | Description |
|---------|-------------|
| `ironclaw` | Start agent in REPL/TUI mode (interactive, default) |
| `ironclaw run` | Explicitly start the agent (same as no subcommand) |
| `ironclaw onboard` | Run the interactive 7-step onboarding wizard |
| `ironclaw onboard --skip-auth` | Onboard without re-authenticating (use existing session) |
| `ironclaw onboard --channels-only` | Reconfigure channels only, skip other wizard steps |
| `ironclaw config init` | Generate a default `config.toml` file |
| `ironclaw config init --output <PATH>` | Write config to a specific path |
| `ironclaw config init --force` | Overwrite existing config file |
| `ironclaw config list` | List all settings and their current values |
| `ironclaw config list --filter <PREFIX>` | Show only settings matching a prefix (e.g., `agent`) |
| `ironclaw config get <PATH>` | Read a specific setting (e.g., `agent.max_parallel_jobs`) |
| `ironclaw config set <PATH> <VALUE>` | Set a runtime setting value |
| `ironclaw config reset <PATH>` | Reset a setting to its compiled-in default |
| `ironclaw config path` | Show where settings are stored (database or disk) |
| `ironclaw tool install <PATH>` | Install a WASM tool from source directory or `.wasm` file |
| `ironclaw tool install --name <NAME>` | Override the tool name during install |
| `ironclaw tool install --skip-build` | Install without recompiling (use existing artifact) |
| `ironclaw tool install --force` | Overwrite an already-installed tool |
| `ironclaw tool list` | List all installed WASM tools |
| `ironclaw tool list --verbose` | Show detailed info including hash and capabilities |
| `ironclaw tool remove <NAME>` | Remove an installed WASM tool |
| `ironclaw tool info <NAME>` | Show details and capabilities for a specific tool |
| `ironclaw tool auth <NAME>` | Configure authentication for a WASM tool (OAuth or manual) |
| `ironclaw registry list` | List available extensions in the local registry |
| `ironclaw registry list --kind <tool\|channel>` | Filter by extension kind |
| `ironclaw registry list --tag <TAG>` | Filter by tag (e.g., `default`, `google`) |
| `ironclaw registry list --verbose` | Show version, auth method, and description |
| `ironclaw registry info <NAME>` | Show detailed info about an extension or bundle |
| `ironclaw registry install <NAME>` | Install an extension or bundle |
| `ironclaw registry install --force` | Force overwrite an already-installed extension |
| `ironclaw registry install --build` | Build from source instead of downloading a pre-built artifact |
| `ironclaw registry install-defaults` | Install the default bundle of recommended extensions |
| `ironclaw mcp add <NAME> <URL>` | Add an MCP server (HTTP transport) |
| `ironclaw mcp add --client-id <ID>` | Add an MCP server with OAuth configuration |
| `ironclaw mcp add --scopes <LIST>` | Specify OAuth scopes (comma-separated) |
| `ironclaw mcp add --description <DESC>` | Add a human-readable description |
| `ironclaw mcp remove <NAME>` | Remove a configured MCP server |
| `ironclaw mcp list` | List configured MCP servers |
| `ironclaw mcp list --verbose` | Show OAuth details and enabled status |
| `ironclaw mcp auth <NAME>` | Run the OAuth flow to authenticate with an MCP server |
| `ironclaw mcp auth --user <ID>` | Authenticate for a specific user ID (default: `default`) |
| `ironclaw mcp test <NAME>` | Test connectivity and list available tools from an MCP server |
| `ironclaw mcp toggle <NAME>` | Toggle an MCP server enabled/disabled |
| `ironclaw mcp toggle --enable` | Explicitly enable a server |
| `ironclaw mcp toggle --disable` | Explicitly disable a server |
| `ironclaw memory search <QUERY>` | Hybrid FTS+vector search over workspace memory |
| `ironclaw memory search --limit <N>` | Limit results (default: 5, max: 50) |
| `ironclaw memory read <PATH>` | Read a file from the workspace (e.g., `MEMORY.md`) |
| `ironclaw memory write <PATH> [CONTENT]` | Write content to a workspace file |
| `ironclaw memory write --append` | Append to an existing file instead of overwriting |
| `ironclaw memory tree [PATH]` | Show the workspace directory tree |
| `ironclaw memory tree --depth <N>` | Limit tree traversal depth (default: 3) |
| `ironclaw memory status` | Show workspace status: file count, directories, identity files |
| `ironclaw pairing list <CHANNEL>` | List pending pairing requests for a channel |
| `ironclaw pairing list --json` | Output pairing requests as JSON |
| `ironclaw pairing approve <CHANNEL> <CODE>` | Approve a DM pairing request by code |
| `ironclaw service install` | Install the OS service (launchd on macOS, systemd on Linux) |
| `ironclaw service start` | Start the installed OS service |
| `ironclaw service stop` | Stop the running OS service |
| `ironclaw service status` | Show OS service status |
| `ironclaw service uninstall` | Remove the OS service and its unit file |
| `ironclaw doctor` | Probe external dependencies and validate configuration |
| `ironclaw status` | Show system health and component diagnostics |
| `ironclaw worker --job-id <UUID>` | Run as a sandboxed worker inside Docker (internal, not for users) |
| `ironclaw claude-bridge --job-id <UUID>` | Run as a Claude Code bridge inside Docker (internal, not for users) |

---

## 3. Service Manager (`service.rs`)

The `ironclaw service` subcommand manages IronClaw as an OS-level background service. The CLI layer (`src/cli/service.rs`) is a thin adapter: it maps `ServiceCommand` enum variants to `ServiceAction` values and delegates to `crate::service::handle_command()`.

**Available actions:**

| Subcommand | ServiceAction | Behavior |
|------------|---------------|----------|
| `install` | `Install` | Registers IronClaw with launchd (macOS) or systemd (Linux); writes the unit/plist file |
| `start` | `Start` | Starts the installed service through the OS service manager |
| `stop` | `Stop` | Stops the running service gracefully |
| `status` | `Status` | Queries the OS service manager for current state |
| `uninstall` | `Uninstall` | Removes the service registration and unit file |

**macOS note:** On macOS, `ironclaw service install` creates a launchd plist. For proper service management with automatic restart on crash and login persistence, launchd is preferred over running `ironclaw` directly in the background. The service installs to `~/Library/LaunchAgents/` for per-user launch agents.

**Linux note:** On Linux, the service integrates with systemd and installs a user unit file to `~/.config/systemd/user/`.

The `run`, `stop`, and `status` operations do not use a PID file managed by IronClaw itself — they delegate entirely to the OS service manager so the lifecycle is owned by a process supervisor that handles crash recovery and clean shutdown.

---

## 4. Doctor Checks (`doctor.rs`)

`ironclaw doctor` is an active diagnostic command that probes dependencies and surfaces problems before they affect normal operation. It prints a summary of each check with pass/fail/skip status.

**Output format:**

```
IronClaw Doctor
===============

  [pass] NEAR AI session: session found (/Users/you/.ironclaw/session.json)
  [FAIL] Database backend: PostgreSQL connection failed: ...
  [skip] Docker: docker not found in PATH
  [skip] cloudflared: cloudflared not found in PATH
  [skip] ngrok: ngrok not found in PATH
  [skip] tailscale: tailscale not found in PATH

  5 passed, 1 failed

  Some checks failed. This is normal if you don't use those features.
```

**Checks performed (in order):**

1. **NEAR AI session** — Verifies that `~/.ironclaw/session.json` (or equivalent) exists and is non-empty. Falls back to checking for `NEARAI_API_KEY` env var. Fails with guidance to run `ironclaw onboard` if neither is found.

2. **Database backend** — Reads `DATABASE_BACKEND` env var (default: `postgres`).
   - For `libsql`/`turso`/`sqlite`: checks whether the database file exists at `LIBSQL_PATH` or the default path. Reports pass even if missing (the file is created on first run).
   - For PostgreSQL: attempts a real TCP connection to `DATABASE_URL` with a 5-second timeout, executes `SELECT 1` to confirm connectivity. Fails with the connection error if unreachable.

3. **Workspace directory** — Checks that `~/.ironclaw/` exists and is a directory. Reports pass if absent (it is created on first agent start).

4. **Docker** (`docker --version`) — Skip if `docker` is not in `$PATH`. Pass if exit code is 0.

5. **cloudflared** (`cloudflared --version`) — Skip if `cloudflared` is not in `$PATH`. Used for tunnel support.

6. **ngrok** (`ngrok version`) — Skip if `ngrok` is not in `$PATH`. Alternative tunnel provider.

7. **tailscale** (`tailscale version`) — Skip if `tailscale` is not in `$PATH`. Used for mesh VPN connectivity.

**Skip semantics:** External binary checks use `Skip` (not `Fail`) when the binary is absent, because these tools are optional. A `Fail` is reserved for things that must work for the configured feature set (NEAR AI auth, database connectivity).

---

## 5. Status Command (`status.rs`)

`ironclaw status` prints a snapshot of all major subsystem states without modifying any data. It reads from environment variables and the filesystem; it does not start the agent or open interactive sessions.

**Output includes:**

| Field | Source | Notes |
|-------|--------|-------|
| Version | `CARGO_PKG_NAME`, `CARGO_PKG_VERSION` | Compiled in at build time |
| Database | `DATABASE_BACKEND` env var | Shows backend type and connection status |
| Session | `~/.ironclaw/session.json` | Checks file existence only |
| Secrets | `SECRETS_MASTER_KEY` env var | Avoids triggering macOS keychain dialogs on status check |
| Embeddings | `settings.embeddings.*` + `OPENAI_API_KEY` | Shows provider and model if enabled |
| WASM Tools | `settings.wasm.tools_dir` | Counts `.wasm` files in the tools directory |
| Channels | `settings.channels.*` | Lists `cli`, `http:<port>`, and count of WASM channel modules |
| Heartbeat | `settings.heartbeat.*` + `HEARTBEAT_ENABLED` | Shows interval in seconds if enabled |
| MCP Servers | Loaded from DB or disk config | Shows `N enabled / M configured` |
| Config path | `crate::bootstrap::ironclaw_env_path()` | Absolute path to the `.env` file |

**Database display detail:**

- For libSQL: shows the path and whether Turso cloud sync is active (`LIBSQL_URL` set).
- For PostgreSQL: attempts a live connection with a 5-second timeout and reports `connected` or the error string.

**Keychain note:** The secrets check deliberately avoids probing the macOS keychain because `get_generic_password()` triggers system authorization dialogs, which is poor UX for a read-only status command. The status output acknowledges that the key may be in the keychain even if `SECRETS_MASTER_KEY` is not set as an environment variable.

---

## 6. Config Commands (`config.rs`)

The `ironclaw config` subcommand provides full read/write access to the IronClaw settings system from the command line.

**Settings resolution priority (highest to lowest):**

```
Environment variable > TOML config file > Database (settings table) > Compiled defaults
```

**Subcommand details:**

**`config init [--output PATH] [--force]`**
Generates a `config.toml` file at `~/.ironclaw/config.toml` (or `--output` path). Reads current settings from the database if connected, so the generated file reflects live configuration. Refuses to overwrite an existing file unless `--force` is passed. The generated file contains all available settings with their current values, ready for editing.

**`config list [--filter PREFIX]`**
Prints all settings as a two-column table (key, value). Long values are truncated at 60 characters. The `--filter` flag limits output to keys matching a dot-notation prefix, e.g., `--filter agent` shows only `agent.*` settings. The source (database or defaults) is printed as a header.

**`config get <PATH>`**
Prints the value of a single setting to stdout. Exits with an error if the path is not recognized. Useful for scripting: `ironclaw config get agent.max_parallel_jobs`.

**`config set <PATH> <VALUE>`**
Persists a setting to the database. Requires a live database connection; fails with a message to check `DATABASE_URL` if the DB is unavailable. The value is attempted as JSON first (for booleans, numbers, objects), then stored as a plain string if JSON parsing fails.

**`config reset <PATH>`**
Removes the override from the database, causing the setting to revert to the TOML or compiled default. Requires a live database connection.

**`config path`**
Shows where settings are stored: `database (settings table)` if connected, or a message indicating PostgreSQL is not connected. Also prints the path to the `.env` file and the TOML config path with its existence status.

**Connection behavior:** All config commands attempt a database connection at startup. If the connection fails, commands that only read (`list`, `get`, `path`) fall back to defaults and continue. Commands that write (`set`, `reset`) require a live connection and exit with an error.

**Available setting namespaces** (from `Settings` struct in codebase):

| Namespace | Examples |
|-----------|---------|
| `agent.*` | `agent.name`, `agent.max_parallel_jobs`, `agent.max_cost_per_day_cents` |
| `embeddings.*` | `embeddings.enabled`, `embeddings.provider`, `embeddings.model` |
| `heartbeat.*` | `heartbeat.enabled`, `heartbeat.interval_secs` |
| `channels.*` | `channels.http_enabled`, `channels.http_port` |
| `wasm.*` | `wasm.enabled`, `wasm.tools_dir` |
| `sandbox.*` | `sandbox.enabled`, `sandbox.memory_limit_mb` |
| `skills.*` | `skills.enabled`, `skills.max_tokens` |

---

## 7. MCP Management (`mcp.rs`)

IronClaw supports the Model Context Protocol (MCP) for connecting to hosted tool providers over HTTP. MCP server configuration is persisted in the database (or a JSON file on disk as fallback).

**MCP server config structure** (as stored internally):

```json
{
  "name": "notion",
  "url": "https://mcp.notion.com",
  "description": "Notion workspace integration",
  "enabled": true,
  "oauth": {
    "client_id": "your-client-id",
    "scopes": ["read_content", "update_content"],
    "authorization_url": "https://api.notion.com/v1/oauth/authorize",
    "token_url": "https://api.notion.com/v1/oauth/token"
  }
}
```

**Subcommand details:**

**`mcp add <NAME> <URL> [OPTIONS]`**
Adds an MCP server entry. If `--client-id` is provided, OAuth is configured and the output message prompts the user to run `ironclaw mcp auth <name>` to complete authentication. Scopes are parsed from the comma-separated `--scopes` argument. The server is saved enabled by default.

**`mcp remove <NAME>`**
Removes a server entry. Exits with an error if the name is not found.

**`mcp list [--verbose]`**
Lists all configured servers. Compact mode shows one line per server with an enabled/disabled indicator (`●`/`○`). Verbose mode shows URL, description, OAuth client ID, and scopes.

**`mcp auth <NAME> [--user <ID>]`**
Runs a browser-based OAuth flow against the server. Uses Dynamic Client Registration (DCR) if the server supports it (no `--client-id` needed). Stores the resulting token in the encrypted secrets store under the user ID. If the server does not support OAuth, prints guidance to manually configure credentials. The flow opens a browser to the provider's authorization URL and waits on `http://localhost:9876/callback` for the redirect.

**`mcp test <NAME> [--user <ID>]`**
Connects to the server, verifies the connection succeeds, and lists all tools exposed by the server (with a truncated description for each). If a stored token exists but returns HTTP 401, prompts to re-authenticate.

**`mcp toggle <NAME> [--enable | --disable]`**
Toggles the server's `enabled` flag. When neither `--enable` nor `--disable` is passed, the flag is flipped from its current state. Disabled servers are skipped when loading tools at agent startup.

**Authentication flow:** The `mcp auth` command uses the shared OAuth infrastructure in `oauth_defaults.rs` — the same callback port (9876) and landing page as WASM tool auth and NEAR AI login. Tokens are stored via the `SecretsStore` trait (backed by PostgreSQL or libSQL AES-256-GCM encrypted storage).

---

## 8. Memory CLI (`memory.rs`)

`ironclaw memory` provides direct CLI access to the workspace/memory system without starting the agent. The workspace uses a filesystem-like path model with hybrid full-text + vector search.

**Subcommand details:**

**`memory search <QUERY> [--limit N]`**
Runs a hybrid search over all workspace memory chunks. Combines FTS (keyword) and vector (semantic, if embeddings are configured) results using Reciprocal Rank Fusion (RRF). Limit defaults to 5, maximum 50. Output shows a relevance bar indicator and a 200-character content preview per result:

```
Found 3 result(s) for "deployment checklist":

1. [=====>] (score: 0.821)
   # Production Deploy Checklist
   1. Run database migrations...

2. [====>] (score: 0.612)
   ...
```

**`memory read <PATH>`**
Reads and prints the full content of a workspace file. The path uses the workspace's logical path model (e.g., `MEMORY.md`, `daily/2024-01-15.md`, `context/vision.md`). Exits with an error if the path does not exist.

**`memory write <PATH> [CONTENT] [--append]`**
Writes content to a workspace path. If `CONTENT` is omitted, reads from stdin (useful for piping). The `--append` flag appends to an existing file rather than overwriting it. Creates the file if it does not exist.

**`memory tree [PATH] [--depth N]`**
Renders the workspace directory structure as an ASCII tree starting from `PATH` (default: root). Depth defaults to 3. Uses `├──` and `└──` connectors with `│` and `    ` indentation.

**`memory status`**
Prints a summary of the workspace:

- User ID (`default`)
- Total file count
- Unique directory count
- Presence of core identity files: `MEMORY.md`, `HEARTBEAT.md`, `IDENTITY.md`, `SOUL.md`, `AGENTS.md`, `USER.md`

**Backend wiring:** The memory CLI connects to the database via the `Database` trait (backend-agnostic). When the `postgres` feature is compiled in, a PostgreSQL-specific code path (`run_memory_command`) is also available. The embeddings provider is passed in optionally; if absent, search falls back to FTS-only.

---

## 9. Registry CLI (`registry.rs`)

`ironclaw registry` manages the local extension registry — a `registry/` directory containing manifests for WASM tools and channel extensions. This is distinct from the MCP server registry.

**Registry discovery:** The command locates the `registry/` directory by searching:

1. `./registry` relative to the current working directory (development usage)
2. Up to 3 parent directories from the executable path (installed binary)
3. `$CARGO_MANIFEST_DIR/registry` (compile-time, dev builds)

If none is found, the command fails with a message to run from the IronClaw repo root or ensure `registry/` is next to the binary.

**Extension kinds:**

| Kind | Description |
|------|-------------|
| `tool` | WASM tool — runs sandboxed in the WASM runtime |
| `channel` | WASM channel — adds a new input/output channel (e.g., Telegram, Slack) |

**Subcommand details:**

**`registry list [--kind <tool|channel>] [--tag <TAG>] [--verbose]`**
Lists all extensions from the catalog. Filters by kind or tag if specified. Compact view shows name, kind, and description. Verbose adds version and auth method. After the list, shows available bundles with a hint to use `registry info <bundle>`.

**`registry info <NAME>`**
Shows full details for an extension or bundle:

- Extension: version, description, keywords, source directory/crate, WASM artifact URL and SHA256, authentication method and secrets, tags.
- Bundle: display name, description, list of member extensions with description and kind, shared auth group name if applicable.

**`registry install <NAME> [--force] [--build]`**
Installs a named extension or bundle.

- By default, downloads the pre-built WASM artifact from the URL in the manifest.
- `--build` compiles from source using `cargo component build` (requires `cargo-component` installed).
- `--force` overwrites already-installed extensions.
- For bundles, installs all member extensions and prints a result table. Prints auth setup hints for extensions requiring credentials.
- After installation, if the extension requires authentication, prints a hint: `ironclaw tool auth <name>`.

**`registry install-defaults [--force] [--build]`**
Shorthand for `registry install default` — installs the `default` bundle, which contains the recommended set of extensions for new users.

---

## 10. Tool CLI (`tool.rs`)

`ironclaw tool` manages WASM tools installed in `~/.ironclaw/tools/`. Tools are `.wasm` component files paired with `.capabilities.json` files that declare their permissions.

**Installation paths:**

- WASM binary: `~/.ironclaw/tools/<name>.wasm`
- Capabilities: `~/.ironclaw/tools/<name>.capabilities.json`

**Subcommand details:**

**`tool install <PATH> [OPTIONS]`**
Installs a WASM tool from either:

- A **source directory** containing `Cargo.toml` — extracts the crate name, runs `cargo component build [--release]`, finds the output `.wasm` file, and copies to the tools directory.
- A **`.wasm` file** — copies directly to the tools directory.

In both cases, auto-detects the capabilities file by looking for `<name>.capabilities.json` or `capabilities.json` alongside the source. Validates the capabilities JSON before copying. Prints the tool name, destination path, size, and first 16 hex characters of the SHA-256 content hash.

**`tool list [--dir DIR] [--verbose]`**
Lists all `.wasm` files in the tools directory, sorted by name. Compact view shows name, human-readable size, and whether a capabilities file is present (`✓`/`✗`). Verbose view adds the full path, first 8 bytes of the content hash, and a capabilities summary (HTTP allowlist hosts, secrets count, workspace access).

**`tool remove <NAME> [--dir DIR]`**
Removes `<name>.wasm` and `<name>.capabilities.json` if present. Exits with an error if the tool does not exist.

**`tool info <NAME|PATH> [--dir DIR]`**
Shows detailed information about a tool:

- Path, size in bytes, full SHA-256 hash
- Full capabilities breakdown: HTTP allowlist (method, host, path prefix), credentials (key name, secret, inject location), rate limits, allowed secrets, tool aliases, workspace read prefixes

**`tool auth <NAME> [--user <ID>]`**
Configures authentication for a tool that declares an `auth` section in its capabilities file. The auth flow is selected based on the capabilities:

1. **Environment variable** — If `auth.env_var` is set and the env var is populated, uses that token directly (optionally validates against `auth.validation_endpoint`).
2. **OAuth (browser)** — If `auth.oauth` is configured, opens a browser-based PKCE authorization flow. Client credentials come from (in priority order): capabilities file → runtime env var → built-in defaults (e.g., Google Desktop App credentials in `oauth_defaults.rs`). Combines scopes from all installed tools sharing the same `secret_name` so a single login covers all Google tools.
3. **Manual entry** — Prompts for a token with hidden input (raw terminal mode). Optionally validates against `auth.validation_endpoint` before saving.

OAuth tokens and refresh tokens are stored separately in the encrypted secrets store. Access tokens record their expiry time for automatic refresh by the runtime.

---

## 11. OAuth Defaults (`oauth_defaults.rs`)

`oauth_defaults.rs` is the shared OAuth infrastructure used by every auth flow in IronClaw: WASM tool auth, MCP server auth, and the NEAR AI login during onboarding.

**Built-in credentials**

IronClaw ships with default Google OAuth "Desktop App" credentials so users do not need to register their own OAuth application. This follows the same practice as tools like `gcloud`, `rclone`, and `gdrive`. Google explicitly documents that the `client_secret` for Desktop App / Installed App OAuth types is not actually secret.

The credentials are embedded at compile time via `option_env!` macros:

```rust
const GOOGLE_CLIENT_ID: &str = match option_env!("IRONCLAW_GOOGLE_CLIENT_ID") {
    Some(v) => v,
    None => "564604149681-efo25d43rs85v0tibdepsmdv5dsrhhr0.apps.googleusercontent.com",
};
```

Override at **compile time** by setting `IRONCLAW_GOOGLE_CLIENT_ID` and `IRONCLAW_GOOGLE_CLIENT_SECRET` before running `cargo build`. Override at **runtime** by setting `GOOGLE_OAUTH_CLIENT_ID` / `GOOGLE_OAUTH_CLIENT_SECRET` env vars; these take priority over built-in defaults.

> **Security Note:** While Google's OAuth flow for installed/desktop apps treats `client_secret` as non-confidential (since it's distributed with the binary), developers should understand the security implications. For production deployments, consider using your own OAuth credentials registered with Google. See [Google's OAuth 2.0 for Installed Applications](https://developers.google.com/identity/protocols/oauth2/installed-app) for details on why this pattern is acceptable.

The `builtin_credentials(secret_name)` function is the lookup point — it returns credentials keyed by the tool's `auth.secret_name` from `capabilities.json`. Currently only `"google_oauth_token"` has built-in credentials; all other providers return `None` and must supply a `client_id` in their capabilities file or via env var.

**Shared callback server**

All OAuth redirects use a single fixed callback port:

```
http://127.0.0.1:9876/callback       (tools, MCP servers)
http://127.0.0.1:9876/auth/callback  (NEAR AI login)
```

The port `9876` is the canonical `OAUTH_CALLBACK_PORT` constant. Only one OAuth flow can run at a time (the port cannot be shared). If another flow is already running, binding fails immediately with a `PortInUse` error.

For remote/VPS deployments where `127.0.0.1` is unreachable from the user's browser, set `IRONCLAW_OAUTH_CALLBACK_URL` to a publicly reachable URL (e.g., `https://myserver.example.com:9876`). The callback listener still binds locally; the env var controls only the redirect URI registered with the provider.

**`wait_for_callback(listener, path_prefix, param_name, display_name)`**

Waits for a GET request matching `path_prefix`, extracts `param_name` from the query string, and serves a branded landing page. Times out after 5 minutes. Error handling:

- `query.contains("error=")` → `OAuthCallbackError::Denied`
- Timeout → `OAuthCallbackError::Timeout`
- Port in use → `OAuthCallbackError::PortInUse`

The landing HTML page is rendered inline with a success (green checkmark) or failure (red X) state, showing the provider name and a message to return to the terminal.

---

## 12. Pairing CLI (`pairing.rs`)

`ironclaw pairing` manages DM pairing — the approval workflow for inbound messages from unknown senders on channels like Telegram or Slack.

When a new sender messages the bot on a configured channel, IronClaw generates a pairing request with a short alphanumeric code (e.g., `ABC12345`). The user must approve the code from the CLI before the bot will respond to that sender.

**`pairing list <CHANNEL> [--json]`**

Lists pending pairing requests for a named channel (e.g., `telegram`, `slack`). Default output shows: code, sender ID, metadata key-value pairs, and creation timestamp. `--json` outputs the raw request array as pretty-printed JSON.

**`pairing approve <CHANNEL> <CODE>`**

Approves a pending request matching the code. On success, prints `Approved <channel> sender <id>`. On failure:

- Wrong code: `No pending pairing request found for code: <CODE>`
- Rate limited: `Too many failed approve attempts. Wait a few minutes.`

The `PairingStore` persists requests on disk (JSON files per channel) and is rate-limited to prevent brute-force approval attacks. The `run_pairing_command_with_store` function accepts an injected store for testability.

---

## 13. Internal Worker Commands

Two subcommands are marked as internal use by the orchestrator and are not intended for direct user invocation:

**`ironclaw worker --job-id <UUID> [--orchestrator-url <URL>] [--max-iterations <N>]`**

Runs IronClaw as a sandboxed worker process inside a Docker container. Connects back to the orchestrator's internal HTTP API (default: `http://host.docker.internal:50051`) to receive tool execution requests and stream results. The orchestrator spawns these containers per-job and terminates them on completion.

**`ironclaw claude-bridge --job-id <UUID> [--orchestrator-url <URL>] [--max-turns <N>] [--model <MODEL>]`**

Runs as a Claude Code bridge inside a Docker container. Spawns the `claude` CLI subprocess and proxies its output back to the orchestrator. Used when Claude Code mode is enabled (`CLAUDE_CODE_ENABLED=true`) to delegate job execution to the Claude CLI rather than the built-in agent loop. The `--model` flag selects the Claude model (default: `sonnet`).

---

## 14. Architecture Notes

**clap derive pattern:** All CLI types use `#[derive(Parser)]`, `#[derive(Subcommand)]`, and `#[command(...)]` attributes. The `Cli` struct in `src/cli/mod.rs` is the root parser; `Command` is the top-level subcommand enum. Each subcommand module exports its own enum (e.g., `ConfigCommand`, `McpCommand`) and a `run_*` async function.

**Database connection for CLI commands:** CLI subcommands that need settings or MCP config attempt a database connection at startup using `Config::from_env()`. If the connection fails, read operations fall back to defaults or disk, while write operations (`config set`, `mcp add`) return an error. This allows `ironclaw config list` to work offline while still preferring the live database when available.

**Secrets store for auth commands:** `tool auth` and `mcp auth` require both a live database connection and `SECRETS_MASTER_KEY` to be set. Without the master key, these commands exit with a message to run `ironclaw onboard` or set the key in `.env`.

**`--no-db` flag:** The global `--no-db` flag skips database initialization entirely, useful for testing CLI commands or running in environments where the database is unavailable. This flag is propagated through `AppBuilderFlags` to `AppBuilder::init_database()`.
