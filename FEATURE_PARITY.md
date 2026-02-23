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
| OpenAI-compatible HTTP API | ✅ | ✅ | /v1/chat/completions, per-request `model` override |
| Canvas hosting | ✅ | ❌ | Agent-driven UI |
| Gateway lock (PID-based) | ✅ | ❌ | |
| launchd/systemd integration | ✅ | ❌ | |
| Bonjour/mDNS discovery | ✅ | ❌ | |
| Tailscale integration | ✅ | ❌ | |
| Health check endpoints | ✅ | ✅ | /api/health + /api/gateway/status |
| `doctor` diagnostics | ✅ | ❌ | |
| Agent event broadcast | ✅ | 🚧 | SSE broadcast manager exists (SseManager) but tool/job-state events not fully wired |
| Channel health monitor | ✅ | ❌ | Auto-restart with configurable interval |
| Presence system | ✅ | ❌ | Beacons on connect, system presence for agents |
| Trusted-proxy auth mode | ✅ | ❌ | Header-based auth for reverse proxies |
| APNs push pipeline | ✅ | ❌ | Wake disconnected iOS nodes via push |
| Oversized payload guard | ✅ | 🚧 | HTTP webhook has 64KB body limit + Content-Length check; no chat.history cap |
| Pre-prompt context diagnostics | ✅ | ❌ | Context size logging before prompt |

### Owner: _Unassigned_

---

## 3. Messaging Channels

| Channel | OpenClaw | IronClaw | Priority | Notes |
|---------|----------|----------|----------|-------|
| CLI/TUI | ✅ | ✅ | - | Ratatui-based TUI |
| HTTP webhook | ✅ | ✅ | - | axum with secret validation |
| REPL (simple) | ✅ | ✅ | - | For testing |
| WASM channels | ❌ | ✅ | - | IronClaw innovation |
| WhatsApp | ✅ | ❌ | P1 | Baileys (Web), same-phone mode with echo detection |
| Telegram | ✅ | ✅ | - | WASM channel(MTProto), DM pairing, caption, /start, bot_username |
| Discord | ✅ | ❌ | P2 | discord.js, thread parent binding inheritance |
| Signal | ✅ | ❌ | P2 | signal-cli |
| Slack | ✅ | ✅ | - | WASM tool |
| iMessage | ✅ | ❌ | P3 | BlueBubbles or Linq recommended |
| Linq | ✅ | ❌ | P3 | Real iMessage via API, no Mac required |
| Feishu/Lark | ✅ | ❌ | P3 | Bitable create app/field tools |
| LINE | ✅ | ❌ | P3 | |
| WebChat | ✅ | ✅ | - | Web gateway chat |
| Matrix | ✅ | ❌ | P3 | E2EE support |
| Mattermost | ✅ | ❌ | P3 | Emoji reactions |
| Google Chat | ✅ | ❌ | P3 | |
| MS Teams | ✅ | ❌ | P3 | |
| Twitch | ✅ | ❌ | P3 | |
| Voice Call | ✅ | ❌ | P3 | Twilio/Telnyx, stale call reaper, pre-cached greeting |
| Nostr | ✅ | ❌ | P3 | |

### Telegram-Specific Features (since Feb 2025)

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Forum topic creation | ✅ | ❌ | Create topics in forum groups |
| channel_post support | ✅ | ❌ | Bot-to-bot communication |
| User message reactions | ✅ | ❌ | Surface inbound reactions |
| sendPoll | ✅ | ❌ | Poll creation via agent |
| Cron/heartbeat topic targeting | ✅ | ❌ | Messages land in correct topic |

### Discord-Specific Features (since Feb 2025)

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Forwarded attachment downloads | ✅ | ❌ | Fetch media from forwarded messages |
| Faster reaction state machine | ✅ | ❌ | Watchdog + debounce |
| Thread parent binding inheritance | ✅ | ❌ | Threads inherit parent routing |

### Slack-Specific Features (since Feb 2025)

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Streaming draft replies | ✅ | ❌ | Partial replies via draft message updates |
| Configurable stream modes | ✅ | ❌ | Per-channel stream behavior |
| Thread ownership | ✅ | ❌ | Thread-level ownership tracking |

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
| Typing indicators | ✅ | 🚧 | TUI + Telegram typing/actionable status prompts; richer parity pending |
| Per-channel ackReaction config | ✅ | ❌ | Customizable acknowledgement reactions |
| Group session priming | ✅ | ❌ | Member roster injected for context |
| Sender_id in trusted metadata | ✅ | ❌ | Exposed in system metadata |

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
| `status` | ✅ | ✅ | - | System status (enriched session details) |
| `agents` | ✅ | ❌ | P3 | Multi-agent management |
| `sessions` | ✅ | ❌ | P3 | Session listing (shows subagent models) |
| `memory` | ✅ | ✅ | - | Memory search CLI |
| `skills` | ✅ | ✅ | - | Skills tools + web API endpoints (install, list, activate) |
| `pairing` | ✅ | ✅ | - | list/approve, account selector |
| `nodes` | ✅ | ❌ | P3 | Device management, remove/clear flows |
| `plugins` | ✅ | ❌ | P3 | Plugin management |
| `hooks` | ✅ | ✅ | P2 | Lifecycle hooks |
| `cron` | ✅ | ❌ | P2 | Scheduled jobs (model/thinking fields in edit) |
| `webhooks` | ✅ | ❌ | P3 | Webhook config |
| `message send` | ✅ | ❌ | P2 | Send to channels |
| `browser` | ✅ | ❌ | P3 | Browser automation |
| `sandbox` | ✅ | ✅ | - | WASM sandbox |
| `doctor` | ✅ | ❌ | P2 | Diagnostics |
| `logs` | ✅ | ❌ | P3 | Query logs |
| `update` | ✅ | ❌ | P3 | Self-update |
| `completion` | ✅ | ❌ | P3 | Shell completion |
| `/subagents spawn` | ✅ | ❌ | P3 | Spawn subagents from chat |
| `/export-session` | ✅ | ❌ | P3 | Export current session transcript |

### Owner: _Unassigned_

---

## 5. Agent System

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Pi agent runtime | ✅ | ➖ | IronClaw uses custom runtime |
| RPC-based execution | ✅ | ✅ | Orchestrator/worker pattern |
| Multi-provider failover | ✅ | ✅ | `FailoverProvider` tries providers sequentially on retryable errors |
| Per-sender sessions | ✅ | ✅ | |
| Global sessions | ✅ | ❌ | Optional shared context |
| Session pruning | ✅ | ❌ | Auto cleanup old sessions |
| Context compaction | ✅ | ✅ | Auto summarization |
| Post-compaction read audit | ✅ | ❌ | Layer 3: workspace rules appended to summaries |
| Post-compaction context injection | ✅ | ❌ | Workspace context as system event |
| Custom system prompts | ✅ | ✅ | Template variables, safety guardrails |
| Skills (modular capabilities) | ✅ | ✅ | Prompt-based skills with trust gating, attenuation, activation criteria, catalog, selector |
| Skill routing blocks | ✅ | 🚧 | ActivationCriteria (keywords, patterns, tags) but no "Use when / Don't use when" blocks |
| Skill path compaction | ✅ | ❌ | ~ prefix to reduce prompt tokens |
| Thinking modes (low/med/high) | ✅ | ❌ | Configurable reasoning depth |
| Per-model thinkingDefault override | ✅ | ❌ | Override thinking level per model |
| Block-level streaming | ✅ | ❌ | |
| Tool-level streaming | ✅ | ❌ | |
| Z.AI tool_stream | ✅ | ❌ | Real-time tool call streaming |
| Plugin tools | ✅ | ✅ | WASM tools |
| Tool policies (allow/deny) | ✅ | ✅ | |
| Exec approvals (`/approve`) | ✅ | ✅ | TUI approval overlay |
| Elevated mode | ✅ | ❌ | Privileged execution |
| Subagent support | ✅ | ✅ | Task framework |
| `/subagents spawn` command | ✅ | ❌ | Spawn from chat |
| Auth profiles | ✅ | ❌ | Multiple auth strategies |
| Generic API key rotation | ✅ | ❌ | Rotate keys across providers |
| Stuck loop detection | ✅ | ❌ | Exponential backoff on stuck agent loops |
| llms.txt discovery | ✅ | ❌ | Auto-discover site metadata |
| Multiple images per tool call | ✅ | ❌ | Single tool call, multiple images |
| URL allowlist (web_search/fetch) | ✅ | ❌ | Restrict web tool targets |
| suppressToolErrors config | ✅ | ❌ | Hide tool errors from user |
| Intent-first tool display | ✅ | ❌ | Details and exec summaries |
| Transcript file size in status | ✅ | ❌ | Show size in session status |

### Owner: _Unassigned_

---

## 6. Model & Provider Support

| Provider | OpenClaw | IronClaw | Priority | Notes |
|----------|----------|----------|----------|-------|
| NEAR AI | ✅ | ✅ | - | Primary provider |
| Anthropic (Claude) | ✅ | 🚧 | - | Via NEAR AI proxy; Opus 4.5, Sonnet 4, Sonnet 4.6 |
| OpenAI | ✅ | 🚧 | - | Via NEAR AI proxy |
| AWS Bedrock | ✅ | ❌ | P3 | |
| Google Gemini | ✅ | ❌ | P3 | |
| NVIDIA API | ✅ | ❌ | P3 | New provider |
| OpenRouter | ✅ | ✅ | - | Via OpenAI-compatible provider (RigAdapter) |
| Tinfoil | ❌ | ✅ | - | Private inference provider (IronClaw-only) |
| OpenAI-compatible | ❌ | ✅ | - | Generic OpenAI-compatible endpoint (RigAdapter) |
| Ollama (local) | ✅ | ✅ | - | via `rig::providers::ollama` (full support) |
| Perplexity | ✅ | ❌ | P3 | Freshness parameter for web_search |
| MiniMax | ✅ | ❌ | P3 | Regional endpoint selection |
| GLM-5 | ✅ | ❌ | P3 | |
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
| Per-model thinkingDefault | ✅ | ❌ | Override thinking level per model in config |
| 1M context beta header | ✅ | ❌ | Anthropic extended context support |

### Owner: _Unassigned_

---

## 7. Media Handling

| Feature | OpenClaw | IronClaw | Priority | Notes |
|---------|----------|----------|----------|-------|
| Image processing (Sharp) | ✅ | ❌ | P2 | Resize, format convert |
| Configurable image resize dims | ✅ | ❌ | P2 | Per-agent dimension config |
| Multiple images per tool call | ✅ | ❌ | P2 | Single tool invocation, multiple images |
| Audio transcription | ✅ | ❌ | P2 | |
| Video support | ✅ | ❌ | P3 | |
| PDF parsing | ✅ | ❌ | P2 | pdfjs-dist |
| MIME detection | ✅ | ❌ | P2 | |
| Media caching | ✅ | ❌ | P3 | |
| Vision model integration | ✅ | ❌ | P2 | Image understanding |
| TTS (Edge TTS) | ✅ | ❌ | P3 | Text-to-speech |
| TTS (OpenAI) | ✅ | ❌ | P3 | |
| Incremental TTS playback | ✅ | ❌ | P3 | iOS progressive playback |
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
| Hook plugins | ✅ | ✅ | Declarative hooks from extension capabilities |
| Provider plugins | ✅ | ❌ | |
| Plugin CLI (`install`, `list`) | ✅ | ✅ | `tool` subcommand |
| ClawHub registry | ✅ | ❌ | Discovery |
| `before_agent_start` hook | ✅ | ❌ | modelOverride/providerOverride support |
| `before_message_write` hook | ✅ | ❌ | Pre-write message interception |
| `llm_input`/`llm_output` hooks | ✅ | ❌ | LLM payload inspection |

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
| Full model compat fields in schema | ✅ | ❌ | pi-ai model compat exposed in config |

### Owner: _Unassigned_

---

## 10. Memory & Knowledge System

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Vector memory | ✅ | ✅ | pgvector |
| Session-based memory | ✅ | ✅ | |
| Hybrid search (BM25 + vector) | ✅ | ✅ | RRF algorithm |
| Temporal decay (hybrid search) | ✅ | ❌ | Opt-in time-based scoring factor |
| MMR re-ranking | ✅ | ❌ | Maximal marginal relevance for result diversity |
| LLM-based query expansion | ✅ | ❌ | Expand FTS queries via LLM |
| OpenAI embeddings | ✅ | ✅ | |
| Gemini embeddings | ✅ | ❌ | |
| Local embeddings | ✅ | ❌ | |
| SQLite-vec backend | ✅ | ❌ | IronClaw uses PostgreSQL |
| LanceDB backend | ✅ | ❌ | Configurable auto-capture max length |
| QMD backend | ✅ | ❌ | |
| Atomic reindexing | ✅ | ✅ | |
| Embeddings batching | ✅ | ✅ | `embed_batch` on EmbeddingProvider trait |
| Citation support | ✅ | ❌ | |
| Memory CLI commands | ✅ | ✅ | `memory search/read/write/tree/status` CLI subcommands |
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
| Apple Watch companion | ✅ | 🚫 | - | Send/receive messages MVP |
| Gateway WebSocket client | ✅ | 🚫 | - | |
| Camera/photo access | ✅ | 🚫 | - | |
| Voice input | ✅ | 🚫 | - | |
| Push-to-talk | ✅ | 🚫 | - | |
| Location sharing | ✅ | 🚫 | - | |
| Node pairing | ✅ | 🚫 | - | |
| APNs push notifications | ✅ | 🚫 | - | Wake disconnected nodes before invoke |
| Share to OpenClaw (iOS) | ✅ | 🚫 | - | iOS share sheet integration |
| Background listening toggle | ✅ | 🚫 | - | iOS background audio |

### Owner: _Unassigned_ (if ever prioritized)

---

## 12. macOS App

| Feature | OpenClaw | IronClaw | Priority | Notes |
|---------|----------|----------|----------|-------|
| SwiftUI native app | ✅ | 🚫 | - | Out of scope |
| Menu bar presence | ✅ | 🚫 | - | Animated menubar icon |
| Bundled gateway | ✅ | 🚫 | - | |
| Canvas hosting | ✅ | 🚫 | - | Agent-controlled panel with placement/resizing |
| Voice wake | ✅ | 🚫 | - | Overlay, mic picker, language selection, live meter |
| Voice wake overlay | ✅ | 🚫 | - | Partial transcripts, adaptive delays, dismiss animations |
| Push-to-talk hotkey | ✅ | 🚫 | - | System-wide hotkey |
| Exec approval dialogs | ✅ | ✅ | - | TUI overlay |
| iMessage integration | ✅ | 🚫 | - | |
| Instances tab | ✅ | 🚫 | - | Presence beacons across instances |
| Agent events debug window | ✅ | 🚫 | - | Real-time event inspector |
| Sparkle auto-updates | ✅ | 🚫 | - | Appcast distribution |

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
| Canvas system (A2UI) | ✅ | ❌ | P3 | Agent-driven UI, improved asset resolution |
| Control UI i18n | ✅ | ❌ | P3 | English, Chinese, Portuguese |
| WebChat theme sync | ✅ | ❌ | P3 | Sync with system dark/light mode |
| Partial output on abort | ✅ | ❌ | P2 | Preserve partial output when aborting |

### Owner: _Unassigned_

---

## 14. Automation

| Feature | OpenClaw | IronClaw | Priority | Notes |
|---------|----------|----------|----------|-------|
| Cron jobs | ✅ | ✅ | - | Routines with cron trigger |
| Cron stagger controls | ✅ | ❌ | P3 | Default stagger for scheduled jobs |
| Cron finished-run webhook | ✅ | ❌ | P3 | Webhook on job completion |
| Timezone support | ✅ | ✅ | - | Via cron expressions |
| One-shot/recurring jobs | ✅ | ✅ | - | Manual + cron triggers |
| Channel health monitor | ✅ | ❌ | P2 | Auto-restart with configurable interval |
| `beforeInbound` hook | ✅ | ✅ | P2 | |
| `beforeOutbound` hook | ✅ | ✅ | P2 | |
| `beforeToolCall` hook | ✅ | ✅ | P2 | |
| `before_agent_start` hook | ✅ | ❌ | P2 | Model/provider override |
| `before_message_write` hook | ✅ | ❌ | P2 | Pre-write interception |
| `onMessage` hook | ✅ | ✅ | - | Routines with event trigger |
| `onSessionStart` hook | ✅ | ✅ | P2 | |
| `onSessionEnd` hook | ✅ | ✅ | P2 | |
| `transcribeAudio` hook | ✅ | ❌ | P3 | |
| `transformResponse` hook | ✅ | ✅ | P2 | |
| `llm_input`/`llm_output` hooks | ✅ | ❌ | P3 | LLM payload inspection |
| Bundled hooks | ✅ | ✅ | P2 | Audit + declarative rule/webhook hooks |
| Plugin hooks | ✅ | ✅ | P3 | Registered from WASM `capabilities.json` |
| Workspace hooks | ✅ | ✅ | P2 | `hooks/hooks.json` and `hooks/*.hook.json` |
| Outbound webhooks | ✅ | ✅ | P2 | Fire-and-forget lifecycle event delivery |
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
| Trusted-proxy auth | ✅ | ❌ | Header-based reverse proxy auth |
| OAuth flows | ✅ | 🚧 | NEAR AI OAuth |
| DM pairing verification | ✅ | ✅ | ironclaw pairing approve, host APIs |
| Allowlist/blocklist | ✅ | 🚧 | allow_from + pairing store |
| Per-group tool policies | ✅ | ❌ | |
| Exec approvals | ✅ | ✅ | TUI overlay |
| TLS 1.3 minimum | ✅ | ✅ | reqwest rustls |
| SSRF protection | ✅ | ✅ | WASM allowlist |
| SSRF IPv6 transition bypass block | ✅ | ❌ | Block IPv4-mapped IPv6 bypasses |
| Cron webhook SSRF guard | ✅ | ❌ | SSRF checks on webhook delivery |
| Loopback-first | ✅ | 🚧 | HTTP binds 0.0.0.0 |
| Docker sandbox | ✅ | ✅ | Orchestrator/worker containers |
| Podman support | ✅ | ❌ | Alternative to Docker |
| WASM sandbox | ❌ | ✅ | IronClaw innovation |
| Sandbox env sanitization | ✅ | 🚧 | Shell tool scrubs env vars (secret detection); docker container env sanitization partial |
| Tool policies | ✅ | ✅ | |
| Elevated mode | ✅ | ❌ | |
| Safe bins allowlist | ✅ | ❌ | Hardened path trust |
| LD*/DYLD* validation | ✅ | ❌ | |
| Path traversal prevention | ✅ | ✅ | Including config includes (OC-06) |
| Credential theft via env injection | ✅ | 🚧 | Shell env scrubbing + command injection detection; no full OC-09 defense |
| Session file permissions (0o600) | ✅ | ✅ | Session token file set to 0o600 in llm/session.rs |
| Skill download path restriction | ✅ | ❌ | Prevent arbitrary write targets |
| Webhook signature verification | ✅ | ✅ | |
| Media URL validation | ✅ | ❌ | |
| Prompt injection defense | ✅ | ✅ | Pattern detection, sanitization |
| Leak detection | ✅ | ✅ | Secret exfiltration |
| Dangerous tool re-enable warning | ✅ | ❌ | Warn when gateway.tools.allow re-enables HTTP tools |

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
| Docker: Chromium + Xvfb | ✅ | ❌ | Optional browser in container |
| Docker: init scripts | ✅ | ❌ | /openclaw-init.d/ support |
| Browser: extraArgs config | ✅ | ❌ | Custom Chrome launch arguments |

### Owner: _Unassigned_

---

## Implementation Priorities

### P0 - Core (Already Done)
- ✅ TUI channel with approval overlays
- ✅ HTTP webhook channel
- ✅ DM pairing (ironclaw pairing list/approve, host APIs)
- ✅ WASM tool sandbox
- ✅ Workspace/memory with hybrid search + embeddings batching
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
- ✅ Skills system (prompt-based with trust gating, attenuation, activation criteria)
- ✅ Session file permissions (0o600)
- ✅ Memory CLI commands (search, read, write, tree, status)
- ✅ Shell env scrubbing + command injection detection
- ✅ Tinfoil private inference provider
- ✅ OpenAI-compatible / OpenRouter provider support

### P1 - High Priority
- ✅ Slack channel (Socket Mode + webhook, WASM)
- ✅ Telegram channel (WASM, DM pairing, caption, /start)
- ❌ WhatsApp channel
- ✅ Multi-provider failover (`FailoverProvider` with retryable error classification)
- ✅ Hooks system (core lifecycle hooks + bundled/plugin/workspace hooks + outbound webhooks)

### P2 - Medium Priority
- ❌ Media handling (images, PDFs)
- ✅ Ollama/local model support (via rig::providers::ollama)
- ❌ Configuration hot-reload
- ❌ Webhook trigger endpoint in web gateway
- ❌ Channel health monitor with auto-restart
- ❌ Partial output preservation on abort

### P3 - Lower Priority
- ❌ Discord channel
- ❌ Signal channel
- ❌ Matrix channel
- ❌ Other messaging platforms
- ❌ TTS/audio features
- ❌ Video support
- 🚧 Skills routing blocks (activation criteria exist, but no "Use when / Don't use when")
- ❌ Plugin registry
- ❌ Streaming (block/tool/Z.AI tool_stream)
- ❌ Memory: temporal decay, MMR re-ranking, query expansion
- ❌ Control UI i18n
- ❌ Stuck loop detection

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
3. **PostgreSQL + libSQL vs SQLite**: Dual-backend (production PG + embedded libSQL for zero-dep local mode)
4. **NEAR AI focus**: Primary provider with session-based auth
5. **No mobile/desktop apps**: Focus on server-side and CLI initially
6. **WASM channels**: Novel extension mechanism not in OpenClaw
7. **Tinfoil private inference**: IronClaw-only provider for private/encrypted inference
8. **GitHub WASM tool**: Native GitHub integration as WASM tool
9. **Prompt-based skills**: Different approach than OpenClaw capability bundles (trust gating, attenuation)

These are intentional architectural choices, not gaps to be filled.
