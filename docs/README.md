# IronClaw Documentation

> Comprehensive developer reference for [IronClaw](https://github.com/nearai/ironclaw) v0.9.0
> — a secure, self-hosted personal AI assistant written in Rust.

**Documentation set for IronClaw v0.9.0, validated against `~/src/ironclaw` source.**

---

## Contents

| Document | Lines | Description |
|----------|------:|-------------|
| [INSTALLATION.md](INSTALLATION.md) | ~690 | Installation, configuration, service setup, troubleshooting |
| [ARCHITECTURE.md](ARCHITECTURE.md) | ~871 | Master architecture: modules, data flows, diagrams |
| [DEVELOPER-REFERENCE.md](DEVELOPER-REFERENCE.md) | ~1065 | Developer reference: errors, config, code review patterns |
| [analysis/agent.md](analysis/agent.md) | ~890 | Agent loop, sessions, jobs, routines, heartbeat, cost guard |
| [analysis/channels.md](analysis/channels.md) | ~886 | REPL, web gateway, HTTP, WASM, webhook channels + full API routes |
| [analysis/cli.md](analysis/cli.md) | ~492 | CLI subcommands, doctor, service manager, MCP, registry |
| [analysis/config.md](analysis/config.md) | ~926 | Configuration system — exhaustive env var reference |
| [analysis/llm.md](analysis/llm.md) | ~745 | LLM backends, multi-provider, retry, cost guard, schema fix |
| [analysis/safety-sandbox.md](analysis/safety-sandbox.md) | ~520 | Safety layer, WASM sandbox, Docker orchestrator, SSRF proxy |
| [analysis/skills-extensions.md](analysis/skills-extensions.md) | ~703 | Skills system, WASM channels, extensions, hooks |
| [analysis/tools.md](analysis/tools.md) | ~1367 | Tool system, all built-in tools, MCP client, WASM tools, builder |
| [analysis/tunnels-pairing.md](analysis/tunnels-pairing.md) | ~345 | Tunnels (cloudflare/ngrok/tailscale/custom), mobile pairing |
| [analysis/worker-orchestrator.md](analysis/worker-orchestrator.md) | ~484 | Worker runtime, Claude bridge, proxy LLM, Docker sandbox |
| [analysis/workspace-memory.md](analysis/workspace-memory.md) | ~726 | Workspace FS, semantic memory, embeddings, hybrid search |
| [analysis/secrets-keychain.md](analysis/secrets-keychain.md) | ~346 | Secrets store, keychain, AES-GCM crypto, credential injection |

---

## About IronClaw

IronClaw is a Rust-based personal AI assistant built by [NEAR AI](https://near.ai) with:

- **Multi-channel**: REPL, web gateway (axum), HTTP webhooks, WASM plugin channels
- **Security-first**: WASM sandbox (wasmtime), Docker isolation (bollard), credential injection, SSRF proxy
- **Self-expanding**: Dynamic WASM tool builder, MCP protocol client, plugin architecture
- **Persistent memory**: Hybrid FTS+vector search (RRF), workspace filesystem, identity files
- **Multiple LLM backends**: NEAR AI, Anthropic, OpenAI, Ollama, OpenAI-compatible, Tinfoil
- **Dual database**: libSQL (embedded, no server required) or PostgreSQL (with pgvector)

### Source Module Statistics (v0.9.0)

| Module | Files | Description |
|--------|------:|-------------|
| `tools/` | 45+ | Tool system: built-in, MCP, WASM, dynamic builder |
| `channels/` | 35+ | Channels: REPL, web gateway, HTTP, WASM plugins |
| `agent/` | 21 | Agent runtime: loop, sessions, jobs, routines, heartbeat |
| `config/` | 17 | Configuration: all env vars and structs |
| `workspace/` | 7 | Memory, embeddings, hybrid FTS+vector search |
| `tunnel/` | 6 | Tunnels: cloudflare, ngrok, tailscale, custom |
| `secrets/` | 5 | Keychain, AES-256-GCM crypto, credential injection |
| `worker/` | 5 | Docker worker: runtime, LLM bridge, proxy |
| **Total** | **260+** | ~115,000 Rust source lines (approximate; measured on v0.9.0, including app code, tests, comments) |

---

## Quick Start (macOS, local mode)

```bash
# Build (libSQL only, no PostgreSQL required)
git clone https://github.com/nearai/ironclaw ~/src/ironclaw
cd ~/src/ironclaw
cargo build --release --no-default-features --features libsql

# Install
install -m 755 target/release/ironclaw ~/.local/bin/ironclaw

# Configure
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

# Run (one-shot)
ironclaw --no-onboard

# Test
curl http://127.0.0.1:3000/api/health
```

See [INSTALLATION.md](INSTALLATION.md) for complete setup, all LLM backends, service configuration, and troubleshooting.

---

## What's New in v0.9.0

### v0.9.0 (2026-02-22)
- **TEE Attestation Shield**: Added hardware-attested TEEs for enhanced security in web gateway UI
- **Configurable Tool Iterations**: New `AGENT_MAX_TOOL_ITERATIONS` setting for agentic loop control
- **Auto-Approve Tools**: New `AGENT_AUTO_APPROVE_TOOLS` for CI/benchmarking
- **X-Accel-Buffering**: SSE endpoint performance improvements

### v0.8.0 (2026-02-20)
- **Extension Registry**: New metadata catalog with onboarding integration
- **New LLM Models**: GPT-5.3 Codex, GPT-5.x family, Claude 4.x series, o4-mini
- **Memory Hygiene**: Wired memory cleanup into heartbeat loop
- **Parallel Tool Execution**: JoinSet-based concurrent tool calls
- **Approval Improvements**: Multi-tool approval resume flow

---

## Version

Documented: IronClaw v0.9.0
Source: [github.com/nearai/ironclaw](https://github.com/nearai/ironclaw)
Docs repo: [github.com/mudrii/ironclaw-docs](https://github.com/mudrii/ironclaw-docs)
Generated: 2026-02-22
