# IronClaw ↔ OpenClaw Feature Parity Matrix

This document tracks feature parity between IronClaw (Rust implementation) and OpenClaw (TypeScript reference implementation). Use this to coordinate work across developers.

**Legend:**
- ✅ Implemented
- 🚧 Partial (in progress or incomplete)
- ❌ Not implemented
- 🔮 Planned (in scope but not started)
- 🚫 Out of scope (intentionally skipped)
- ➖ N/A (not applicable to Rust implementation)

---

## 1. Architecture

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Hub-and-spoke architecture | ✅ | ✅ | Web gateway as central hub |
| WebSocket control plane | ✅ | ✅ | Gateway with WebSocket + SSE |
| Single-user system | ✅ | ✅ | |
| Multi-agent routing | ✅ | ❌ | Workspace isolation per-agent |
| Session-based messaging | ✅ | ✅ | Per-sender sessions |
| Loopback-first networking | ✅ | ✅ | HTTP binds to 0.0.0.0 but can be configured |

### Owner: _Unassigned_

---

## 2. Gateway System

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Gateway control plane | ✅ | ✅ | Web gateway with 40+ API endpoints |
| HTTP endpoints for Control UI | ✅ | ✅ | Web dashboard with chat, memory, jobs, logs, extensions |
| Channel connection lifecycle | ✅ | ✅ | ChannelManager + WebSocket tracker |
| Session management/routing | ✅ | ✅ | SessionManager exists |
| Configuration hot-reload | ✅ | ❌ | |
| Network modes (loopback/LAN/remote) | ✅ | 🚧 | HTTP only |
| OpenAI-compatible HTTP API | ✅ | ✅ | /v1/chat/completions |
| Canvas hosting | ✅ | ❌ | Agent-driven UI |
| Gateway lock (PID-based) | ✅ | ❌ | |
| launchd/systemd integration | ✅ | ❌ | |
| Bonjour/mDNS discovery | ✅ | ❌ | |
| Tailscale integration | ✅ | ❌ | |
| Health check endpoints | ✅ | ✅ | /api/health + /api/gateway/status |
| `doctor` diagnostics | ✅ | ❌ | |

### Owner: _Unassigned_

---

## 3. Messaging Channels

| Channel | OpenClaw | IronClaw | Priority | Notes |
|---------|----------|----------|----------|-------|
| CLI/TUI | ✅ | ✅ | - | Ratatui-based TUI |
| HTTP webhook | ✅ | ✅ | - | axum with secret validation |
| REPL (simple) | ✅ | ✅ | - | For testing |
| WASM channels | ❌ | ✅ | - | IronClaw innovation |
| WhatsApp | ✅ | ❌ | P1 | Baileys (Web) |
| Telegram | ✅ | ✅ | - | WASM channel(MTProto), DM pairing, caption, /start, bot_username |
| Discord | ✅ | ❌ | P2 | discord.js |
| Signal | ✅ | ❌ | P2 | signal-cli |
| Slack | ✅ | ✅ | - | WASM tool |
| iMessage | ✅ | ❌ | P3 | BlueBubbles recommended |
| Feishu/Lark | ✅ | ❌ | P3 | |
| LINE | ✅ | ❌ | P3 | |
| WebChat | ✅ | ✅ | - | Web gateway chat |
| Matrix | ✅ | ❌ | P3 | E2EE support |
| Mattermost | ✅ | ❌ | P3 | |
| Google Chat | ✅ | ❌ | P3 | |
| MS Teams | ✅ | ❌ | P3 | |
| Twitch | ✅ | ❌ | P3 | |
| Voice Call | ✅ | ❌ | P3 | Twilio/Telnyx |
| Nostr | ✅ | ❌ | P3 | |

### Channel Features

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| DM pairing codes | ✅ | ✅ | `ironclaw pairing list/approve`, host APIs |
| Allowlist/blocklist | ✅ | 🚧 | allow_from + pairing store |
| Self-message bypass | ✅ | ❌ | Own messages skip pairing |
| Mention-based activation | ✅ | ✅ | bot_username + respond_to_all_group_messages |
| Per-group tool policies | ✅ | ❌ | Allow/deny specific tools |
| Thread isolation | ✅ | ✅ | Separate sessions per thread |
| Per-channel media limits | ✅ | 🚧 | Caption support for media; no size limits |
| Typing indicators | ✅ | 🚧 | TUI shows status |

### Owner: _Unassigned_

---

## 4. CLI Commands

| Command | OpenClaw | IronClaw | Priority | Notes |
|---------|----------|----------|----------|-------|
| `run` (agent) | ✅ | ✅ | - | Default command |
| `tool install/list/remove` | ✅ | ✅ | - | WASM tools |
| `gateway start/stop` | ✅ | ❌ | P2 | |
| `onboard` (wizard) | ✅ | ✅ | - | Interactive setup |
| `tui` | ✅ | ✅ | - | Ratatui TUI |
| `config` | ✅ | ✅ | - | Read/write config |
| `channels` | ✅ | ❌ | P2 | Channel management |
| `models` | ✅ | 🚧 | - | Model selector in TUI |
| `status` | ✅ | ✅ | - | System status |
| `agents` | ✅ | ❌ | P3 | Multi-agent management |
| `sessions` | ✅ | ❌ | P3 | Session listing |
| `memory` | ✅ | ✅ | - | Memory search CLI |
| `skills` | ✅ | ❌ | P3 | Agent skills |
| `pairing` | ✅ | ✅ | - | list/approve for channel DM pairing |
| `nodes` | ✅ | ❌ | P3 | Device management |
| `plugins` | ✅ | ❌ | P3 | Plugin management |
| `hooks` | ✅ | ✅ | P2 | Lifecycle hooks |
| `cron` | ✅ | ❌ | P2 | Scheduled jobs |
| `webhooks` | ✅ | ❌ | P3 | Webhook config |
| `message send` | ✅ | ❌ | P2 | Send to channels |
| `browser` | ✅ | ❌ | P3 | Browser automation |
| `sandbox` | ✅ | ✅ | - | WASM sandbox |
| `doctor` | ✅ | ❌ | P2 | Diagnostics |
| `logs` | ✅ | ❌ | P3 | Query logs |
| `update` | ✅ | ❌ | P3 | Self-update |
| `completion` | ✅ | ❌ | P3 | Shell completion |

### Owner: _Unassigned_

---

## 5. Agent System

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Pi agent runtime | ✅ | ✅ | ARM64 builds + MCP GPIO support |
| RPC-based execution | ✅ | ✅ | Orchestrator/worker pattern |
| Multi-provider failover | ✅ | ✅ | `FailoverProvider` tries providers sequentially on retryable errors |
| Per-sender sessions | ✅ | ✅ | |
| Global sessions | ✅ | ❌ | Optional shared context |
| Session pruning | ✅ | ❌ | Auto cleanup old sessions |
| Context compaction | ✅ | ✅ | Auto summarization |
| Custom system prompts | ✅ | ✅ | Template variables |
| Skills (modular capabilities) | ✅ | ❌ | Capability bundles |
| Thinking modes (low/med/high) | ✅ | ❌ | Configurable reasoning depth |
| Block-level streaming | ✅ | ❌ | |
| Tool-level streaming | ✅ | ❌ | |
| Plugin tools | ✅ | ✅ | WASM tools |
| Tool policies (allow/deny) | ✅ | ✅ | |
| Exec approvals (`/approve`) | ✅ | ✅ | TUI approval overlay |
| Elevated mode | ✅ | ❌ | Privileged execution |
| Subagent support | ✅ | ✅ | Task framework |
| Auth profiles | ✅ | ❌ | Multiple auth strategies |

### Owner: _Unassigned_

---

## 6. Model & Provider Support

| Provider | OpenClaw | IronClaw | Priority | Notes |
|----------|----------|----------|----------|-------|
| NEAR AI | ✅ | ✅ | - | Primary provider |
| Anthropic (Claude) | ✅ | 🚧 | - | Via NEAR AI proxy |
| OpenAI | ✅ | 🚧 | - | Via NEAR AI proxy |
| AWS Bedrock | ✅ | ❌ | P3 | |
| Google Gemini | ✅ | ❌ | P3 | |
| OpenRouter | ✅ | ❌ | P3 | |
| Ollama (local) | ✅ | ✅ | - | via `rig::providers::ollama` (full support) |
| node-llama-cpp | ✅ | ➖ | - | N/A for Rust |
| llama.cpp (native) | ❌ | 🔮 | P3 | Rust bindings |

### Model Features

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Auto-discovery | ✅ | ❌ | |
| Failover chains | ✅ | ✅ | `FailoverProvider` with configurable `fallback_model` |
| Cooldown management | ✅ | ✅ | Lock-free per-provider cooldown in `FailoverProvider` |
| Per-session model override | ✅ | ✅ | Model selector in TUI |
| Model selection UI | ✅ | ✅ | TUI keyboard shortcut |

### Owner: _Unassigned_

---

## 7. Media Handling

| Feature | OpenClaw | IronClaw | Priority | Notes |
|---------|----------|----------|----------|-------|
| Image processing (Sharp) | ✅ | ❌ | P2 | Resize, format convert |
| Audio transcription | ✅ | ❌ | P2 | |
| Video support | ✅ | ❌ | P3 | |
| PDF parsing | ✅ | ❌ | P2 | pdfjs-dist |
| MIME detection | ✅ | ❌ | P2 | |
| Media caching | ✅ | ❌ | P3 | |
| Vision model integration | ✅ | ❌ | P2 | Image understanding |
| TTS (Edge TTS) | ✅ | ❌ | P3 | Text-to-speech |
| TTS (OpenAI) | ✅ | ❌ | P3 | |
| Sticker-to-image | ✅ | ❌ | P3 | Telegram stickers |

### Owner: _Unassigned_

---

## 8. Plugin & Extension System

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Dynamic loading | ✅ | ✅ | WASM modules |
| Manifest validation | ✅ | ✅ | WASM metadata |
| HTTP path registration | ✅ | ❌ | Plugin routes |
| Workspace-relative install | ✅ | ✅ | ~/.ironclaw/tools/ |
| Channel plugins | ✅ | ✅ | WASM channels |
| Auth plugins | ✅ | ❌ | |
| Memory plugins | ✅ | ❌ | Custom backends |
| Tool plugins | ✅ | ✅ | WASM tools |
| Hook plugins | ✅ | ❌ | |
| Provider plugins | ✅ | ❌ | |
| Plugin CLI (`install`, `list`) | ✅ | ✅ | `tool` subcommand |
| ClawHub registry | ✅ | ❌ | Discovery |

### Owner: _Unassigned_

---

## 9. Configuration System

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Primary config file | ✅ `~/.openclaw/openclaw.json` | ✅ `.env` | Different formats |
| JSON5 support | ✅ | ❌ | Comments, trailing commas |
| YAML alternative | ✅ | ❌ | |
| Environment variable interpolation | ✅ | ✅ | `${VAR}` |
| Config validation/schema | ✅ | ✅ | Type-safe Config struct |
| Hot-reload | ✅ | ❌ | |
| Legacy migration | ✅ | ➖ | |
| State directory | ✅ `~/.openclaw-state/` | ✅ `~/.ironclaw/` | |
| Credentials directory | ✅ | ✅ | Session files |

### Owner: _Unassigned_

---

## 10. Memory & Knowledge System

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Vector memory | ✅ | ✅ | pgvector |
| Session-based memory | ✅ | ✅ | |
| Hybrid search (BM25 + vector) | ✅ | ✅ | RRF algorithm |
| OpenAI embeddings | ✅ | ✅ | |
| Gemini embeddings | ✅ | ❌ | |
| Local embeddings | ✅ | ❌ | |
| SQLite-vec backend | ✅ | ❌ | IronClaw uses PostgreSQL |
| LanceDB backend | ✅ | ❌ | |
| QMD backend | ✅ | ❌ | |
| Atomic reindexing | ✅ | ✅ | |
| Embeddings batching | ✅ | ❌ | |
| Citation support | ✅ | ❌ | |
| Memory CLI commands | ✅ | ❌ | `memory search/index/status` |
| Flexible path structure | ✅ | ✅ | Filesystem-like API |
| Identity files (AGENTS.md, etc.) | ✅ | ✅ | |
| Daily logs | ✅ | ✅ | |
| Heartbeat checklist | ✅ | ✅ | HEARTBEAT.md |

### Owner: _Unassigned_

---

## 11. Mobile Apps

| Feature | OpenClaw | IronClaw | Priority | Notes |
|---------|----------|----------|----------|-------|
| iOS app (SwiftUI) | ✅ | 🚫 | - | Out of scope initially |
| Android app (Kotlin) | ✅ | 🚫 | - | Out of scope initially |
| Gateway WebSocket client | ✅ | 🚫 | - | |
| Camera/photo access | ✅ | 🚫 | - | |
| Voice input | ✅ | 🚫 | - | |
| Push-to-talk | ✅ | 🚫 | - | |
| Location sharing | ✅ | 🚫 | - | |
| Node pairing | ✅ | 🚫 | - | |

### Owner: _Unassigned_ (if ever prioritized)

---

## 12. macOS App

| Feature | OpenClaw | IronClaw | Priority | Notes |
|---------|----------|----------|----------|-------|
| SwiftUI native app | ✅ | 🚫 | - | Out of scope |
| Menu bar presence | ✅ | 🚫 | - | |
| Bundled gateway | ✅ | 🚫 | - | |
| Canvas hosting | ✅ | 🚫 | - | |
| Voice wake | ✅ | 🚫 | - | |
| Exec approval dialogs | ✅ | ✅ | - | TUI overlay |
| iMessage integration | ✅ | 🚫 | - | |

### Owner: _Unassigned_ (if ever prioritized)

---

## 13. Web Interface

| Feature | OpenClaw | IronClaw | Priority | Notes |
|---------|----------|----------|----------|-------|
| Control UI Dashboard | ✅ | ✅ | - | Web gateway with chat, memory, jobs, logs, extensions |
| Channel status view | ✅ | 🚧 | P2 | Gateway status widget, full channel view pending |
| Agent management | ✅ | ❌ | P3 | |
| Model selection | ✅ | ✅ | - | TUI only |
| Config editing | ✅ | ❌ | P3 | |
| Debug/logs viewer | ✅ | ✅ | - | Real-time log streaming with level/target filters |
| WebChat interface | ✅ | ✅ | - | Web gateway chat with SSE/WebSocket |
| Canvas system (A2UI) | ✅ | ❌ | P3 | Agent-driven UI |

### Owner: _Unassigned_

---

## 14. Automation

| Feature | OpenClaw | IronClaw | Priority | Notes |
|---------|----------|----------|----------|-------|
| Cron jobs | ✅ | ✅ | - | Routines with cron trigger |
| Timezone support | ✅ | ✅ | - | Via cron expressions |
| One-shot/recurring jobs | ✅ | ✅ | - | Manual + cron triggers |
| `beforeInbound` hook | ✅ | ✅ | P2 | |
| `beforeOutbound` hook | ✅ | ✅ | P2 | |
| `beforeToolCall` hook | ✅ | ✅ | P2 | |
| `onMessage` hook | ✅ | ✅ | - | Routines with event trigger |
| `onSessionStart` hook | ✅ | ✅ | P2 | |
| `onSessionEnd` hook | ✅ | ✅ | P2 | |
| `transcribeAudio` hook | ✅ | ❌ | P3 | |
| `transformResponse` hook | ✅ | ✅ | P2 | |
| Bundled hooks | ✅ | ❌ | P2 | |
| Plugin hooks | ✅ | ❌ | P3 | |
| Workspace hooks | ✅ | ❌ | P2 | Inline code |
| Outbound webhooks | ✅ | ❌ | P2 | |
| Heartbeat system | ✅ | ✅ | - | Periodic execution |
| Gmail pub/sub | ✅ | ❌ | P3 | |

### Owner: _Unassigned_

---

## 15. Security Features

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Gateway token auth | ✅ | ✅ | Bearer token auth on web gateway |
| Device pairing | ✅ | ❌ | |
| Tailscale identity | ✅ | ❌ | |
| OAuth flows | ✅ | 🚧 | NEAR AI OAuth |
| DM pairing verification | ✅ | ✅ | ironclaw pairing approve, host APIs |
| Allowlist/blocklist | ✅ | 🚧 | allow_from + pairing store |
| Per-group tool policies | ✅ | ❌ | |
| Exec approvals | ✅ | ✅ | TUI overlay |
| TLS 1.3 minimum | ✅ | ✅ | reqwest rustls |
| SSRF protection | ✅ | ✅ | WASM allowlist |
| Loopback-first | ✅ | 🚧 | HTTP binds 0.0.0.0 |
| Docker sandbox | ✅ | ✅ | Orchestrator/worker containers |
| WASM sandbox | ❌ | ✅ | IronClaw innovation |
| Tool policies | ✅ | ✅ | |
| Elevated mode | ✅ | ❌ | |
| Safe bins allowlist | ✅ | ❌ | |
| LD*/DYLD* validation | ✅ | ❌ | |
| Path traversal prevention | ✅ | ✅ | |
| Webhook signature verification | ✅ | ✅ | |
| Media URL validation | ✅ | ❌ | |
| Prompt injection defense | ✅ | ✅ | Pattern detection, sanitization |
| Leak detection | ✅ | ✅ | Secret exfiltration |

### Owner: _Unassigned_

---

## 16. Development & Build System

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Primary language | TypeScript | Rust | Different ecosystems |
| Build tool | tsdown | cargo | |
| Type checking | TypeScript/tsgo | rustc | |
| Linting | Oxlint | clippy | |
| Formatting | Oxfmt | rustfmt | |
| Package manager | pnpm | cargo | |
| Test framework | Vitest | built-in | |
| Coverage | V8 | tarpaulin/llvm-cov | |
| CI/CD | GitHub Actions | GitHub Actions | |
| Pre-commit hooks | prek | - | Consider adding |

### Owner: _Unassigned_

---

## 17. Edge & IoT System

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| GPIO Control | ✅ | ✅ | via `mcp-gpio` (MCP server) |
| Hardware Daemon | ✅ | ✅ | Systemd service with allowlists |
| I2C/SPI Support | ✅ | ❌ | Planned for future MCPs |
| Camera Feed | ✅ | ❌ | Planned |

---

## Implementation Priorities

### P0 - Core (Already Done)
- ✅ TUI channel with approval overlays
- ✅ HTTP webhook channel
- ✅ DM pairing (ironclaw pairing list/approve, host APIs)
- ✅ WASM tool sandbox
- ✅ Workspace/memory with hybrid search
- ✅ Prompt injection defense
- ✅ Heartbeat system
- ✅ Session management
- ✅ Context compaction
- ✅ Model selection
- ✅ Gateway control plane + WebSocket
- ✅ Web Control UI (chat, memory, jobs, logs, extensions, routines)
- ✅ WebChat channel (web gateway)
- ✅ Slack channel (WASM tool)
- ✅ Telegram channel (WASM tool, MTProto)
- ✅ Docker sandbox (orchestrator/worker)
- ✅ Cron job scheduling (routines)
- ✅ CLI subcommands (onboard, config, status, memory)
- ✅ Gateway token auth

### P1 - High Priority
- ❌ Slack channel (real implementation)
- ✅ Telegram channel (WASM, DM pairing, caption, /start)
- ❌ WhatsApp channel
- ✅ Multi-provider failover (`FailoverProvider` with retryable error classification)
- ✅ Hooks system (beforeInbound, beforeToolCall, beforeOutbound, onSessionStart, onSessionEnd, transformResponse)
- ✅ Edge/IoT Support (ARM64, GPIO MCP, Hardware Daemon)

### P2 - Medium Priority
- ❌ Media handling (images, PDFs)
- ❌ Ollama/local model support
- ❌ Configuration hot-reload
- ❌ Webhook trigger endpoint in web gateway

### P3 - Lower Priority
- ❌ Discord channel
- ❌ Signal channel
- ❌ Matrix channel
- ❌ Other messaging platforms
- ❌ TTS/audio features
- ❌ Video support
- ❌ Skills system
- ❌ Plugin registry

---

## How to Contribute

1. **Claim a section**: Edit this file and add your name/handle to the "Owner" field
2. **Create a tracking issue**: Link to GitHub issue for the feature area
3. **Update status**: Change ❌ to 🚧 when starting, ✅ when complete
4. **Add notes**: Document any design decisions or deviations

### Coordination

- Each major section should have one owner to avoid conflicts
- Owners can delegate sub-features to others
- Update this file as part of your PR

---

## Deviations from OpenClaw

IronClaw intentionally differs from OpenClaw in these ways:

1. **Rust vs TypeScript**: Native performance, memory safety, single binary distribution
2. **WASM sandbox vs Docker**: Lighter weight, faster startup, capability-based security
3. **PostgreSQL vs SQLite**: Better suited for production deployments
4. **NEAR AI focus**: Primary provider with session-based auth
5. **No mobile/desktop apps**: Focus on server-side and CLI initially
6. **WASM channels**: Novel extension mechanism not in OpenClaw

These are intentional architectural choices, not gaps to be filled.
