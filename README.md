<p align="center">
  <img src="ironclaw.png" alt="IronClaw" width="200"/>
</p>

<h1 align="center">IronClaw</h1>

<p align="center">
  <strong>LLM-powered autonomous agent for the NEAR AI marketplace</strong>
</p>

<p align="center">
  <a href="#features">Features</a> •
  <a href="#installation">Installation</a> •
  <a href="#configuration">Configuration</a> •
  <a href="#architecture">Architecture</a> •
  <a href="#security">Security</a>
</p>

---

## Features

- **Multi-channel input** - CLI, HTTP webhooks, Slack, Telegram
- **Parallel job execution** - Concurrent task processing with isolated contexts
- **Extensible tools** - Built-in tools + MCP protocol + WASM sandbox
- **Persistent memory** - Hybrid search (FTS + vector) with chunked documents
- **Prompt injection defense** - Pattern detection, content sanitization, policy enforcement
- **Self-repair** - Automatic detection and recovery of stuck jobs
- **Heartbeat system** - Proactive periodic execution for background tasks

## Installation

### Prerequisites

- Rust 1.85+
- PostgreSQL 15+ with pgvector extension
- NEAR AI session token

### Build

```bash
# Clone the repository
git clone https://github.com/nearai/near-agent.git
cd near-agent

# Build
cargo build --release

# Run tests
cargo test
```

### Database Setup

```bash
# Create database
createdb near_agent

# Enable pgvector
psql near_agent -c "CREATE EXTENSION IF NOT EXISTS vector;"

# Run migrations
refinery migrate -c refinery.toml
```

## Configuration

Copy `.env.example` to `.env` and configure:

```bash
# Required
DATABASE_URL=postgres://user:pass@localhost/near_agent
NEARAI_SESSION_TOKEN=sess_...

# Optional: Enable channels
SLACK_BOT_TOKEN=xoxb-...
TELEGRAM_BOT_TOKEN=...
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

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                         Channels                                 │
│  ┌─────┐  ┌──────┐  ┌───────┐  ┌──────────┐                    │
│  │ CLI │  │ HTTP │  │ Slack │  │ Telegram │                    │
│  └──┬──┘  └──┬───┘  └───┬───┘  └────┬─────┘                    │
│     └────────┴──────────┴───────────┘                           │
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

## Security

### WASM Sandbox

Untrusted tools run in a sandboxed WASM environment with:

- **Capability-based permissions** - Explicit opt-in for HTTP, secrets, tool invocation
- **Endpoint allowlisting** - HTTP requests only to approved hosts/paths
- **Credential injection** - Secrets injected at host boundary, never exposed to WASM
- **Leak detection** - Scans requests and responses for secret exfiltration
- **Rate limiting** - Per-tool request limits (per-minute and per-hour)
- **Resource limits** - Memory, CPU, and execution time constraints

```
WASM ──► Allowlist ──► Leak Scan ──► Credential ──► Execute ──► Leak Scan ──► WASM
         Validator     (request)     Injector       Request     (response)
```

### Prompt Injection Defense

- Pattern-based detection of injection attempts
- Content sanitization and escaping
- Policy rules with severity levels (Block/Warn/Review/Sanitize)
- Tool output wrapping for LLM context

## Usage

### CLI Mode

```bash
# Start interactive CLI
cargo run

# With debug logging
RUST_LOG=near_agent=debug cargo run
```

### HTTP Server

```bash
# Start with HTTP webhook server
HTTP_PORT=8080 cargo run

# Send a request
curl -X POST http://localhost:8080/webhook \
  -H "Content-Type: application/json" \
  -d '{"message": "Hello, agent!"}'
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

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
