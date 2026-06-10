<p align="center">
  <img src="ironclaw.png?v=2" alt="IronClaw" width="200"/>
</p>

<h1 align="center">IronClaw</h1>

<p align="center">
  <strong>Your secure personal AI assistant, always on your side</strong>
</p>

<p align="center">
  <a href="#license"><img src="https://img.shields.io/badge/license-MIT%20OR%20Apache%202.0-blue.svg" alt="License: MIT OR Apache-2.0" /></a>
  <a href="https://t.me/ironclawAI"><img src="https://img.shields.io/badge/Telegram-%40ironclawAI-26A5E4?style=flat&logo=telegram&logoColor=white" alt="Telegram: @ironclawAI" /></a>
  <a href="https://www.reddit.com/r/ironclawAI/"><img src="https://img.shields.io/badge/Reddit-r%2FironclawAI-FF4500?style=flat&logo=reddit&logoColor=white" alt="Reddit: r/ironclawAI" /></a>
  <a href="https://gitcgr.com/nearai/ironclaw">
    <img src="https://gitcgr.com/badge/nearai/ironclaw.svg" alt="gitcgr" />
  </a>
</p>

<p align="center">
  <a href="README.md">English</a> |
  <a href="README.zh-CN.md">简体中文</a> |
  <a href="README.ru.md">Русский</a> |
  <a href="README.ja.md">日本語</a> |
  <a href="README.ko.md">한국어</a>
</p>

<p align="center">
  <a href="#ironclaw-reborn-quick-start">Reborn Quick Start</a> •
  <a href="#philosophy">Philosophy</a> •
  <a href="#features">Features</a> •
  <a href="#installation">Installation</a> •
  <a href="#configuration">Configuration</a> •
  <a href="#security">Security</a> •
  <a href="#architecture">Architecture</a>
</p>

---

## IronClaw Reborn Quick Start

IronClaw Reborn is the standalone runtime on the `reborn-integration` branch.
It uses the separate `ironclaw-reborn` binary from the
`ironclaw_reborn_cli` package and a separate Reborn state root. It does not use
the legacy `ironclaw` state directory as its config root.

For the older `ironclaw` binary, see [Installation](#installation) and
[Legacy IronClaw Usage](#legacy-ironclaw-usage).

### Build or run the binary

From the repo root:

```bash
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- --help
```

Or build it first:

```bash
cargo build -p ironclaw_reborn_cli --bin ironclaw-reborn
./target/debug/ironclaw-reborn --help
```

The default Reborn home is `$HOME/.ironclaw/reborn`. Override it with an
absolute path when you want isolated state:

```bash
export IRONCLAW_REBORN_HOME="$PWD/.reborn-home"
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- config path
```

`config path` and `doctor` are safe diagnostics; they report the resolved home,
profile, `config.toml`, `providers.json`, and `v1_state: not-used`.
They do not create Reborn state or seed config files.

### Configure the model route

The CLI-native way to configure Reborn's default model route is:

```bash
export IRONCLAW_REBORN_HOME="$PWD/.reborn-home"
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- models set-provider openai --model gpt-5-mini
```

That writes `$IRONCLAW_REBORN_HOME/config.toml` with `[llm.default]` and the
provider's credential env-var name. Check it with:

```bash
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- models status
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- models list openai
```

For OpenAI, set the secret value in the environment before starting:

```bash
export OPENAI_API_KEY="sk-..."
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- run --message "hello"
```

Omit `--message` or use `repl` for an interactive stdin session:

```bash
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- repl
```

### `config.toml` shape

`config init` creates editable starter files:

```bash
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- config init
```

It writes:

- `$IRONCLAW_REBORN_HOME/config.toml`
- `$IRONCLAW_REBORN_HOME/providers.json`

A minimal configured model route looks like:

```toml
[llm.default]
provider_id = "openai"
model = "gpt-5-mini"
api_key_env = "OPENAI_API_KEY"
```

`config.toml` may also include optional sections such as `[boot]`,
`[identity]`, `[runner]`, and `[skills]`; `config init` writes commented
guidance for the supported fields.

If `config.toml` is missing, the first stateful runtime start through `run`,
`repl`, or `serve` seeds a sparse file with `api_version` and the safe
`local-dev` boot profile. Read-only commands and `run --dry-run` stay
side-effect-free. One-off environment selections such as
`IRONCLAW_REBORN_PROFILE=local-dev-yolo` are not persisted into the seeded
file.

Important: `api_key_env` is the name of an environment variable, not the secret
itself. Reborn rejects inline secret-shaped values in `config.toml` and
`providers.json`.

Production storage uses the same env-only pattern. A production Reborn config
may name the PostgreSQL URL variable, but must not contain the raw URL:

```toml
[storage]
backend = "postgres"
url_env = "IRONCLAW_REBORN_POSTGRES_URL"
secret_master_key_env = "IRONCLAW_REBORN_SECRET_MASTER_KEY"
# Optional; defaults to 16. Keep below the PostgreSQL server's max_connections
# after reserving capacity for migrations and operator sessions.
pool_max_size = 16

[policy]
deployment_mode = "hosted_multi_tenant"
default_profile = "secure_default"
```

Set `IRONCLAW_REBORN_POSTGRES_URL` in the process environment, and set
`IRONCLAW_REBORN_SECRET_MASTER_KEY` to independent cryptographic key material.
Managed remote PostgreSQL providers must use TLS, for example by appending
`sslmode=require`.
Production `run` also requires an explicit `[policy]` section. The first
production launch slice supports runtime policies that do not require a
tenant-sandbox process binding.

Once `[llm.default]` exists, that config selects the provider. `LLM_BACKEND` is
only an env fallback when no default LLM slot is configured. To switch providers
after writing config, use `models set-provider <provider>` or edit
`[llm.default].provider_id`.

### Env-only model selection

If `$IRONCLAW_REBORN_HOME/config.toml` is absent or has no `[llm.default]`,
Reborn can resolve the LLM from environment variables. A sparse first-run
seeded config does not include `[llm.default]`, so env-only model selection
continues to work:

```bash
export IRONCLAW_REBORN_HOME="$PWD/.reborn-env-only"
export LLM_BACKEND=openai
export OPENAI_API_KEY="sk-..."
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- run --message "hello"
```

Common provider env vars:

| Provider | Selector | Required env |
| --- | --- | --- |
| OpenAI | `LLM_BACKEND=openai` | `OPENAI_API_KEY`; optional `OPENAI_MODEL`, `OPENAI_BASE_URL` |
| Anthropic | `LLM_BACKEND=anthropic` | `ANTHROPIC_API_KEY`; optional `ANTHROPIC_MODEL`, `ANTHROPIC_BASE_URL` |
| OpenAI-compatible | `LLM_BACKEND=openai_compatible` | `LLM_BASE_URL`; optional `LLM_API_KEY`, `LLM_MODEL` |
| OpenRouter | `LLM_BACKEND=openrouter` | `OPENROUTER_API_KEY`; optional `OPENROUTER_MODEL` |
| Ollama | `LLM_BACKEND=ollama` | no key; optional `OLLAMA_BASE_URL`, `OLLAMA_MODEL` |
| Codex auth | `LLM_BACKEND=openai_codex` | `LLM_USE_CODEX_AUTH=true` or `CODEX_AUTH_PATH`; optional `OPENAI_CODEX_MODEL` |

Use `models list <provider>` to see the exact provider metadata compiled into
the current branch.

### Startup variables

| Variable | Purpose |
| --- | --- |
| `IRONCLAW_REBORN_HOME` | Absolute Reborn state root. Defaults to `$HOME/.ironclaw/reborn`. The resolver rejects unsafe paths and v1 state-root aliases such as `$HOME/.ironclaw`. |
| `IRONCLAW_REBORN_PROFILE` | Boot profile selector. Supported values: `local-dev`, `local-dev-yolo`, `production`, `migration-dry-run`. |
| `IRONCLAW_REBORN_POSTGRES_URL` | Production PostgreSQL storage URL when `[storage].backend = "postgres"` and `[storage].url_env` names this variable. Keep it out of `config.toml`; remote providers must use TLS. |
| `IRONCLAW_REBORN_SECRET_MASTER_KEY` | Production Reborn secret master key when `[storage].secret_master_key_env` names this variable. Keep it independent from the database URL and out of `config.toml`. |
| `IRONCLAW_REBORN_LOG` | Tracing filter for the Reborn binary, for example `debug,ironclaw_reborn=trace`. |

`run` and `repl` currently support `local-dev` and `local-dev-yolo` runtime
composition. `local-dev-yolo` grants trusted-laptop host access and must be
confirmed explicitly:

```bash
export IRONCLAW_REBORN_PROFILE=local-dev-yolo
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- repl --confirm-host-access
```

### WebUI service

The Reborn WebUI is compiled behind the `webui-v2-beta` Cargo feature. Build or
run the binary with that feature to enable the `serve` command:

```bash
cargo run -q -p ironclaw_reborn_cli --features webui-v2-beta --bin ironclaw-reborn -- serve --help
cargo build -p ironclaw_reborn_cli --features webui-v2-beta --bin ironclaw-reborn
```

The WebUI listener defaults to `127.0.0.1:3000`. The service requires an
env-bearer token and a user id at startup. It also needs the model route from
the earlier section, including that provider's credential env var:

```bash
export IRONCLAW_REBORN_HOME="$PWD/.reborn-home"
export OPENAI_API_KEY="sk-..." # or the required env var for your configured provider
export IRONCLAW_REBORN_WEBUI_TOKEN="$(openssl rand -hex 32)"
export IRONCLAW_REBORN_WEBUI_USER_ID="reborn-cli"

cargo run -q -p ironclaw_reborn_cli --features webui-v2-beta --bin ironclaw-reborn -- serve
```

Equivalent `config.toml` listener configuration:

```toml
[webui]
listen_host = "127.0.0.1"
listen_port = 3000
env_token_var = "IRONCLAW_REBORN_WEBUI_TOKEN"
env_user_id_var = "IRONCLAW_REBORN_WEBUI_USER_ID"
allowed_origins = ["http://127.0.0.1:3000", "http://localhost:3000"]
canonical_host = "127.0.0.1:3000"
```

`env_token_var` and `env_user_id_var` are env-var names. Keep the actual token
and user id in the environment.

Required WebUI env vars:

| Variable | Purpose |
| --- | --- |
| `IRONCLAW_REBORN_WEBUI_TOKEN` | Bearer token for WebUI requests. If SSO is enabled, this also signs sessions and must be at least 32 bytes. |
| `IRONCLAW_REBORN_WEBUI_USER_ID` | Reborn owner/user id for env-bearer requests. If `[identity].default_owner` is configured, it must match this value. |

Optional WebUI SSO env vars:

| Variable | Purpose |
| --- | --- |
| `IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_ID` | Enables Google SSO when set. |
| `IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_SECRET` | Required when Google SSO is enabled. |
| `IRONCLAW_REBORN_WEBUI_GOOGLE_ALLOWED_HD` | Optional Google hosted-domain restriction. |
| `IRONCLAW_REBORN_WEBUI_GITHUB_CLIENT_ID` | Enables GitHub SSO when set. |
| `IRONCLAW_REBORN_WEBUI_GITHUB_CLIENT_SECRET` | Required when GitHub SSO is enabled. |
| `IRONCLAW_REBORN_WEBUI_ALLOWED_EMAIL_DOMAINS` | Required when any SSO provider is enabled. Comma-separated verified email domains. |
| `IRONCLAW_REBORN_WEBUI_BASE_URL` | Public base URL used for OAuth callbacks. Non-loopback deployments must use `https://`. |
| `IRONCLAW_REBORN_WEBUI_OAUTH_HTTP_TIMEOUT_SECS` | Optional OAuth HTTP timeout override. |

For Google SSO, create a Google OAuth web client and register the Reborn WebUI
redirect URI as:

```text
{IRONCLAW_REBORN_WEBUI_BASE_URL}/auth/callback/google
```

For example, with `IRONCLAW_REBORN_WEBUI_BASE_URL=https://ironclaw.example.com`,
the authorized redirect URI in Google Cloud is:

```text
https://ironclaw.example.com/auth/callback/google
```

Do not include a trailing slash in `IRONCLAW_REBORN_WEBUI_BASE_URL`; Reborn
trims it before building callback URLs. If the base URL is omitted, Reborn uses
the actual listener address, such as `http://127.0.0.1:3000`, which is suitable
only for loopback/local OAuth testing. Public or non-loopback SSO deployments
must set an `https://` base URL.

Complete Google SSO startup env:

```bash
export IRONCLAW_REBORN_HOME="/var/lib/ironclaw-reborn"
export IRONCLAW_REBORN_PROFILE=local-dev
export OPENAI_API_KEY="sk-..." # or the required env var for your configured provider
export IRONCLAW_REBORN_WEBUI_TOKEN="$(openssl rand -hex 32)"
export IRONCLAW_REBORN_WEBUI_USER_ID="reborn-cli"
export IRONCLAW_REBORN_WEBUI_BASE_URL="https://ironclaw.example.com"
export IRONCLAW_REBORN_WEBUI_ALLOWED_EMAIL_DOMAINS="example.com,team.example.com"
export IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_ID="..."
export IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_SECRET="..."

cargo run -q -p ironclaw_reborn_cli --features webui-v2-beta --bin ironclaw-reborn -- serve --host 0.0.0.0 --port 3000
```

`IRONCLAW_REBORN_WEBUI_ALLOWED_EMAIL_DOMAINS` is the actual admission
allowlist. Google `hd` is only an optional provider-side hosted-domain hint; do
not rely on it instead of the Reborn allowed-domain list. `IRONCLAW_REBORN_HOME`
selects the state/config root for this service. `IRONCLAW_REBORN_PROFILE`
defaults to `local-dev`; `local-dev-yolo` grants trusted-laptop host access and
cannot be served on a non-loopback host.

Use `serve --host <ip> --port <port>` to override the listener from the CLI.
Binding to a non-loopback host is production-sensitive. `local-dev-yolo` serve
mode also requires `--confirm-host-access` and refuses non-loopback hosts.

### Slack service

Slack support is compiled behind the `slack-v2-host-beta` Cargo feature. That
feature includes `webui-v2-beta`, so Slack runs on the same `serve` command:

```bash
export IRONCLAW_REBORN_HOME="$PWD/.reborn-home"
export OPENAI_API_KEY="sk-..." # or the required env var for your configured provider
export IRONCLAW_REBORN_WEBUI_TOKEN="$(openssl rand -hex 32)"
export IRONCLAW_REBORN_WEBUI_USER_ID="reborn-cli"
export IRONCLAW_REBORN_SLACK_SIGNING_SECRET="..."
export IRONCLAW_REBORN_SLACK_BOT_TOKEN="xoxb-..."

cargo run -q -p ironclaw_reborn_cli --features slack-v2-host-beta --bin ironclaw-reborn -- serve
```

Slack env vars alone do not enable Slack. Add a `[slack]` section to
`config.toml`:

```toml
[slack]
enabled = true
installation_id = "install-alpha"
team_id = "T123"
api_app_id = "A123"
# slack_user_id = "U123" # optional legacy static user mapping
# user_id = "reborn-cli" # defaults to the WebUI authenticated user
signing_secret_env = "IRONCLAW_REBORN_SLACK_SIGNING_SECRET"
bot_token_env = "IRONCLAW_REBORN_SLACK_BOT_TOKEN"
```

Required Slack settings and env vars:

| Name | Purpose |
| --- | --- |
| `[slack].enabled = true` | Mounts the Slack route during `serve`. |
| `[slack].installation_id` | Stable local installation id. |
| `[slack].team_id` | Slack workspace/team id. |
| `[slack].api_app_id` | Slack app id. |
| `IRONCLAW_REBORN_SLACK_SIGNING_SECRET` | Slack request signing secret, or the env var named by `[slack].signing_secret_env`. |
| `IRONCLAW_REBORN_SLACK_BOT_TOKEN` | Slack bot token, or the env var named by `[slack].bot_token_env`. |

More detailed command notes live in [`docs/reborn-binary.md`](docs/reborn-binary.md).

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

- **Multi-channel** - REPL, HTTP webhooks, WASM channels (Telegram, Slack), and web gateway
- **Docker Sandbox** - Isolated container execution with per-job tokens and orchestrator/worker pattern
- **Web Gateway** - Browser UI with real-time SSE/WebSocket streaming
- **Routines** - Cron schedules, event triggers, webhook handlers for background automation
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

- Rust 1.92+
- PostgreSQL 15+ with [pgvector](https://github.com/pgvector/pgvector) extension
- NEAR AI account (authentication handled via setup wizard)
- `libclang` and a working C toolchain if you build the WeChat voice/SILK path from source

## Download or Build

Visit [Releases page](https://github.com/nearai/ironclaw/releases/) to see the latest updates.

<details>
  <summary>Install via Windows Installer (Windows)</summary>

Download the [Windows Installer](https://github.com/nearai/ironclaw/releases/latest/download/ironclaw-x86_64-pc-windows-msvc.msi) and run it.

</details>

<details>
  <summary>Install via powershell script (Windows)</summary>

```sh
irm https://github.com/nearai/ironclaw/releases/latest/download/ironclaw-installer.ps1 | iex
```

</details>

<details>
  <summary>Install via shell script (macOS, Linux, Windows/WSL)</summary>

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/nearai/ironclaw/releases/latest/download/ironclaw-installer.sh | sh
```
</details>

<details>
  <summary>Install via Homebrew (macOS/Linux)</summary>

```sh
brew install ironclaw
```

</details>

<details>
  <summary>Compile the source code (Cargo on Windows, Linux, macOS)</summary>

Install it with `cargo`, just make sure you have [Rust](https://rustup.rs) installed on your computer.

```bash
# Clone the repository
git clone https://github.com/nearai/ironclaw.git
cd ironclaw

# Build
cargo build --release

# Run tests
cargo test
```

For **full release** (after modifying channel sources), run `./scripts/build-all.sh` to rebuild channels first.

> **Optional:** WeChat voice notes (`audio/silk`) require the standalone
> `ironclaw-silk-decoder` helper to be transcribable. It's excluded from the
> default workspace build because `silk-codec` pulls in `bindgen`/`libclang`.
> Build it separately with `./crates/ironclaw_silk_decoder/build.sh` (needs
> libclang + a C toolchain) and put the resulting binary on `$PATH`, beside
> the `ironclaw` binary, or pointed at by `IRONCLAW_SILK_DECODER`. Without
> it, voice messages are still delivered — just as raw `audio/silk` blobs.

</details>

### Database Setup

```bash
# Create database
createdb ironclaw

# Enable pgvector
psql ironclaw -c "CREATE EXTENSION IF NOT EXISTS vector;"
```

## Configuration

Run the setup wizard to configure IronClaw:

```bash
ironclaw onboard
```

The wizard handles database connection, NEAR AI authentication (via browser OAuth),
and secrets encryption (using your system keychain). Settings are persisted in the
connected database; bootstrap variables (e.g. `DATABASE_URL`, `LLM_BACKEND`) are
written to `~/.ironclaw/.env` so they are available before the database connects.

### Alternative LLM Providers

IronClaw defaults to NEAR AI but supports many LLM providers out of the box.
Built-in providers include **Anthropic**, **OpenAI**, **GitHub Copilot**, **Google Gemini**, **MiniMax**,
**Mistral**, and **Ollama** (local). OpenAI-compatible services like **OpenRouter**
(300+ models), **Together AI**, **Fireworks AI**, and self-hosted servers (**vLLM**,
**LiteLLM**) are also supported.

Select your provider in the wizard, or set environment variables directly:

```env
# Example: MiniMax (built-in, 204K context)
LLM_BACKEND=minimax
MINIMAX_API_KEY=...

# Example: OpenAI-compatible endpoint
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://openrouter.ai/api/v1
LLM_API_KEY=sk-or-...
LLM_MODEL=anthropic/claude-sonnet-4
```

See [docs/capabilities/llm-providers.md](docs/capabilities/llm-providers.md) for a full provider guide.

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
┌────────────────────────────────────────────────────────────────┐
│                          Channels                              │
│  ┌──────┐  ┌──────┐   ┌─────────────┐  ┌─────────────┐         │
│  │ REPL │  │ HTTP │   │WASM Channels│  │ Web Gateway │         │
│  └──┬───┘  └──┬───┘   └──────┬──────┘  │ (SSE + WS)  │         │
│     │         │              │         └──────┬──────┘         │
│     └─────────┴──────────────┴────────────────┘                │
│                              │                                 │
│                    ┌─────────▼─────────┐                       │
│                    │    Agent Loop     │  Intent routing       │
│                    └────┬──────────┬───┘                       │
│                         │          │                           │
│              ┌──────────▼────┐  ┌──▼───────────────┐           │
│              │  Scheduler    │  │ Routines Engine  │           │
│              │(parallel jobs)│  │(cron, event, wh) │           │
│              └──────┬────────┘  └────────┬─────────┘           │
│                     │                    │                     │
│       ┌─────────────┼────────────────────┘                     │
│       │             │                                          │
│   ┌───▼─────┐  ┌────▼────────────────┐                         │
│   │ Local   │  │    Orchestrator     │                         │
│   │Workers  │  │  ┌───────────────┐  │                         │
│   │(in-proc)│  │  │ Docker Sandbox│  │                         │
│   └───┬─────┘  │  │   Containers  │  │                         │
│       │        │  │ ┌───────────┐ │  │                         │
│       │        │  │ │Worker / CC│ │  │                         │
│       │        │  │ └───────────┘ │  │                         │
│       │        │  └───────────────┘  │                         │
│       │        └─────────┬───────────┘                         │
│       └──────────────────┤                                     │
│                          │                                     │
│              ┌───────────▼──────────┐                          │
│              │    Tool Registry     │                          │
│              │  Built-in, MCP, WASM │                          │
│              └──────────────────────┘                          │
└────────────────────────────────────────────────────────────────┘
```

### Core Components

| Component | Purpose |
|-----------|---------|
| **Agent Loop** | Main message handling and job coordination |
| **Router** | Classifies user intent (command, query, task) |
| **Scheduler** | Manages parallel job execution with priorities |
| **Worker** | Executes jobs with LLM reasoning and tool calls |
| **Orchestrator** | Container lifecycle, LLM proxying, per-job auth |
| **Web Gateway** | Browser UI with chat, memory, jobs, logs, extensions, routines |
| **Routines Engine** | Scheduled (cron) and reactive (event, webhook) background tasks |
| **Workspace** | Persistent memory with hybrid search |
| **Safety Layer** | Prompt injection defense and content sanitization |

## Legacy IronClaw Usage

Engine v2 is opt-in right now. If you want to run the new engine instead of the legacy agent loop, start IronClaw with `ENGINE_V2=true`. See [Engine v2 architecture](docs/internal/engine-v2-architecture.md#enabling-engine-v2) for more details.

```bash
# First-time setup (configures database, auth, etc.)
ironclaw onboard

# Start interactive REPL
cargo run

# Start interactive REPL with engine v2
ENGINE_V2=true cargo run

# Engine v2 with debug logging
ENGINE_V2=true RUST_LOG=ironclaw=debug cargo run
```

## Development

```bash
# Format code
cargo fmt

# Lint
cargo clippy --all --benches --tests --examples --all-features

# Run tests
createdb ironclaw_test
cargo test

# Run specific test
cargo test test_name
```

- **Channels**: See [docs/channels/overview.mdx](docs/channels/overview.mdx) for setup of Telegram, Discord, and other channels.
- **Changing channel sources**: Run `./channels-src/telegram/build.sh` before `cargo build` so the updated WASM is bundled.

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
