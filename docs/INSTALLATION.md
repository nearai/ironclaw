# IronClaw Installation & Deployment Guide

> Version: v0.12.0 | Tested on: macOS 15 (Apple Silicon), macOS 14 (Intel), Linux

Complete guide for installing, configuring, and deploying IronClaw as a personal AI assistant.

Related guides: [LLM_PROVIDERS.md](LLM_PROVIDERS.md), [TELEGRAM_SETUP.md](TELEGRAM_SETUP.md), [SIGNAL_SETUP.md](SIGNAL_SETUP.md), [BUILDING_CHANNELS.md](BUILDING_CHANNELS.md).

---

## Table of Contents

1. [Quick Start (5 minutes)](#1-quick-start-5-minutes)
2. [Prerequisites](#2-prerequisites)
3. [Installation Options](#3-installation-options)
4. [Configuration](#4-configuration)
5. [LLM Backend Setup](#5-llm-backend-setup)
6. [Embeddings (Semantic Memory)](#6-embeddings-semantic-memory)
7. [Run Modes](#7-run-modes)
8. [Service Setup: macOS (launchd)](#8-service-setup-macos-launchd)
9. [Service Setup: Linux (systemd)](#9-service-setup-linux-systemd)
10. [CLI Commands](#10-cli-commands)
11. [Verify Your Installation](#11-verify-your-installation)
12. [Known Issues & Workarounds](#12-known-issues--workarounds)
13. [Troubleshooting](#13-troubleshooting)
14. [Updating IronClaw](#14-updating-ironclaw)

---

## 1. Quick Start (5 minutes)

Fastest way to get IronClaw running:

```bash
# 1. Install binary
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/nearai/ironclaw/releases/latest/download/ironclaw-installer.sh | sh

# 2. Add to PATH
export PATH="$HOME/.local/bin:$PATH"

# 3. Create minimal config
mkdir -p ~/.ironclaw
cat > ~/.ironclaw/.env << 'EOF'
DATABASE_BACKEND=libsql
LLM_BACKEND=openai
OPENAI_API_KEY=sk-proj-YOUR-KEY-HERE
GATEWAY_PORT=3000
CLI_ENABLED=false
EOF

# 4. Run
ironclaw
```

Access the web UI at `http://localhost:3000`

**That's it!** For detailed configuration and production setup, continue reading.

---

## 2. Prerequisites

### Required

| Requirement | Version | Notes |
|-------------|---------|-------|
| OS | macOS 13+ / Linux / Windows WSL | Native Windows supported via installer |
| LLM API key | — | OpenAI, Anthropic, NEAR AI, Ollama, or OpenAI-compatible |

### Optional (feature-dependent)

| Requirement | When Needed |
|-------------|-------------|
| Rust 1.92+ | Building from source |
| PostgreSQL 15+ with pgvector | PostgreSQL backend (default) |
| Docker / Podman | Docker sandbox for shell tools |

---

## 3. Installation Options

### 3.1 Pre-built Binary (Recommended)

**macOS (Homebrew — easiest):**
```bash
brew install ironclaw
```

**macOS / Linux / WSL (shell installer):**
```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/nearai/ironclaw/releases/latest/download/ironclaw-installer.sh | sh
```

**Windows (PowerShell):**
```powershell
irm https://github.com/nearai/ironclaw/releases/latest/download/ironclaw-installer.ps1 | iex
```

**Verify:**
```bash
ironclaw --version
# Expected: ironclaw 0.12.0
```

### 3.2 Build from Source

Use when you need local patches, specific features, or unreleased code.

```bash
# Clone
git clone https://github.com/nearai/ironclaw.git
cd ironclaw

# Build with libSQL (zero-dependency, recommended for personal use)
cargo build --release --no-default-features --features libsql

# Or build with PostgreSQL support (default)
cargo build --release

# Install
install -m 755 target/release/ironclaw ~/.local/bin/ironclaw
```

**Build options:**

| Backend | Command | Use Case |
|---------|---------|----------|
| libSQL only | `--no-default-features --features libsql` | Zero-dependency, no DB server |
| PostgreSQL + libSQL | (default) | Production, full features |
| With WASM building | Add `rustup target add wasm32-wasip2` | Dynamic tool building |

**Build stats:** ~9 min cold build, ~3 min incremental, ~49MB binary (macOS arm64)

---

## 4. Configuration

### 4.1 Configuration Files

IronClaw loads configuration in priority order (later overrides earlier):

1. Shell environment variables
2. `./.env` in current directory
3. `~/.ironclaw/.env` (recommended location)
4. `~/.ironclaw/config.toml` (optional TOML overlay)
5. Database settings table
6. Compiled-in defaults

### 4.2 Minimal Configuration

Create `~/.ironclaw/.env`:

```bash
# Database (libSQL = zero-dependency, no server needed)
DATABASE_BACKEND=libsql

# LLM Backend
LLM_BACKEND=openai
OPENAI_API_KEY=sk-proj-YOUR-KEY-HERE

# Web Gateway
GATEWAY_ENABLED=true
GATEWAY_PORT=3000
GATEWAY_AUTH_TOKEN=<generate with: openssl rand -hex 32>

# CRITICAL: Disable REPL for service/daemon mode
CLI_ENABLED=false
```

### 4.3 Full Configuration Reference

```bash
##############################################
# Database
##############################################
DATABASE_BACKEND=libsql          # libsql (local) or postgres
# DATABASE_URL="postgres://user:pass@host/db"  # Required if postgres
# LIBSQL_PATH="~/.ironclaw/ironclaw.db"        # Default location

##############################################
# LLM Backend
##############################################
LLM_BACKEND=openai               # openai, anthropic, nearai, ollama, openai_compatible, tinfoil

# OpenAI
OPENAI_API_KEY=sk-proj-...
OPENAI_MODEL=gpt-4o              # or gpt-4-turbo, gpt-4o-mini, o1, o3-mini

# Anthropic (alternative)
# LLM_BACKEND=anthropic
# ANTHROPIC_API_KEY=sk-ant-api03-...
# ANTHROPIC_MODEL=claude-sonnet-4-20250514

# NEAR AI (alternative)
# LLM_BACKEND=nearai
# NEARAI_API_KEY=your-nearai-key
# NEARAI_MODEL=fireworks::accounts/fireworks/models/llama4-maverick-instruct-basic
# NEARAI_CHEAP_MODEL=claude-haiku-4-20250514  # For smart routing (v0.10.0+)

# Smart Routing (v0.10.0+)
# Routes simple queries to NEARAI_CHEAP_MODEL, complex to NEARAI_MODEL
# SMART_ROUTING_CASCADE=true            # Escalate uncertain cheap-model responses

# Ollama (local, no API cost)
# LLM_BACKEND=ollama
# OLLAMA_BASE_URL=http://localhost:11434
# OLLAMA_MODEL=llama3

# OpenAI-compatible (vLLM, Together, Groq, OpenRouter, etc.)
# LLM_BACKEND=openai_compatible
# LLM_BASE_URL=https://api.groq.com/openai/v1
# LLM_API_KEY=gsk_...
# LLM_MODEL=llama-3.3-70b-versatile
# LLM_EXTRA_HEADERS="HTTP-Referer:https://myapp.com,X-Title:MyApp"  # Custom headers (v0.10.0+)

##############################################
# Embeddings (Semantic Memory)
##############################################
EMBEDDING_ENABLED=true
EMBEDDING_PROVIDER=openai
EMBEDDING_MODEL=text-embedding-3-small   # 1536-dim, recommended

##############################################
# Agent
##############################################
AGENT_NAME=ironclaw
AGENT_MAX_PARALLEL_JOBS=5
AGENT_JOB_TIMEOUT_SECS=3600
# AGENT_AUTO_APPROVE_TOOLS=false      # Skip tool approvals (CI/benchmarks)
# AGENT_MAX_TOOL_ITERATIONS=50        # Max tool calls per agentic loop turn

##############################################
# Web Gateway
##############################################
GATEWAY_ENABLED=true
GATEWAY_HOST=127.0.0.1
GATEWAY_PORT=3000
GATEWAY_AUTH_TOKEN=<your-32-byte-hex-token>

##############################################
# Docker Sandbox
##############################################
SANDBOX_ENABLED=true
SANDBOX_POLICY=readonly            # readonly, workspace_write, full_access
SANDBOX_TIMEOUT_SECS=120
SANDBOX_MEMORY_LIMIT_MB=2048

##############################################
# Claude Code Mode (optional)
##############################################
# CLAUDE_CODE_ENABLED=false
# CLAUDE_CODE_MODEL=sonnet
# CLAUDE_CODE_MAX_TURNS=50

##############################################
# Logging
##############################################
RUST_LOG=ironclaw=info,tower_http=info
```

---

## 5. LLM Backend Setup

### OpenAI

```bash
LLM_BACKEND=openai
OPENAI_API_KEY=sk-proj-...
OPENAI_MODEL=gpt-4o
```

### Anthropic (Claude)

```bash
LLM_BACKEND=anthropic
ANTHROPIC_API_KEY=sk-ant-api03-...   # Standard API key ONLY
ANTHROPIC_MODEL=claude-sonnet-4-20250514
```

> **Important:** OAuth tokens (`sk-ant-oat01-*`) do NOT work. Use standard API keys from [platform.claude.com](https://platform.claude.com).

### NEAR AI

```bash
LLM_BACKEND=nearai
# Option A: API key (headless)
NEARAI_API_KEY=your-nearai-api-key

# Option B: Interactive auth (run once)
ironclaw onboard
```

### Ollama (Local, Free)

```bash
# Install and start Ollama
brew install ollama
ollama pull llama3.2
ollama serve

# IronClaw config
LLM_BACKEND=ollama
OLLAMA_MODEL=llama3.2
```

### OpenAI-Compatible (vLLM, Together, Groq, OpenRouter)

```bash
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://api.groq.com/openai/v1
LLM_API_KEY=gsk_...
LLM_MODEL=llama-3.3-70b-versatile
# Custom HTTP headers (v0.10.0+): comma-separated Key:Value pairs
# LLM_EXTRA_HEADERS="HTTP-Referer:https://myapp.com,X-Title:MyApp"
```

As of v0.12.0, **OpenRouter** is available as a dedicated preset option in the wizard (option 5), pre-configured with `https://openrouter.ai/api/v1`.

---

## 6. Embeddings (Semantic Memory)

Embeddings enable hybrid FTS+vector memory search. Without embeddings, only keyword search is available.

```bash
EMBEDDING_ENABLED=true
EMBEDDING_PROVIDER=openai
EMBEDDING_MODEL=text-embedding-3-small   # 1536-dim
```

> **libSQL limitation:** Only supports 1536-dimension embeddings. Use `text-embedding-3-small`. The `text-embedding-3-large` model (3072 dims) requires PostgreSQL.

Verify embeddings are active in logs:
```
INFO Embeddings enabled via OpenAI (model: text-embedding-3-small, dim: 1536)
```

---

## 7. Run Modes

### Interactive Mode (Development)

```bash
ironclaw
# REPL active, reads from stdin
```

### Service Mode (Production)

Set `CLI_ENABLED=false` to prevent REPL from reading stdin (required for launchd/systemd).

```bash
# In ~/.ironclaw/.env:
CLI_ENABLED=false

# Run
ironclaw
```

### One-shot Mode

```bash
ironclaw --message "What is the capital of France?"
# or
ironclaw -m "What is the capital of France?"
```

---

## 8. Service Setup: macOS (launchd)

### 8.1 Create LaunchAgent

Create `~/Library/LaunchAgents/ai.ironclaw.plist`:

> **Note:** Replace all instances of `YOUR_USERNAME` with your actual macOS username in the plist below.

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
    </array>
    
    <key>EnvironmentVariables</key>
    <dict>
        <key>HOME</key>
        <string>/Users/YOUR_USERNAME</string>
        <key>PATH</key>
        <string>/Users/YOUR_USERNAME/.local/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin</string>
        <key>CLI_ENABLED</key>
        <string>false</string>
        <key>GATEWAY_PORT</key>
        <string>3000</string>
        <!-- Replace YOUR_TOKEN_HERE with output of: openssl rand -hex 32 -->
        <key>GATEWAY_AUTH_TOKEN</key>
        <string>YOUR_TOKEN_HERE</string>
        <key>SANDBOX_ENABLED</key>
        <string>true</string>
        <key>RUST_LOG</key>
        <string>ironclaw=info,tower_http=info</string>
    </dict>
    
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    
    <key>StandardOutPath</key>
    <string>/Users/YOUR_USERNAME/.ironclaw/logs/stdout.log</string>
    <key>StandardErrorPath</key>
    <string>/Users/YOUR_USERNAME/.ironclaw/logs/stderr.log</string>
</dict>
</plist>
```

### 8.2 Enable the Service

```bash
# Create log directory
mkdir -p ~/.ironclaw/logs

# Load and start
launchctl load ~/Library/LaunchAgents/ai.ironclaw.plist

# Verify
launchctl list | grep ironclaw
```

### 8.3 Manage the Service

```bash
# Stop
launchctl unload ~/Library/LaunchAgents/ai.ironclaw.plist

# Restart
launchctl unload ~/Library/LaunchAgents/ai.ironclaw.plist
launchctl load ~/Library/LaunchAgents/ai.ironclaw.plist

# View logs
tail -f ~/.ironclaw/logs/stderr.log
```

---

## 9. Service Setup: Linux (systemd)

### 9.1 Create Service File

Create `~/.config/systemd/user/ironclaw.service`:

```ini
[Unit]
Description=IronClaw AI Assistant
After=network.target

[Service]
Type=simple
ExecStart=%h/.local/bin/ironclaw
WorkingDirectory=%h

# Environment
Environment=GATEWAY_ENABLED=true
Environment=GATEWAY_HOST=127.0.0.1
Environment=GATEWAY_PORT=3000
Environment=GATEWAY_AUTH_TOKEN=YOUR_TOKEN_HERE
# Generate with: openssl rand -hex 32
Environment=CLI_ENABLED=false
Environment=SANDBOX_ENABLED=true
Environment=RUST_LOG=ironclaw=info

# Logging
StandardOutput=append:%h/.ironclaw/logs/stdout.log
StandardError=append:%h/.ironclaw/logs/stderr.log

[Install]
WantedBy=default.target
```

### 9.2 Enable the Service

```bash
# Create log directory
mkdir -p ~/.ironclaw/logs

# Reload systemd
systemctl --user daemon-reload

# Enable and start
systemctl --user enable ironclaw
systemctl --user start ironclaw

# Check status
systemctl --user status ironclaw
```

---

## 10. CLI Commands

### Memory Commands

Direct workspace operations without starting the agent:

```bash
# Search workspace (hybrid FTS + semantic with PostgreSQL, FTS-only with libSQL)
ironclaw memory search "deployment notes"

# Read a workspace file
ironclaw memory read context/project.md

# Write to workspace
ironclaw memory write notes/meeting.md "Key decisions..."
ironclaw memory write notes/log.md "New entry" --append

# Show workspace tree
ironclaw memory tree
ironclaw memory status
```

### Registry Commands

Browse and install extensions:

```bash
# Search for extensions
ironclaw registry list

# Optional filtering for specific providers
ironclaw registry list | grep github-tools

# Get extension info
ironclaw registry info github-tools

# Install an extension
ironclaw registry install github-tools
```

As of v0.12.0, skills are **enabled by default** — no configuration needed to activate the skills system.

---

## 11. Verify Your Installation

### Health Check

```bash
curl http://127.0.0.1:3000/api/health
# Expected: {"status":"ok"}
```

### Send a Test Message

```bash
TOKEN="your-gateway-token"

curl -X POST http://127.0.0.1:3000/api/chat/send \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"content": "Say: INSTALLATION TEST PASSED"}'
```

### Check Startup Logs

Healthy startup shows:

```
INFO LLM retry wrapper enabled max_retries=3
INFO Safety layer initialized
INFO Registered 4 built-in tools
INFO Embeddings enabled via OpenAI (model: text-embedding-3-small, dim: 1536)
INFO Tool registry initialized with 14 total tools
INFO Web gateway enabled on 127.0.0.1:3000
INFO Agent ironclaw ready and listening
```

---

## 12. Known Issues & Workarounds

### Service exits immediately (REPL EOF crash)

**Symptom:** Service stops right after starting.

**Cause:** `CLI_ENABLED=true` (default) + stdin from `/dev/null` → REPL reads EOF → exits.

**Fix:** Set `CLI_ENABLED=false` in your `.env` or service configuration.

### OpenAI 400 Bad Request on tool calls

**Symptom:** `400 Bad Request: "array schema missing items"`

**Cause:** Tool schema uses `"type": ["string", "object"]` array syntax.

**Fix:** This is fixed in v0.10.0. If you encounter it, update IronClaw.

### Context length exceeded errors

**Symptom:** `ContextLengthExceeded` error mid-conversation.

**Cause:** Conversation context grew beyond the model's token limit.

**Fix:** IronClaw v0.11.0+ automatically detects this and compacts the context (summarizes or truncates old turns) before retrying. No action needed. To tune, use `AGENT_MAX_TOOL_ITERATIONS` to limit per-turn tool calls.

### Anthropic OAuth tokens rejected

**Symptom:** `sk-ant-oat01-...` causes "invalid x-api-key" error.

**Fix:** Use standard API keys (`sk-ant-api03-*`) from [platform.claude.com](https://platform.claude.com).

### libSQL memory search returns no results

**Symptom:** Semantic queries return empty results.

**Cause:** libSQL supports vector search via `vector_top_k()` when embeddings are enabled. If embeddings are disabled or missing, it falls back to FTS5 keyword-only search.

**Fix:** Enable embeddings and ensure memory chunks have embeddings. Use keyword-rich queries as fallback, or switch to PostgreSQL for more advanced hybrid search.

### libSQL embedding dimension mismatch

**Symptom:** Embedding operations fail.

**Cause:** libSQL schema uses `F32_BLOB(1536)` - only 1536 dims supported.

**Fix:** Use `text-embedding-3-small` (1536 dims). `text-embedding-3-large` (3072 dims) requires PostgreSQL.

---

## 13. Troubleshooting

### Service not starting

```bash
# Check service status (macOS)
launchctl list | grep ironclaw

# Check for errors
tail -50 ~/.ironclaw/logs/stderr.log

# Common issues:
# "Shutdown command received" → CLI_ENABLED=false missing
# "failed to connect to database" → DATABASE_URL misconfigured
# "invalid API key" → Check API key
# "Address already in use" → GATEWAY_PORT in use by another process
```

### Port already in use

```bash
lsof -i :3000
# Kill the process or change GATEWAY_PORT
```

### Database errors

```bash
# libSQL: check file permissions
ls -la ~/.ironclaw/

# PostgreSQL: test connection
psql "$DATABASE_URL" -c "SELECT 1"
```

### LLM connection errors

```bash
# Test OpenAI
curl -H "Authorization: Bearer $OPENAI_API_KEY" \
  https://api.openai.com/v1/models | head -c 200

# Test Anthropic
curl -H "x-api-key: $ANTHROPIC_API_KEY" \
  -H "anthropic-version: 2023-06-01" \
  https://api.anthropic.com/v1/models | head -c 200

# Test Ollama
curl http://localhost:11434/api/tags
```

### Enable debug logging

```bash
RUST_LOG=ironclaw=debug,tower_http=debug ironclaw --no-onboard
```

---

## 14. Updating IronClaw

### Pre-built Binary

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/nearai/ironclaw/releases/latest/download/ironclaw-installer.sh | sh
```

### Built from Source

```bash
cd ~/src/ironclaw
git pull
cargo build --release --no-default-features --features libsql
install -m 755 target/release/ironclaw ~/.local/bin/ironclaw

# Restart service (macOS)
launchctl unload ~/Library/LaunchAgents/ai.ironclaw.plist
launchctl load ~/Library/LaunchAgents/ai.ironclaw.plist
```

---

*Source: IronClaw v0.12.0 · See also: [ARCHITECTURE.md](ARCHITECTURE.md), [DEVELOPER-REFERENCE.md](DEVELOPER-REFERENCE.md)*
