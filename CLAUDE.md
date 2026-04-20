# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Repo Is

**LunarWing** is a hard fork of IronClaw (originally by NearAI), started February 2026. The core daemon lives in `ic/`. The product name is LunarWing; `ic/` and `ironclaw` are internal/binary names from upstream.

This is a self-hosted, privacy-first AI agent. The fork prioritizes XMPP/OMEMO, Gotify, scheduled routines, systemd deployment, and open-protocol channels. Proprietary channels (Slack, Discord, Telegram) are intentionally unsupported. Upstream compatibility is not a goal.

## Build & Test

All Rust work happens inside `ic/`. Run these from `ic/`:

```bash
cargo fmt
cargo clippy --all --benches --tests --examples --all-features  # zero warnings required
cargo test                          # unit tests
cargo test --features integration   # + PostgreSQL tests
cargo test test_name -- --nocapture # single test
cargo build --release --bin ironclaw
RUST_LOG=ironclaw=debug cargo run
```

For XMPP bridge work, build the bridge binary first before a full workspace build.

## Repo Structure

```
ic/                         # Main daemon (Rust) — see ic/CLAUDE.md
  src/                      # Source tree
  crates/                   # Extracted crates (ironclaw_safety, ironclaw_engine, etc.)
  channels-src/             # WASM channel sources (xmpp, weechat, darkirc, etc.)
  tools-src/                # WASM tool sources (gotify, github, google-*, etc.)
  bridges/xmpp-bridge/      # Standalone XMPP bridge service (separate process)
  migrations/               # Refinery DB migrations (PostgreSQL + libSQL)
  tests/                    # Integration + E2E tests
  systemd/                  # Systemd unit files
  scripts/                  # Operational scripts
codex4ironclaw/             # Persistent Codex Worker container — see codex4ironclaw/CLAUDE.md
ic-infrastructure-health-check/  # Health check service
tensorzero-proxy-configurations/ # TensorZero HTTP proxy routing
ic_sm / xmpp_bridge / etc.  # Supporting services and resources
docs/                       # Documentation
```

> `ic/customic/` is unused and will be removed. Ignore it.

## Key Guidance Docs

Before modifying complex areas, read the relevant spec. Specs are authoritative.

| Area | Spec |
|------|------|
| Agent rules & repo contract | `AGENTS.md` |
| Fork goals & protected behavior | `FORK_CONTEXT.md` |
| Main daemon development | `ic/CLAUDE.md` |
| Agent loop, sessions, routines | `ic/src/agent/CLAUDE.md` |
| Web gateway / REST / WebSocket | `ic/src/channels/web/CLAUDE.md` |
| Database dual-backend | `ic/src/db/CLAUDE.md` |
| LLM providers | `ic/src/llm/CLAUDE.md` |
| Tools system | `ic/src/tools/README.md` |
| Workspace / memory | `ic/src/workspace/README.md` |
| E2E tests | `ic/tests/e2e/CLAUDE.md` |
| Network security policy | `ic/src/NETWORK_SECURITY.md` |

## Architecture Overview

- **Channels** normalize external input into `IncomingMessage`; `ChannelManager` merges all active streams.
- **Agent** owns session/turn handling, the LLM↔tool loop, approvals, and routines.
- **AppBuilder** is the composition root — wires DB, secrets, LLMs, tools, workspace, extensions, hooks before the agent starts.
- **Web gateway** is a browser-facing API/UI over the same agent/session/tool systems, not a separate product path.
- **XMPP bridge** runs as a separate systemd service; OMEMO happens in the bridge, not the main daemon.
- **WASM sandbox** (wasmtime) provides isolated execution for third-party tools and channels.
- **Dual DB backend**: PostgreSQL (primary) + libSQL/Turso. All new persistence must support both.

Key extensibility traits: `Database`, `Channel`, `Tool`, `LlmProvider`, `EmbeddingProvider`, `Hook`, `Tunnel`.

## Protected Runtime Behavior

Do not break without explicit approval:

- XMPP bridge operation and OMEMO encrypted chat (1:1 and group)
- XMPP group chat self-message suppression and live rate-limit control
- Gotify WASM tool usage
- WASM channel/tool loading
- Scheduled routines and manual routine runs
- Gateway status/config endpoints
- Systemd deployment units and watchdog service/timer

## Service Operations

- `xmpp-bridge.service` has `PartOf=ironclaw.service` — IronClaw restarts can cascade to the bridge. Do not assume the bridge caused a stop just because both restarted.
- Use `scripts/xmpp-rate-limit.sh` for live XMPP outbound rate-limit changes (`status`, `set <n>`, `off`, `reset`).
- Use `scripts/xmpp-configure.sh` for bridge room/configuration checks.
- Watchdog: `scripts/ironclaw-watchdog.sh`, logs to `/var/log/ironclaw-watchdog.log`.
- Prefer read-only diagnostics first (`systemctl status`, `journalctl`, gateway endpoints) before restarting services.
- **Codex must not restart services or deploy binaries unless explicitly asked.**

## Deployment & Secrets

Secrets may live in `/home/cmc/.ironclaw/.env`, systemd service environment, DB rows, or WASM auth state. Never print secret values in logs, diffs, or responses.

For live DB checks, use read-only SQL unless the user explicitly requests mutation. Stop the service before mutating routine state; back up the DB first.

## Gotify Pattern

Gotify is a WASM tool, not a channel. Routines needing Gotify notifications should call the `gotify` tool in their prompt and return the same message as backup output. See `docs/GOTIFY_ROUTINE_PROMPT.md` for the working prompt pattern.

## XMPP / OMEMO Known Behavior

- Bridge rejects conflicting runtime config with HTTP 409 until restarted.
- OMEMO encrypted group chat may take several messages after restart before decrypting reliably.
- Group OMEMO requires the room to be configured as encrypted in both client setup and bridge/runtime config.
