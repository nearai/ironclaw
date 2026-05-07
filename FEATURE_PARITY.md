# IronClaw ↔ OpenClaw Feature Parity Matrix

This document tracks feature parity between IronClaw (Rust implementation) and OpenClaw (TypeScript reference implementation). Use this to coordinate work across developers.

**Legend:**

- ✅ Implemented
- 🚧 Partial (in progress or incomplete)
- ❌ Not implemented
- 🔮 Planned (in scope but not started)
- 🚫 Out of scope (intentionally skipped)
- ➖ N/A (not applicable to Rust implementation)

**Last reviewed against OpenClaw PRs:** 2026-05-02 (merged 2026-03-11 through 2026-04-30, OpenClaw releases 2026.3.11 → 2026.4.30)

---

## 1. Architecture

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Hub-and-spoke architecture | ✅ | ✅ | Web gateway as central hub |
| WebSocket control plane | ✅ | ✅ | Gateway with WebSocket + SSE |
| Single-user system | ✅ | ✅ | Explicit instance owner scope for persistent routines, secrets, jobs, settings, extensions, and workspace memory |
| Multi-agent routing | ✅ | ❌ | Workspace isolation per-agent |
| Session-based messaging | ✅ | ✅ | Owner scope is separate from sender identity and conversation scope |
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
| Health check endpoints | ✅ | ✅ | /api/health + /api/gateway/status + /healthz + /readyz, with channel-backed readiness probes |
| `doctor` diagnostics | ✅ | 🚧 | 17 checks: settings, LLM, DB, embeddings, routines, gateway, MCP, skills, secrets, service, Docker daemon, Kubernetes cluster, tunnel binaries |
| Agent event broadcast | ✅ | 🚧 | SSE broadcast manager exists (SseManager) but tool/job-state events not fully wired |
| Channel health monitor | ✅ | ❌ | Auto-restart with configurable interval |
| Presence system | ✅ | ❌ | Beacons on connect, system presence for agents |
| Trusted-proxy auth mode | ✅ | ❌ | Header-based auth for reverse proxies; `trustedProxy.allowLoopback` for same-host reverse proxies |
| APNs push pipeline | ✅ | ❌ | Wake disconnected iOS nodes via push; iOS push relay with App Attest verification |
| Oversized payload guard | ✅ | 🚧 | HTTP webhook has 64KB body limit + Content-Length check; no chat.history cap |
| Pre-prompt context diagnostics | ✅ | 🚧 | Token breakdown logged before LLM call (conversational dispatcher path); other LLM entry points not yet covered |
| OpenAI-compat `/v1/models`, `/v1/embeddings` | ✅ | ❌ | Discovery + embeddings on top of `/v1/chat/completions` |
| Outbound proxy routing | ✅ | ❌ | `proxy.enabled` + `proxy.proxyUrl`/`OPENCLAW_PROXY_URL` with strict http forward-proxy validation, loopback bypass; `openclaw proxy validate` |
| Diagnostics export bundle | ✅ | ❌ | Sanitized logs/status/health/config/stability snapshots for bug reports |
| Startup diagnostics timeline | ✅ | ❌ | Opt-in lifecycle/plugin-load phase tracing |
| Event-loop readiness in `/readyz` | ✅ | ❌ | Event-loop delay (p99/max), utilization, CPU ratio, `degraded` flag |
| OpenTelemetry exporter pipeline | ✅ | ❌ | Bundled `diagnostics-otel` plugin: model-call, tool, exec, outbound, context-assembly, memory pressure, harness lifecycle spans/metrics; W3C traceparent propagation; signal-specific OTLP endpoints |
| Prometheus exporter | ✅ | ❌ | Bundled `diagnostics-prometheus` plugin with protected scrape route |
| Stability snapshots / payload-free liveness | ✅ | ❌ | Default-on stability recording, event-loop delay/CPU snapshots in stability bundles |

### Owner: _Unassigned_

---

## 3. Messaging Channels

| Channel | OpenClaw | IronClaw | Priority | Notes |
|---------|----------|----------|----------|-------|
| CLI/TUI | ✅ | ✅ | - | Ratatui-based TUI |
| HTTP webhook | ✅ | ✅ | - | axum with secret validation |
| REPL (simple) | ✅ | ✅ | - | For testing |
| WASM channels | ❌ | ✅ | - | IronClaw innovation; host resolves owner scope vs sender identity |
| WhatsApp | ✅ | ❌ | P1 | Baileys (Web), same-phone mode with echo detection |
| Telegram | ✅ | ✅ | - | WASM channel(MTProto), polling-first setup, DM pairing, caption, /start, bot_username, DM topics, web/UI ownership claim flow, owner-scoped persistence |
| Discord | ✅ | 🚧 | P2 | Gateway `MESSAGE_CREATE` intake restored via websocket queue + WASM poll; Gateway DMs now respect pairing; thread parent binding inheritance and reply/thread parity still incomplete |
| Signal | ✅ | ✅ | P2 | signal-cli daemonPC, SSE listener HTTP/JSON-R, user/group allowlists, DM pairing |
| Slack | ✅ | ✅ | - | WASM tool |
| iMessage | ✅ | ❌ | P3 | BlueBubbles or Linq recommended |
| Linq | ✅ | ❌ | P3 | Real iMessage via API, no Mac required |
| Feishu/Lark | ✅ | 🚧 | P3 | WASM channel with Event Subscription v2.0; Bitable/Docx tools planned |
| LINE | ✅ | ❌ | P3 | |
| WeChat (iLink bot) | ✅ | 🚧 | P2 | Extension-first channel (`channels-src/wechat`), single-account DM flow with QR login, typing, image send/receive, inbound file/voice/video handling, outbound image/video/file media, and SILK-to-WAV voice fallback; multi-account remains deferred |
| WebChat | ✅ | ✅ | - | Web gateway chat |
| Matrix | ✅ | ❌ | P3 | E2EE support |
| Mattermost | ✅ | ❌ | P3 | Emoji reactions, interactive buttons, model picker |
| Google Chat | ✅ | ❌ | P3 | |
| MS Teams | ✅ | ❌ | P3 | |
| Twitch | ✅ | ❌ | P3 | |
| Voice Call | ✅ | ❌ | P3 | Twilio/Telnyx/Plivo, stale call reaper, `voicecall setup`/`smoke`, `openclaw_agent_consult` realtime tool, agent-scoped voice agents, dedicated STT/TTS providers (Deepgram, ElevenLabs, Mistral, OpenAI/xAI realtime) |
| Google Meet | ✅ | ❌ | P3 | Bundled participant plugin: Google OAuth, explicit URL joins, Chrome+Twilio realtime transports, paired chrome-node support, attendance/artifact exports, calendar-backed exports, `googlemeet doctor` |
| Yuanbao (Tencent) | ✅ | ❌ | P3 | External plugin (`openclaw-plugin-yuanbao`) for WebSocket bot DMs and group chats |
| WeCom | ✅ | ❌ | P3 | Official external plugin pinned to npm release |
| Nostr | ✅ | ❌ | P3 | |

### Telegram-Specific Features (since Feb 2025)

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Forum topic creation | ✅ | ❌ | Create topics in forum groups; `message thread create` CLI; learns human topic names from service messages |
| channel_post support | ✅ | ❌ | Bot-to-bot communication |
| User message reactions | ✅ | ❌ | Surface inbound reactions |
| sendPoll | ✅ | ❌ | Poll creation via agent |
| Cron/heartbeat topic targeting | ✅ | ❌ | Messages land in correct topic; cron `--thread-id`, explicit `:topic:` precedence |
| DM topics support | ✅ | ❌ | Agent/topic bindings in DMs and agent-scoped SessionKeys |
| Persistent ACP topic binding | ✅ | ❌ | ACP harness sessions can pin to Telegram forum or DM topics |
| sendVoice (voice note replies) | ✅ | ✅ | audio/ogg attachments sent as voice notes; prerequisite for TTS (#90) |
| Native quote replies + retry | ✅ | ❌ | `reply_parameters.quote` with fallback when `QUOTE_TEXT_INVALID` |
| Polling stall watchdog + liveness | ✅ | ❌ | Configurable `pollingStallThresholdMs`, status/doctor warnings, dedicated `getUpdates` confirmation |
| HTML mode + chunking | ✅ | ❌ | Long HTML messages chunked, plain-text fallback |
| Photo dimension preflight | ✅ | ❌ | Falls back to document send when photo dims invalid |
| Webhook-mode setWebhook recovery | ✅ | ❌ | Retries `setWebhook` after recoverable network failures |

### Discord-Specific Features (since Feb 2025)

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Forwarded attachment downloads | ✅ | ❌ | Fetch media from forwarded messages |
| Faster reaction state machine | ✅ | ❌ | Watchdog + debounce |
| Thread parent binding inheritance | ✅ | ❌ | Threads inherit parent routing |
| Persistent components/forms across restarts | ✅ | ❌ | Active buttons/selects/forms keep working across Gateway restarts until expiry |
| `autoArchiveDuration` per-channel | ✅ | ❌ | 1h/1d/3d/1w archive duration for auto-created threads |
| Auto thread name generation | ✅ | ❌ | LLM-generated concise titles (`autoThreadName: "generated"`) |
| Voice channel responses | ✅ | ❌ | `channels.discord.voice.model` LLM override; voice mode auto-rejoin after RESUMED |
| CJK reply chunking | ✅ | ❌ | Splits long CJK replies at punctuation/code-point-safe boundaries |

### Slack-Specific Features (since Feb 2025)

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Streaming draft replies | ✅ | ❌ | Partial replies via draft message updates |
| Configurable stream modes | ✅ | ❌ | Per-channel stream behavior |
| Thread ownership | ✅ | 🚧 | Reply participation memory is restart-stable and TTL-bounded; once the bot joins a thread, follow-ups inherit channel visibility. Full thread-level ownership tracking is still missing |
| Download-file action | ✅ | ❌ | On-demand attachment downloads via message actions |
| App Home tab views | ✅ | ❌ | Default Home view on `app_home_opened`, included in setup manifests |
| Persistent thread participation | ✅ | ❌ | Bot-participated threads tracked across restarts |
| Block Kit limit hardening | ✅ | ❌ | Auto-truncates buttons/selects/values, drops oversized link URLs while preserving valid blocks |
| Socket Mode pong tuning | ✅ | ❌ | `clientPingTimeout`, `serverPingTimeout`, `pingPongLoggingEnabled` |
| Native model picker (`/models`) | ✅ | ❌ | Provider/model chooser via interactive components |

### Mattermost-Specific Features (since Mar 2026)

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Interactive buttons | ✅ | ❌ | Clickable message buttons with signed callback flow; slash callback validation hardened |
| Interactive model picker | ✅ | ❌ | In-channel provider/model chooser |
| `replyToMode` thread reply control | ✅ | ❌ | Top-level posts can start thread-scoped sessions; `all`/`first`/never modes |
| Streaming draft preview | ✅ | ❌ | Thinking, tool activity, partial reply text streamed into a single draft post |
| WebSocket ping/pong keepalives | ✅ | ❌ | Stale TCP drops reconnect instead of leaving monitoring idle |
| DM-vs-channel routing fixes | ✅ | ❌ | DM replies stay top-level; channel/group reply roots preserved |

### Feishu/Lark-Specific Features (since Mar 2026)

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Doc/table actions | ✅ | ❌ | `feishu_doc` supports tables, positional insert, color_text, image upload, and file upload |
| Rich-text embedded media extraction | ✅ | ❌ | Pull video/media attachments from post messages |
| Native interactive cards | ✅ | ❌ | Outgoing replies sent as native cards with clickable buttons |
| Schema 2.0 card action callbacks | ✅ | ❌ | Accept new `context.open_chat_id` shape |
| Streaming cards | ✅ | ❌ | Single live card per turn with throttled edits, topic-thread streaming |
| WebSocket retry/backoff | ✅ | ❌ | Monitor-owned reconnects after SDK retry exhaustion |
| Voice-note transcription | ✅ | ❌ | Inbound voice via shared media audio path |
| Bitable placeholder cleanup | ✅ | ❌ | Remove default-valued rows in create-app cleanup |

### QQBot-Specific Features (since Mar 2026)

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Engine architecture rewrite | ✅ | ❌ | Self-contained engine with QR onboarding, native `/bot-approve`, per-account resource stacks, credential backup/restore |
| Group chat full support | ✅ | ❌ | History tracking, @-mention gating, activation modes, per-group config, FIFO queue |
| C2C `stream_messages` | ✅ | ❌ | StreamingController lifecycle manager |
| Chunked media upload | ✅ | ❌ | Unified `sendMedia` for large files |

### BlueBubbles-Specific Features (since Mar 2026)

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Persistent inbound GUID dedupe | ✅ | ❌ | File-backed cache survives restart, 7-12x cron-duplicate fix |
| Catchup replay | ✅ | ❌ | Per-account cursor + `/api/v1/message/query?after=` pass on restart |
| Reply-context API fallback | ✅ | ❌ | Opt-in fetch for reply-context cache misses |
| TTS opus-in-CAF voice memos | ✅ | ❌ | Pre-transcoded native voice-memo bubbles via `tts.voice.preferAudioFileFormat` |
| Per-group `systemPrompt` injection | ✅ | ❌ | Group-specific behavioral instructions with `*` wildcard |
| Per-message catchup retry ceiling | ✅ | ❌ | `catchup.maxFailureRetries` to skip persistently failing messages |

### Channel Features

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| DM pairing codes | ✅ | ✅ | `ironclaw pairing list/approve`, host APIs |
| Allowlist/blocklist | ✅ | 🚧 | `allow_from` + pairing store + hardened command/group allowlists |
| Self-message bypass | ✅ | ❌ | Own messages skip pairing |
| Mention-based activation | ✅ | ✅ | bot_username + respond_to_all_group_messages |
| Per-group tool policies | ✅ | ❌ | Allow/deny specific tools |
| Thread isolation | ✅ | ✅ | Separate sessions per thread/topic |
| Per-channel media limits | ✅ | 🚧 | Caption support plus `mediaMaxMb` enforcement for WhatsApp, Telegram, and Discord |
| Typing indicators | ✅ | 🚧 | TUI + channel typing, with configurable silence timeout; richer parity pending |
| Per-channel ackReaction config | ✅ | ❌ | Customizable acknowledgement reactions/scopes |
| Group session priming | ✅ | ❌ | Member roster injected for context |
| Sender_id in trusted metadata | ✅ | ❌ | Exposed in system metadata |
| Per-group `systemPrompt` injection | ✅ | ❌ | Per-group/per-direct system prompts injected via `GroupSystemPrompt` (Telegram, Discord, WhatsApp, BlueBubbles) |
| Visible reply enforcement | ✅ | ❌ | `messages.visibleReplies` requires output via `message(action=send)`; group-scope override available |
| Active-run steering queue | ✅ | ❌ | `messages.queue` `steer` mode (default) drains queued messages at next model boundary; `queue` legacy one-at-a-time |
| Tool-progress streaming into previews | ✅ | ❌ | Tool progress shown in live preview edits (Discord/Slack/Telegram/Mattermost/Matrix) |
| `dmPolicy="open"` semantics | ✅ | 🚧 | Public open-DM only with effective wildcard; pairing-store senders no longer count for DM audits (OpenClaw fixed across all channels) |

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
| `config` | ✅ | ✅ | - | Read/write config plus validate/path helpers |
| `backup` | ✅ | ❌ | P3 | Create/verify local backup archives |
| `channels` | ✅ | 🚧 | P2 | `list` implemented; `enable`/`disable`/`status` deferred pending config source unification |
| `models` | ✅ | 🚧 | P1 | `models list [<provider>]` (`--verbose`, `--json`; fetches live model list when provider specified), `models status` (`--json`), `models set <model>`, `models set-provider <provider> [--model model]` (alias normalization, config.toml + .env persistence). Remaining: `set` doesn't validate model against live list. |
| `status` | ✅ | ✅ | - | System status (enriched session details) |
| `agents` | ✅ | ❌ | P3 | Multi-agent management |
| `sessions` | ✅ | ❌ | P3 | Session listing (shows subagent models) |
| `memory` | ✅ | ✅ | - | Memory search CLI |
| `skills` | ✅ | ✅ | - | CLI subcommands (list, search, info) + agent tools + web API endpoints |
| `pairing` | ✅ | ✅ | - | list/approve, account selector |
| `nodes` | ✅ | ❌ | P3 | Device management, remove/clear flows |
| `plugins` | ✅ | ❌ | P3 | Plugin management |
| `hooks` | ✅ | ✅ | P2 | `hooks list` (bundled + plugin discovery, `--verbose`, `--json`) |
| `cron` | ✅ | 🚧 | P2 | list/create/edit/enable/disable/delete/history; TODO: `cron run`, model/thinking fields |
| `webhooks` | ✅ | ❌ | P3 | Webhook config |
| `message send` | ✅ | ❌ | P2 | Send to channels |
| `browser` | ✅ | ❌ | P3 | Browser automation |
| `sandbox` | ✅ | ✅ | - | WASM sandbox |
| `doctor` | ✅ | 🚧 | P2 | 16 subsystem checks |
| `logs` | ✅ | 🚧 | P3 | `logs` (gateway.log tail), `--follow` (SSE live stream), `--level` (get/set). No DB-persisted log history. |
| `update` | ✅ | ❌ | P3 | Self-update; `OPENCLAW_NO_AUTO_UPDATE=1` kill-switch |
| `completion` | ✅ | ✅ | - | Shell completion |
| `migrate` | ✅ | ❌ | P3 | Bundled importers for Claude Code, Claude Desktop, Hermes (config, MCP servers, skills, command prompts, model providers, credentials) |
| `proxy validate` | ✅ | ❌ | P3 | Verify effective proxy config, reachability, allow/deny destinations |
| `plugins registry` | ✅ | ❌ | P3 | Inspect persisted plugin registry; `--refresh` repair |
| `plugins deps` | ✅ | ❌ | P3 | Inspect/repair bundled plugin runtime dependencies |
| `infer model run --gateway` | ✅ | ❌ | P3 | Raw model probes via Gateway; image `--file` + `--prompt` + `--timeout-ms` overrides |
| `infer image describe`/`describe-many` | ✅ | ❌ | P3 | Custom vision prompts/timeouts |
| `qa` (suite/telegram/credentials) | ✅ | ❌ | P3 | QA Lab CI runner with `--allow-failures` opt-out |
| `voicecall setup`/`smoke`/`continue` | ✅ | ❌ | P3 | Voice call provider readiness, dry-run smoke, gateway-delegated continue |
| `googlemeet doctor`/`recover-tab` | ✅ | ❌ | P3 | Meet OAuth/browser-state diagnostics, tab recovery |
| `matrix verify`/`encryption setup` | ✅ | ❌ | P3 | E2EE setup, recovery key rotation, cross-signing trust |
| `nodes remove` | ✅ | ❌ | P3 | Remove stale gateway-owned node pairing records |
| `nodes list` (paired view) | ✅ | ❌ | P3 | Default paired-node view with pending fallback |
| `cron run` / `cron edit --thread-id` | ✅ | 🚧 | P2 | Already partial; OpenClaw added cron stagger, finished-run webhook, `--failure-alert-include-skipped` |
| `sessions export-trajectory` | ✅ | ❌ | P3 | Per-run trajectory bundles with redacted transcripts/runtime events/prompts |
| `/subagents spawn` | ✅ | ❌ | P3 | Spawn subagents from chat |
| `/export-session` | ✅ | ❌ | P3 | Export current session transcript |
| `/export-trajectory` (chat) | ✅ | ❌ | P3 | Per-run exec-approved trajectory bundle, owner-only delivery |
| `/diagnostics` (owner-only) | ✅ | ❌ | P3 | Owner-only diagnostics export with sensitive-data preamble |
| `/codex computer-use status/install` | ✅ | ❌ | P3 | Codex desktop control setup with marketplace discovery |
| `/dock-*` route switches | ✅ | ❌ | P3 | Switch active session reply route through `session.identityLinks` |
| `--container` / `OPENCLAW_CONTAINER` | ✅ | ❌ | P3 | Run CLI commands inside running Docker/Podman container |

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
| Session pruning | ✅ | ❌ | Auto cleanup old sessions; oversized `sessions.json` rotation removed; entry/age caps enforced at load |
| Context compaction | ✅ | ✅ | Auto summarization |
| Compaction model override | ✅ | ❌ | Use a dedicated provider/model for summarization only; `agents.defaults.compaction.memoryFlush.model` exact override |
| Compaction mid-turn precheck | ✅ | ❌ | `agents.defaults.compaction.midTurnPrecheck` triggers before next tool call instead of end-of-turn |
| Post-compaction read audit | ✅ | ❌ | Layer 3: workspace rules appended to summaries |
| Post-compaction context injection | ✅ | ❌ | Workspace context as system event |
| Compaction start/end notices | ✅ | ❌ | Opt-in lifecycle notices during compaction |
| Custom system prompts | ✅ | ✅ | Template variables, safety guardrails |
| Skills (modular capabilities) | ✅ | ✅ | Prompt-based skills with trust gating, attenuation, activation criteria, catalog, selector |
| Skill Workshop plugin | ✅ | ❌ | Captures reusable workflow corrections as pending or auto-applied workspace skills, threshold-based reviewer |
| Grouped skill directories | ✅ | ✅ | `skills/<group>/<skill>/SKILL.md` discovery |
| Skill installer metadata | ✅ | ❌ | One-click install recipes (npm/pip), API key entry, source metadata |
| Skill routing blocks | ✅ | 🚧 | ActivationCriteria (keywords, patterns, tags) but no "Use when / Don't use when" blocks |
| Skill path compaction | ✅ | ❌ | ~ prefix to reduce prompt tokens |
| Thinking modes (off/minimal/low/medium/high/xhigh/adaptive/max) | ✅ | 🚧 | thinkingConfig for Gemini models; no per-level control yet; Anthropic Opus 4.7 `xhigh`+`adaptive`+`max`; DeepSeek V4 `xhigh`/`max` |
| Per-model thinkingDefault override | ✅ | ❌ | Override thinking level per model; Anthropic Claude 4.6/4.7 defaults to adaptive |
| Adaptive→provider thinking maps | ✅ | ❌ | `/think adaptive` maps to Gemini dynamic thinking, Anthropic adaptive, OpenAI flex |
| Native Codex app-server runtime | ✅ | ➖ | New embedded Codex harness with PreToolUse/PostToolUse/PermissionRequest relay; replaces ACP for `codex/*` models |
| Codex Computer Use | ✅ | ❌ | Desktop control setup with marketplace discovery, fail-closed MCP checks |
| Codex hooks bridge | ✅ | ❌ | Codex-native tool hooks → OpenClaw plugin hooks/approvals |
| Codex sub-agent metadata | ✅ | ❌ | Native Codex sub-agent session metadata without nested gateway patch |
| Codex context-engine integration | ✅ | ❌ | Bootstrap, assembly, post-turn maintenance, engine-owned compaction in Codex sessions |
| Active Memory plugin | ✅ | ❌ | Dedicated memory sub-agent right before main reply; configurable message/recent/full context modes; partial-recall on timeout; per-conversation `allowedChatIds`/`deniedChatIds` filters |
| Inferred follow-up commitments | ✅ | ❌ | Opt-in hidden batched extraction with per-agent/per-channel scoping, heartbeat delivery, CLI management; `commitments.enabled`/`maxPerDay` |
| `sessions_yield` | ✅ | ❌ | Orchestrators end current turn immediately, skip queued tool work, carry hidden follow-up payload to next turn |
| Subagent forked context | ✅ | ❌ | Optional inherit-requester-transcript for native `sessions_spawn` |
| `agents.defaults.contextInjection: "never"` | ✅ | ❌ | Disable workspace bootstrap injection per-agent |
| `agents.defaults.experimental.localModelLean` | ✅ | ❌ | Drop heavyweight default tools for weaker local models |
| `agents.files.get/set` workspace tools | ✅ | ❌ | Realpath-via-fd safety, symlink alias rejection |
| Trajectory export | ✅ | ❌ | Default-on local trajectory capture; `/export-trajectory` bundles with redacted transcripts/events/artifacts |
| Block-level streaming | ✅ | ❌ | |
| Tool-level streaming | ✅ | ❌ | |
| Z.AI tool_stream | ✅ | ❌ | Real-time tool call streaming |
| Plugin tools | ✅ | ✅ | WASM tools |
| Tool policies (allow/deny) | ✅ | ✅ | |
| Exec approvals (`/approve`) | ✅ | ✅ | TUI approval overlay |
| Tool inventory cache | ✅ | ❌ | Coalesced effective-tool inventory cache with channel-registry invalidation |
| Pending exec approval `errorMessage` cleanup | ✅ | ❌ | Failed restart-interrupted approval-pending sessions instead of replaying stale ids |
| Elevated mode | ✅ | ❌ | Privileged execution |
| Subagent support | ✅ | ✅ | Task framework; spawn-by-account-aware bindings, model overrides preserved |
| `/subagents spawn` command | ✅ | ❌ | Spawn from chat |
| Auth profiles | ✅ | ❌ | Multiple auth strategies; replaceDefaultModels migration semantics |
| Generic API key rotation | ✅ | ❌ | Rotate keys across providers |
| Stuck loop detection | ✅ | ❌ | Exponential backoff on stuck agent loops; unknown-tool guard default-on |
| llms.txt discovery | ✅ | ❌ | Auto-discover site metadata |
| Multiple images per tool call | ✅ | ❌ | Single tool call, multiple images |
| URL allowlist (web_search/fetch) | ✅ | ❌ | Restrict web tool targets |
| suppressToolErrors config | ✅ | ❌ | Hide tool errors from user |
| Intent-first tool display | ✅ | ❌ | Details and exec summaries |
| Transcript file size in status | ✅ | ❌ | Show size in session status |
| Stuck-session recovery | ✅ | ❌ | Conservative recovery releases stale lanes while preserving active embedded runs/replies |
| `Runner:` in `/status` | ✅ | ❌ | Reports embedded Pi/CLI-backed/ACP harness in session status |
| Voice Wake routing | ✅ | ❌ | Wake phrases can target named agent or session via gateway routing APIs |

### Owner: _Unassigned_

---

## 6. Model & Provider Support

| Provider | OpenClaw | IronClaw | Priority | Notes |
|----------|----------|----------|----------|-------|
| NEAR AI | ✅ | ✅ | - | Primary provider |
| Anthropic (Claude) | ✅ | 🚧 | - | Via NEAR AI proxy; Opus 4.7 (default, adaptive+xhigh+max), Opus 4.6, Sonnet 4.6 |
| OpenAI | ✅ | 🚧 | - | Via NEAR AI proxy; GPT-5.5 default, GPT-5.4-pro forward-compat, Codex OAuth, Responses API; image generation (`gpt-image-2`) via Codex OAuth |
| OpenAI Codex (native app-server) | ✅ | ➖ | - | App-server >=0.125.0 with native MCP hooks, dynamic tools, approval relay |
| AWS Bedrock | ✅ | ✅ | - | Native Converse API; Claude Opus 4.7 thinking profile (xhigh/adaptive/max); IAM bearer token refresh for Mantle |
| Google Gemini | ✅ | ✅ | - | OAuth (PKCE + S256), function calling, thinkingConfig, generationConfig; TTS (`gemini-embedding-2-preview`); image gen native API; ADC-backed Vertex |
| Google Gemini Live (realtime) | ✅ | ❌ | - | Realtime voice provider for Voice Call/Google Meet, bidirectional audio + function calls |
| io.net | ✅ | ✅ | P3 | Via `ionet` adapter |
| Mistral | ✅ | ✅ | P3 | Via `mistral` adapter; Voice Call streaming STT |
| Yandex AI Studio | ✅ | ✅ | P3 | Via `yandex` adapter |
| Cloudflare Workers AI | ✅ | ✅ | P3 | Via `cloudflare` adapter |
| NVIDIA API | ✅ | ✅ | P3 | Via `nvidia` adapter; OpenClaw added bundled provider with API-key onboarding, static catalog, literal model-ref picker, NIM string-content compat |
| OpenRouter | ✅ | ✅ | - | Via OpenAI-compatible provider; OpenClaw added native video generation, `openrouter:auto`/`openrouter:free` aliases, Hunter/Healer Alpha, free-model fallback for `models scan` |
| Tinfoil | ❌ | ✅ | - | Private inference provider (IronClaw-only) |
| OpenAI-compatible | ❌ | ✅ | - | Generic OpenAI-compatible endpoint (RigAdapter); OpenAI-style image inputs default missing `image_url.detail` to `auto` |
| GitHub Copilot | ✅ | ✅ | - | Dedicated provider with OAuth token exchange; default Opus model is `claude-opus-4.7`; GUI/RPC wizard device-code auth; `gpt-5.4` xhigh thinking |
| Ollama (local) | ✅ | ✅ | - | OpenClaw added Cloud + Local + cloud-only modes, browser sign-in, signed `/api/experimental/web_search`, `params.num_ctx`/`params.think`/`params.keep_alive`, `/api/show` capability detection |
| Perplexity | ✅ | ❌ | P3 | Freshness parameter for web_search |
| MiniMax | ✅ | ❌ | P3 | Regional endpoint selection; portal OAuth + Token Plan + `MINIMAX_API_KEY`; image-01, music-2.6, video; `MiniMax-VL-01` for vision |
| GLM-5 | ✅ | ✅ | P3 | Via Z.AI provider (`zai`) using OpenAI-compatible chat completions |
| Tencent Cloud (TokenHub) | ✅ | ❌ | P3 | Bundled provider; Hy3 catalog with tiered pricing |
| DeepInfra | ✅ | ❌ | P3 | Bundled provider with `DEEPINFRA_API_KEY`, dynamic OpenAI-compatible discovery, image gen/edit, image/audio understanding, TTS, text-to-video, embeddings |
| Cerebras | ✅ | ❌ | P3 | Bundled plugin with onboarding, static catalog, manifest endpoint metadata |
| Z.AI / GLM-5 | ✅ | ✅ | - | OpenClaw added bundled GLM catalog/auth in plugin manifest, `params.preserveThinking` for `reasoning_content` replay |
| Qwen / Model Studio | ✅ | ❌ | P3 | Standard DashScope endpoints (CN + global) + Coding Plan; vLLM Qwen thinking controls |
| DeepSeek | ✅ | ❌ | P3 | V4 Pro/V4 Flash bundled, V4 Flash onboarding default, native `xhigh`/`max` thinking levels, `reasoning_content` replay support |
| Moonshot / Kimi | ✅ | ❌ | P3 | Kimi K2.6 default; native Anthropic-format tool calls; CN API endpoint support; `kimi-coding` web search via `KIMI_API_KEY` |
| xAI | ✅ | ❌ | P3 | Image gen (`grok-imagine-image`/`pro`), reference-image edits, six TTS voices (MP3/WAV/PCM/G.711), `grok-stt` audio transcription, realtime STT for Voice Call |
| Tencent Yuanbao | ✅ | ❌ | P3 | External plugin (`openclaw-plugin-yuanbao`) for chat |
| Vercel AI Gateway | ✅ | ❌ | P3 | Provider-owned thinking levels for trusted upstream refs |
| Codex/OpenAI image generation | ✅ | ❌ | P2 | `gpt-image-2`/`gpt-image-1.5` via Codex OAuth or API key; multipart reference-image edits; Azure deployment-scoped image URLs |
| OpenRouter image/video generation | ✅ | ❌ | P3 | Image gen + reference edits; native video generation through `video_generate` |
| MiniMax music/video | ✅ | ❌ | P3 | `music-2.6`, `video_generate`, `MiniMax-portal` registration |
| Google Veo (video gen) | ✅ | ❌ | P3 | Direct MLDev `video.uri` downloads; REST `predictLongRunning` fallback |
| fal Seedance 2.0 | ✅ | ❌ | P3 | Reference-to-video with multi-image/video/audio input |
| Comfy (image/video/music) | ✅ | ❌ | P3 | `plugins.entries.comfy.config` workflow + cloud auth |
| node-llama-cpp | ✅ | ➖ | - | OpenClaw made it optional (no longer auto-installed); local embeddings now opt-in |
| llama.cpp (native) | ❌ | 🔮 | P3 | Rust bindings |

### Model Features

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Auto-discovery | ✅ | ❌ | Manifest-backed `modelCatalog` with aliases/suppressions; cold installed-index fast path |
| Failover chains | ✅ | ✅ | `FailoverProvider` with configurable `fallback_model` |
| Cooldown management | ✅ | ✅ | Lock-free per-provider cooldown in `FailoverProvider` |
| Per-session model override | ✅ | ✅ | Model selector in TUI |
| Model selection UI | ✅ | ✅ | TUI keyboard shortcut; OpenClaw added Quick Settings, mobile-aware picker |
| Per-model thinkingDefault | ✅ | ❌ | Override thinking level per model in config |
| 1M context support | ✅ | ❌ | Anthropic extended context beta + OpenAI Codex GPT-5.4 1M context; Claude Opus 4.7 + claude-cli normalized to 1M |
| Fast mode (`/fast`) | ✅ | ❌ | Anthropic `service_tier` + OpenAI `gpt-5.4-fast`; `/fast` toggle, TUI/Control UI/ACP, per-model defaults |
| Tiered model pricing | ✅ | ❌ | Pricing tiers from cached catalogs (Moonshot Kimi K2.6/K2.5, Hy3) for usage reports |
| `models scan` (free-model fallback) | ✅ | ❌ | Public OpenRouter free-model metadata when no `OPENROUTER_API_KEY` |
| Model catalog stale cache fallback | ✅ | ❌ | Serve last successful catalog while stale reloads refresh in background |
| `models.pricing.enabled` | ✅ | ❌ | Skip startup OpenRouter/LiteLLM pricing-catalog fetches for offline installs |
| Auth status card | ✅ | ❌ | OAuth token health + provider rate-limit pressure with `models.authStatus` RPC |
| Model fallback metadata | ✅ | ❌ | `model.fallback_step` trajectory events with from/to + chain position + final outcome |
| `prompt_cache_key` opt-in | ✅ | ❌ | `compat.supportsPromptCacheKey` per-provider opt-in |
| Replay normalization | ✅ | ❌ | Repair displaced/missing tool results, Anthropic/Bedrock thinking signature stripping, OpenAI Responses orphaned reasoning, Codex aborted-output replay |

### TTS / STT / Realtime Voice

| Feature | OpenClaw | IronClaw | Priority | Notes |
|---------|----------|----------|----------|-------|
| TTS (Microsoft / Edge) | ✅ | ❌ | P3 | Auto-enabled bundled provider; legacy `messages.tts.providers.edge` voices |
| TTS (OpenAI) | ✅ | ❌ | P3 | OpenAI-compatible `/audio/speech` |
| TTS (ElevenLabs v3) | ✅ | ❌ | P3 | `eleven_v3` model surfaced; PCM telephony |
| TTS (Google Gemini) | ✅ | ❌ | P3 | `audioProfile` + `speakerName` prompt control; PCM-to-Opus voice notes |
| TTS (Azure Speech) | ✅ | ❌ | P3 | Bundled provider, Speech-resource auth, SSML, native Ogg/Opus |
| TTS (Inworld) | ✅ | ❌ | P3 | Streaming synthesis, voice-note + PCM telephony |
| TTS (Volcengine/BytePlus Seed Speech) | ✅ | ❌ | P3 | Bundled provider, Ogg/Opus voice notes, MP3 file output |
| TTS (Xiaomi MiMo) | ✅ | ❌ | P3 | MP3/WAV + voice-note Opus transcoding |
| TTS (Local CLI) | ✅ | ❌ | P3 | Bundled local command speech provider with file/stdout/Opus/PCM |
| TTS (Gradium) | ✅ | ❌ | P3 | Bundled TTS provider with voice-note + telephony output |
| TTS (OpenRouter) | ✅ | ❌ | P3 | OpenAI-compatible `/audio/speech` via `OPENROUTER_API_KEY` |
| TTS (xAI) | ✅ | ❌ | P3 | Six grok voices, MP3/WAV/PCM/G.711 |
| TTS (DeepInfra) | ✅ | ❌ | P3 | Bundled provider |
| TTS (MiniMax) | ✅ | ❌ | P3 | Portal OAuth + Token Plan; HD model ids |
| TTS (Tinfoil/local MLX) | ✅ | ❌ | P3 | macOS Talk experimental MLX provider |
| TTS personas | ✅ | ❌ | P3 | Provider-aware personas with deterministic provider binding, `/tts persona`, Gemini `audio-profile-v1`, OpenAI instructions |
| Auto-TTS controls | ✅ | ❌ | P3 | `/tts latest`, `/tts chat on\|off\|default`; per-account/per-agent overrides |
| Talk Mode (browser realtime) | ✅ | ❌ | P3 | OpenAI Realtime + Google Live WebRTC/WS; ephemeral client secrets; `openclaw_agent_consult` handoff |
| STT (OpenAI Realtime) | ✅ | ❌ | P3 | Voice Call streaming transcription |
| STT (xAI realtime) | ✅ | ❌ | P3 | Voice Call streaming via `grok-stt` |
| STT (Deepgram) | ✅ | ❌ | P3 | Voice Call streaming |
| STT (ElevenLabs Scribe v2) | ✅ | ❌ | P3 | Batch + streaming inbound transcription |
| STT (Mistral) | ✅ | ❌ | P3 | Voice Call streaming |
| STT (SenseAudio) | ✅ | ❌ | P3 | Bundled batch audio transcription via `tools.media.audio` |
| STT (local Whisper CLI) | ✅ | ❌ | P3 | Configured/key-backed STT preferred over auto-detected local Whisper |

### Owner: _Unassigned_

---

## 7. Media Handling

| Feature | OpenClaw | IronClaw | Priority | Notes |
|---------|----------|----------|----------|-------|
| Image processing (Sharp) | ✅ | ❌ | P2 | Resize, format convert |
| Configurable image resize dims | ✅ | ❌ | P2 | Per-agent dimension config |
| Multiple images per tool call | ✅ | ❌ | P2 | Single tool invocation, multiple images |
| Audio transcription | ✅ | ❌ | P2 | Multiple providers (see TTS/STT subsection in Section 6) |
| Video support | ✅ | ❌ | P3 | OpenRouter native video gen, MiniMax video, Google Veo, fal Seedance, OpenAI Sora |
| PDF analysis tool | ✅ | ❌ | P2 | Native Anthropic/Gemini path with text/image extraction fallback; bundled `document-extract` plugin owns `pdfjs-dist` |
| PDF parsing | ✅ | 🚧 | P2 | Uploaded document attachments parse via `pdf-extract`; no `pdfjs-dist` fallback path |
| MIME detection | ✅ | ❌ | P2 | Bounded MIME sniff + ZIP archive preflight |
| Media caching | ✅ | ❌ | P3 | |
| Vision model integration | ✅ | ❌ | P2 | Image understanding; `agents.defaults.imageModel`, Codex app-server image turns, configured-provider exact match |
| Image generation | ✅ | ❌ | P2 | OpenAI `gpt-image-2` / `gpt-image-1.5`, OpenRouter, Gemini, MiniMax `image-01`; quality + format + background hints |
| Music generation | ✅ | ❌ | P3 | MiniMax `music-2.6`, fal, video-to-music workflows |
| Multimodal memory indexing | ✅ | ❌ | P3 | Image + audio indexing for `memorySearch.extraPaths` via Gemini `gemini-embedding-2-preview` |
| Audio-as-voice routing | ✅ | ❌ | P2 | `[[audio_as_voice]]` directives on text tool-result `MEDIA:` payloads |
| TTS providers | ✅ | ❌ | P2 | See TTS/STT subsection in Section 6 |
| Incremental TTS playback | ✅ | ❌ | P3 | iOS progressive playback |
| Sticker-to-image | ✅ | ❌ | P3 | Telegram stickers |
| Per-channel media limits | ✅ | 🚧 | P2 | `mediaMaxMb` enforcement (already in Section 3); Signal `getAttachment` honors `mediaMaxMb` with base64 headroom |

### Owner: _Unassigned_

---

## 8. Plugin & Extension System

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Dynamic loading | ✅ | ✅ | WASM modules |
| Manifest validation | ✅ | ✅ | WASM metadata; `modelCatalog`, `channelConfigs`, `setup.providers`, `setup.requiresRuntime`, `activation.onStartup` contracts |
| HTTP path registration | ✅ | ❌ | Plugin routes |
| Workspace-relative install | ✅ | ✅ | ~/.ironclaw/tools/ |
| Channel plugins | ✅ | ✅ | WASM channels |
| Auth plugins | ✅ | ❌ | |
| Memory plugins | ✅ | ❌ | Custom backends + selectable memory slot |
| Context-engine plugins | ✅ | ❌ | Custom context management + subagent/context hooks; `info.id` slot match enforced |
| Tool plugins | ✅ | ✅ | WASM tools |
| Hook plugins | ✅ | ✅ | Declarative hooks from extension capabilities |
| Provider plugins | ✅ | ❌ | Manifest-backed catalogs/aliases/suppressions; setup auth metadata |
| Plugin CLI (`install`, `list`) | ✅ | ✅ | `tool` subcommand |
| ClawHub registry | ✅ | ❌ | Discovery; install scope `--profile`, `npm:` install prefix to skip ClawHub lookup, `clawhub:` install records |
| `git:` plugin installs | ✅ | ❌ | First-class `git:` install with ref checkout, commit metadata, `plugins update` for git sources |
| `before_agent_start` hook | ✅ | ❌ | modelOverride/providerOverride support |
| `before_agent_finalize` hook | ✅ | ❌ | New finalize hook with run/message/sender/session/trace correlation |
| `before_message_write` hook | ✅ | ❌ | Pre-write message interception |
| `before_dispatch` hook | ✅ | ❌ | Canonical inbound metadata; route handled replies through normal final delivery |
| `before_compaction`/`after_compaction` hooks | ✅ | ❌ | Codex-native compaction lifecycle |
| `llm_input`/`llm_output` hooks | ✅ | ❌ | LLM payload inspection (Codex app-server included) |
| `model_call_started`/`ended` hooks | ✅ | ❌ | Metadata-only, no prompts/responses/headers/raw provider request IDs |
| `cron_changed` hook | ✅ | ❌ | Typed cron lifecycle observer |
| `gateway_start` hook context | ✅ | ❌ | Startup config, workspace dir, live cron getter |
| `agent_end` observation hooks | ✅ | ❌ | 30s timeout for non-settling hooks |
| Plugin SDK state store | ✅ | ❌ | SQLite-backed `api.runtime.state.openKeyedStore` for restart-safe keyed registries with TTL/eviction |
| Plugin SDK Codex extensions | ✅ | ❌ | Async `tool_result` middleware, `after_tool_call` for Codex tool runs |
| Persisted plugin registry | ✅ | ❌ | Cold registry index, `openclaw plugins registry` inspection, `--refresh` repair |
| `plugins deps --repair` | ✅ | ❌ | Bundled runtime-deps inspect + repair without rerunning plugin runtime |
| Plugin install conflict-aware writes | ✅ | ❌ | Install/uninstall config writes are conflict-aware; managed plugin files removed only after config commit |
| Plugin compatibility registry | ✅ | ❌ | Central deprecation registry with dated owners + replacements + 3-month removal targets |
| Layered runtime-deps roots | ✅ | ❌ | `OPENCLAW_PLUGIN_STAGE_DIR` resolves read-only preinstalled deps before installing missing deps |
| Bundled provider catalogs in manifest | ✅ | ❌ | DeepInfra, Cerebras, Mistral, Moonshot, DeepSeek, Tencent, StepFun, Venice, Fireworks, Together, Groq, Qianfan, Xiaomi, BytePlus, Volcano Engine, NVIDIA |

### Owner: _Unassigned_

---

## 9. Configuration System

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Primary config file | ✅ `~/.openclaw/openclaw.json` | ✅ `.env` | Different formats |
| JSON5 support | ✅ | ❌ | Comments, trailing commas |
| YAML alternative | ✅ | ❌ | |
| Environment variable interpolation | ✅ | ✅ | `${VAR}` |
| Config validation/schema | ✅ | ✅ | Type-safe Config struct + `openclaw config validate`; OpenClaw added top-3 issue surface for `config.set/patch/apply` |
| Hot-reload | ✅ | ❌ | Many plugins now re-read live runtime config (memory-lancedb, active-memory, github-copilot, ollama, openai, amazon-bedrock, codex, skill-workshop, diffs, gateway-tool); `OPENCLAW_NO_AUTO_UPDATE=1` kill-switch |
| Legacy migration | ✅ | ➖ | OpenClaw dropped automatic migrations older than two months |
| State directory | ✅ `~/.openclaw-state/` | ✅ `~/.ironclaw/` | |
| Credentials directory | ✅ | ✅ | Session files |
| Full model compat fields in schema | ✅ | ❌ | pi-ai model compat exposed in config |
| `models.pricing.enabled` | ✅ | ❌ | Skip OpenRouter/LiteLLM pricing fetches for offline installs |
| `agents.list[].contextTokens` | ✅ | ❌ | Per-agent context window override |
| `gateway.handshakeTimeoutMs` | ✅ | ❌ | Tunable WebSocket pre-auth handshake budget |
| `--profile <name>` | ✅ | ❌ | Plugin install destinations resolve from active profile state dir |
| Config recovery on clobber | ✅ | ❌ | Restore last-known-good config on critical clobber signatures (missing metadata, missing `gateway.mode`, sharp size drops); foreground/service notices include rejected paths |
| Modular `$include` files | ✅ | ❌ | Single-file top-level includes for isolated mutations; `plugins install`/`update` updates `plugins.json5` instead of flattening |
| `config set --merge`/`--replace` | ✅ | ❌ | Additive vs intentional clobber for provider model maps |
| Wrapper-based service install | ✅ | ❌ | `--wrapper`/`OPENCLAW_WRAPPER` validated executable LaunchAgent/systemd wrappers |

### Owner: _Unassigned_

---

## 10. Memory & Knowledge System

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Vector memory | ✅ | ✅ | pgvector |
| Session-based memory | ✅ | ✅ | |
| Hybrid search (BM25 + vector) | ✅ | ✅ | RRF algorithm; vectorScore + textScore exposed alongside combined score |
| Temporal decay (hybrid search) | ✅ | ❌ | Opt-in time-based scoring factor |
| MMR re-ranking | ✅ | ❌ | Maximal marginal relevance for result diversity |
| LLM-based query expansion | ✅ | ❌ | Expand FTS queries via LLM |
| OpenAI embeddings | ✅ | ✅ | |
| Bedrock embeddings | ❌ | ✅ | Reuses Bedrock region/profile auth for Titan Text Embeddings V2 |
| Gemini embeddings | ✅ | ❌ | `gemini-embedding-2-preview` with configurable output dimensions, automatic reindex on dim change |
| GitHub Copilot embeddings | ✅ | ❌ | Provider with token refresh, payload validation, remote overrides |
| Ollama embeddings | ✅ | ✅ | OpenClaw moved to `/api/embed` with batched `input`; per-host cache keys; non-batch concurrency knob |
| Local embeddings | ✅ | ❌ | `node-llama-cpp` now optional install |
| Asymmetric embedding endpoints | ✅ | ❌ | `inputType`/`queryInputType`/`documentInputType` for retrieval prefixes (Ollama: `nomic-embed-text`, `qwen3-embedding`, `mxbai-embed-large`) |
| SQLite-vec backend | ✅ | ❌ | IronClaw uses PostgreSQL; bundled-plugin runtime-deps mirror sqlite-vec |
| LanceDB backend | ✅ | ❌ | Configurable auto-capture max length; cloud storage support; OpenAI-compatible float embeddings, ZhiPu/DashScope normalization |
| QMD backend | ✅ | ❌ | Multi-collection `-c` filters, `--mask` collection patterns, opt-in `memory.qmd.update.startup` |
| Active Memory plugin | ✅ | ❌ | Memory sub-agent before main reply; partial recall on timeout; `allowedChatIds`/`deniedChatIds`; visible status fields |
| Memory wiki (people-aware) | ✅ | ❌ | Canonical aliases, person cards, relationship graphs, privacy/provenance reports, search modes (find-person/route-question/source-evidence/raw-claim) |
| Dreaming (REM cycles) | ✅ | ❌ | `## Light Sleep`/`## REM Sleep` phase blocks; `dreaming.storage.mode = "separate"` default; `dreaming.model` override |
| `recallMaxChars` cap | ✅ | ❌ | Bound recall embedding queries for small Ollama embedding models |
| `corpus=sessions` ranking | ✅ | ❌ | Session transcript hits with visibility/agent-to-agent policy |
| Atomic reindexing | ✅ | ✅ | |
| Embeddings batching | ✅ | ✅ | `embed_batch` on EmbeddingProvider trait |
| Citation support | ✅ | ❌ | |
| Memory CLI commands | ✅ | ✅ | `memory search/read/write/tree/status` CLI subcommands |
| `openclaw ltm list` | ✅ | ❌ | Real LanceDB LTM rows with `--limit`/createdAt ordering |
| Flexible path structure | ✅ | ✅ | Filesystem-like API |
| Identity files (AGENTS.md, etc.) | ✅ | ✅ | |
| Daily logs | ✅ | ✅ | |
| Heartbeat checklist | ✅ | ✅ | HEARTBEAT.md |
| Hybrid post-compaction reindex | ✅ | ❌ | `agents.defaults.compaction.postIndexSync`; `memorySearch.sync.sessions.postCompactionForce` |

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
| Control UI Dashboard | ✅ | ✅ | - | Web gateway with chat, memory, jobs, logs, extensions; modular Overview/Chat/Config/Agent/Session views, command palette, mobile bottom tabs |
| Channel status view | ✅ | 🚧 | P2 | Gateway status widget, full channel view pending |
| Agent management | ✅ | ❌ | P3 | Agent Tool Access panel with compact live-tool chips, collapsible groups, per-tool toggles |
| Model selection | ✅ | ✅ | - | TUI only |
| Config editing | ✅ | ❌ | P3 | Raw config pending-changes diff panel with redacted reveal |
| Debug/logs viewer | ✅ | ✅ | - | Real-time log streaming with level/target filters |
| WebChat interface | ✅ | ✅ | - | Web gateway chat with SSE/WebSocket |
| Canvas system (A2UI) | ✅ | ❌ | P3 | Agent-driven UI, improved asset resolution; macOS canvas hosts pushed A2UI without auto-reload |
| Control UI i18n | ✅ | ❌ | P3 | English, Chinese, Portuguese; expanded with Persian (fa), Dutch (nl), Vietnamese (vi), Italian (it), Arabic (ar), Thai (th), Traditional Chinese (zh-TW) |
| WebChat theme sync | ✅ | ❌ | P3 | Sync with system dark/light mode |
| Partial output on abort | ✅ | ❌ | P2 | Preserve partial output when aborting |
| PWA + Web Push | ✅ | ❌ | P3 | PWA install + Web Push notifications for Gateway chat |
| Talk Mode (browser realtime voice) | ✅ | ❌ | P3 | OpenAI Realtime + Google Live WebSocket; Gateway-minted ephemeral secrets; backend realtime relay |
| Steer queued messages | ✅ | ❌ | P3 | Steer action on queued messages injects follow-up into active run without retyping |
| Quick Settings dashboard | ✅ | ❌ | P3 | Refreshed grid + presets + quick-create flows + assistant avatar overrides |
| Markdown preview dialog | ✅ | ❌ | P3 | Lazy markdown preview + `@create-markdown/preview` v2 system theme |
| Cron job dashboard | ✅ | ❌ | P3 | Cron prompts/run summaries as sanitized markdown |
| Personal identity (operator) | ✅ | ❌ | P3 | Browser-local operator name + avatar through shared chat/avatar path |
| Trajectory export UI | ✅ | ❌ | P3 | Owner-private export approval flow |
| Restart-impacting Dreaming confirm | ✅ | ❌ | P3 | Restart warning before applying Dreaming mode changes |
| Mobile chat settings sheet | ✅ | ❌ | P3 | Persists mobile state through Lit-managed view-state |

### Owner: _Unassigned_

---

## 14. Automation

| Feature | OpenClaw | IronClaw | Priority | Notes |
|---------|----------|----------|----------|-------|
| Cron jobs | ✅ | ✅ | - | Routines with cron trigger; runtime state split into `jobs-state.json`; `sessionTarget: "current"`/`session:<id>` bindings |
| Per-job model fallback override | ✅ | ❌ | P2 | `payload.fallbacks` overrides agent-level fallbacks |
| Cron stagger controls | ✅ | ❌ | P3 | Default stagger for scheduled jobs |
| Cron finished-run webhook | ✅ | ❌ | P3 | Webhook on job completion |
| `--thread-id` cron CLI | ✅ | 🚧 | P2 | Telegram forum topic delivery for scheduled announcements |
| `failureAlert.includeSkipped` | ✅ | ❌ | P3 | Persistently skipped jobs alert without counting skips as exec errors |
| `delivery.threadId` (gateway cron schemas) | ✅ | ❌ | P2 | Telegram forum topics + threaded channel destinations |
| Cron `nested` lane | ✅ | ❌ | P3 | `cron.maxConcurrentRuns` applies to dedicated `cron-nested` lane; non-cron flows keep their own lane |
| Cron stuck-session timeout | ✅ | ❌ | P3 | Aborts/cleans timed-out isolated turns before recording timeout |
| Timezone support | ✅ | ✅ | - | Via cron expressions; `--at` honors local wall-clock time across DST |
| One-shot/recurring jobs | ✅ | ✅ | - | Manual + cron triggers |
| Channel health monitor | ✅ | ❌ | P2 | Auto-restart with configurable interval |
| `beforeInbound` hook | ✅ | ✅ | P2 | |
| `beforeOutbound` hook | ✅ | ✅ | P2 | |
| `beforeToolCall` hook | ✅ | ✅ | P2 | |
| `before_agent_start` hook | ✅ | ❌ | P2 | Model/provider override |
| `before_agent_finalize` hook | ✅ | ❌ | P2 | Run/message/sender/session/trace correlation |
| `before_message_write` hook | ✅ | ❌ | P2 | Pre-write interception |
| `before_dispatch` hook | ✅ | ❌ | P2 | Canonical inbound metadata; idempotency-key dedupe for hook agent deliveries |
| `before_compaction`/`after_compaction` | ✅ | ❌ | P3 | Codex-native compaction lifecycle |
| `onMessage` hook | ✅ | ✅ | - | Routines with event trigger |
| Structured system-event routines | ✅ | ✅ | P2 | `system_event` trigger + `event_emit` tool for event-driven automation |
| `onSessionStart` hook | ✅ | ✅ | P2 | |
| `onSessionEnd` hook | ✅ | ✅ | P2 | |
| `transcribeAudio` hook | ✅ | ❌ | P3 | |
| `transformResponse` hook | ✅ | ✅ | P2 | |
| `llm_input`/`llm_output` hooks | ✅ | ❌ | P3 | LLM payload inspection (Codex app-server included) |
| `model_call_started`/`ended` hooks | ✅ | ❌ | P3 | Metadata-only model/provider call telemetry |
| `cron_changed` hook | ✅ | ❌ | P3 | Typed gateway-owned cron lifecycle observer |
| Cron `jobId` hook context | ✅ | ❌ | P3 | Hook context carries originating job id |
| Bundled hooks | ✅ | ✅ | P2 | Audit + declarative rule/webhook hooks |
| Plugin hooks | ✅ | ✅ | P3 | Registered from WASM `capabilities.json` |
| Workspace hooks | ✅ | ✅ | P2 | `hooks/hooks.json` and `hooks/*.hook.json`; realpath-fail-closed |
| Outbound webhooks | ✅ | ✅ | P2 | Fire-and-forget lifecycle event delivery |
| Heartbeat system | ✅ | ✅ | - | Periodic execution; `heartbeat.skipWhenBusy` for nested lane pressure; deferred under cron load |
| Gmail pub/sub | ✅ | ❌ | P3 | |
| Inferred follow-up commitments | ✅ | ❌ | P3 | Heartbeat-delivered reminders; opt-in batched extraction |

### Owner: _Unassigned_

---

## 15. Security Features

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Gateway token auth | ✅ | ✅ | Bearer token auth on web gateway; per-request resolution for `secrets.reload`; method-specific least-privilege scopes for CLI Gateway calls |
| Device pairing | ✅ | ❌ | Single-use bootstrap setup codes; metadata-upgrade auto-approval for shared-secret loopback; scope/role/metadata pairing approval flows |
| Tailscale identity | ✅ | ❌ | Tailscale-authenticated Control UI bypass for browser device identity |
| Trusted-proxy auth | ✅ | ❌ | Header-based reverse proxy auth; `trustedProxy.allowLoopback` |
| OAuth flows | ✅ | 🚧 | NEAR AI OAuth + Gemini OAuth (PKCE, S256) + hosted extension/MCP OAuth broker; external auth-proxy rollout still pending; OpenClaw added bootstrap-token redemption scope allowlist |
| DM pairing verification | ✅ | ✅ | ironclaw pairing approve, host APIs |
| Allowlist/blocklist | ✅ | 🚧 | allow_from + pairing store; canonical `dmPolicy="open"` only with effective wildcard across all channels |
| Per-group tool policies | ✅ | ❌ | Group-id validation against session/spawned context before applying group-scoped tool policies |
| Exec approvals | ✅ | ✅ | TUI overlay; `allow-once` idempotent grace; PATH-resolved basenames; secret redaction in approval prompts; Unicode normalization + zero-width stripping |
| Owner allowlists | ✅ | ❌ | `commands.ownerAllowFrom` bootstrapped from first approved DM pairing; channel-prefixed entries scoped to matching providers |
| TLS 1.3 minimum | ✅ | ✅ | reqwest rustls |
| SSRF protection | ✅ | ✅ | WASM allowlist; OpenClaw extended SSRF guard to BlueBubbles, Synology Chat, LINE, QQBot direct-upload, Tlon uploads, browser tabs/snapshots, voice-call Twilio webhooks, web fetch (incl. `fc00::/7` opt-in) |
| SSRF IPv6 transition bypass block | ✅ | ❌ | Block IPv4-mapped IPv6 bypasses |
| Cron webhook SSRF guard | ✅ | ❌ | SSRF checks on webhook delivery |
| Loopback-first | ✅ | 🚧 | HTTP binds 0.0.0.0 |
| Docker sandbox | ✅ | ✅ | Orchestrator/worker containers; opt-in `sandbox.docker.gpus` passthrough |
| Kubernetes runtime | ❌ | 🚧 | Stage 2 project-backed runtime with job lifecycle, bootstrap-delivered project content, and explicit workspace-change apply flow; see [sandboxed tools docs](docs/capabilities/sandboxed-tools.mdx) |
| Podman support | ✅ | ❌ | `--container` accepts both Docker + Podman |
| WASM sandbox | ❌ | ✅ | IronClaw innovation |
| Sandbox env sanitization | ✅ | 🚧 | Shell tool scrubs env vars (secret detection); docker container env sanitization partial |
| `OPENCLAW_*` env block | ✅ | ❌ | Untrusted workspace `.env` cannot inject OpenClaw runtime-control vars |
| Workspace `.env` injection blocks | ✅ | ❌ | Block `CLOUDSDK_PYTHON`, ambient Homebrew, Windows system PATH vars, `MINIMAX_API_HOST`, `npm_execpath` |
| Tool policies | ✅ | ✅ | |
| Elevated mode | ✅ | ❌ | |
| Safe bins allowlist | ✅ | ❌ | Hardened path trust; non-user-writable absolute helpers for CLI/ffmpeg/OpenSSL |
| LD*/DYLD* validation | ✅ | ❌ | Block Mercurial/Rust/Make env redirects in host exec sanitization |
| Path traversal prevention | ✅ | ✅ | Including config includes (OC-06) + workspace-only tool mounts; `realpath`-via-fd safety on agents.files.get/set |
| Credential theft via env injection | ✅ | 🚧 | Shell env scrubbing + command injection detection; no full OC-09 defense |
| Session file permissions (0o600) | ✅ | ✅ | Session token file set to 0o600 in llm/session.rs |
| Skill download path restriction | ✅ | ❌ | Validated download roots prevent arbitrary write targets |
| Skill installer metadata validation | ✅ | ❌ | Strict per-PM regex allowlists; URL protocol allowlist; sanitize metadata for terminal output |
| Webhook signature verification | ✅ | ✅ | Padded timing-safe compare even on wrong-length signatures (Nextcloud Talk, Feishu, LINE, Zalo) |
| Media URL validation | ✅ | ❌ | Reject non-HTTP(S) inbound attachment URLs; reject remote-host `file://` URLs in webchat embedding path |
| Prompt injection defense | ✅ | ✅ | Pattern detection, sanitization; OpenClaw added chat-template special-token stripping (Qwen/ChatML, Llama, Gemma, Mistral, Phi, GPT-OSS) |
| Internal scaffolding stripping | ✅ | ❌ | `<system-reminder>`/`<previous_response>` stripped at final delivery boundary |
| Leak detection | ✅ | ✅ | Secret exfiltration |
| Dangerous tool re-enable warning | ✅ | ❌ | Warn when gateway.tools.allow re-enables HTTP tools |
| OpenGrep static analysis | ✅ | ❌ | Bundled rulepack + source-rule compiler + provenance check; PR/full scan workflows + SARIF upload to GitHub Code Scanning |
| Logging redaction expansion | ✅ | ❌ | Tencent/Alibaba/HuggingFace/Replicate API keys; payment credential field names; `sk-*`/Bearer/Authorization tokens at console + file sinks |
| Trace context propagation | ✅ | ❌ | W3C `traceparent` from trusted model-call context; replaces caller-supplied values |
| Forwarded-header IP detection | ✅ | ❌ | Treat any `Forwarded`/`X-Forwarded-*`/`X-Real-IP` as proxied before pairing locality checks |
| Trusted-content sanitization | ✅ | ❌ | Group/channel names rendered through fenced untrusted-metadata JSON; vCard/contact/location free-text neutralization |
| Per-tool MCP loopback policy | ✅ | ❌ | Owner-only tool visibility derived from authenticated owner-vs-non-owner bearers; no caller-controlled owner header |
| Mobile pairing TLS requirement | ✅ | ❌ | Plaintext `ws://` only on loopback; `OPENCLAW_ALLOW_INSECURE_PRIVATE_WS` for trusted private nets |
| Webhook auth rate-limit | ✅ | ❌ | Pre-auth `429` for bad webhook secrets (Zalo, etc.) |

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
- 🚧 Kubernetes runtime (Stage 2 project-backed runtime with explicit workspace-change apply flow; see [sandboxed tools docs](docs/capabilities/sandboxed-tools.mdx))
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

- ❌ Slack channel (real implementation)
- ✅ Telegram channel (WASM, polling-first setup, DM pairing, caption, /start)
- ❌ WhatsApp channel
- ✅ Multi-provider failover (`FailoverProvider` with retryable error classification)
- ✅ Hooks system (core lifecycle hooks + bundled/plugin/workspace hooks + outbound webhooks)

### P2 - Medium Priority

- ❌ Media handling (images, PDFs)
- ✅ Ollama/local model support (via rig::providers::ollama)
- ❌ Configuration hot-reload
- ✅ Tool-driven webhook ingress (`/webhook/tools/{tool}` -> host-verified + tool-normalized `system_event` routines)
- ❌ Channel health monitor with auto-restart
- ❌ Partial output preservation on abort

### P3 - Lower Priority

- ❌ Discord channel
- ❌ Matrix channel
- ❌ Other messaging platforms (Yuanbao, WeCom, Google Meet, Voice Call)
- ❌ TTS/audio features (12+ providers added in OpenClaw; see Section 6 TTS/STT subsection)
- ❌ Video support (OpenRouter/MiniMax/Veo/fal/Sora)
- 🚧 Skills routing blocks (activation criteria exist, but no "Use when / Don't use when")
- ❌ Plugin registry / persisted plugin index / `git:` installs
- ❌ Streaming (block/tool/Z.AI tool_stream)
- ❌ Memory: temporal decay, MMR re-ranking, query expansion, multimodal indexing, people-aware wiki
- ❌ Control UI i18n (now 12+ locales upstream)
- ❌ Stuck loop detection
- ❌ Codex native app-server runtime + Computer Use
- ❌ Talk Mode / realtime voice (browser + backend)
- ❌ OpenTelemetry diagnostics + Prometheus exporter
- ❌ Active Memory + Skill Workshop + Trajectory export
- ❌ Outbound proxy routing + `proxy validate`
- ❌ `migrate` (Claude/Hermes import)

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
