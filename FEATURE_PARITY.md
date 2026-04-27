# IronClaw ↔ OpenClaw Feature Parity Matrix

This document tracks feature parity between IronClaw (Rust implementation) and OpenClaw (TypeScript reference implementation). Use this to coordinate work across developers.

**Legend:**

- ✅ Implemented
- 🚧 Partial (in progress or incomplete)
- ❌ Not implemented
- 🔮 Planned (in scope but not started)
- 🚫 Out of scope (intentionally skipped)
- ➖ N/A (not applicable to Rust implementation)

**Last reviewed against OpenClaw PRs:** 2026-03-10 (merged 2026-02-24 through 2026-03-10)

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
| `doctor` diagnostics | ✅ | 🚧 | 16 checks: settings, LLM, DB, embeddings, routines, gateway, MCP, skills, secrets, service, Docker daemon, tunnel binaries |
| Agent event broadcast | ✅ | 🚧 | SSE broadcast manager exists (SseManager) but tool/job-state events not fully wired |
| Channel health monitor | ✅ | ❌ | Auto-restart with configurable interval |
| Presence system | ✅ | ❌ | Beacons on connect, system presence for agents |
| Trusted-proxy auth mode | ✅ | ❌ | Header-based auth for reverse proxies |
| APNs push pipeline | ✅ | ❌ | Wake disconnected iOS nodes via push |
| Oversized payload guard | ✅ | 🚧 | HTTP webhook has 64KB body limit + Content-Length check; no chat.history cap |
| Pre-prompt context diagnostics | ✅ | 🚧 | Token breakdown logged before LLM call (conversational dispatcher path); other LLM entry points not yet covered |

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
| WebChat | ✅ | ✅ | - | Web gateway chat |
| Matrix | ✅ | ❌ | P3 | E2EE support |
| Mattermost | ✅ | ❌ | P3 | Emoji reactions, interactive buttons, model picker |
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
| DM topics support | ✅ | ❌ | Agent/topic bindings in DMs and agent-scoped SessionKeys |
| Persistent ACP topic binding | ✅ | ❌ | ACP harness sessions can pin to Telegram forum or DM topics |
| sendVoice (voice note replies) | ✅ | ✅ | audio/ogg attachments sent as voice notes; prerequisite for TTS (#90) |

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
| Thread ownership | ✅ | 🚧 | Reply participation memory now persists with TTL-bounded tracking; full thread-level ownership tracking is still missing |
| Download-file action | ✅ | ❌ | On-demand attachment downloads via message actions |

### Mattermost-Specific Features (since Mar 2026)

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Interactive buttons | ✅ | ❌ | Clickable message buttons with signed callback flow |
| Interactive model picker | ✅ | ❌ | In-channel provider/model chooser |

### Feishu/Lark-Specific Features (since Mar 2026)

| Feature | OpenClaw | IronClaw | Notes |
|---------|----------|----------|-------|
| Doc/table actions | ✅ | ❌ | `feishu_doc` supports tables, positional insert, color_text, image upload, and file upload |
| Rich-text embedded media extraction | ✅ | ❌ | Pull video/media attachments from post messages |

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
| `traces` | ➖ | 🚧 | - | IronClaw-native Trace Commons MVP: opt-in local redaction, autonomous submission with scoped policy preflight, private ingest/review/export/credit/admin helpers, optional env or DB-backed ingest tenant policies for allowed consent scopes and trace-card uses with a require-policy production gate plus admin-token policy management and safe tenant-policy audit metadata, scoped export/retention/vector/benchmark worker token roles with dedicated benchmark-convert, retention-maintenance, vector-index, and utility-credit worker routes plus CLI wrappers, optional fail-closed export guardrails for explicit accepted/low-risk/consent-scoped replay, benchmark, and ranker-training corpus slices with caller-supplied export purposes plus CLI ranker `--purpose` support, tenant-policy and signed-claim allowed-use ABAC for replay/benchmark/ranker export requests, downstream source selection, process-evaluation workers, and utility-credit jobs, deployment-configurable per-request export item caps and contributor-only hourly tenant/principal submission quotas, hardened local Privacy Filter sidecar guardrails, governed delayed-credit mutation with DB-backed reviewer metadata reads and typed DB audit metadata, pending-vs-settled credit separation, idempotent benchmark/ranker candidate/ranker pair utility credit events with DB-preserved ranking utility types and terminal-trace ledger exclusion from contributor totals, and a trusted offline utility-credit worker route that appends accepted-trace regression/training/ranking utility credit idempotently without exposing reviewer_bonus or abuse_penalty, hash-chained file-backed audit rows with maintenance verification plus DB mirror hash fields, canonical audit payloads, payload hash recomputation, DB-native audit append sequencing with stale predecessor rejection, and projection-drift verification, local credit/status summaries plus authenticated web held-queue and richer credit/review activity with aggregate read-audit breadcrumbs, optional DB dual-write/backfill plus isolated per-item backfill failure reporting and a production fail-closed required DB mirror write cutover for critical submission/revocation/review/credit/export/audit/content-read rows with contributor, tenant policy, reviewer metadata, replay selection, file-side credit/audit/replay-manifest backfill repair, policy-gated DB object-ref-backed replay envelope reads with tenant/object-ref/hash verification and an object-ref-required production gate, DB-primary review decision metadata and object-ref-backed envelope reads with required non-empty decision reasons, no missing-metadata file resurrection, reviewed-envelope `review_snapshot` object refs, an object-ref-required production gate, and a dedicated benchmark/ranker source object-ref production gate before derived export publication, compact export-manifest metadata/invalidation/listing plus CLI listing, purpose-indexed reviewer trace listing, replay export item snapshots/invalidation with source object refs, audited schema-versioned benchmark conversion lifecycle metadata plus source-invalidation lifecycle revocation and a CLI lifecycle-update helper, benchmark/ranker provenance invalidation with derived, active vector, and file/service-local artifact object refs, audit reads/mirroring with reviewer queue/audit-log reads, tenant policy admin reads/writes with CLI get/set helpers, safe admin config-status reads for cutover and object-store flags, and retention/purge/revocation-tombstone invalidation reads, source-hashed exports with just-before-publish DB source revalidation, first-writer-wins hash-bearing file/DB revocation tombstones that block same-tenant re-ingest by submission id, redaction hash, or canonical-summary hash and DB-only revocation scoped to owner/reviewer principals, and per-trace replay/review plus per-source benchmark/ranker/vector content-read events including `object_ref_id` for DB object-ref reads and vector indexing, maintenance DB reconciliation with compact `blocking_gaps` and optional fail-closed clean promotion gating, derived presence/status/hash/invalid-source diagnostics, credit-ledger and canonical audit-event ID gap diagnostics, split export-manifest kind diagnostics, export item object-ref and invalid-source diagnostics, separate object-ref presence/readability/hash-mismatch diagnostics, vector index gap diagnostics, and reader-projection parity for contributor credit, reviewer metadata, analytics, audit, and export manifests, maintenance repair for already file-marked DB revocations, retention expiry maintenance with expired-source export cache pruning and explicit-purpose expired-artifact purge, vector-entry metadata/indexing with deterministic redacted-summary nearest-neighbor scoring plus encrypted redacted vector payload worker-intermediate object refs, encrypted artifact sidecar plus service-owned local encrypted object-store mode with object-primary submit/review envelope body storage, object-primary replay export reads, and object-primary benchmark/ranker derived export artifact/provenance storage, initial PostgreSQL RLS policies with transaction-local tenant context, local autonomous `queue-status` diagnostics plus authenticated web queue-status and credit-notice reads, periodic delayed credit notices with ledger deltas and credit-event counts, EdDSA/Ed25519 signed upload-claim verification through default/keyed public-key config and JSON/file keysets with activation windows plus safe EdDSA key counts, with managed issuer distribution/governance still remaining. Not an OpenClaw parity feature. |
| `update` | ✅ | ❌ | P3 | Self-update |
| `completion` | ✅ | ✅ | - | Shell completion |
| `/subagents spawn` | ✅ | ❌ | P3 | Spawn subagents from chat |
| `/export-session` | ✅ | ❌ | P3 | Export current session transcript |

Trace Commons incremental note: reviewer quarantine and active-learning queues now surface prioritization metadata, including `review_age_hours`, `review_escalation_state`, and `review_escalation_reasons`, so CLI non-JSON output can show SLA pressure and escalation causes during triage. DB-backed review leases now let reviewer/admin principals claim or release tenant-scoped quarantined traces, expose lease assignment metadata in review queues, support `all`, `mine`, `available`, `active`, and `expired` lease filters in API/CLI/web operator queues, block other reviewers from finalizing while a lease is active, and mirror claim/release rows with typed safe audit metadata instead of empty placeholders. Analytics can now suppress aggregate cells below a configured minimum count while reporting the suppression threshold and number of hidden buckets. Tenant token entries can now carry optional RFC3339 `expires_at`/`expires` attributes, and the ingest service can accept optional HS256 signed tenant claims that bind tenant id, actor principal, role, issuer/audience when configured, allowed consent scopes/uses, and expiry without enumerating every bearer token; claim allow-lists now constrain submission, replay exports, benchmark/ranker dataset generation, process-evaluation workers, and utility-credit jobs. Operator docs now pin production asymmetric upload-claim governance to managed issuer/key rotation with EdDSA/Ed25519, leaving static tokens and HS256 claims as internal bridge paths. Keyed signed-token secrets and EdDSA public-key files support `kid`-selected rotation, deployments can cap signed-claim lifetimes by requiring `iat` and bounding `exp - iat`, require JWT IDs before accepting signed claims, emergency-denylist signed-claim JWT IDs by `jti`, and config status exposes only key/EdDSA-key/denylist/max-TTL/JTI-policy counts while submitted audit rows record only the safe auth method plus hashed principal. Retention maintenance also honors `TRACE_COMMONS_LEGAL_HOLD_RETENTION_POLICIES` so configured policy classes are skipped for new expiration and purge passes, and DB-backed maintenance runs now write durable retention job/item ledger rows for resumable expire/purge/revoke bookkeeping with admin-only API plus CLI and web-operator reads for tenant-scoped jobs and per-submission lifecycle items. Maintenance DB reconciliation now runs after the retention ledger write and reports DB retention job/item counts plus current-run retention job or item-count gaps as promotion blockers. Process-evaluation workers have a CLI submit helper for `POST /v1/workers/process-evaluation`, store bounded rubric metadata under the `process_evaluation` worker kind, mirror typed hash/count-only audit metadata, can optionally append idempotent `training_utility` delayed credit for the evaluated accepted submission using an external reference, preserve separate DB derived rows per evaluator version while feeding content-free process-evaluation analytics by label, rating, and score band without double-counting DB-backed submissions, and now require tenant policy or signed-claim evaluation-use ABAC before reading or labeling accepted trace bodies. Utility-credit workers now also require the source trace plus tenant policy or signed claim to allow the requested regression/evaluation, model-training, or ranking-training utility use before appending delayed credit. Object-primary envelope writes now use unique encrypted artifact object ids per logical snapshot so review/process-evaluation writes do not overwrite ciphertext behind older submitted-envelope object refs, terminal-trace status sync can explain retained-but-excluded delayed ledger rows without exposing them through contributor credit-event reads, web enqueue/submit and CLI queue writes reject crafted requests/envelopes that try to include message text or tool payloads disallowed by the standing policy, DB stores now reject derived rows, vector entries, and export manifest items whose object, derived, or vector refs do not belong to the same tenant/submission, periodic local credit notices now include delayed ledger deltas plus credit-event counts, CLI status sync resets credit notices when delayed-credit explanations change even without a numeric delta, and autonomous runtime capture skips ineligible current traces instead of leaving held queue files while preserving queue flush/credit notices.

This push also adds local autonomous queue diagnostics/status surfaces: CLI `traces queue-status` reports readiness, bearer-token environment presence, queue/held counts, retry/manual-review/policy hold counts, next retry time, durable flush/status-sync telemetry, retryable submission failure counters, sanitized held-reason counts, and local credit summaries; authenticated web `/api/traces/queue-status` reports scoped queue/held diagnostics plus the same durable telemetry, and `/api/traces/credit-notice` marks due notices. Due credit notices now carry local acknowledge/snooze state: CLI `traces credit --notice --ack` and authenticated `POST /api/traces/credit-notice` acknowledgement suppress the current credit fingerprint until credit changes, while `--snooze-hours` and the matching web action suppress it until a bounded deadline without exposing trace bodies or explanation text in the fingerprint. The agent loop now runs a periodic Trace Commons queue worker for opted-in owner/active-user scopes, stores retryable submission failures as typed redacted sidecars with capped backoff, skips retry-held envelopes until due, records durable scoped telemetry for queue/status-sync attempts, and broadcasts returned credit notices. EdDSA/Ed25519 public-key verification is available through default or `kid`-selected key config and JSON/file keysets with optional activation windows, with safe total/EdDSA config-status counts; managed issuer distribution/governance remains planned work.

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
| Compaction model override | ✅ | ❌ | Use a dedicated provider/model for summarization only |
| Post-compaction read audit | ✅ | ❌ | Layer 3: workspace rules appended to summaries |
| Post-compaction context injection | ✅ | ❌ | Workspace context as system event |
| Custom system prompts | ✅ | ✅ | Template variables, safety guardrails |
| Skills (modular capabilities) | ✅ | ✅ | Prompt-based skills with trust gating, attenuation, activation criteria, catalog, selector |
| Skill routing blocks | ✅ | 🚧 | ActivationCriteria (keywords, patterns, tags) but no "Use when / Don't use when" blocks |
| Skill path compaction | ✅ | ❌ | ~ prefix to reduce prompt tokens |
| Thinking modes (off/minimal/low/medium/high/xhigh/adaptive) | ✅ | 🚧 | thinkingConfig for Gemini models (thinkingBudget/thinkingLevel); no per-level control yet |
| Per-model thinkingDefault override | ✅ | ❌ | Override thinking level per model; Anthropic Claude 4.6 defaults to adaptive |
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
| Anthropic (Claude) | ✅ | 🚧 | - | Via NEAR AI proxy; Opus 4.5, Sonnet 4, Sonnet 4.6, adaptive thinking default |
| OpenAI | ✅ | 🚧 | - | Via NEAR AI proxy; GPT-5.4 + Codex OAuth |
| AWS Bedrock | ✅ | ✅ | - | Native Converse API via aws-sdk-bedrockruntime (requires `--features bedrock`) |
| Google Gemini | ✅ | ✅ | - | OAuth (PKCE + S256), function calling, thinkingConfig, generationConfig |
| io.net | ✅ | ✅ | P3 | Via `ionet` adapter |
| Mistral | ✅ | ✅ | P3 | Via `mistral` adapter |
| Yandex AI Studio | ✅ | ✅ | P3 | Via `yandex` adapter |
| Cloudflare Workers AI | ✅ | ✅ | P3 | Via `cloudflare` adapter |
| NVIDIA API | ✅ | ✅ | P3 | Via `nvidia` adapter and `providers.json` |
| OpenRouter | ✅ | ✅ | - | Via OpenAI-compatible provider (RigAdapter) |
| Tinfoil | ❌ | ✅ | - | Private inference provider (IronClaw-only) |
| OpenAI-compatible | ❌ | ✅ | - | Generic OpenAI-compatible endpoint (RigAdapter) |
| GitHub Copilot | ✅ | ✅ | - | Dedicated provider with OAuth token exchange (`GithubCopilotProvider`) |
| Ollama (local) | ✅ | ✅ | - | via `rig::providers::ollama` (full support) |
| Perplexity | ✅ | ❌ | P3 | Freshness parameter for web_search |
| MiniMax | ✅ | ❌ | P3 | Regional endpoint selection |
| GLM-5 | ✅ | ✅ | P3 | Via Z.AI provider (`zai`) using OpenAI-compatible chat completions |
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
| 1M context support | ✅ | ❌ | Anthropic extended context beta + OpenAI Codex GPT-5.4 1M context |

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
| PDF analysis tool | ✅ | ❌ | P2 | Native Anthropic/Gemini path with text/image extraction fallback |
| PDF parsing | ✅ | ❌ | P2 | `pdfjs-dist` fallback path |
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
| Memory plugins | ✅ | ❌ | Custom backends + selectable memory slot |
| Context-engine plugins | ✅ | ❌ | Custom context management + subagent/context hooks |
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
| Config validation/schema | ✅ | ✅ | Type-safe Config struct + `openclaw config validate` |
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
| Bedrock embeddings | ❌ | ✅ | Reuses Bedrock region/profile auth for Titan Text Embeddings V2 |
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
| Per-job model fallback override | ✅ | ❌ | P2 | `payload.fallbacks` overrides agent-level fallbacks |
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
| Structured system-event routines | ✅ | ✅ | P2 | `system_event` trigger + `event_emit` tool for event-driven automation |
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
| OAuth flows | ✅ | 🚧 | NEAR AI OAuth + Gemini OAuth (PKCE, S256) + hosted extension/MCP OAuth broker; external auth-proxy rollout still pending |
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
| Path traversal prevention | ✅ | ✅ | Including config includes (OC-06) + workspace-only tool mounts |
| Credential theft via env injection | ✅ | 🚧 | Shell env scrubbing + command injection detection; no full OC-09 defense |
| Session file permissions (0o600) | ✅ | ✅ | Session token file set to 0o600 in llm/session.rs |
| Skill download path restriction | ✅ | ❌ | Validated download roots prevent arbitrary write targets |
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
