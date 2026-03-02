# IronClaw Documentation

> Comprehensive developer reference for [IronClaw](https://github.com/nearai/ironclaw) v0.13.0
> — a secure, self-hosted personal AI assistant written in Rust.

**Documentation set for IronClaw v0.13.0, validated against release tag `v0.13.0` (`291913338`).**

---

## Contents

| Document | Lines | Description |
|----------|------:|-------------|
| [INSTALLATION.md](INSTALLATION.md) | ~715 | Installation, configuration, service setup, troubleshooting |
| [LLM_PROVIDERS.md](LLM_PROVIDERS.md) | ~178 | LLM backend configuration quick guide (NEAR AI, OpenAI, Anthropic, Ollama, OpenAI-compatible) |
| [TELEGRAM_SETUP.md](TELEGRAM_SETUP.md) | ~137 | Telegram channel setup with DM pairing flow and webhook/polling modes |
| [SIGNAL_SETUP.md](SIGNAL_SETUP.md) | ~200 | Signal channel setup via signal-cli HTTP daemon |
| [BUILDING_CHANNELS.md](BUILDING_CHANNELS.md) | ~442 | WASM channel authoring and build/deploy workflow |
| [ARCHITECTURE.md](ARCHITECTURE.md) | ~877 | Master architecture: modules, data flows, diagrams |
| [AGENT_README.md](AGENT_README.md) | ~1245 | Agent reference: errors, config, code review patterns |
| [analysis/agent.md](analysis/agent.md) | ~930 | Agent loop, sessions, jobs, routines, heartbeat, cost guard |
| [analysis/channels.md](analysis/channels.md) | ~1017 | REPL, web gateway, HTTP, WASM, webhook channels + full API routes |
| [analysis/cli.md](analysis/cli.md) | ~505 | CLI subcommands, doctor, service manager, MCP, registry |
| [analysis/config.md](analysis/config.md) | ~1034 | Configuration system — exhaustive env var reference |
| [analysis/llm.md](analysis/llm.md) | ~803 | LLM backends, multi-provider, retry, cost guard, schema fix |
| [analysis/safety-sandbox.md](analysis/safety-sandbox.md) | ~520 | Safety layer, WASM sandbox, Docker orchestrator, SSRF proxy |
| [analysis/skills-extensions.md](analysis/skills-extensions.md) | ~736 | Skills system, WASM channels, extensions, hooks |
| [analysis/tools.md](analysis/tools.md) | ~1515 | Tool system, all built-in tools, MCP client, WASM tools, builder |
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

### Source Module Statistics (v0.13.0)

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
| **Total (`src/`)** | **250** | ~113,000+ Rust source lines in `src/` (v0.13.0 tag snapshot) |
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

### v0.13.0 (2026-03-02)

#### Added

- add tool setup command + GitHub setup schema ([#438](https://github.com/nearai/ironclaw/pull/438))
- add web_fetch built-in tool ([#435](https://github.com/nearai/ironclaw/pull/435))
- DB-backed Jobs tab + scheduler-dispatched local jobs ([#436](https://github.com/nearai/ironclaw/pull/436))
- OAuth setup UI for WASM tools + display name labels ([#437](https://github.com/nearai/ironclaw/pull/437))
- auto-detect libsql when `ironclaw.db` exists ([#399](https://github.com/nearai/ironclaw/pull/399))
- slash command autocomplete + `/status` and `/list` ([#404](https://github.com/nearai/ironclaw/pull/404))
- deliver notifications to all installed channels ([#398](https://github.com/nearai/ironclaw/pull/398))
- persist tool calls, restore approvals on thread switch, and Web UI fixes ([#382](https://github.com/nearai/ironclaw/pull/382))
- add `IRONCLAW_BASE_DIR` env var with LazyLock caching ([#397](https://github.com/nearai/ironclaw/pull/397))
- feat(signal) attachment upload and message tool ([#375](https://github.com/nearai/ironclaw/pull/375))

#### Fixed

- host-based credential injection to the WASM channel wrapper ([#421](https://github.com/nearai/ironclaw/pull/421))
- pre-validate Cloudflare tunnel token by spawning `cloudflared` ([#446](https://github.com/nearai/ironclaw/pull/446))
- quick fixes: `#330`, `#338`, `#344`, `#358`, `#417`, `#419` (bundled as [#428](https://github.com/nearai/ironclaw/pull/428))
- persist channel activation state across restarts ([#432](https://github.com/nearai/ironclaw/pull/432))
- init WASM runtime eagerly regardless of tools directory existence ([#401](https://github.com/nearai/ironclaw/pull/401))
- add TLS support for PostgreSQL connections ([#363](https://github.com/nearai/ironclaw/pull/363), [#427](https://github.com/nearai/ironclaw/pull/427))
- scan inbound messages for leaked secrets ([#433](https://github.com/nearai/ironclaw/pull/433))
- use `tailscale funnel --bg` for proper tunnel setup ([#430](https://github.com/nearai/ironclaw/pull/430))
- normalize secret names to lowercase for case-insensitive matching ([#413](https://github.com/nearai/ironclaw/pull/413), [#431](https://github.com/nearai/ironclaw/pull/431))
- persist model name to `.env` so dotted names survive restart ([#426](https://github.com/nearai/ironclaw/pull/426))
- setup flow validates cloudflared binary and token ([#424](https://github.com/nearai/ironclaw/pull/424), [#423](https://github.com/nearai/ironclaw/pull/423))
- guard `zsh compdef` call to prevent pre-compinit errors ([#422](https://github.com/nearai/ironclaw/pull/422))
- Telegram: remove restart button and validate token on setup ([#434](https://github.com/nearai/ironclaw/pull/434))
- web UI routines tab shows all routines regardless of creating channel ([#391](https://github.com/nearai/ironclaw/pull/391))
- Discord Ed25519 signature verification and capabilities header alias fixes ([#148](https://github.com/nearai/ironclaw/pull/148), [#372](https://github.com/nearai/ironclaw/pull/372))
- prevent duplicate WASM channel activation on startup ([#390](https://github.com/nearai/ironclaw/pull/390))

#### Other

- rename `WasmBuildable::repo_url` to `source_dir` ([#445](https://github.com/nearai/ironclaw/pull/445))
- improve `--help` with `about`, `examples`, and `color` output ([#371](https://github.com/nearai/ironclaw/pull/371))
- add automated QA: schema validator, CI matrix, Docker build, and `P1` test coverage ([#353](https://github.com/nearai/ironclaw/pull/353))

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

Documented: IronClaw v0.13.0
Release tag: [`v0.13.0`](https://github.com/nearai/ironclaw/releases/tag/v0.13.0) (`291913338`, 2026-03-02)
Source: [github.com/nearai/ironclaw](https://github.com/nearai/ironclaw)
Docs repo: [github.com/mudrii/ironclaw-docs](https://github.com/mudrii/ironclaw-docs)
Generated: 2026-03-02
