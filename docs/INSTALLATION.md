# IronClaw Installation Guide

Practical installation guide based on real deployment experience.
Covers two paths: **pre-built binary** (fastest) and **build from source** (required for customization or bug fixes).

---

## Table of Contents

1. [Prerequisites](#1-prerequisites)
2. [Path A: Pre-built Binary](#2-path-a-pre-built-binary)
3. [Path B: Build from Source](#3-path-b-build-from-source)
4. [Database Setup](#4-database-setup)
5. [Configuration](#5-configuration)
6. [LLM Provider Setup](#6-llm-provider-setup)
7. [Run Modes](#7-run-modes)
8. [Service Mode: macOS (launchd)](#8-service-mode-macos-launchd)
9. [Service Mode: Linux (systemd)](#9-service-mode-linux-systemd)
10. [Verify the Installation](#10-verify-the-installation)
11. [Updating IronClaw](#11-updating-ironclaw)
12. [Known Issues and Gotchas](#12-known-issues-and-gotchas)

---

## 1. Prerequisites

### All platforms

| Requirement | Version | Notes |
|-------------|---------|-------|
| OS | macOS 13+ / Linux / Windows WSL | Native Windows supported via installer |
| Database | PostgreSQL 15+ **or** none | libSQL/SQLite embedded mode needs no DB server |
| LLM API | Any supported provider | See §6 |

### Build from source only

| Requirement | Version | Install |
|-------------|---------|---------|
| Rust | 1.92+ | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh` |
| pgvector | latest | Only if using PostgreSQL backend |

**macOS (Apple Silicon):** No extra steps — all dependencies via Cargo.

---

## 2. Path A: Pre-built Binary

The fastest way. Downloads a signed binary from the GitHub Releases page.

### macOS / Linux / WSL

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/nearai/ironclaw/releases/latest/download/ironclaw-installer.sh | sh
```

The installer places the binary at `~/.local/bin/ironclaw`. Make sure `~/.local/bin` is in your `$PATH`:

```bash
# Add to ~/.zshrc or ~/.bashrc if not already present
export PATH="$HOME/.local/bin:$PATH"
```

### Windows

```powershell
# PowerShell installer
irm https://github.com/nearai/ironclaw/releases/latest/download/ironclaw-installer.ps1 | iex
```

Or download the [Windows MSI installer](https://github.com/nearai/ironclaw/releases/latest/download/ironclaw-x86_64-pc-windows-msvc.msi) directly.

### Verify

```bash
ironclaw --version
```

---

## 3. Path B: Build from Source

Use this when you need to apply local patches, use a specific feature flag, or work with unreleased code.

### 3.1 Clone the repository

```bash
git clone https://github.com/nearai/ironclaw.git
cd ironclaw
```

### 3.2 Choose your database backend (feature flag)

IronClaw supports two backends selected at **compile time** via Cargo feature flags:

| Backend | Feature flag | Use case |
|---------|-------------|----------|
| PostgreSQL | `postgres` (default) | Production, full feature set |
| libSQL / SQLite | `libsql` | Zero-dependency local mode, no DB server needed |

```bash
# PostgreSQL (default) — requires PostgreSQL server
cargo build --release

# libSQL (SQLite embedded) — no external DB needed
cargo build --release --no-default-features --features libsql

# Both backends compiled in
cargo build --release --features "postgres,libsql"
```

**Recommendation for local/personal use:** `--no-default-features --features libsql` — no database server to manage, data stored at `~/.ironclaw/ironclaw.db`.

### 3.3 Install the binary

**Important:** Use `install` rather than `cp`. The `install` command is atomic (writes to a temp file then renames) and never prompts for overwrite confirmation — essential when replacing a running binary.

```bash
# Install to ~/.local/bin (no sudo needed)
mkdir -p ~/.local/bin
install -m 755 target/release/ironclaw ~/.local/bin/ironclaw

# Or install to /usr/local/bin (requires sudo)
sudo install -m 755 target/release/ironclaw /usr/local/bin/ironclaw
```

Verify:
```bash
ironclaw --version
which ironclaw
```

---

## 4. Database Setup

### Option A: libSQL (recommended for personal use)

No setup required. The database file is created automatically at `~/.ironclaw/ironclaw.db` on first run.

Set in your environment or `.env`:
```bash
DATABASE_BACKEND=libsql
# LIBSQL_PATH=~/.ironclaw/ironclaw.db  # default, can be omitted
```

**libSQL limitations** (as of v0.9.0):
- Workspace vector search not available (FTS keyword search only)
- Secrets store requires PostgreSQL
- No encryption at rest — use FileVault (macOS) or LUKS (Linux) for sensitive data

### Option B: PostgreSQL

```bash
# Create database
createdb ironclaw

# Enable pgvector extension
psql ironclaw -c "CREATE EXTENSION IF NOT EXISTS vector;"
```

Set in your environment or `.env`:
```bash
DATABASE_BACKEND=postgres
DATABASE_URL=postgres://localhost/ironclaw
```

For Turso cloud (libSQL remote):
```bash
DATABASE_BACKEND=libsql
LIBSQL_URL=libsql://your-db.turso.io
LIBSQL_AUTH_TOKEN=your-token
```

---

## 5. Configuration

### 5.1 Run the setup wizard (recommended for first run)

```bash
ironclaw onboard
```

The 7-step wizard handles:
- Database connection verification
- NEAR AI browser OAuth (GitHub or Google login)
- LLM provider selection
- Secrets encryption setup
- Web gateway configuration

Settings are persisted in the database. Bootstrap variables (`DATABASE_URL`, `LLM_BACKEND`) are written to `~/.ironclaw/.env` so they are available before the database connects.

### 5.2 Manual configuration (.env file)

Create `~/.ironclaw/.env` with your settings. The full reference with all options is in the repo at `.env.example`. Minimum working configuration:

**With NEAR AI (default):**
```bash
DATABASE_BACKEND=libsql
GATEWAY_ENABLED=true
GATEWAY_PORT=3001
GATEWAY_AUTH_TOKEN=<generate with: openssl rand -hex 32>
CLI_ENABLED=false   # required for service/daemon mode
```

**With OpenAI:**
```bash
DATABASE_BACKEND=libsql
LLM_BACKEND=openai
OPENAI_API_KEY=sk-...
OPENAI_MODEL=gpt-4o
EMBEDDING_ENABLED=true
EMBEDDING_PROVIDER=openai
EMBEDDING_MODEL=text-embedding-3-small
GATEWAY_ENABLED=true
GATEWAY_PORT=3001
GATEWAY_AUTH_TOKEN=<generate with: openssl rand -hex 32>
CLI_ENABLED=false
```

### 5.3 Configuration priority

Settings are loaded in this order (later overrides earlier):

```
compiled defaults
  → ~/.ironclaw/.env
  → ./.env (current directory)
  → shell environment variables
  → INJECTED_VARS (secrets injected at runtime)
```

Shell environment always wins. Variables set in the launchd plist or systemd unit override `.env` files.

---

## 6. LLM Provider Setup

IronClaw works with many LLM providers. Set `LLM_BACKEND` and the required credentials.

| Provider | `LLM_BACKEND` | Required vars | Notes |
|----------|---------------|---------------|-------|
| NEAR AI | `nearai` (default) | OAuth on first run | Multi-model; browser login |
| OpenAI | `openai` | `OPENAI_API_KEY` | GPT-4o, o3, etc. |
| Anthropic | `anthropic` | `ANTHROPIC_API_KEY` | Claude models |
| Ollama (local) | `ollama` | `OLLAMA_BASE_URL` (default: `http://localhost:11434`) | No API key |
| OpenRouter | `openai_compatible` | `LLM_BASE_URL`, `LLM_API_KEY` | 300+ models |
| Together AI | `openai_compatible` | `LLM_BASE_URL`, `LLM_API_KEY` | Fast inference |
| vLLM / LiteLLM | `openai_compatible` | `LLM_BASE_URL` | Self-hosted |
| Tinfoil (TEE) | `tinfoil` | `TINFOIL_API_KEY` | Private inference in hardware TEE |

### NEAR AI (default)

No config needed for basic use. On first run, `ironclaw onboard` opens a browser for OAuth.

```bash
# Optional overrides
NEARAI_MODEL=zai-org/GLM-5-FP8
NEARAI_BASE_URL=https://private.near.ai
```

For hosting/service mode where browser OAuth is not possible, use an API key:
```bash
NEARAI_API_KEY=<your-nearai-api-key>
```

### OpenAI

```bash
LLM_BACKEND=openai
OPENAI_API_KEY=sk-...
OPENAI_MODEL=gpt-4o        # or gpt-4o-mini, o3-mini
```

### Anthropic

```bash
LLM_BACKEND=anthropic
ANTHROPIC_API_KEY=sk-ant-...
# Default model: claude-sonnet-4-20250514
```

### Ollama (local inference)

```bash
# Pull model first
ollama pull llama3.2

LLM_BACKEND=ollama
OLLAMA_MODEL=llama3.2
# OLLAMA_BASE_URL=http://localhost:11434  # default
```

### OpenRouter (300+ models via one API key)

```bash
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://openrouter.ai/api/v1
LLM_API_KEY=sk-or-...
LLM_MODEL=anthropic/claude-sonnet-4   # see openrouter.ai/models
# Optional: attribution headers
LLM_EXTRA_HEADERS=HTTP-Referer:https://myapp.com,X-Title:MyApp
```

Full provider guide: `docs/LLM_PROVIDERS.md` in the repository.

---

## 7. Run Modes

### Interactive (CLI / REPL)

```bash
ironclaw
```

Launches the interactive terminal REPL. Use for interactive development and testing.

```bash
# Skip onboarding if already configured
ironclaw --no-onboard
```

### Service mode (headless)

Set `CLI_ENABLED=false` to prevent the REPL from starting. **This is required when running as a background service** — without it, the REPL reads EOF from `/dev/null` stdin and triggers graceful shutdown immediately.

```bash
CLI_ENABLED=false ironclaw --no-onboard
```

Access via the web gateway: `http://localhost:3000` (or your configured `GATEWAY_PORT`)

### REPL

IronClaw's terminal interactive mode is the REPL started by `ironclaw`.

---

## 8. Service Mode: macOS (launchd)

Run IronClaw as a LaunchAgent so it starts automatically on login and restarts on crash.

### 8.1 Create the log directory

```bash
mkdir -p ~/.ironclaw/logs
```

### 8.2 Create the LaunchAgent plist

Save as `~/Library/LaunchAgents/ai.ironclaw.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>ai.ironclaw</string>

    <key>ProgramArguments</key>
    <array>
        <string>/Users/YOUR_USERNAME/.local/bin/ironclaw</string>
        <string>--no-onboard</string>
    </array>

    <key>WorkingDirectory</key>
    <string>/Users/YOUR_USERNAME/.ironclaw</string>

    <key>EnvironmentVariables</key>
    <dict>
        <!-- Database: libSQL (no server required) -->
        <key>DATABASE_BACKEND</key><string>libsql</string>

        <!-- LLM Provider -->
        <key>LLM_BACKEND</key><string>openai</string>
        <key>OPENAI_API_KEY</key><string>sk-...</string>
        <key>OPENAI_MODEL</key><string>gpt-4o</string>

        <!-- Embeddings (optional but recommended) -->
        <key>EMBEDDING_ENABLED</key><string>true</string>
        <key>EMBEDDING_PROVIDER</key><string>openai</string>
        <key>EMBEDDING_MODEL</key><string>text-embedding-3-small</string>

        <!-- Agent -->
        <key>AGENT_NAME</key><string>ironclaw</string>
        <key>AGENT_MAX_PARALLEL_JOBS</key><string>3</string>
        <key>AGENT_JOB_TIMEOUT_SECS</key><string>3600</string>

        <!-- Web gateway -->
        <key>GATEWAY_ENABLED</key><string>true</string>
        <key>GATEWAY_HOST</key><string>127.0.0.1</string>
        <key>GATEWAY_PORT</key><string>3001</string>
        <key>GATEWAY_AUTH_TOKEN</key><string>YOUR_TOKEN_HERE</string>

        <!-- CRITICAL: must be false for service mode -->
        <!-- Without this, REPL reads EOF from /dev/null and exits immediately -->
        <key>CLI_ENABLED</key><string>false</string>

        <!-- Sandbox disabled for simple deployments -->
        <key>SANDBOX_ENABLED</key><string>false</string>

        <!-- Logging -->
        <key>RUST_LOG</key><string>ironclaw=info,tower_http=info</string>

        <!-- Required for PATH and HOME resolution -->
        <key>HOME</key><string>/Users/YOUR_USERNAME</string>
        <key>PATH</key>
        <string>/Users/YOUR_USERNAME/.local/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin</string>
    </dict>

    <!-- Restart on crash -->
    <key>KeepAlive</key><true/>
    <!-- Start at login -->
    <key>RunAtLoad</key><true/>
    <!-- Minimum seconds between restarts -->
    <key>ThrottleInterval</key><integer>10</integer>

    <key>StandardOutPath</key>
    <string>/Users/YOUR_USERNAME/.ironclaw/logs/stdout.log</string>
    <key>StandardErrorPath</key>
    <string>/Users/YOUR_USERNAME/.ironclaw/logs/stderr.log</string>
</dict>
</plist>
```

Replace `YOUR_USERNAME` and `YOUR_TOKEN_HERE` throughout. Generate a token with:
```bash
openssl rand -hex 32
```

### 8.3 Load and start the service

```bash
# Load (registers with launchd, starts if RunAtLoad=true)
launchctl load ~/Library/LaunchAgents/ai.ironclaw.plist

# Start manually (if not auto-started)
launchctl start ai.ironclaw

# Check status (PID > 0 = running)
launchctl list | grep ironclaw
```

### 8.4 Service management commands

```bash
# Stop
launchctl stop ai.ironclaw

# Restart (stop + start)
launchctl stop ai.ironclaw && launchctl start ai.ironclaw

# Unload (remove from launchd entirely)
launchctl unload ~/Library/LaunchAgents/ai.ironclaw.plist

# View live logs
tail -f ~/.ironclaw/logs/stdout.log
tail -f ~/.ironclaw/logs/stderr.log
```

---

## 9. Service Mode: Linux (systemd)

### 9.1 Create the log directory

```bash
mkdir -p ~/.ironclaw/logs
```

### 9.2 Create the systemd unit file

Save as `~/.config/systemd/user/ironclaw.service`:

```ini
[Unit]
Description=IronClaw AI Assistant
After=network.target

[Service]
Type=simple
ExecStart=%h/.local/bin/ironclaw --no-onboard
WorkingDirectory=%h/.ironclaw
Restart=always
RestartSec=10

# CRITICAL: prevents REPL EOF crash
Environment=CLI_ENABLED=false

# Database
Environment=DATABASE_BACKEND=libsql

# LLM
Environment=LLM_BACKEND=openai
Environment=OPENAI_API_KEY=sk-...
Environment=OPENAI_MODEL=gpt-4o

# Gateway
Environment=GATEWAY_ENABLED=true
Environment=GATEWAY_HOST=127.0.0.1
Environment=GATEWAY_PORT=3001
Environment=GATEWAY_AUTH_TOKEN=YOUR_TOKEN_HERE

# Logging
Environment=RUST_LOG=ironclaw=info
StandardOutput=append:%h/.ironclaw/logs/stdout.log
StandardError=append:%h/.ironclaw/logs/stderr.log

[Install]
WantedBy=default.target
```

### 9.3 Enable and start

```bash
# Reload systemd after creating/editing the unit
systemctl --user daemon-reload

# Enable to start at login
systemctl --user enable ironclaw

# Start now
systemctl --user start ironclaw

# Check status
systemctl --user status ironclaw
```

### 9.4 Service management

```bash
# Restart
systemctl --user restart ironclaw

# Stop
systemctl --user stop ironclaw

# Live logs
journalctl --user -u ironclaw -f
# Or from log files
tail -f ~/.ironclaw/logs/stdout.log
```

---

## 10. Verify the Installation

### Health check (web gateway)

```bash
curl -s \
  -H "Authorization: Bearer $GATEWAY_AUTH_TOKEN" \
  http://127.0.0.1:3001/api/health
```

Expected response: `{"status":"ok",...}`

### Send a test message

```bash
curl -s -X POST \
  -H "Authorization: Bearer $GATEWAY_AUTH_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"message": "hello, what time is it?"}' \
  http://127.0.0.1:3001/api/chat
```

### Check process

```bash
# macOS launchd
launchctl list | grep ironclaw
# Output: PID  0  ai.ironclaw  ← PID=0 means not running

# Linux systemd
systemctl --user status ironclaw
```

### Check logs for errors

```bash
tail -50 ~/.ironclaw/logs/stderr.log
```

---

## 11. Updating IronClaw

### Pre-built binary update

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/nearai/ironclaw/releases/latest/download/ironclaw-installer.sh | sh
```

Then restart the service:
```bash
# macOS
launchctl stop ai.ironclaw && launchctl start ai.ironclaw

# Linux
systemctl --user restart ironclaw
```

### Build from source update

```bash
cd ~/src/ironclaw

# Pull latest — check for local patches first
git status
git stash        # if you have local patches
git pull origin main
git stash pop    # reapply local patches (resolve any conflicts)

# Rebuild
cargo build --release --no-default-features --features libsql

# Stop service
launchctl stop ai.ironclaw          # macOS
# systemctl --user stop ironclaw    # Linux

# Replace binary (atomic, no prompt)
install -m 755 target/release/ironclaw ~/.local/bin/ironclaw

# Restart service
launchctl start ai.ironclaw         # macOS
# systemctl --user start ironclaw   # Linux

# Verify
ironclaw --version
curl -s -H "Authorization: Bearer $GATEWAY_AUTH_TOKEN" http://127.0.0.1:3001/api/health
```

**If you have local patches** that upstream hasn't merged, always `git stash` before pulling and `git stash pop` after. Check if your patch is still needed — upstream may have fixed it differently.

---

## 12. Known Issues and Gotchas

### `CLI_ENABLED=false` is mandatory for service mode

**Symptom:** Service starts then exits immediately (within 1–2 seconds).

**Cause:** When running as a launchd/systemd service, stdin is `/dev/null`. With `CLI_ENABLED=true` (default), IronClaw starts the REPL, reads EOF from `/dev/null`, interprets it as user disconnect, and initiates graceful shutdown.

**Fix:** Always set `CLI_ENABLED=false` in your service environment.

---

### Use `install` not `cp` for binary replacement

**Symptom:** `cp` prompts `overwrite /usr/local/bin/ironclaw? (y/n)` and hangs in scripts.

**Fix:** `install -m 755 src dst` — atomic write, never prompts.

---

### OpenAI 400 Bad Request on tool calls

**Symptom:** Tool calls fail with HTTP 400 from OpenAI; error mentions schema validation.

**Cause:** IronClaw's HTTP tool schema (as of v0.9.0) uses the invalid `"type": ["object", "array", ...]` array syntax for the `body` field. OpenAI's API rejects array type values.

**Fix:** Edit `src/tools/builtin/http.rs` and remove the `"type"` line from the `body` property:

```rust
// Before (broken)
"body": {
    "type": ["object", "array", "string", "number", "boolean", "null"],
    "description": "Request body (for POST/PUT/PATCH)"
},

// After (fixed)
"body": {
    "description": "Request body (for POST/PUT/PATCH)"
},
```

This fix must be re-applied after every `git pull` until upstream merges the fix.

---

### `sudo` required for `/usr/local/bin`

**Symptom:** `install: /usr/local/bin/...: Permission denied`

**Fix options:**
- Use `sudo install -m 755 ...` (requires interactive terminal for password)
- Install to `~/.local/bin/` instead (no sudo needed):
  ```bash
  mkdir -p ~/.local/bin
  install -m 755 target/release/ironclaw ~/.local/bin/ironclaw
  ```
  Then ensure `~/.local/bin` is in your `PATH` and update the `ProgramArguments` in your plist.

---

### `PATH` and `HOME` must be explicit in launchd plist

**Symptom:** IronClaw cannot find other binaries (e.g., `docker`, `claude`) when run as a launchd service.

**Cause:** launchd does not inherit the user's shell environment.

**Fix:** Set `HOME` and `PATH` explicitly in the `EnvironmentVariables` dict in the plist.

---

### Podman instead of Docker

If using Podman (rootless) instead of Docker, set:
```bash
DOCKER_HOST=unix:///run/user/$(id -u)/podman/podman.sock
```

in your plist or systemd unit. The sandbox module uses bollard which reads `DOCKER_HOST`.

---

### libSQL does not support workspace vector search

**Symptom:** `memory_search` returns no results for semantic queries even when documents exist.

**Cause:** The libSQL backend implements FTS5 keyword search only; vector search (pgvector) is not yet wired in.

**Fix:** Use keyword-rich queries with libSQL, or switch to PostgreSQL for full hybrid search.

---

*Source: IronClaw v0.9.0 · Based on hands-on deployment experience · See also: DEPLOYMENT.md, ARCHITECTURE.md*
