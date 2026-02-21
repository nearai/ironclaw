# IronClaw — Deployment & Operations Guide

> Version: v0.9.0 | Tested on: macOS 15 (Apple Silicon), macOS 14 (Intel)

This guide covers building IronClaw from source, installing it, configuring it, and
running it as a persistent background service on macOS and Linux.

---

## Table of Contents

1. [Prerequisites](#1-prerequisites)
2. [Build Options](#2-build-options)
3. [Install the Binary](#3-install-the-binary)
4. [Configuration](#4-configuration)
5. [First Run & Doctor](#5-first-run--doctor)
6. [macOS LaunchAgent (Recommended)](#6-macos-launchagent-recommended)
7. [Linux systemd Service](#7-linux-systemd-service)
8. [LLM Backend Quickstart](#8-llm-backend-quickstart)
9. [Embedding (Semantic Memory)](#9-embedding-semantic-memory)
10. [Testing Your Deployment](#10-testing-your-deployment)
11. [Known Issues & Workarounds](#11-known-issues--workarounds)
12. [Troubleshooting](#12-troubleshooting)
13. [Updating IronClaw](#13-updating-ironclaw)

---

## 1. Prerequisites

### Required

| Requirement | Version | Notes |
|-------------|---------|-------|
| Rust toolchain | 1.92+ | via `rustup` or Homebrew |
| OS | macOS 13+ / Linux (glibc 2.31+) | |
| LLM API key | — | OpenAI, Anthropic, NEAR AI, or Ollama |

### Optional (feature-dependent)

| Requirement | When Needed |
|-------------|-------------|
| PostgreSQL 15+ with pgvector | `--features postgres` (default) |
| Docker / Podman | Docker sandbox for shell tools |
| `wasm-tools` CLI | Building WASM tools (dynamic tool builder) |
| `wasm32-wasip2` Rust target | Compiling WASM tools |
| `cloudflared` / `ngrok` | Public tunnel access |
| Tailscale | Tailscale Funnel access |

### Check Rust version

```bash
rustc --version   # Must be 1.92+
cargo --version
```

If using Homebrew Rust (not rustup), `rustup` commands are unavailable.
The WASM target (`wasm32-wasip2`) requires rustup. For local embedded use
without WASM tool building, Homebrew Rust is sufficient.

---

## 2. Build Options

### Option A: libSQL only (recommended for local/personal use)

No PostgreSQL required. Uses embedded SQLite-compatible libSQL.

```bash
cd ~/src/ironclaw
cargo build --release --no-default-features --features libsql
```

Build time: ~9 minutes cold, ~3 minutes incremental.
Binary size: ~49MB (release, macOS arm64).

### Option B: Full (postgres + libSQL, default)

```bash
cd ~/src/ironclaw
cargo build --release
```

Requires PostgreSQL 15+ with pgvector extension at runtime.

### Option C: With WASM tool building support

Requires rustup (not Homebrew Rust):

```bash
rustup target add wasm32-wasip2
cargo install wasm-tools
cargo build --release --no-default-features --features libsql
```

### Build verification

```bash
./target/release/ironclaw --version
# Expected: ironclaw 0.9.0
```

---

## 3. Install the Binary

Use `install` (not `cp`) — it replaces atomically without interactive prompts,
even if the destination exists and a service is running with the old binary:

```bash
# User-local install (recommended, no sudo)
install -m 755 ~/src/ironclaw/target/release/ironclaw ~/.local/bin/ironclaw

# System-wide install (requires sudo)
sudo install -m 755 ~/src/ironclaw/target/release/ironclaw /usr/local/bin/ironclaw
```

Ensure the install directory is on your PATH:

```bash
# Add to ~/.zshrc or ~/.bashrc if needed:
export PATH="$HOME/.local/bin:$PATH"

# Verify:
which ironclaw
ironclaw --version
```

---

## 4. Configuration

IronClaw is configured via environment variables loaded from `~/.ironclaw/.env`.

### Create the config directory

```bash
mkdir -p ~/.ironclaw/logs
```

### Full annotated .env template

```bash
# ~/.ironclaw/.env — IronClaw runtime configuration
# Format: KEY=VALUE  (no quotes needed for simple values, use quotes for values with spaces)
# Priority: shell env vars > ./.env > ~/.ironclaw/.env > compiled defaults

##############################################
# DATABASE
##############################################

# Options: libsql (embedded, no server), postgres (requires PostgreSQL + pgvector)
DATABASE_BACKEND=libsql

# For libSQL: data stored at ~/.ironclaw/ironclaw.db (automatic, no config needed)
# For PostgreSQL: provide connection string
# DATABASE_URL=postgresql://user:password@localhost:5432/ironclaw

##############################################
# LLM BACKEND
##############################################

# Options: nearai, openai, anthropic, ollama, openai_compatible, tinfoil
# Default if unset: nearai (NEAR AI proxy — requires NEAR AI account)
LLM_BACKEND=openai

# --- OpenAI ---
OPENAI_API_KEY=sk-proj-...
OPENAI_MODEL=gpt-4o
# Other models: gpt-4-turbo, gpt-4o-mini, o1, o1-mini, o3-mini, o4-mini
# GPT-5 family: gpt-5.3-codex, gpt-5.3, gpt-5.2, gpt-5.1, gpt-5.0

# --- Anthropic ---
# LLM_BACKEND=anthropic
# ANTHROPIC_API_KEY=sk-ant-api03-...
# ANTHROPIC_MODEL=claude-opus-4-5-20250514
# Claude 4.x series: claude-opus-4-5, claude-sonnet-4-6, claude-haiku-4-5
# Note: sk-ant-oat01-* OAuth tokens do NOT work here (see Known Issues)

# --- NEAR AI (default) ---
# LLM_BACKEND=nearai
# NEARAI_API_KEY=...   (recommended for headless/service deployments)
# Or run onboarding for browser OAuth/session setup:
# ironclaw onboard

# --- Ollama (local LLM) ---
# LLM_BACKEND=ollama
# OLLAMA_MODEL=llama3.2
# OLLAMA_BASE_URL=http://localhost:11434  (default)

# --- OpenAI-Compatible (any endpoint: vLLM, LiteLLM, Together, etc.) ---
# LLM_BACKEND=openai_compatible
# LLM_BASE_URL=https://api.together.xyz/v1
# LLM_API_KEY=your-together-api-key
# LLM_MODEL=meta-llama/Llama-3-70b-chat-hf

# --- Tinfoil (private inference) ---
# LLM_BACKEND=tinfoil
# TINFOIL_API_KEY=...

##############################################
# EMBEDDINGS (Semantic Memory Search)
##############################################

# Enable vector embeddings for hybrid FTS+vector memory search
EMBEDDING_ENABLED=true

# Provider: currently only openai supported
EMBEDDING_PROVIDER=openai

# Model options:
# text-embedding-3-small — 1536 dims, fast, cheap (~$0.02/1M tokens)
# text-embedding-3-large — 3072 dims, more accurate, 5x cost
EMBEDDING_MODEL=text-embedding-3-small

# Dimensions must match the model
# EMBEDDING_DIMENSION=1536

##############################################
# AGENT
##############################################

# Display name for the agent (used in logs and UI)
AGENT_NAME=ironclaw

# Maximum number of concurrent jobs (requests processed in parallel)
AGENT_MAX_PARALLEL_JOBS=3

# Per-job timeout in seconds (3600 = 1 hour)
AGENT_JOB_TIMEOUT_SECS=3600

# Maximum tool-call iterations per agentic loop invocation (default: 50)
AGENT_MAX_TOOL_ITERATIONS=50

# Skip tool approval checks entirely. For benchmarks/CI (default: false)
# AGENT_AUTO_APPROVE_TOOLS=false

# Enable planning phase before tool execution (default: false)
# AGENT_USE_PLANNING=false

##############################################
# WEB GATEWAY
##############################################

# Enable the web UI and REST API
GATEWAY_ENABLED=true

# Listen address (127.0.0.1 = localhost only, 0.0.0.0 = all interfaces)
GATEWAY_HOST=127.0.0.1

# Port for the web gateway (default: 3000)
GATEWAY_PORT=3000

# Bearer auth token — ALL API requests require: Authorization: Bearer <token>
# Generate a secure random token: openssl rand -hex 32
GATEWAY_AUTH_TOKEN=<your-32-byte-hex-token-here>

##############################################
# CHANNELS
##############################################

# Enable the interactive REPL (terminal stdin/stdout)
# IMPORTANT: Set to false when running as a background service (launchd/systemd)
# When launchd runs IronClaw, stdin is /dev/null. The REPL reads stdin and
# gets an immediate EOF, which it interprets as "quit" — causing the service
# to exit immediately. CLI_ENABLED=false disables the REPL entirely.
CLI_ENABLED=false

##############################################
# SECURITY / SANDBOX
##############################################

# Enable WASM sandbox for built-in and dynamic tools
SANDBOX_ENABLED=true

##############################################
# PROACTIVE EXECUTION
##############################################

# Enable heartbeat: agent runs proactively on a schedule without user input
HEARTBEAT_ENABLED=false
# HEARTBEAT_INTERVAL_SECS=3600

##############################################
# LOGGING
##############################################

# Tracing log filter — use standard env_filter syntax
# Examples:
#   ironclaw=debug               — verbose IronClaw output
#   ironclaw=info,tower_http=info — normal operation
#   ironclaw=warn                — minimal output
RUST_LOG=ironclaw=info,tower_http=info
```

### Generate a secure auth token

```bash
openssl rand -hex 32
# Example output: a13da7efdbe51ef238283f60492e457d369bcde75618fe30184b0faf387c617c
```

---

## 5. First Run & Doctor

### Run the setup wizard (first time)

```bash
ironclaw setup
```

This walks through LLM configuration interactively and creates `~/.ironclaw/.env`.

### Run the doctor

```bash
ironclaw doctor
```

The doctor checks:

- LLM connectivity (can reach API, API key is valid)
- Database connection and migration status
- Required env vars present
- Embedding API reachable (if `EMBEDDING_ENABLED=true`)
- Gateway port available (if `GATEWAY_ENABLED=true`)
- WASM sandbox operational

Expected clean output: all items show ✓ PASS.

### Test interactively (REPL mode)

```bash
# Temporarily enable CLI for testing:
CLI_ENABLED=true ironclaw
# Type a message, press Enter, verify LLM responds
# Type 'quit' or Ctrl-D to exit
```

---

## 6. macOS LaunchAgent (Recommended)

LaunchAgent runs IronClaw as a persistent background service that:

- Starts automatically at login
- Restarts automatically on crash (KeepAlive)
- Logs to files instead of terminal

### Create the plist

Create `~/Library/LaunchAgents/ai.ironclaw.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
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

    <!-- Inline env vars — these override ~/.ironclaw/.env -->
    <key>EnvironmentVariables</key>
    <dict>
        <!-- Database -->
        <key>DATABASE_BACKEND</key><string>libsql</string>

        <!-- LLM Backend -->
        <key>LLM_BACKEND</key><string>openai</string>
        <key>OPENAI_API_KEY</key><string>sk-proj-YOUR_KEY_HERE</string>
        <key>OPENAI_MODEL</key><string>gpt-4o</string>

        <!-- Embeddings -->
        <key>EMBEDDING_ENABLED</key><string>true</string>
        <key>EMBEDDING_PROVIDER</key><string>openai</string>
        <key>EMBEDDING_MODEL</key><string>text-embedding-3-small</string>

        <!-- Agent -->
        <key>AGENT_NAME</key><string>ironclaw</string>
        <key>AGENT_MAX_PARALLEL_JOBS</key><string>3</string>
        <key>AGENT_JOB_TIMEOUT_SECS</key><string>3600</string>

        <!-- Web Gateway -->
        <key>GATEWAY_ENABLED</key><string>true</string>
        <key>GATEWAY_HOST</key><string>127.0.0.1</string>
        <key>GATEWAY_PORT</key><string>3002</string>
        <key>GATEWAY_AUTH_TOKEN</key><string>YOUR_GATEWAY_TOKEN_HERE</string>

        <!-- CRITICAL: Disable REPL in service mode -->
        <!-- Without this, launchd's /dev/null stdin causes immediate EOF -->
        <!-- which the REPL interprets as "quit", crashing the service -->
        <key>CLI_ENABLED</key><string>false</string>

        <!-- Security -->
        <key>SANDBOX_ENABLED</key><string>true</string>
        <key>HEARTBEAT_ENABLED</key><string>false</string>

        <!-- Logging -->
        <key>RUST_LOG</key><string>ironclaw=info,tower_http=info</string>

        <!-- Required system vars -->
        <key>HOME</key><string>/Users/YOUR_USERNAME</string>
        <key>PATH</key><string>/Users/YOUR_USERNAME/.local/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin</string>
    </dict>

    <!-- Restart on crash -->
    <key>KeepAlive</key><true/>

    <!-- Start at login -->
    <key>RunAtLoad</key><true/>

    <!-- Minimum 10s between restart attempts -->
    <key>ThrottleInterval</key><integer>10</integer>

    <!-- Log files -->
    <key>StandardOutPath</key>
    <string>/Users/YOUR_USERNAME/.ironclaw/logs/stdout.log</string>
    <key>StandardErrorPath</key>
    <string>/Users/YOUR_USERNAME/.ironclaw/logs/stderr.log</string>
</dict>
</plist>
```

Replace `YOUR_USERNAME` with your actual username (`echo $USER`).

### Load and start the service

```bash
# Create log directory
mkdir -p ~/.ironclaw/logs

# Load and start (macOS 13+)
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/ai.ironclaw.plist

# Verify it started
launchctl list | grep ironclaw
# Expected: <PID>    0    ai.ironclaw
# PID > 0 means running; exit code 0 means no crash
```

### Service management commands

```bash
# Stop service (but keep registered, restarts at next login)
launchctl bootout gui/$(id -u)/ai.ironclaw

# Restart service (stop + start)
launchctl bootout gui/$(id -u)/ai.ironclaw 2>/dev/null; sleep 2
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/ai.ironclaw.plist

# Check status
launchctl list | grep ironclaw

# View logs (live tail)
tail -f ~/.ironclaw/logs/stderr.log
tail -f ~/.ironclaw/logs/stdout.log

# Check last N lines of error log
tail -50 ~/.ironclaw/logs/stderr.log
```

### Update binary without service downtime

```bash
# Build new binary
cargo build --release --no-default-features --features libsql

# Replace binary atomically (no interactive prompt, works even while service runs)
install -m 755 ~/src/ironclaw/target/release/ironclaw ~/.local/bin/ironclaw

# Restart service to use new binary
launchctl bootout gui/$(id -u)/ai.ironclaw 2>/dev/null
sleep 2
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/ai.ironclaw.plist
```

> **Why `install` not `cp`?** The `cp` command prompts interactively when the
> destination file exists. `install` atomically replaces it (write to temp, rename)
> with no prompt. It's the correct POSIX tool for deploying executables.

---

## 7. Linux systemd Service

### Create the unit file

Create `/etc/systemd/system/ironclaw.service` (system service) or
`~/.config/systemd/user/ironclaw.service` (user service):

```ini
[Unit]
Description=IronClaw AI Assistant
After=network.target
Wants=network-online.target

[Service]
Type=simple
User=%i
WorkingDirectory=%h/.ironclaw
ExecStart=%h/.local/bin/ironclaw --no-onboard

# Load config from ~/.ironclaw/.env
EnvironmentFile=%h/.ironclaw/.env

# CRITICAL: Override CLI_ENABLED to prevent stdin EOF crash
Environment=CLI_ENABLED=false

# Restart on failure
Restart=on-failure
RestartSec=10

# Log to journald
StandardOutput=journal
StandardError=journal
SyslogIdentifier=ironclaw

[Install]
WantedBy=default.target
```

### Enable and start

```bash
# For user service (recommended):
systemctl --user daemon-reload
systemctl --user enable ironclaw
systemctl --user start ironclaw
systemctl --user status ironclaw

# View logs:
journalctl --user -u ironclaw -f

# For system service (as root):
systemctl daemon-reload
systemctl enable ironclaw@YOUR_USERNAME
systemctl start ironclaw@YOUR_USERNAME
```

---

## 8. LLM Backend Quickstart

### OpenAI (gpt-4o)

```bash
LLM_BACKEND=openai
OPENAI_API_KEY=sk-proj-...
OPENAI_MODEL=gpt-4o   # or gpt-4-turbo, gpt-4o-mini, o1, o3-mini
```

### Anthropic (Claude)

```bash
LLM_BACKEND=anthropic
ANTHROPIC_API_KEY=sk-ant-api03-...   # Standard API key ONLY
ANTHROPIC_MODEL=claude-opus-4-5      # or claude-sonnet-4-6, claude-haiku-4-5
```

> **Important:** Anthropic OAuth tokens (`sk-ant-oat01-*`) do NOT work
> with the standard `x-api-key` header. Only `sk-ant-api03-*` API keys work.

### NEAR AI (default)

```bash
LLM_BACKEND=nearai
# Option A (service/headless friendly): API key
NEARAI_API_KEY=<your-nearai-api-key>

# Option B (interactive): run onboarding once to authenticate
ironclaw onboard
```

### Ollama (local, no API cost)

```bash
# Install and start Ollama first:
brew install ollama
ollama pull llama3.2
ollama serve   # Runs on http://localhost:11434

# IronClaw config:
LLM_BACKEND=ollama
OLLAMA_MODEL=llama3.2
# OLLAMA_BASE_URL=http://localhost:11434  (default, usually not needed)
```

### Any OpenAI-Compatible Endpoint

Works with: vLLM, LiteLLM, Together AI, Groq, OpenRouter, Fireworks, etc.

```bash
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://api.groq.com/openai/v1
LLM_API_KEY=gsk_...
LLM_MODEL=llama-3.3-70b-versatile

# Or for a local vLLM instance:
LLM_BASE_URL=http://localhost:8000/v1
LLM_API_KEY=EMPTY
LLM_MODEL=meta-llama/Llama-3-8B-Instruct

# Optional: Extra HTTP headers (e.g., OpenRouter attribution)
LLM_EXTRA_HEADERS="HTTP-Referer:https://myapp.com,X-Title:MyApp"
```

---

## 9. Embedding (Semantic Memory)

Embeddings enable hybrid FTS+vector memory search (Reciprocal Rank Fusion).
Without embeddings, only full-text search is available.

```bash
EMBEDDING_ENABLED=true
EMBEDDING_PROVIDER=openai
EMBEDDING_MODEL=text-embedding-3-small   # 1536-dim, recommended
# EMBEDDING_MODEL=text-embedding-3-large  # 3072-dim, more accurate, 5x cost
```

> **libSQL limitation:** The libSQL backend only supports 1536-dimension embeddings.
> Use `text-embedding-3-small`. The `text-embedding-3-large` model (3072 dims) requires PostgreSQL backend.

The OpenAI API key for embeddings is read from `OPENAI_API_KEY` (same as LLM
if using OpenAI backend, or set independently otherwise).

### Verify embeddings are active

Look for this line in the startup log:

```
INFO Embeddings enabled via OpenAI (model: text-embedding-3-small, dim: 1536)
```

---

## 10. CLI Commands (Memory, Registry)

### Memory Commands

IronClaw provides CLI commands for direct workspace operations:

```bash
# Search workspace
ironclaw memory search "deployment notes"

# Read/write workspace files
ironclaw memory read context/project.md
ironclaw memory write notes/meeting.md "Key decisions..."
ironclaw memory write notes/log.md "New entry" --append

# Directory tree and status
ironclaw memory tree
ironclaw memory status
```

### Registry Commands

Browse and install extensions from ClawHub registry:

```bash
# Search for extensions
ironclaw registry search github

# Get extension details
ironclaw registry info github-tools

# Install an extension
ironclaw registry install github-tools
```

---

## 11. Testing Your Deployment

### Health check (no auth required)

```bash
curl http://127.0.0.1:3002/api/health
# Expected: {"status":"healthy","channel":"gateway"}
```

### Send a chat message

```bash
TOKEN="your-gateway-auth-token-here"

curl -X POST http://127.0.0.1:3002/api/chat/send \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"content": "Hello! What can you do?"}'

# Expected: {"message_id":"<uuid>","status":"accepted"}
```

### Read the response (wait ~5–10 seconds)

```bash
sleep 8
curl -H "Authorization: Bearer $TOKEN" \
  "http://127.0.0.1:3002/api/chat/history?limit=2"
```

### Check active jobs

```bash
curl -H "Authorization: Bearer $TOKEN" http://127.0.0.1:3002/api/jobs
```

### Open Web UI in browser

```
http://127.0.0.1:3002/
```

### Verify with complete test script

```bash
#!/bin/zsh
TOKEN="your-token-here"
BASE="http://127.0.0.1:3002"

echo "=== Health ==="
curl -s "$BASE/api/health" | python3 -m json.tool

echo "\n=== Send message ==="
MSG_ID=$(curl -s -X POST "$BASE/api/chat/send" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"content": "Say: DEPLOYMENT TEST PASSED"}' | python3 -c "import sys,json; print(json.load(sys.stdin)['message_id'])")
echo "Message ID: $MSG_ID"

echo "\n=== Wait for response ==="
sleep 10

echo "\n=== Chat history ==="
curl -s -H "Authorization: Bearer $TOKEN" "$BASE/api/chat/history?limit=2" | python3 -m json.tool
```

---

## 11. Known Issues & Workarounds

### Issue 1: Service exits immediately after start (REPL EOF crash)

**Symptom:** `launchctl list | grep ironclaw` shows PID=`-` (not running), or
the log shows "Shutdown command received" right after "Agent ironclaw ready".

**Cause:** The REPL channel reads `stdin`. When launchd runs IronClaw, stdin
is redirected from `/dev/null`. The REPL reads EOF immediately, interprets it
as a "quit" command, and triggers graceful shutdown.

**Fix:** Set `CLI_ENABLED=false` in your `.env` or plist `EnvironmentVariables`.

```bash
# In ~/.ironclaw/.env:
CLI_ENABLED=false

# Or in the plist EnvironmentVariables dict:
# <key>CLI_ENABLED</key><string>false</string>
```

---

### Issue 2: OpenAI 400 Bad Request on tool calls (JSON Schema array type)

**Symptom:** All chat requests fail with:

```
400 Bad Request: "array schema missing items"
In context=('properties', 'data', 'type', '2')
```

**Cause:** The `json` and `http` built-in tools used `"type": ["string", "object", ...]`
JSON Schema array syntax. OpenAI's API rejects this.

**Fix (already applied in this deployment):** Remove the `"type"` field from
parameters that accept multiple types. Omitting `"type"` means "any JSON value",
which is OpenAI-compatible and semantically correct.

Files fixed:

- `src/tools/builtin/json.rs:31` — removed `"type"` from `data` parameter
- `src/tools/builtin/http.rs:192` — removed `"type"` from `body` parameter

If you encounter this issue in a fresh build, apply the fix and rebuild:

```bash
# In src/tools/builtin/json.rs, change:
#   "data": { "type": ["string", "object", ...], "description": "..." }
# To:
#   "data": { "description": "..." }

# In src/tools/builtin/http.rs, change similarly for "body" parameter
cargo build --release --no-default-features --features libsql
install -m 755 target/release/ironclaw ~/.local/bin/ironclaw
# Then restart service
```

---

### Issue 3: Anthropic OAuth tokens rejected

**Symptom:** `ANTHROPIC_API_KEY=sk-ant-oat01-...` causes "invalid x-api-key" error.

**Cause:** `sk-ant-oat01-*` tokens are Anthropic OAuth tokens (issued by Claude.ai
or third-party apps). They are NOT standard API keys. The Anthropic API rejects
them when sent via `x-api-key` header. OAuth auth is not yet supported.

**Fix:** Use a standard Anthropic API key (`sk-ant-api03-*`) from
[console.anthropic.com](https://console.anthropic.com).

---

### Issue 4: Model not found with openai_compatible backend

**Symptom:** 404 or "model not found" when using `LLM_BACKEND=openai_compatible`.

**Cause:** Some endpoints use different model name formats than expected.
Also, some routing aliases (like OpenClaw's internal `gpt-5.3-codex`) do not
exist as real model names in the underlying API.

**Fix:** Check the provider's documentation for exact model names. For example:

```bash
# Groq: llama-3.3-70b-versatile (not llama3.3-70b)
# Together: meta-llama/Llama-3-70b-chat-hf (namespace/model format)
# Local vLLM: exact HuggingFace model ID
```

---

### Issue 5: `cp` prompts for overwrite when replacing binary

**Symptom:** `cp target/release/ironclaw ~/.local/bin/ironclaw` asks:
`overwrite '/Users/you/.local/bin/ironclaw'? (y/n [n])`

**Fix:** Use `install` instead (no prompt, atomic replace):

```bash
install -m 755 target/release/ironclaw ~/.local/bin/ironclaw
```

---

## 12. Troubleshooting

### Service not starting

```bash
# Check service registration
launchctl list | grep ironclaw

# Check for crash (exit code != 0)
launchctl list ai.ironclaw

# Read startup errors
tail -50 ~/.ironclaw/logs/stderr.log

# Common patterns:
# "Shutdown command received" → CLI_ENABLED=false missing
# "failed to connect to database" → DATABASE_BACKEND or DATABASE_URL misconfigured
# "invalid API key" → API key wrong or missing
# "Address already in use" → Another process on GATEWAY_PORT
```

### Port already in use

```bash
# Find what's using port 3002
lsof -i :3002

# Change port in .env:
GATEWAY_PORT=3003
# Update plist too, then restart service
```

### Database errors

```bash
# libsql: check file permissions
ls -la ~/.ironclaw/
# Should be owned by your user

# postgres: test connection
psql "$DATABASE_URL" -c "SELECT 1"

# Reset database (WARNING: deletes all data)
rm ~/.ironclaw/ironclaw.db
# Service will recreate and re-run migrations on next start
```

### LLM connection errors

```bash
# Test API key directly:
# OpenAI:
curl -H "Authorization: Bearer $OPENAI_API_KEY" \
  https://api.openai.com/v1/models | head -c 200

# Anthropic:
curl -H "x-api-key: $ANTHROPIC_API_KEY" \
  -H "anthropic-version: 2023-06-01" \
  https://api.anthropic.com/v1/models | head -c 200

# Ollama:
curl http://localhost:11434/api/tags
```

### Verbose logging

```bash
# Temporarily enable debug logging
RUST_LOG=ironclaw=debug,tower_http=debug ironclaw --no-onboard

# Or update RUST_LOG in .env and restart service
```

### Check startup log for component initialization

Healthy startup sequence in `stderr.log`:

```
INFO LLM retry wrapper enabled max_retries=3
INFO Safety layer initialized
INFO Registered 4 built-in tools
INFO Registered 4 built-in tools
INFO Embeddings enabled via OpenAI (model: text-embedding-3-small, dim: 1536)
INFO Registered 4 memory tools
INFO Registered 5 development tools
INFO Registered software builder tool
INFO Tool registry initialized with 14 total tools
INFO Web gateway enabled on 127.0.0.1:3002
INFO Agent initialized, starting main loop...
INFO Started channel: gateway
INFO Agent ironclaw ready and listening
```

---

## 13. Updating IronClaw

```bash
# 1. Pull latest source
cd ~/src/ironclaw
git pull

# 2. Build
cargo build --release --no-default-features --features libsql

# 3. Replace binary atomically
install -m 755 target/release/ironclaw ~/.local/bin/ironclaw

# 4. Restart service
launchctl bootout gui/$(id -u)/ai.ironclaw 2>/dev/null
sleep 2
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/ai.ironclaw.plist

# 5. Verify
launchctl list | grep ironclaw
curl http://127.0.0.1:3002/api/health
```

---

## What's New in v0.9.0

### v0.9.0 (2026-02-21)
- **TEE Attestation Shield**: Hardware-attested TEEs for enhanced security in web gateway UI
- **Configurable Tool Iterations**: `AGENT_MAX_TOOL_ITERATIONS` setting (default: 50)
- **Auto-Approve Tools**: `AGENT_AUTO_APPROVE_TOOLS` for CI/benchmarking
- **Planning Phase**: `AGENT_USE_PLANNING` for pre-execution planning
- **X-Accel-Buffering**: SSE endpoint performance improvements

### v0.8.0 (2026-02-20)
- **Extension Registry**: Metadata catalog with onboarding integration
- **New LLM Models**: GPT-5.3 Codex, GPT-5.x family, Claude 4.x series, o4-mini
- **Memory Hygiene**: Automatic memory cleanup in heartbeat loop
- **Parallel Tool Execution**: JoinSet-based concurrent tool calls

*Generated from deployment experience on macOS 15 (Apple Silicon) with IronClaw v0.9.0, libSQL backend, OpenAI gpt-4o, launchd service.*
