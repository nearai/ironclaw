<p align="center">
  <img src="ironclaw.png" alt="IronClaw" width="200"/>
</p>

<h1 align="center">IronClaw</h1>

<p align="center">
  <strong>Your secure personal AI assistant, always on your side</strong>
</p>

<p align="center">
  <a href="#philosophy">Philosophy</a> •
  <a href="#features">Features</a> •
  <a href="#installation">Installation</a> •
  <a href="#configuration">Configuration</a> •
  <a href="#security">Security</a> •
  <a href="#architecture">Architecture</a>
</p>

---

## Philosophy

IronClaw is built on a simple principle: **your AI assistant should work for you, not against you**.

In a world where AI systems are increasingly opaque about data handling and aligned with corporate interests, IronClaw takes a different approach:

- **Your data stays yours** - All information is stored locally, encrypted, and never leaves your control
- **Transparency by design** - Open source, auditable, no hidden telemetry or data harvesting
- **Self-expanding capabilities** - Build new tools on the fly without waiting for vendor updates
- **Defense in depth** - Multiple security layers protect against prompt injection and data exfiltration

IronClaw is the AI assistant you can actually trust with your personal and professional life.

## Features

### Security First

- **WASM Sandbox** - Untrusted tools run in isolated WebAssembly containers with capability-based permissions
- **Credential Protection** - Secrets are never exposed to tools; injected at the host boundary with leak detection
- **Prompt Injection Defense** - Pattern detection, content sanitization, and policy enforcement
- **Endpoint Allowlisting** - HTTP requests only to explicitly approved hosts and paths

### Always Available

- **Multi-channel** - Reach your assistant via CLI, Telegram, WhatsApp, Slack, or HTTP webhooks
- **Heartbeat System** - Proactive background execution for monitoring and maintenance tasks
- **Parallel Jobs** - Handle multiple requests concurrently with isolated contexts
- **Self-repair** - Automatic detection and recovery of stuck operations

### Self-Expanding

- **Dynamic Tool Building** - Describe what you need, and IronClaw builds it as a WASM tool
- **MCP Protocol** - Connect to Model Context Protocol servers for additional capabilities
- **Plugin Architecture** - Drop in new WASM tools and channels without restarting

### Persistent Memory

- **Hybrid Search** - Full-text + vector search using Reciprocal Rank Fusion
- **Workspace Filesystem** - Flexible path-based storage for notes, logs, and context
- **Identity Files** - Maintain consistent personality and preferences across sessions

## Installation

### Prerequisites

- Rust 1.85+
- PostgreSQL 15+ with pgvector extension
- NEAR AI session token (or other LLM provider)

### Build

```bash
# Clone the repository
git clone https://github.com/nearai/ironclaw.git
cd ironclaw

# Build
cargo build --release

# Run tests
cargo test
```

### Database Setup

```bash
# Create database
createdb ironclaw

# Enable pgvector
psql ironclaw -c "CREATE EXTENSION IF NOT EXISTS vector;"

# Run migrations
refinery migrate -c refinery.toml
```

## Configuration

Copy `.env.example` to `.env` and configure:

```bash
# Required
DATABASE_URL=postgres://user:pass@localhost/ironclaw
NEARAI_SESSION_TOKEN=sess_...

# Optional: Enable channels
TELEGRAM_BOT_TOKEN=...
WHATSAPP_ACCESS_TOKEN=...
SLACK_BOT_TOKEN=xoxb-...
HTTP_PORT=8080
```

### Environment Variables

| Variable | Description | Required |
|----------|-------------|----------|
| `DATABASE_URL` | PostgreSQL connection string | Yes |
| `NEARAI_SESSION_TOKEN` | NEAR AI authentication token | Yes |
| `NEARAI_MODEL` | Model to use (default: claude-3-5-sonnet) | No |
| `AGENT_MAX_PARALLEL_JOBS` | Max concurrent jobs (default: 5) | No |
| `SECRETS_MASTER_KEY` | 32+ byte key for secret encryption | For secrets |

## Security

IronClaw implements defense in depth to protect your data and prevent misuse.

### WASM Sandbox

All untrusted tools run in isolated WebAssembly containers:

- **Capability-based permissions** - Explicit opt-in for HTTP, secrets, tool invocation
- **Endpoint allowlisting** - HTTP requests only to approved hosts/paths
- **Credential injection** - Secrets injected at host boundary, never exposed to WASM code
- **Leak detection** - Scans requests and responses for secret exfiltration attempts
- **Rate limiting** - Per-tool request limits to prevent abuse
- **Resource limits** - Memory, CPU, and execution time constraints

```
WASM ──► Allowlist ──► Leak Scan ──► Credential ──► Execute ──► Leak Scan ──► WASM
         Validator     (request)     Injector       Request     (response)
```

### Prompt Injection Defense

External content passes through multiple security layers:

- Pattern-based detection of injection attempts
- Content sanitization and escaping
- Policy rules with severity levels (Block/Warn/Review/Sanitize)
- Tool output wrapping for safe LLM context injection

### Data Protection

- All data stored locally in your PostgreSQL database
- Secrets encrypted with AES-256-GCM
- No telemetry, analytics, or data sharing
- Full audit log of all tool executions

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                         Channels                                 │
│  ┌─────┐  ┌──────────┐  ┌──────────┐  ┌───────┐                │
│  │ CLI │  │ Telegram │  │ WhatsApp │  │ Slack │                │
│  └──┬──┘  └────┬─────┘  └────┬─────┘  └───┬───┘                │
│     └──────────┴─────────────┴────────────┘                     │
│                         │                                        │
│                    ┌────▼────┐                                  │
│                    │  Router │  Intent classification           │
│                    └────┬────┘                                  │
│                         │                                        │
│              ┌──────────▼──────────┐                            │
│              │     Scheduler       │  Parallel job management   │
│              └──────────┬──────────┘                            │
│                         │                                        │
│         ┌───────────────┼───────────────┐                       │
│         ▼               ▼               ▼                       │
│    ┌─────────┐    ┌─────────┐    ┌─────────┐                   │
│    │ Worker  │    │ Worker  │    │ Worker  │  LLM reasoning    │
│    └────┬────┘    └────┬────┘    └────┬────┘                   │
│         └───────────────┼───────────────┘                       │
│                         │                                        │
│              ┌──────────▼──────────┐                            │
│              │   Tool Registry     │                            │
│              │  ┌───────────────┐  │                            │
│              │  │ Built-in      │  │                            │
│              │  │ MCP           │  │                            │
│              │  │ WASM Sandbox  │  │                            │
│              │  └───────────────┘  │                            │
│              └─────────────────────┘                            │
└─────────────────────────────────────────────────────────────────┘
```

### Core Components

| Component | Purpose |
|-----------|---------|
| **Agent Loop** | Main message handling and job coordination |
| **Router** | Classifies user intent (command, query, task) |
| **Scheduler** | Manages parallel job execution with priorities |
| **Worker** | Executes jobs with LLM reasoning and tool calls |
| **Workspace** | Persistent memory with hybrid search |
| **Safety Layer** | Prompt injection defense and content sanitization |

## Usage

### CLI Mode

```bash
# Start interactive CLI
cargo run

# With debug logging
RUST_LOG=ironclaw=debug cargo run
```

### HTTP Server

```bash
# Start with HTTP webhook server
HTTP_PORT=8080 cargo run

# Send a request
curl -X POST http://localhost:8080/webhook \
  -H "Content-Type: application/json" \
  -d '{"message": "Hello, IronClaw!"}'
```

## Development

```bash
# Format code
cargo fmt

# Lint
cargo clippy --all --benches --tests --examples --all-features

# Run tests
cargo test

# Run specific test
cargo test test_name
```

## OpenClaw Heritage

IronClaw is a Rust reimplementation inspired by [OpenClaw](https://github.com/openclaw/openclaw). See [FEATURE_PARITY.md](FEATURE_PARITY.md) for the complete tracking matrix.

Key differences:

- **Rust vs TypeScript** - Native performance, memory safety, single binary
- **WASM sandbox vs Docker** - Lightweight, capability-based security
- **PostgreSQL vs SQLite** - Production-ready persistence
- **Security-first design** - Multiple defense layers, credential protection

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
