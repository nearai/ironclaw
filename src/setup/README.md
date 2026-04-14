# Setup / Onboarding Specification

This document is the authoritative specification for IronClaw's onboarding
wizard. Any code change to `src/setup/` **must** keep this document in sync.
If a future contributor or coding agent modifies setup behavior, update this
file first, then adjust the code to match.

---

## Entry Points

```
ironclaw onboard [--skip-auth] [--channels-only] [--provider-only] [--quick]
```

Explicit invocation. Loads `.env` files, runs the wizard, exits.

```
ironclaw          (first run, no database configured)
```

Auto-detection via `check_onboard_needed()` in `main.rs`. Skips onboarding
when `ONBOARD_COMPLETED` env var is set (written to `~/.ironclaw/.env` by
the wizard). Otherwise triggers when no database is configured:
- `DATABASE_URL` env var is set
- `LIBSQL_PATH` env var is set
- `~/.ironclaw/ironclaw.db` exists on disk

Auto-triggered onboarding uses **quick mode** by default.

The `--no-onboard` CLI flag suppresses auto-detection.

---

## Startup Sequence (main.rs)

```
1. Parse CLI args
2. If Command::Onboard  → load .env, run wizard, exit
3. If Command::Run or no command:
   a. Load .env files (dotenvy::dotenv() then load_ironclaw_env())
   b. check_onboard_needed() → run wizard if needed
   c. Config::from_env()     → build config from env vars
   d. Create SessionManager  → load session token
   e. ensure_authenticated() → validate session (NEAR AI only)
   f. ... rest of agent startup
```

**Critical ordering:** `.env` files must be loaded (step 3a) before
`Config::from_env()` (step 3c) because bootstrap vars like
`DATABASE_BACKEND` live in `~/.ironclaw/.env`.

---

## Quick Mode

Quick mode (`--quick` flag, or auto-triggered on first run) provides a
near-instant onboarding experience by auto-defaulting everything except
the LLM provider and model selection.

```
auto_setup_database()    → libsql at ~/.ironclaw/ironclaw.db (zero prompts)
auto_setup_security()    → keychain or env var (zero prompts)
Step 1/2: Inference Provider  ← only interactive step
Step 2/2: Model Selection     ← only interactive step
       ↓
   save_and_summarize()      → includes tip to run `ironclaw onboard`
```

**`auto_setup_database()`:** Uses existing env vars if set (`DATABASE_URL`
for postgres, `LIBSQL_PATH` for libsql) without prompting. Otherwise
defaults to libsql at `~/.ironclaw/ironclaw.db`, creates the database,
and runs migrations silently. Falls back to interactive mode only when
just the postgres feature is compiled and no `DATABASE_URL` is set.

**`auto_setup_security()`:** Checks for existing `SECRETS_MASTER_KEY`
env var or OS keychain key. If neither exists, generates a new key and
stores it in the keychain (macOS) or env var (Linux/other). Zero prompts
except unavoidable macOS keychain dialogs.

**`.env` preservation (fix for #751):** `write_bootstrap_env()` now uses
`upsert_bootstrap_vars()` instead of `save_bootstrap_env()`, preserving
user-added variables like `HTTP_HOST` across re-onboarding.

The full 9-step wizard remains available via `ironclaw onboard`.

---

## The 9-Step Wizard

### Overview

```
Step 1: Database Connection
Step 2: Security (master key)
Step 3: Inference Provider          ← skipped if --skip-auth
Step 4: Model Selection
Step 5: Embeddings
Step 6: Channel Configuration
Step 7: Extensions (tools)
Step 8: Container Sandbox
Step 9: Background Tasks (heartbeat)
       ↓
   save_and_summarize()
```

`--channels-only` mode runs only Step 6, skipping everything else.

**Personal onboarding** happens conversationally during the user's first interaction
with the running assistant (not during the wizard). The `## First-Run Bootstrap` block in
`src/workspace/mod.rs` injects onboarding instructions from `BOOTSTRAP.md` into the system
prompt on first run. Once the agent writes a profile via `memory_write` and deletes
`BOOTSTRAP.md`, the block stops injecting.

---

### Step 1: Database Connection

**Module:** `wizard.rs` → `step_database()`

**Goal:** Select backend, establish connection, run migrations.

**Init delegation:** Backend-specific connection logic lives in `src/db/mod.rs`
(`connect_without_migrations()`), not in the wizard. The wizard calls
`test_database_connection()` which delegates to the db module factory. Feature-flag
branching (`#[cfg(feature = ...)]`) is confined to `src/db/mod.rs`. PostgreSQL
validation (version >= 15, pgvector) is handled by `validate_postgres()` in
`src/db/mod.rs`.

**Decision tree:**

```
Both features compiled?
├─ Yes → DATABASE_BACKEND env var set?
│  ├─ Yes → use that backend
│  └─ No  → interactive selection (PostgreSQL vs libSQL)
├─ Only postgres feature → prompt for DATABASE_URL, test connection
└─ Only libsql feature  → prompt for path, test connection
```

**PostgreSQL path:**
1. Check `DATABASE_URL` from env or settings
2. Test connection via `connect_without_migrations()` (validates version, pgvector)
3. Optionally run migrations

**libSQL path:**
1. Offer local path (default: `~/.ironclaw/ironclaw.db`)
2. Optional Turso cloud sync (URL + auth token)
3. Test connection via `connect_without_migrations()`
4. Always run migrations (idempotent CREATE IF NOT EXISTS)

**Invariant:** After Step 1, `self.db` is `Some(Arc<dyn Database>)`.
This is required for settings persistence in `save_and_summarize()`.

---

### Step 2: Security (Master Key)

**Module:** `wizard.rs` → `step_security()`

**Goal:** Configure encryption for API tokens and secrets.

**Decision tree:**

```
SECRETS_MASTER_KEY env var set?
├─ Yes → use env var, done
└─ No  → try get_master_key() from OS keychain
   ├─ Ok(bytes) → cache in self.secrets_crypto, ask "use existing?"
   │  ├─ Yes → done (keychain)
   │  └─ No  → clear cache, fall through to options
   └─ Err   → fall through to options
              ├─ OS Keychain: generate + store + build SecretsCrypto
              ├─ Env variable: generate + print export command
              └─ Skip: disable secrets features
```

**CRITICAL CAVEAT: macOS Keychain Dialogs**

On macOS, `security_framework::get_generic_password()` can trigger TWO
system dialogs:
1. "Enter your password to unlock the keychain" (keychain locked)
2. "Allow ironclaw to access this keychain item" (per-app authorization)

This is OS-level behavior we cannot prevent. To minimize pain:

- **Use `get_master_key()` not `has_master_key()`** in step 2. Both call
  the same underlying API, but `get_master_key()` returns the key bytes
  so we can cache them. `has_master_key()` throws them away, forcing a
  second keychain access later.

- **Build `SecretsCrypto` eagerly.** When the keychain key is retrieved,
  immediately construct `SecretsCrypto` and store in `self.secrets_crypto`.
  Later calls to `init_secrets_context()` check this field first, avoiding
  redundant keychain probes.

- **Never probe the keychain in read-only commands** (e.g., `ironclaw status`).
  The status command reports "env not set (keychain may be configured)"
  rather than triggering system dialogs.

**Invariant:** After Step 2, `self.secrets_crypto` is `Some` if the user
chose Keychain or generated a new key. It may be `None` if the user chose
env-var mode or skipped secrets.

---

### Step 3: Inference Provider

**Module:** `wizard.rs` → `step_inference_provider()`

**Goal:** Choose LLM backend and authenticate.

**Providers:**

| Provider | Auth Method | Secret Name | Env Var |
|----------|-------------|-------------|---------|
| NEAR AI Chat | Browser OAuth or session token | - | `NEARAI_SESSION_TOKEN` |
| NEAR AI Cloud | API key | `llm_nearai_api_key` | `NEARAI_API_KEY` |
| Anthropic | API key | `llm_anthropic_api_key` | `ANTHROPIC_API_KEY` |
| OpenAI | API key | `llm_openai_api_key` | `OPENAI_API_KEY` |
| GitHub Copilot | OAuth token | `llm_github_copilot_token` | `GITHUB_COPILOT_TOKEN` |
| Ollama | None | - | - |
| OpenRouter | API key | `llm_openrouter_api_key` | `OPENROUTER_API_KEY` |
| OpenAI-compatible | Optional API key | `llm_compatible_api_key` | `LLM_API_KEY` |
| AWS Bedrock | AWS credentials (IAM, SSO, instance roles) | - | - |

**OpenRouter** is a standalone registry provider (`providers.json` id `"openrouter"`)
with its own secret name and env var. It is **not** stored as `openai_compatible`.

**OpenRouter** (`setup.kind = "api_key"` in `providers.json`):
- Standalone provider with base URL `https://openrouter.ai/api/v1`
- Delegates to `setup_api_key_provider()` with display name "OpenRouter"
- API key is required (`api_key_required: true`)
- Default model: `openai/gpt-4o`

**API-key providers** (`setup_api_key_provider`):
1. Check env var → if set, ask to reuse, persist to secrets store
2. Otherwise prompt for key entry via `secret_input()`
3. Store encrypted in secrets via `init_secrets_context()`
4. **Cache key in `self.llm_api_key`** for model fetching in Step 4
5. Preserve `selected_model` on a same-backend re-run; clear it only when
   switching to a different backend

**GitHub Copilot** (`setup_github_copilot`):
- Offers **GitHub device login** (recommended) or manual token paste
- Device login uses the VS Code Copilot OAuth client and stores the resulting token as `llm_github_copilot_token`
- Validates the token against `https://api.githubcopilot.com/models` before saving
- Injects `GITHUB_COPILOT_TOKEN` into the config overlay for immediate provider use

**NEAR AI** (`setup_nearai`):
- Calls `session_manager.ensure_authenticated()` which shows the auth menu:
  - Options 1-2 (GitHub/Google): browser OAuth → **NEAR AI Chat** mode
    (Responses API at `private.near.ai`, session token auth)
  - Option 4: NEAR AI Cloud API key → **NEAR AI Cloud** mode
    (Chat Completions API at `cloud-api.near.ai`, API key auth)
- **NEAR AI Chat** path: session token saved to `~/.ironclaw/session.json`.
  Hosting providers can set `NEARAI_SESSION_TOKEN` env var directly (takes
  precedence over file-based tokens).
- **NEAR AI Cloud** path: `NEARAI_API_KEY` saved to `~/.ironclaw/.env`
  (bootstrap) and encrypted secrets store (`llm_nearai_api_key`).
  `LlmConfig::resolve()` auto-selects `ChatCompletions` mode when the
  API key is present.

**`self.llm_api_key` caching:** The wizard caches the API key as
`Option<SecretString>` so that Step 4 (model fetching) and Step 5
(embeddings) can use it without re-reading from the secrets store or
mutating environment variables.

---

### Step 4: Model Selection

**Module:** `wizard.rs` → `step_model_selection()`

**Goal:** Choose which model to use.

**Flow:**
1. If model already set → offer to keep it
2. Fetch models from provider API (5-second timeout)
3. On timeout or error → use static fallback list
4. Present list + "Custom model ID" escape hatch
5. Store in `self.settings.selected_model`

**Model fetchers pass the cached API key explicitly:**
```rust
let cached = self.llm_api_key.as_ref().map(|k| k.expose_secret().to_string());
let models = fetch_anthropic_models(cached.as_deref()).await;
```

This avoids mutating environment variables. The fetcher checks the explicit
key first, then falls back to the standard env var.

---

### Step 5: Embeddings

**Module:** `wizard.rs` → `step_embeddings()`

**Goal:** Configure semantic search for workspace memory.

**Flow:**
1. Ask "Enable semantic search?" (default: yes)
2. Detect available providers:
   - NEAR AI: if backend is `nearai` OR valid session exists
   - AWS Bedrock: if backend is `bedrock`
   - OpenAI: if `OPENAI_API_KEY` in env OR (backend is `openai` AND cached key)
3. If both available → let user choose
4. If only one → use it
5. If neither → disable embeddings

**Default model:** `text-embedding-3-small` for NEAR AI/OpenAI, `amazon.titan-embed-text-v2:0` for AWS Bedrock

---

### Step 6: Channel Configuration

**Module:** `wizard.rs` → `step_channels()`, delegating to `channels.rs`

**Goal:** Enable input channels (TUI, HTTP, Telegram, etc.).

**Sub-steps:**

```
6a. Tunnel setup (if webhook channels needed)
6b. Discover WASM channels from ~/.ironclaw/channels/
6c. Build channel options: discovered + bundled + registry catalog
6d. Multi-select: CLI/TUI, HTTP, all available channels
6e. Install missing bundled channels (copy WASM binaries)
6f. Install missing registry channels (download artifacts, fallback to source build)
6g. Initialize SecretsContext (for token storage)
6h. Setup HTTP webhook (if selected)
6i. Setup each WASM channel (secrets, owner binding)
```

**Channel sources** (priority order for installation):
1. Already installed in `~/.ironclaw/channels/`
2. Bundled channels (pre-compiled in `channels-src/`)
3. Registry channels (`registry/channels/*.json`, download-first with source fallback)

**Tunnel setup** (`setup_tunnel`):
- Options: ngrok, Cloudflare Tunnel, localtunnel, custom URL
- Validates HTTPS requirement
- Stored in `self.settings.tunnel.public_url`

**WASM channel setup** (`setup_wasm_channel`):
- Reads `capabilities.json` for `setup.required_secrets`
- For each secret: check existing, prompt or auto-generate, validate regex
- Save each secret via `SecretsContext`

**Telegram special case** (`setup_telegram`):
- Validates bot token via Telegram `getMe` API
- Owner binding: polls `getUpdates` for 120s to capture sender's user ID
- Optional webhook secret auto-generation for webhook mode

**SecretsContext creation** (`init_secrets_context`):
1. Check `self.secrets_crypto` (set in Step 2) → use if available
2. Else try `SECRETS_MASTER_KEY` env var
3. Else try `get_master_key()` from keychain (only in `channels_only` mode)
4. Create secrets store using `self.db` (`Arc<dyn Database>`)

---

### Step 7: Extensions (Tools)

**Module:** `wizard.rs` → `step_extensions()`

**Goal:** Install WASM tools from the extension registry.

**Flow:**
1. Load `RegistryCatalog` from `registry/` directory
2. If registry not found, print info and skip
3. List all tool manifests from the catalog
4. Discover already-installed tools in `~/.ironclaw/tools/`
5. Multi-select: show all registry tools with display name, auth method,
   and description. Pre-check tools tagged `"default"` and already installed.
6. For each selected tool not yet installed, install via
   `RegistryInstaller::install_with_source_fallback()` (download-first,
   fallback to source build)
7. Print consolidated auth hints (deduplicated by provider, e.g. one hint
   for all Google tools sharing `google_oauth_token`)

**Google OAuth note:** Google-suite tools can use a shared built-in desktop OAuth
client for quick local setup. If the browser flow fails with "This app is blocked",
the user must provide `GOOGLE_OAUTH_CLIENT_ID` and `GOOGLE_OAUTH_CLIENT_SECRET`
or enter the matching client credentials in the extension Setup tab, then retry
the auth flow.

**Registry lookup** (`load_registry_catalog`):
Searches for `registry/` directory in order:
1. Current working directory
2. Next to the executable
3. `CARGO_MANIFEST_DIR` (compile-time, dev builds)

---

### Step 8: Container Sandbox

**Module:** `wizard.rs` → `step_docker_sandbox()`, `setup_docker_runtime()`,
`setup_kubernetes_runtime()`

**Goal:** Select a container runtime, configure it, validate connectivity,
and persist the choice.

The sandbox isolates LLM-initiated commands (shell, file writes, tool calls)
inside ephemeral containers. Without a sandbox, those commands run directly
on the host with no isolation.

**Runtime selection:** When both `docker` and `kubernetes` features are
compiled, the wizard presents an interactive menu to choose the runtime.
When only one feature is compiled, that runtime is used automatically.
The choice is persisted to `sandbox.container_runtime` in the DB settings
table and respected at startup via `resolve_runtime_backend()`.

**Flow (both features compiled):**

1. Ask "Enable container sandbox?" (default: no)
2. If declined → `sandbox.enabled = false`, done
3. Present runtime menu: Docker / Kubernetes
4. Dispatch to `setup_docker_runtime()` or `setup_kubernetes_runtime()`
5. Store selected runtime in `sandbox.container_runtime`

**Docker sub-flow (`setup_docker_runtime`):**

1. Call `check_docker()` to detect Docker installation
2. If available → enable sandbox, check/pull worker image via
   `ensure_worker_image()`
3. If not installed or not running → print platform hint, offer retry
4. On retry success → enable + ensure image; on failure → disable

**Kubernetes sub-flow (`setup_kubernetes_runtime`):**

1. Prompt for namespace with default "ironclaw"
2. Store namespace in `sandbox.k8s_namespace`
3. Connect to cluster via `KubernetesRuntime::connect(namespace)`
4. If cluster reachable (`is_available()`) → `sandbox.enabled = true`
5. If not reachable → offer retry; on failure → disable

Note: the Kubernetes path does not verify worker image pullability at
wizard time (deferred to first workload creation). It currently enables the
Stage 2 project-backed runtime: worker pods can run, and project-backed jobs
receive workspace content plus per-job config through orchestrator-served
bootstrap artifacts. Read-only sandboxed shell policies can advance further
once Kubernetes-native network enforcement is available, but workspace-write
one-shot commands still stay on Docker until uploaded workspaces can sync
writes back.

Stage 3 prerequisite reporting is available via `ironclaw doctor`. Today it
checks two operator-supplied signals:

- `IRONCLAW_K8S_NATIVE_NETWORK_CONTROLS=true`
- `IRONCLAW_K8S_PROJECTED_RUNTIME_CONFIG=true`

When both are present, doctor reports that read-only sandboxed one-shot
commands can use uploaded workspaces, runtime config can use projected files,
and the remaining cluster-side prerequisites are ready. Workspace-write
one-shot commands still need Docker until workspace write-back exists, so the
runtime stays on the Stage 2 contract.

**Flow (no runtime feature compiled):**

1. Ask "Enable container sandbox?"
2. `sandbox.enabled = false` with info message

---

### Step 9: Heartbeat

**Module:** `wizard.rs` → `step_heartbeat()`

**Goal:** Configure periodic background execution.

**Flow:**
1. Ask "Enable heartbeat?" (default: no)
2. If yes: interval in minutes (default: 30), notification channel
3. Store in `self.settings.heartbeat`

---

## Settings Persistence

### Two-Layer Architecture

Settings are persisted in two places:

**Layer 1: `~/.ironclaw/.env`** (bootstrap vars)

Contains only the settings needed BEFORE database connection. Written by
`save_bootstrap_env()` in `bootstrap.rs`.

```env
DATABASE_BACKEND="libsql"
LIBSQL_PATH="/Users/name/.ironclaw/ironclaw.db"
SECRETS_MASTER_KEY="..."   # only if env key source selected
ONBOARD_COMPLETED="true"
```

Or for PostgreSQL:
```env
DATABASE_BACKEND="postgres"
DATABASE_URL="postgres://user:pass@localhost/ironclaw"
SECRETS_MASTER_KEY="..."
ONBOARD_COMPLETED="true"
```

**Why separate?** Chicken-and-egg: you need `DATABASE_BACKEND` to know
which database to connect to, and `SECRETS_MASTER_KEY` to decrypt the
secrets store — neither can be stored in the database. LLM settings
(`LLM_BACKEND`, base URLs, model names) are persisted to the DB via
`persist_settings()` and loaded after connection. API keys are stored
encrypted in the secrets DB.

**Layer 2: Database settings table** (everything else)

All other settings are stored as key-value pairs in the `settings` table,
keyed by `(user_id, key)`. Written by `set_all_settings()`.

Settings are serialized via `Settings::to_db_map()` as dotted paths:
```
database_backend = "libsql"
llm_backend = "nearai"
selected_model = "anthropic/claude-sonnet-4-5"
embeddings.enabled = "true"
embeddings.provider = "nearai"
channels.http_enabled = "true"
heartbeat.enabled = "true"
heartbeat.interval_secs = "300"
```

### Incremental Persistence

Settings are persisted **after every successful step**, not just at the end.
This prevents data loss if a later step fails (e.g., the user enters an
API key in step 3 but step 5 crashes — they won't need to re-enter it).

**`persist_after_step()`** is called after each step in `run()` and:
1. Writes bootstrap vars to `~/.ironclaw/.env` via `write_bootstrap_env()`
2. Writes all current settings to the database via `persist_settings()`
3. Silently ignores errors (e.g., if called before Step 1 establishes a DB)

**`try_load_existing_settings()`** is called after Step 1 establishes a
database connection. It loads any previously saved settings from the
database using `get_all_settings("default")` → `Settings::from_db_map()`
→ `merge_from()`. This recovers progress from prior partial wizard runs.

**Ordering after Step 1 is critical:**

```
step_database()                        → sets DB fields in self.settings
let step1 = self.settings.clone()      → snapshot Step 1 choices
try_load_existing_settings()           → merge DB values into self.settings
self.settings.merge_from(&step1)       → re-apply Step 1 (fresh wins over stale)
persist_after_step()                   → save merged state
```

This ordering ensures:
- Prior progress (steps 2-7 from a previous partial run) is recovered
- Fresh Step 1 choices override stale DB values (not the reverse)
- The first DB persist doesn't clobber prior settings with defaults

### save_and_summarize()

Final step of the wizard:

```
1. Mark onboard_completed = true
2. Call persist_settings() for final write (idempotent — ensures
   onboard_completed flag is saved)
3. Call write_bootstrap_env() for final .env write (idempotent)
4. Print configuration summary
```

Bootstrap vars written to `~/.ironclaw/.env` (only true chicken-and-egg vars
that are needed before the DB is connected):
- `DATABASE_BACKEND` (always)
- `DATABASE_URL` (if postgres)
- `LIBSQL_PATH` (if libsql)
- `LIBSQL_URL` (if turso sync)
- `SECRETS_MASTER_KEY` (if env key source selected in Step 2)
- `ONBOARD_COMPLETED` (always, "true")
- Channel/sandbox vars: `CLAUDE_CODE_ENABLED`, `SIGNAL_HTTP_URL`, `SIGNAL_ACCOUNT`, etc. (channel init may precede DB)

LLM settings (`LLM_BACKEND`, `LLM_BASE_URL`, model, API keys) are persisted
to the DB via `persist_settings()` and loaded by `Config::from_db_with_toml()`
after connection. API keys are stored encrypted in the secrets DB and injected
via `inject_llm_keys_from_secrets()`.

**Invariant:** Both Layer 1 and Layer 2 must be written. If the database
write fails, the wizard returns an error and the `.env` file is not written.

### Legacy Migration

`bootstrap.rs` handles one-time upgrades from older config formats:
- `bootstrap.json` → extracts `DATABASE_URL`, writes `.env`, renames to `.migrated`
- `settings.json` → migrated to database via `migrate_disk_to_db()`

---

## Settings Struct

**Module:** `settings.rs`

```rust
pub struct Settings {
    // Meta
    pub onboard_completed: bool,

    // Step 1: Database
    pub database_backend: Option<String>,    // "postgres" | "libsql"
    pub database_url: Option<String>,
    pub libsql_path: Option<String>,
    pub libsql_url: Option<String>,

    // Step 2: Security
    pub secrets_master_key_source: KeySource, // Keychain | Env | None

    // Step 3: Inference
    pub llm_backend: Option<String>,         // "nearai" | "anthropic" | "openai" | "github_copilot" | "ollama" | "openai_compatible" | "bedrock"
    pub ollama_base_url: Option<String>,
    pub openai_compatible_base_url: Option<String>,

    // Step 4: Model
    pub selected_model: Option<String>,

    // Step 5: Embeddings
    pub embeddings: EmbeddingsSettings,      // enabled, provider, model

    // Step 6: Channels
    pub tunnel: TunnelSettings,              // provider, public_url
    pub channels: ChannelSettings,           // http config, telegram owner, etc.

    // Step 7: Heartbeat
    pub heartbeat: HeartbeatSettings,        // enabled, interval, notify

    // Advanced (not in wizard, set via `ironclaw config set`)
    pub agent: AgentSettings,
    pub wasm: WasmSettings,
    pub sandbox: SandboxSettings,
    pub safety: SafetySettings,
    pub builder: BuilderSettings,
}
```

**KeySource enum:** `Keychain | Env | None`

---

## Secrets Flow

### SecretsContext

Thin wrapper for setup-time secret operations:

```rust
pub struct SecretsContext {
    store: Arc<dyn SecretsStore>,
    user_id: String,
}
```

Created by `init_secrets_context()` which:
1. Gets `SecretsCrypto` from `self.secrets_crypto` or loads from keychain/env
2. Creates the appropriate backend store:
   - If both features compiled: respects `self.settings.database_backend`
   - Tries selected backend first, falls back to the other
3. Returns `SecretsContext` wrapping the store

### Secret Storage

Secrets are encrypted with AES-256-GCM using the master key, then stored
in the database `secrets` table. The wizard writes secrets like:

```
telegram_bot_token    → encrypted bot token
telegram_webhook_secret → encrypted webhook HMAC secret
llm_anthropic_api_key → encrypted API key
```

---

## Prompt Utilities

**Module:** `prompts.rs`

| Function | Description |
|----------|-------------|
| `select_one(label, options)` | Numbered single-choice menu |
| `select_many(label, options, defaults)` | Checkbox multi-select (raw terminal mode) |
| `input(label)` | Single line text input |
| `optional_input(label, hint)` | Text input that can be empty |
| `secret_input(label)` | Hidden input (shows `*` per char), returns `SecretString` |
| `confirm(label, default)` | `[Y/n]` or `[y/N]` prompt |
| `print_header(text)` | Bold section header with underline |
| `print_step(n, total, text)` | `[1/7] Step Name` |
| `print_success(text)` | Green `✓` prefix (ANSI color), message in default color |
| `print_error(text)` | Red `✗` prefix (ANSI color), message in default color |
| `print_info(text)` | Blue `ℹ` prefix (ANSI color), message in default color |

`select_many` uses `crossterm` raw mode for arrow key navigation.
Must properly restore terminal state on all exit paths.

---

## Platform Caveats

### macOS Keychain

- `get_generic_password()` triggers system dialogs (unlock + authorize)
- Two dialogs per call is normal, not a bug
- Cache the result after first access to avoid repeat prompts
- Never probe keychain in read-only commands (`status`, `--help`)
- Service name: `"ironclaw"`, account: `"master_key"`

### Linux Secret Service

- Uses GNOME Keyring or KWallet via `secret-service` crate
- May need `gnome-keyring` daemon running
- Collection unlock may prompt for password

### Remote Server Authentication

On remote/VPS servers, the browser-based OAuth flow for NEAR AI may not
work because `http://127.0.0.1:9876` is unreachable from the user's
local browser.

**Solutions:**

1. **NEAR AI Cloud API key (option 4 in auth menu):** Get an API key
   from `https://cloud.near.ai` and paste it into the terminal. No
   local listener is needed. The key is saved to `~/.ironclaw/.env`
   and the encrypted secrets store. Uses the OpenAI-compatible
   ChatCompletions API mode.

2. **Custom callback URL:** Set `IRONCLAW_OAUTH_CALLBACK_URL` to a
   publicly accessible URL (e.g., via SSH tunnel or reverse proxy) that
   forwards to port 9876 on the server:
   ```bash
   export IRONCLAW_OAUTH_CALLBACK_URL=https://myserver.example.com:9876
   ```

The `callback_url()` function in `src/auth/oauth.rs` checks this env var
and falls back to `http://127.0.0.1:{OAUTH_CALLBACK_PORT}`.

### URL Passwords

- `#` is common in URL-encoded passwords (`%23` decoded)
- `.env` values must be double-quoted to preserve `#`
- Display masked: `postgres://user:****@host/db`

### Telegram API

- Bot token format: `123456:ABC-DEF...`
- Token goes in URL path: `https://api.telegram.org/bot{TOKEN}/method`
- Webhook secret header: `X-Telegram-Bot-Api-Secret-Token`
- Owner binding polls `getUpdates` (must delete webhook first)

---

## Testing

Tests live in `mod tests {}` at the bottom of each file.

**What to test when modifying setup:**

- Settings round-trip: `to_db_map()` then `from_db_map()` preserves values
- Bootstrap `.env`: dotenvy can parse what `save_bootstrap_env()` writes
- Model fetchers: static fallback works when API is unreachable
- Channel discovery: handles missing dir, invalid JSON, deduplication
- Prompt functions: not tested (interactive I/O), but ensure error paths
  don't panic

**Run setup tests:**
```bash
cargo test --lib -- setup
cargo test --lib -- bootstrap
```

---

## Modification Checklist

When changing the onboarding flow:

1. Update this README first with the intended behavior change
2. If adding a new wizard step:
   - Add to the step enum in `run()`, adjust `total_steps`
   - Add corresponding settings fields to `Settings`
   - Add `to_db_map` / `from_db_map` serialization
   - If the setting is needed before DB connection, add to `save_bootstrap_env()`
3. If adding a new provider or channel:
   - Add to the selection menu in the appropriate step
   - Add authentication flow (API key or OAuth)
   - Add model fetcher with static fallback + 5s timeout
4. If touching keychain:
   - Cache the result, never call `get_master_key()` twice
   - Test on macOS (dialog behavior differs from Linux)
5. If touching secrets:
   - Ensure `init_secrets_context()` respects the selected database backend
   - Test with both postgres and libsql features
6. Run the full shipping checklist:
   ```bash
   cargo fmt
   cargo clippy --all --benches --tests --examples --all-features -- -D warnings
   cargo test --lib -- setup bootstrap
   ```
7. Test a fresh onboarding: `rm -rf ~/.ironclaw && cargo run`
