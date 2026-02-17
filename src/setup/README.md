# Setup / Onboarding Specification

This document is the authoritative specification for IronClaw's onboarding
wizard. Any code change to `src/setup/` **must** keep this document in sync.
If a future contributor or coding agent modifies setup behavior, update this
file first, then adjust the code to match.

---

## Entry Points

```
ironclaw onboard [--skip-auth] [--channels-only]
```

Explicit invocation. Loads `.env` files, runs the wizard, exits.

```
ironclaw          (first run, no database configured)
```

Auto-detection via `check_onboard_needed()` in `main.rs`. Triggers when
none of these are true:
- `DATABASE_URL` env var is set
- `LIBSQL_PATH` env var is set
- `~/.ironclaw/ironclaw.db` exists on disk

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

## The 7-Step Wizard

### Overview

```
Step 1: Database Connection
Step 2: Security (master key)
Step 3: Inference Provider          ← skipped if --skip-auth
Step 4: Model Selection
Step 5: Embeddings
Step 6: Channel Configuration
Step 7: Background Tasks (heartbeat)
       ↓
   save_and_summarize()
```

`--channels-only` mode runs only Step 6, skipping everything else.

---

### Step 1: Database Connection

**Module:** `wizard.rs` → `step_database()`

**Goal:** Select backend, establish connection, run migrations.

**Decision tree:**

```
Both features compiled?
├─ Yes → DATABASE_BACKEND env var set?
│  ├─ Yes → use that backend
│  └─ No  → interactive selection (PostgreSQL vs libSQL)
├─ Only postgres feature → step_database_postgres()
└─ Only libsql feature  → step_database_libsql()
```

**PostgreSQL path** (`step_database_postgres`):
1. Check `DATABASE_URL` from env or settings
2. Test connection (creates `deadpool_postgres::Pool`)
3. Optionally run refinery migrations
4. Store pool in `self.db_pool`

**libSQL path** (`step_database_libsql`):
1. Offer local path (default: `~/.ironclaw/ironclaw.db`)
2. Optional Turso cloud sync (URL + auth token)
3. Test connection (creates `LibSqlBackend`)
4. Always run migrations (idempotent CREATE IF NOT EXISTS)
5. Store backend in `self.db_backend`

**Invariant:** After Step 1, exactly one of `self.db_pool` or
`self.db_backend` is `Some`. This is required for settings persistence
in `save_and_summarize()`.

---

### Step 2: Security (Master Key)

**Module:** `wizard.rs` → `step_security()`

**Goal:** Configure encryption for API tokens and secrets.

**Policy constraint (must not regress):** Runtime startup and agent flows MUST NOT
silently auto-bootstrap security configuration (no automatic key generation/storage
outside the explicit onboarding flow). If Step 2 is skipped or incomplete, auth-dependent
extension operations must return actionable setup guidance.

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
| NEAR AI | Browser OAuth | (session token) | `NEARAI_SESSION_TOKEN` |
| Anthropic | API key | `anthropic_api_key` | `ANTHROPIC_API_KEY` |
| OpenAI | API key | `openai_api_key` | `OPENAI_API_KEY` |
| Ollama | None | - | - |
| OpenAI-compatible | Optional API key | `llm_compatible_api_key` | `LLM_API_KEY` |

**API-key providers** (`setup_api_key_provider`):
1. Check env var → if set, ask to reuse, persist to secrets store
2. Otherwise prompt for key entry via `secret_input()`
3. Store encrypted in secrets via `init_secrets_context()`
4. **Cache key in `self.llm_api_key`** for model fetching in Step 4

**NEAR AI** (`setup_nearai`):
- Calls `session_manager.ensure_authenticated()` which opens browser
- Session token saved to `~/.ironclaw/session.json`

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
   - OpenAI: if `OPENAI_API_KEY` in env OR (backend is `openai` AND cached key)
3. If both available → let user choose
4. If only one → use it
5. If neither → disable embeddings

**Default model:** `text-embedding-3-small` (for both providers)

---

### Step 6: Channel Configuration

**Module:** `wizard.rs` → `step_channels()`, delegating to `channels.rs`

**Goal:** Enable input channels (TUI, HTTP, Telegram, etc.).

**Sub-steps:**

```
6a. Tunnel setup (if webhook channels needed)
6b. Discover WASM channels from ~/.ironclaw/channels/
6c. Multi-select: CLI/TUI, HTTP, discovered channels, bundled channels
6d. Install missing bundled channels (copy WASM binaries)
6e. Initialize SecretsContext (for token storage)
6f. Setup HTTP webhook (if selected)
6g. Setup each WASM channel (secrets, owner binding)
```

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
- Optional webhook secret generation

**SecretsContext creation** (`init_secrets_context`):
1. Check `self.secrets_crypto` (set in Step 2) → use if available
2. Else try `SECRETS_MASTER_KEY` env var
3. Else try `get_master_key()` from keychain (only in `channels_only` mode)
4. Create backend-appropriate secrets store (respects selected database backend)

---

### Step 7: Heartbeat

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
```

Or for PostgreSQL:
```env
DATABASE_BACKEND="postgres"
DATABASE_URL="postgres://user:pass@localhost/ironclaw"
```

**Why separate?** Chicken-and-egg: you need `DATABASE_BACKEND` to know
which database to connect to, so it can't be stored in the database.

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

### save_and_summarize()

Final step of the wizard:

```
1. Mark onboard_completed = true
2. Write ALL settings to database (try postgres pool, then libSQL backend)
3. Write bootstrap vars to ~/.ironclaw/.env:
   - DATABASE_BACKEND (always)
   - DATABASE_URL     (if postgres)
   - LIBSQL_PATH      (if libsql)
   - LIBSQL_URL       (if turso sync)
4. Print configuration summary
```

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
    pub llm_backend: Option<String>,         // "nearai" | "anthropic" | "openai" | "ollama" | "openai_compatible"
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
anthropic_api_key     → encrypted API key
```

### Behavior when secrets are missing

If no master key is configured (no env value and no keychain key), extension
auth operations must not attempt automatic security setup.

Expected behavior:
- Allow install/search/activate/list for anonymous-capable MCP servers
- If auth is needed (401/unauthorized), return a clear message that secrets are not configured
- Instruct user to run `ironclaw onboard` (Step 2) or set `SECRETS_MASTER_KEY`
- Require restart before retrying auth operations

This keeps security setup explicit and user-controlled while preserving no-auth flows.

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
| `print_success(text)` | Green checkmark prefix |
| `print_error(text)` | Red X prefix |
| `print_info(text)` | Blue info prefix |

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

### Quick local secrets setup + verification

1. Run onboarding and complete security step:
   - `ironclaw onboard`
2. Choose one:
   - Keychain mode (recommended local dev), or
   - Env mode (`SECRETS_MASTER_KEY` in `~/.ironclaw/.env`)
3. Restart IronClaw.
4. Validate extension flow:
   - `ironclaw mcp list`
   - `ironclaw mcp add exa --url https://mcp.exa.ai/mcp`
   - `ironclaw mcp auth exa`
   - `ironclaw mcp test exa`
5. Missing-secrets regression check:
   - Temporarily unset/remove `SECRETS_MASTER_KEY` and keychain key
   - Start IronClaw and run `tool_install` + `tool_activate` for a no-auth server
   - Confirm no-auth MCP still works when supported by the server
   - Trigger auth (`tool_auth` or 401 path) and confirm setup guidance is shown (no auto-bootstrap)

### Linux Secret Service

- Uses GNOME Keyring or KWallet via `secret-service` crate
- May need `gnome-keyring` daemon running
- Collection unlock may prompt for password

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
