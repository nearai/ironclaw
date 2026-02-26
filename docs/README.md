# IronClaw Documentation

> Comprehensive developer reference for [IronClaw](https://github.com/nearai/ironclaw) v0.12.0
> — a secure, self-hosted personal AI assistant written in Rust.

**Documentation set for IronClaw v0.12.0, validated against tag `v0.12.0` (`1156884`) in `~/src/ironclaw`.**

---

## Contents

| Document | Lines | Description |
|----------|------:|-------------|
| [INSTALLATION.md](INSTALLATION.md) | ~715 | Installation, configuration, service setup, troubleshooting |
| [LLM_PROVIDERS.md](LLM_PROVIDERS.md) | ~174 | LLM backend configuration quick guide (NEAR AI, OpenAI, Anthropic, Ollama, OpenAI-compatible) |
| [TELEGRAM_SETUP.md](TELEGRAM_SETUP.md) | ~137 | Telegram channel setup with DM pairing flow and webhook/polling modes |
| [SIGNAL_SETUP.md](SIGNAL_SETUP.md) | ~120 | Signal channel setup via signal-cli HTTP daemon |
| [BUILDING_CHANNELS.md](BUILDING_CHANNELS.md) | ~442 | WASM channel authoring and build/deploy workflow |
| [ARCHITECTURE.md](ARCHITECTURE.md) | ~876 | Master architecture: modules, data flows, diagrams |
| [DEVELOPER-REFERENCE.md](DEVELOPER-REFERENCE.md) | ~1072 | Developer reference: errors, config, code review patterns |
| [analysis/agent.md](analysis/agent.md) | ~930 | Agent loop, sessions, jobs, routines, heartbeat, cost guard |
| [analysis/channels.md](analysis/channels.md) | ~906 | REPL, web gateway, HTTP, WASM, webhook channels + full API routes |
| [analysis/cli.md](analysis/cli.md) | ~504 | CLI subcommands, doctor, service manager, MCP, registry |
| [analysis/config.md](analysis/config.md) | ~928 | Configuration system — exhaustive env var reference |
| [analysis/llm.md](analysis/llm.md) | ~803 | LLM backends, multi-provider, retry, cost guard, schema fix |
| [analysis/safety-sandbox.md](analysis/safety-sandbox.md) | ~520 | Safety layer, WASM sandbox, Docker orchestrator, SSRF proxy |
| [analysis/skills-extensions.md](analysis/skills-extensions.md) | ~729 | Skills system, WASM channels, extensions, hooks |
| [analysis/tools.md](analysis/tools.md) | ~1465 | Tool system, all built-in tools, MCP client, WASM tools, builder |
| [analysis/tunnels-pairing.md](analysis/tunnels-pairing.md) | ~347 | Tunnels (cloudflare/ngrok/tailscale/custom), mobile pairing |
| [analysis/worker-orchestrator.md](analysis/worker-orchestrator.md) | ~485 | Worker runtime, Claude bridge, proxy LLM, Docker sandbox |
| [analysis/workspace-memory.md](analysis/workspace-memory.md) | ~730 | Workspace FS, semantic memory, embeddings, hybrid search |
| [analysis/secrets-keychain.md](analysis/secrets-keychain.md) | ~349 | Secrets store, keychain, AES-GCM crypto, credential injection |

---

## About IronClaw

IronClaw is a Rust-based personal AI assistant built by [NEAR AI](https://near.ai) with:

- **Multi-channel**: REPL, web gateway (axum), HTTP webhooks, WASM plugin channels, native Signal channel
- **Security-first**: WASM sandbox (wasmtime), Docker isolation (bollard), credential injection, SSRF proxy
- **Self-expanding**: Dynamic WASM tool builder, MCP protocol client, plugin architecture
- **Persistent memory**: Hybrid FTS+vector search (RRF), workspace filesystem, identity files
- **Multiple LLM backends**: NEAR AI, Anthropic, OpenAI, Ollama, OpenAI-compatible, Tinfoil
- **Dual database**: libSQL (embedded, no server required) or PostgreSQL (with pgvector)

### Source Module Statistics (v0.12.0)

| Module | Files | Description |
|--------|------:|-------------|
| `tools/` | 41 | Tool system: built-in, MCP, WASM, dynamic builder, rate limiter, HTML-to-Markdown |
| `channels/` | 34 | Channels: REPL, web gateway, HTTP, native Signal, WASM plugins (with pairing + hot-activate) |
| `agent/` | 21 | Agent runtime: loop, sessions, jobs, routines, heartbeat, context compaction |
| `config/` | 17 | Configuration: all env vars and structs |
| `workspace/` | 7 | Memory, embeddings, hybrid FTS+vector search |
| `llm/` | 12 | LLM backends, smart routing provider, reliability wrappers |
| `tunnel/` | 6 | Tunnels: cloudflare, ngrok, tailscale, custom |
| `secrets/` | 5 | Keychain, AES-256-GCM crypto, credential injection |
| `worker/` | 5 | Docker worker: runtime, LLM bridge, proxy |
| **Total (`src/`)** | **250** | ~113,000+ Rust source lines in `src/` (v0.12.0 tag snapshot) |
| **Total (repo-wide)** | **293** | ~129,000+ Rust source lines including tests, channel/tool source trees, and helper binaries |

---

## Quick Start (macOS, local mode)

**Fastest option — Homebrew:**
```bash
brew install ironclaw
```

**Or install pre-built binary:**
```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/nearai/ironclaw/releases/latest/download/ironclaw-installer.sh | sh
```

**Or build from source (libSQL, no PostgreSQL required):**
```bash
git clone https://github.com/nearai/ironclaw ~/src/ironclaw
cd ~/src/ironclaw
cargo build --release --no-default-features --features libsql
install -m 755 target/release/ironclaw ~/.local/bin/ironclaw
```

**Configure and run:**
```bash
mkdir -p ~/.ironclaw
cat > ~/.ironclaw/.env <<'EOF'
DATABASE_BACKEND=libsql
LLM_BACKEND=openai
OPENAI_API_KEY=sk-proj-...
GATEWAY_ENABLED=true
GATEWAY_PORT=3000
GATEWAY_AUTH_TOKEN=REPLACE_WITH_SECURE_TOKEN
CLI_ENABLED=false
RUST_LOG=ironclaw=info
EOF

# Generate a secure token: openssl rand -hex 32
# Replace REPLACE_WITH_SECURE_TOKEN above with the output

# Run
ironclaw --no-onboard

# Test
curl http://127.0.0.1:3000/api/health
```

See [INSTALLATION.md](INSTALLATION.md) for complete setup and deployment, [LLM_PROVIDERS.md](LLM_PROVIDERS.md) for backend-specific examples, [TELEGRAM_SETUP.md](TELEGRAM_SETUP.md) or [SIGNAL_SETUP.md](SIGNAL_SETUP.md) for messaging channel setup, and [BUILDING_CHANNELS.md](BUILDING_CHANNELS.md) for custom WASM channels.

---

## What's New

### v0.12.0 (2026-02-26)
- **Signal Channel**: Native Signal messaging via signal-cli HTTP daemon — first-class channel alongside Telegram with tool approval workflow, DM pairing, group support, and allowlist controls
- **OpenRouter Preset**: Setup wizard now includes OpenRouter as a dedicated provider option (200+ models via single API key)
- **Web UI — Tool Activity Cards**: Inline tool execution cards with animated spinner, elapsed timer, and auto-collapsing summary after response
- **Web UI — WASM Channel Setup Flow**: Improved setup stepper (Installed → Configured → Active) with state-aware action buttons
- **Web UI — Newest-First Logs**: Log viewer now displays most recent entries at the top
- **`--version` Flag**: `ironclaw --version` now officially supported, outputs `ironclaw 0.12.0`
- **Skills Enabled by Default**: Skills system now active by default with fixed registry and install pipeline
- **MCP Registry URL Fixes**: Corrected 6 MCP endpoint URLs, removed non-existent Google Drive and Google Calendar entries
- **Docker Build Fix**: Dockerfile now correctly copies `migrations/`, `registry/`, `channels-src/`, `wit/` directories
- **Extension Name Collision Fix**: Telegram and Slack tool registry names renamed to avoid conflicts with channel entries

### v0.11.1 (2026-02-23)
- **CI/CD Fix**: Resolved release pipeline issue allowing custom `release.yml` jobs

### v0.11.0 (2026-02-23)
- **Context Auto-Compaction**: Automatic compaction with retry on `ContextLengthExceeded` — three strategies: Summarize (LLM-generated summary written to `daily/{date}.md`), Truncate (drop oldest turns), MoveToWorkspace (archive full turns)
- **Completion improvements**: Better handling of completion edge cases

### v0.10.0 (2026-02-22)
- **Smart Routing Provider**: Cost-optimized model selection — routes Simple queries to cheap models (e.g., Haiku), Complex queries to primary models (Sonnet/Opus), with cascade escalation on uncertain responses (`SMART_ROUTING_CASCADE`, `NEARAI_CHEAP_MODEL`)
- **Rate Limiting for Built-in Tools**: Per-tool, per-user sliding window rate limiting (per-minute and per-hour limits)
- **WASM Channel Enhancements**: Hot-activate WASM channels, channel-first prompts, unified artifact resolution
- **Pairing/Permission System**: All WASM channels now support device pairing and permissions
- **Group Chat Privacy**: Privacy controls and channel-aware prompts with safety hardening
- **Embedded Registry Catalog**: Offline-capable extension discovery with WASM bundle install pipeline
- **Token Usage & Cost Tracking**: Gateway status popover shows real-time token usage and cost
- **Custom HTTP Headers**: Support for `LLM_EXTRA_HEADERS` on OpenAI-compatible providers
- **HTML-to-Markdown Conversion**: New built-in tool for converting HTML content
- **FullJob Routine Mode**: Scheduler dispatch for routine jobs
- **Startup Optimization**: Startup time reduced from ~15s to ~2s
- **Homebrew Install**: `brew install ironclaw` now available
- **Web UI Refresh**: Agent-market design language, dashboard favicon, Chrome extension test skill

### v0.9.0 (2026-02-21)
- **TEE Attestation Shield**: Hardware-attested TEEs for enhanced security in web gateway UI
- **Configurable Tool Iterations**: New `AGENT_MAX_TOOL_ITERATIONS` setting for agentic loop control
- **Auto-Approve Tools**: New `AGENT_AUTO_APPROVE_TOOLS` for CI/benchmarking
- **X-Accel-Buffering**: SSE endpoint performance improvements

---

## Version

Documented: IronClaw v0.12.0
Release tag: [`v0.12.0`](https://github.com/nearai/ironclaw/releases/tag/v0.12.0) (`1156884`, 2026-02-26)
Source: [github.com/nearai/ironclaw](https://github.com/nearai/ironclaw)
Docs repo: [github.com/mudrii/ironclaw-docs](https://github.com/mudrii/ironclaw-docs)
Generated: 2026-02-26
